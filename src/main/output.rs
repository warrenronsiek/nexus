// @feature runtime
// @spec docs/features/runtime.md
use super::*;

pub(super) async fn request_and_print(
    loaded: &nexus::LoadedConfig,
    request: ServiceRequest,
) -> Result<()> {
    let result = NexusClient::new(loaded, None)
        .request(&request)
        .await
        .context("Nexus is not reachable")?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
