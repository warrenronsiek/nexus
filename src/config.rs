// @feature configuration
// @spec docs/features/configuration.md
// @entrypoint Config::load
// @boundary dynamic-config
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub coordination: CoordinationConfig,
    pub privacy: PrivacyConfig,
    pub analyst: AnalystConfig,
    pub review: ReviewConfig,
    pub storage: StorageConfig,
    pub runtime: RuntimeConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CoordinationConfig {
    pub claim_ttl_seconds: i64,
    pub reconcile_seconds: u64,
    pub record_ignored_advisories: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PrivacyConfig {
    pub store_prompt_synopsis: bool,
    pub prompt_synopsis_max_chars: usize,
    pub store_full_prompts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AnalystConfig {
    pub enabled: bool,
    pub provider: ModelProvider,
    pub timeout_seconds: u64,
    pub codex: ModelConfig,
    pub claude: ModelConfig,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelProvider {
    Codex,
    Claude,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReviewConfig {
    pub enabled: bool,
    pub implementation_provider: ModelProvider,
    pub provider: ModelProvider,
    pub timeout_seconds: u64,
    pub skill_root: Option<PathBuf>,
    pub codex: ModelConfig,
    pub claude: ModelConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelConfig {
    pub command: String,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageConfig {
    pub database_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeConfig {
    pub socket_path: PathBuf,
    pub lock_path: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: 1,
            coordination: CoordinationConfig::default(),
            privacy: PrivacyConfig::default(),
            analyst: AnalystConfig::default(),
            review: ReviewConfig::default(),
            storage: StorageConfig::default(),
            runtime: RuntimeConfig::default(),
        }
    }
}

impl Default for CoordinationConfig {
    fn default() -> Self {
        Self {
            claim_ttl_seconds: 120,
            reconcile_seconds: 30,
            record_ignored_advisories: true,
        }
    }
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            store_prompt_synopsis: true,
            prompt_synopsis_max_chars: 240,
            store_full_prompts: false,
        }
    }
}

impl Default for AnalystConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: ModelProvider::Codex,
            timeout_seconds: 60,
            codex: ModelConfig::codex(),
            claude: ModelConfig::claude(),
        }
    }
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            implementation_provider: ModelProvider::Codex,
            provider: ModelProvider::Claude,
            timeout_seconds: 180,
            skill_root: None,
            codex: ModelConfig::codex(),
            claude: ModelConfig::claude(),
        }
    }
}

impl ModelConfig {
    fn codex() -> Self {
        Self {
            command: "codex".into(),
            model: None,
            reasoning_effort: Some("medium".into()),
        }
    }

    fn claude() -> Self {
        Self {
            command: "claude".into(),
            model: None,
            reasoning_effort: None,
        }
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            database_path: default_state_dir().join("nexus.db"),
        }
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        let state = default_state_dir();
        Self {
            socket_path: state.join("nexus.sock"),
            lock_path: state.join("nexus.lock"),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LoadedConfig {
    pub config: Config,
    pub sources: Vec<PathBuf>,
    pub hash: String,
}

impl Config {
    pub fn load(explicit: Option<&Path>, project_root: Option<&Path>) -> Result<LoadedConfig> {
        let mut value = toml::Value::try_from(Config::default())?;
        let mut sources = Vec::new();

        let user_path = explicit.map(Path::to_path_buf).or_else(|| {
            std::env::var_os("NEXUS_CONFIG")
                .map(PathBuf::from)
                .or_else(|| dirs::config_dir().map(|path| path.join("nexus/config.toml")))
        });
        if let Some(path) = user_path {
            if path.exists() {
                merge_file(&mut value, &path)?;
                sources.push(path);
            } else if explicit.is_some() || std::env::var_os("NEXUS_CONFIG").is_some() {
                bail!("configuration file does not exist: {}", path.display());
            }
        }

        if let Some(root) = project_root {
            if let Some(path) = project_config_path(root) {
                if path.exists() && !sources.contains(&path) {
                    merge_file(&mut value, &path)?;
                    sources.push(path);
                }
            }
        }

        apply_env(&mut value)?;
        let config: Config = value.try_into().context("invalid Nexus configuration")?;
        config.validate()?;
        let encoded = toml::to_string(&config)?;
        let hash = blake3::hash(encoded.as_bytes()).to_hex().to_string();
        Ok(LoadedConfig {
            config,
            sources,
            hash,
        })
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            bail!(
                "unsupported schema_version {}; expected 1",
                self.schema_version
            );
        }
        if self.coordination.claim_ttl_seconds <= 0 {
            bail!("coordination.claim_ttl_seconds must be positive");
        }
        if self.privacy.prompt_synopsis_max_chars == 0 {
            bail!("privacy.prompt_synopsis_max_chars must be positive");
        }
        if self.review.timeout_seconds == 0 {
            bail!("review.timeout_seconds must be positive");
        }
        if self.review.implementation_provider == self.review.provider {
            bail!("review.provider must differ from review.implementation_provider");
        }
        Ok(())
    }
}

fn default_state_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("NEXUS_STATE_DIR") {
        return PathBuf::from(path);
    }
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("nexus")
}

fn merge_file(base: &mut toml::Value, path: &Path) -> Result<()> {
    let contents =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let overlay: toml::Value =
        toml::from_str(&contents).with_context(|| format!("parse {}", path.display()))?;
    merge(base, overlay);
    Ok(())
}

fn merge(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base), toml::Value::Table(overlay)) => {
            for (key, value) in overlay {
                if let Some(existing) = base.get_mut(&key) {
                    merge(existing, value);
                } else {
                    base.insert(key, value);
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

fn apply_env(value: &mut toml::Value) -> Result<()> {
    let table = value
        .as_table_mut()
        .context("configuration root must be a table")?;
    for override_ in [
        EnvironmentOverride::new("NEXUS_DATABASE_PATH", "storage", "database_path"),
        EnvironmentOverride::new("NEXUS_SOCKET_PATH", "runtime", "socket_path"),
        EnvironmentOverride::new("NEXUS_ANALYST_PROVIDER", "analyst", "provider"),
        EnvironmentOverride::new("NEXUS_REVIEW_PROVIDER", "review", "provider"),
        EnvironmentOverride::new(
            "NEXUS_IMPLEMENTATION_PROVIDER",
            "review",
            "implementation_provider",
        ),
    ] {
        override_.apply(table)?;
    }
    Ok(())
}

struct EnvironmentOverride {
    variable: &'static str,
    section: &'static str,
    property: &'static str,
}

impl EnvironmentOverride {
    const fn new(variable: &'static str, section: &'static str, property: &'static str) -> Self {
        Self {
            variable,
            section,
            property,
        }
    }

    fn apply(self, root: &mut toml::map::Map<String, toml::Value>) -> Result<()> {
        let Some(value) = std::env::var_os(self.variable) else {
            return Ok(());
        };
        root.get_mut(self.section)
            .and_then(toml::Value::as_table_mut)
            .with_context(|| format!("missing configuration section {}", self.section))?
            .insert(
                self.property.into(),
                toml::Value::String(value.to_string_lossy().into_owned()),
            );
        Ok(())
    }
}

fn project_config_path(root: &Path) -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["-C", root.to_str()?, "rev-parse", "--git-common-dir"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8(output.stdout).ok()?;
    let common = PathBuf::from(raw.trim());
    let common = if common.is_absolute() {
        common
    } else {
        root.join(common)
    };
    Some(common.join("nexus.toml"))
}
