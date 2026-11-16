// @feature installation
// @feature runtime
// @spec docs/features/installation.md
// @spec docs/features/runtime.md
// @boundary codex-control: bounded Unix WebSocket JSON-RPC and hook-file JSON stay inside this adapter; only exact setup-generated user hooks are eligible for trust updates.
use anyhow::{ensure, Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tokio_tungstenite::{
    tungstenite::{protocol::WebSocketConfig, Message},
    WebSocketStream,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Hook {
    key: String,
    source_path: PathBuf,
    current_hash: String,
    trust_status: String,
}
#[derive(Deserialize)]
struct Hooks {
    data: Vec<HookEntry>,
}
#[derive(Deserialize)]
struct HookEntry {
    hooks: Vec<Hook>,
    errors: Vec<Value>,
}

pub(super) fn running_home() -> Option<PathBuf> {
    let home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))?;
    home.join("app-server-control/app-server-control.sock")
        .exists()
        .then_some(home)
}

pub(super) async fn refresh(
    home: Option<&Path>,
    executable: &Path,
    explicit: Option<&Path>,
) -> Result<Option<String>> {
    let Some(home) = home else {
        return Ok(None);
    };
    let retained = super::install::versioned_executable(executable)?;
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        reload(
            &home.join("app-server-control/app-server-control.sock"),
            home,
            &retained,
            explicit,
        ),
    )
    .await
    .context("Codex companion reload timed out")??;
    Ok(Some("Reloaded Nexus MCP configuration through the running Codex daemon; other hosts must reconnect their companions.".into()))
}

async fn reload(
    socket: &Path,
    home: &Path,
    executable: &Path,
    explicit: Option<&Path>,
) -> Result<()> {
    let source = home.join("hooks.json");
    let installed: Value = serde_json::from_slice(&std::fs::read(&source)?)?;
    let generated = crate::agents::integration::hook_configuration(
        crate::agents::integration::Host::Codex,
        executable,
        explicit,
    );
    let definitions = HookDefinitions {
        source,
        installed,
        generated,
    };
    let mut rpc = Rpc::connect(socket).await?;
    let result = rpc.refresh_hooks(&definitions).await;
    let _ = rpc.socket.close(None).await;
    result
}

struct HookDefinitions {
    source: PathBuf,
    installed: Value,
    generated: Value,
}

fn matches_generated(hook: &Hook, source: &Path, installed: &Value, generated: &Value) -> bool {
    if hook.source_path != source {
        return false;
    }
    let Some(key) = hook.key.strip_prefix(&format!("{}:", source.display())) else {
        return false;
    };
    let parts: Vec<_> = key.split(':').collect();
    let [event, group, handler] = parts.as_slice() else {
        return false;
    };
    let (Ok(group), Ok(handler)) = (group.parse::<usize>(), handler.parse::<usize>()) else {
        return false;
    };
    generated["hooks"].as_object().is_some_and(|events| {
        events.iter().any(|(name, expected)| {
            if name.to_lowercase() != event.replace('_', "") {
                return false;
            }
            let actual = &installed["hooks"][name][group];
            expected.as_array().is_some_and(|groups| {
                groups.iter().any(|expected_group| {
                    actual["matcher"] == expected_group["matcher"]
                        && expected_group["hooks"]
                            .as_array()
                            .is_some_and(|handlers| handlers.contains(&actual["hooks"][handler]))
                })
            })
        })
    })
}

struct Rpc {
    socket: WebSocketStream<tokio::net::UnixStream>,
    id: u64,
}
impl Rpc {
    async fn connect(path: &Path) -> Result<Self> {
        let stream = tokio::net::UnixStream::connect(path)
            .await
            .context("connect to running Codex control socket")?;
        let limits = WebSocketConfig::default()
            .max_message_size(Some(1024 * 1024))
            .max_frame_size(Some(1024 * 1024));
        let (socket, _) =
            tokio_tungstenite::client_async_with_config("ws://localhost/", stream, Some(limits))
                .await?;
        let mut rpc = Self { socket, id: 0 };
        rpc.call("initialize", json!({"clientInfo":{"name":"nexus-update","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}})).await?;
        rpc.send(json!({"method":"initialized"})).await?;
        Ok(rpc)
    }
    async fn hooks(&mut self) -> Result<Hooks> {
        let params = json!({"cwds":[std::env::current_dir()?]});
        Ok(serde_json::from_value(
            self.call("hooks/list", params).await?,
        )?)
    }
    async fn refresh_hooks(&mut self, definitions: &HookDefinitions) -> Result<()> {
        let hooks = self.hooks().await?;
        ensure!(
            hooks.data.iter().all(|entry| entry.errors.is_empty()),
            "Codex hook discovery reported errors"
        );
        for hook in hooks
            .data
            .iter()
            .flat_map(|entry| &entry.hooks)
            .filter(|hook| {
                hook.trust_status != "trusted"
                    && matches_generated(
                        hook,
                        &definitions.source,
                        &definitions.installed,
                        &definitions.generated,
                    )
            })
        {
            self.call("config/value/write", json!({"keyPath":format!("hooks.state.{}.trusted_hash",serde_json::to_string(&hook.key)?),"value":hook.current_hash,"mergeStrategy":"replace"})).await?;
        }
        self.call("config/mcpServer/reload", Value::Null)
            .await
            .map(|_| ())
    }

    async fn send(&mut self, request: Value) -> Result<()> {
        self.socket
            .send(Message::Text(serde_json::to_string(&request)?.into()))
            .await?;
        Ok(())
    }
    async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.id += 1;
        self.send(json!({"id":self.id,"method":method,"params":params}))
            .await?;
        while let Some(message) = self.socket.next().await {
            let message = message?;
            if !message.is_text() {
                continue;
            }
            let response: Value = serde_json::from_str(message.to_text()?)?;
            if response["id"] == self.id {
                ensure!(
                    response.get("error").is_none(),
                    "Codex control rejected {method}"
                );
                return response
                    .get("result")
                    .cloned()
                    .context("Codex response has no result");
            }
        }
        anyhow::bail!("Codex control socket closed before responding")
    }
}

#[cfg(test)]
mod tests;
