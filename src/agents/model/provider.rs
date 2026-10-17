// @feature analyst
// @feature commit-review
// @feature agent-memory
// @spec docs/features/analyst.md
// @spec docs/features/commit-review.md
// @spec docs/features/agent-memory.md
// @boundary provider-environment
use crate::config::{ModelConfig, ModelProvider};
use std::ffi::{OsStr, OsString};
use std::os::unix::process::CommandExt;
use std::process::Command;

const COMMON_PROVIDER_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LANG",
    "LC_ALL",
    "TERM",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "XDG_DATA_HOME",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "CURL_CA_BUNDLE",
    "REQUESTS_CA_BUNDLE",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
];
const CODEX_ENV_ALLOWLIST: &[&str] = &[
    "CODEX_HOME",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "OPENAI_ORG_ID",
    "OPENAI_PROJECT_ID",
    "AZURE_OPENAI_API_KEY",
    "AZURE_OPENAI_ENDPOINT",
    "OPENAI_API_VERSION",
];
const CLAUDE_ENV_ALLOWLIST: &[&str] = &[
    "CLAUDE_CONFIG_DIR",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
];
const CLAUDE_BEDROCK_ENV_ALLOWLIST: &[&str] = &[
    "CLAUDE_CODE_USE_BEDROCK",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_REGION",
    "AWS_DEFAULT_REGION",
    "AWS_PROFILE",
    "AWS_CONFIG_FILE",
    "AWS_SHARED_CREDENTIALS_FILE",
];
const CLAUDE_VERTEX_ENV_ALLOWLIST: &[&str] = &[
    "CLAUDE_CODE_USE_VERTEX",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "GOOGLE_CLOUD_PROJECT",
    "CLOUD_ML_REGION",
    "ANTHROPIC_VERTEX_PROJECT_ID",
];
const NEXUS_SECRET_ENVIRONMENT: &[&str] = &[
    "NEXUS_SECRET",
    "NEXUS_SECRET_FILE",
    "NEXUS_CLIENT_SECRET_FILE",
    "NEXUS_DATABASE_URL",
    "NEXUS_DATABASE_PATH",
    "NEXUS_TEST_DATABASE_URL",
];

pub(super) fn command(provider: ModelProvider, model: &ModelConfig) -> Command {
    let mut command = Command::new(&model.command);
    configure_provider_environment(&mut command, provider);
    command.process_group(0);
    match provider {
        ModelProvider::Codex => configure_codex(&mut command, model),
        ModelProvider::Claude => configure_claude(&mut command, model),
    };
    command
}

fn configure_codex<'a>(command: &'a mut Command, model: &ModelConfig) -> &'a mut Command {
    command.args([
        "exec",
        "--sandbox",
        "read-only",
        "--skip-git-repo-check",
        "--ephemeral",
        "--json",
    ]);
    if let Some(model) = &model.model {
        command.args(["--model", model]);
    }
    if let Some(effort) = &model.reasoning_effort {
        command
            .arg("-c")
            .arg(format!("model_reasoning_effort={effort:?}"));
    }
    command.arg("-")
}

fn configure_claude<'a>(command: &'a mut Command, model: &ModelConfig) -> &'a mut Command {
    command.args([
        "-p",
        "--output-format",
        "json",
        "--permission-mode",
        "plan",
        "--no-session-persistence",
    ]);
    if let Some(model) = &model.model {
        command.args(["--model", model]);
    }
    if let Some(effort) = &model.reasoning_effort {
        command.args(["--effort", effort]);
    }
    command
}

fn configure_provider_environment(command: &mut Command, provider: ModelProvider) {
    apply_provider_environment(command, provider, std::env::vars_os());
}

fn apply_provider_environment(
    command: &mut Command,
    provider: ModelProvider,
    environment: impl IntoIterator<Item = (OsString, OsString)>,
) {
    let environment = environment.into_iter().collect::<Vec<_>>();
    let claude_api_key = (provider == ModelProvider::Claude)
        .then(|| {
            environment
                .iter()
                .find(|(key, value)| key == "NEXUS_CLAUDE_KEY" && !value.is_empty())
        })
        .flatten()
        .map(|(_, value)| value.clone());
    let policy = ProviderEnvironmentPolicy {
        provider,
        bedrock_enabled: environment_flag_enabled(&environment, "CLAUDE_CODE_USE_BEDROCK"),
        vertex_enabled: environment_flag_enabled(&environment, "CLAUDE_CODE_USE_VERTEX"),
    };
    command.env_clear();
    for (key, value) in environment {
        if policy.permits(&key) {
            command.env(key, value);
        }
    }
    if let Some(key) = claude_api_key {
        command.env("ANTHROPIC_API_KEY", key);
        command.env_remove("CLAUDE_CODE_OAUTH_TOKEN");
        command.env_remove("ANTHROPIC_AUTH_TOKEN");
    }
    for key in NEXUS_SECRET_ENVIRONMENT {
        command.env_remove(key);
    }
}

struct ProviderEnvironmentPolicy {
    provider: ModelProvider,
    bedrock_enabled: bool,
    vertex_enabled: bool,
}

impl ProviderEnvironmentPolicy {
    fn permits(&self, key: &OsStr) -> bool {
        allowlisted(COMMON_PROVIDER_ENV_ALLOWLIST, key)
            || match self.provider {
                ModelProvider::Codex => allowlisted(CODEX_ENV_ALLOWLIST, key),
                ModelProvider::Claude => {
                    allowlisted(CLAUDE_ENV_ALLOWLIST, key)
                        || (self.bedrock_enabled && allowlisted(CLAUDE_BEDROCK_ENV_ALLOWLIST, key))
                        || (self.vertex_enabled && allowlisted(CLAUDE_VERTEX_ENV_ALLOWLIST, key))
                }
            }
    }
}

fn allowlisted(allowlist: &[&str], key: &OsStr) -> bool {
    allowlist.iter().any(|allowed| key == *allowed)
}

fn environment_flag_enabled(environment: &[(OsString, OsString)], name: &str) -> bool {
    environment.iter().any(|(key, value)| {
        key == name
            && value.to_str().is_some_and(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider_environment(provider: ModelProvider, cloud_mode: Option<&str>) -> String {
        let mut command = Command::new("/usr/bin/env");
        let mut environment = vec![
            (OsString::from("PATH"), OsString::from("/usr/bin:/bin")),
            (
                OsString::from("OPENAI_API_KEY"),
                OsString::from("openai-test-key"),
            ),
            (
                OsString::from("AZURE_OPENAI_API_KEY"),
                OsString::from("azure-test-key"),
            ),
            (
                OsString::from("CLAUDE_CODE_OAUTH_TOKEN"),
                OsString::from("claude-test-token"),
            ),
            (
                OsString::from("ANTHROPIC_API_KEY"),
                OsString::from("anthropic-test-key"),
            ),
            (
                OsString::from("AWS_ACCESS_KEY_ID"),
                OsString::from("aws-test-key"),
            ),
            (
                OsString::from("GOOGLE_APPLICATION_CREDENTIALS"),
                OsString::from("/private/google-credentials.json"),
            ),
            (
                OsString::from("NEXUS_SECRET"),
                OsString::from("service-bearer-secret"),
            ),
            (
                OsString::from("UNRELATED_PROCESS_SECRET"),
                OsString::from("must-not-be-inherited"),
            ),
        ];
        if let Some(cloud_mode) = cloud_mode {
            environment.push((OsString::from(cloud_mode), OsString::from("1")));
        }
        apply_provider_environment(&mut command, provider, environment);
        String::from_utf8(command.output().unwrap().stdout).unwrap()
    }

    #[test]
    fn nexus_claude_key_selects_api_auth_only_for_claude() {
        for provider in [ModelProvider::Claude, ModelProvider::Codex] {
            let mut command = Command::new("/usr/bin/env");
            apply_provider_environment(
                &mut command,
                provider,
                [
                    ("NEXUS_CLAUDE_KEY".into(), "nexus-api-test-key".into()),
                    ("ANTHROPIC_API_KEY".into(), "ambient-test-key".into()),
                    ("CLAUDE_CODE_OAUTH_TOKEN".into(), "oauth-test-token".into()),
                    ("ANTHROPIC_AUTH_TOKEN".into(), "auth-test-token".into()),
                ],
            );
            let output = command.output().unwrap();
            let environment = String::from_utf8(output.stdout).unwrap();
            assert!(!environment.contains("NEXUS_CLAUDE_KEY="));
            if provider == ModelProvider::Claude {
                assert!(environment.contains("ANTHROPIC_API_KEY=nexus-api-test-key"));
                assert!(!environment.contains("CLAUDE_CODE_OAUTH_TOKEN="));
                assert!(!environment.contains("ANTHROPIC_AUTH_TOKEN="));
            } else {
                assert!(!environment.contains("ANTHROPIC_API_KEY="));
            }
        }
    }

    #[test]
    fn codex_environment_excludes_other_provider_and_nexus_credentials() {
        let environment = provider_environment(ModelProvider::Codex, None);

        assert!(!environment.contains("NEXUS_SECRET="));
        assert!(!environment.contains("UNRELATED_PROCESS_SECRET="));
        assert!(environment.contains("OPENAI_API_KEY=openai-test-key"));
        assert!(environment.contains("AZURE_OPENAI_API_KEY=azure-test-key"));
        assert!(!environment.contains("CLAUDE_CODE_OAUTH_TOKEN="));
        assert!(!environment.contains("ANTHROPIC_API_KEY="));
        assert!(!environment.contains("AWS_ACCESS_KEY_ID="));
        assert!(!environment.contains("GOOGLE_APPLICATION_CREDENTIALS="));
        assert!(environment.contains("PATH=/usr/bin:/bin"));
    }

    #[test]
    fn claude_environment_excludes_openai_and_inactive_cloud_credentials() {
        let environment = provider_environment(ModelProvider::Claude, None);

        assert!(!environment.contains("OPENAI_API_KEY="));
        assert!(!environment.contains("AZURE_OPENAI_API_KEY="));
        assert!(environment.contains("CLAUDE_CODE_OAUTH_TOKEN=claude-test-token"));
        assert!(environment.contains("ANTHROPIC_API_KEY=anthropic-test-key"));
        assert!(!environment.contains("AWS_ACCESS_KEY_ID="));
        assert!(!environment.contains("GOOGLE_APPLICATION_CREDENTIALS="));
        assert!(environment.contains("PATH=/usr/bin:/bin"));
    }

    #[test]
    fn claude_environment_keeps_only_the_selected_cloud_credentials() {
        let bedrock = provider_environment(ModelProvider::Claude, Some("CLAUDE_CODE_USE_BEDROCK"));
        assert!(bedrock.contains("CLAUDE_CODE_USE_BEDROCK=1"));
        assert!(bedrock.contains("AWS_ACCESS_KEY_ID=aws-test-key"));
        assert!(!bedrock.contains("GOOGLE_APPLICATION_CREDENTIALS="));

        let vertex = provider_environment(ModelProvider::Claude, Some("CLAUDE_CODE_USE_VERTEX"));
        assert!(vertex.contains("CLAUDE_CODE_USE_VERTEX=1"));
        assert!(vertex.contains("GOOGLE_APPLICATION_CREDENTIALS=/private/google-credentials.json"));
        assert!(!vertex.contains("AWS_ACCESS_KEY_ID="));
    }
}
