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

pub(super) fn print_integration(
    host: IntegrationHost,
    explicit_config: Option<&Path>,
) -> Result<()> {
    let host = match host {
        IntegrationHost::Codex => Host::Codex,
        IntegrationHost::Claude => Host::Claude,
    };
    let executable = std::env::current_exe()?;
    println!(
        "{}",
        serde_json::to_string_pretty(&integration::instructions(
            host,
            &executable,
            explicit_config,
        ))?
    );
    Ok(())
}
