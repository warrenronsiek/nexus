// @feature installation
// @spec docs/features/installation.md
// @boundary companion-fixtures: isolated Unix WebSocket server exercises the actual daemon framing without contacting a running host.
use super::*;

#[tokio::test]
async fn trusts_only_matching_generated_hooks_before_reloading_companions() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("codex home");
    std::fs::create_dir(&home).unwrap();
    let executable = directory.path().join("retained nexus");
    let generated = crate::agents::integration::hook_configuration(
        crate::agents::integration::Host::Codex,
        &executable,
        None,
    );
    let source = home.join("hooks.json");
    std::fs::write(&source, serde_json::to_vec(&generated).unwrap()).unwrap();
    let socket = directory.path().join("daemon.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(serve(listener, source));
    reload(&socket, &home, &executable, None).await.unwrap();
    let calls = server.await.unwrap();
    assert_eq!(calls.last().unwrap()["method"], "config/mcpServer/reload");
    assert_eq!(
        calls
            .iter()
            .filter(|r| r["method"] == "config/value/write")
            .count(),
        1
    );
    assert!(!calls.iter().any(|r| r["method"]
        .as_str()
        .is_some_and(|m| m.contains("thread/") || m.contains("shutdown"))));
}

async fn serve(listener: tokio::net::UnixListener, source: PathBuf) -> Vec<Value> {
    use futures_util::{SinkExt, StreamExt};
    let (stream, _) = listener.accept().await.unwrap();
    let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
    let mut calls = Vec::new();
    let mut trusted = false;
    while let Some(Ok(message)) = socket.next().await {
        if !message.is_text() {
            break;
        }
        let request: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        calls.push(request.clone());
        if request.get("id").is_none() {
            continue;
        }
        let result = match request["method"].as_str().unwrap() {
            "hooks/list" => json!({"data":[{"errors":[],"hooks":[
                {"key":format!("{}:session_start:0:0",source.display()),"sourcePath":source,"currentHash":format!("sha256:{}","a".repeat(64)),"trustStatus":if trusted {"trusted"} else {"modified"}},
                {"key":"unrelated","sourcePath":source,"currentHash":format!("sha256:{}","b".repeat(64)),"trustStatus":"untrusted"}
            ]}]}),
            "config/value/write" => {
                assert_eq!(
                    request["params"]["keyPath"],
                    format!(
                        "hooks.state.{}.trusted_hash",
                        serde_json::to_string(&format!("{}:session_start:0:0", source.display()))
                            .unwrap()
                    )
                );
                trusted = true;
                json!({})
            }
            _ => json!({}),
        };
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                json!({"id":request["id"],"result":result})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        if request["method"] == "config/mcpServer/reload" {
            break;
        }
    }
    calls
}

#[tokio::test]
#[ignore = "requires an existing local Codex daemon; reads metadata only"]
async fn running_codex_daemon_accepts_control_handshake_and_hook_discovery() {
    let home = running_home().expect("running Codex control socket");
    let path = home.join("app-server-control/app-server-control.sock");
    let mut rpc = tokio::time::timeout(std::time::Duration::from_secs(5), Rpc::connect(&path))
        .await
        .unwrap()
        .unwrap();
    let hooks = rpc.hooks().await.unwrap();
    assert!(hooks.data.iter().all(|entry| entry.errors.is_empty()));
    rpc.socket.close(None).await.unwrap();
}
