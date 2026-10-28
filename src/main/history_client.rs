// @feature runtime
// @feature work-history
// @feature code-concerns
// @spec docs/features/runtime.md
// @spec docs/features/work-history.md
// @spec docs/features/code-concerns.md
// @boundary typed-history-response: the command-specific adapter owns payloads; this client binds and checks transport
use anyhow::{bail, Result};
use nexus::coordination::{
    api::{ContextCommand, ServiceRequest},
    domain::HookContext,
};
use nexus::runtime::client::NexusClient;
use nexus::LoadedConfig;
use serde_json::Value;
use std::path::Path;

pub(crate) struct HistoryClient<Action> {
    client: NexusClient,
    context: HookContext,
    request: fn(ContextCommand<Action>) -> ServiceRequest,
    label: &'static str,
}
impl<Action> HistoryClient<Action> {
    pub(crate) fn new(
        loaded: &LoadedConfig,
        explicit: Option<&Path>,
        context: HookContext,
        request: fn(ContextCommand<Action>) -> ServiceRequest,
        label: &'static str,
    ) -> Self {
        Self {
            client: NexusClient::new(loaded, explicit),
            context,
            request,
            label,
        }
    }
    pub(crate) async fn send(&self, command: Action) -> Result<Value> {
        let response = self
            .client
            .ensure_and_request(&(self.request)(ContextCommand {
                context: self.context.clone(),
                command,
            }))
            .await?;
        if response.get("ok").and_then(Value::as_bool) != Some(true) {
            bail!(
                "{} request failed: {}",
                self.label,
                response
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("invalid response")
            );
        }
        Ok(response)
    }
}
