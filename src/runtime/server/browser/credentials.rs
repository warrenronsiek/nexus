// @feature runtime
// @feature configuration
// @spec docs/features/runtime.md
// @spec docs/features/configuration.md
// @boundary browser-credential-file: private descriptor-validated password verifier, never serialized as service configuration.
use crate::secret_file::OpenedSecretFile;
use anyhow::{ensure, Context, Result};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use uuid::Uuid;

const MAX_PASSWORD_BYTES: usize = 1024;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Credentials {
    version: u32,
    pub(super) username: String,
    password_hash: String,
}
impl Credentials {
    pub(super) fn load(path: &Path) -> Result<Option<Self>> {
        let Some(bytes) = read_private(path, 4096)? else {
            return Ok(None);
        };
        let credentials: Self =
            serde_json::from_str(&bytes).context("decode browser credentials")?;
        ensure!(
            credentials.version == 1,
            "unsupported browser credential version"
        );
        validate_username(&credentials.username)?;
        let hash = PasswordHash::new(&credentials.password_hash)
            .map_err(|_| anyhow::anyhow!("invalid browser password verifier"))?;
        ensure!(
            hash.algorithm.as_str() == "argon2id"
                && hash.version == Some(19)
                && hash.params.get_decimal("m") == Some(19456)
                && hash.params.get_decimal("t") == Some(2)
                && hash.params.get_decimal("p") == Some(1)
                && hash.salt.is_some()
                && hash.hash.is_some(),
            "unsupported browser password verifier parameters"
        );
        Ok(Some(credentials))
    }
    fn from_password(username: &str, password: &[u8]) -> Result<Self> {
        let salt = SaltString::encode_b64(Uuid::new_v4().as_bytes())
            .map_err(|_| anyhow::anyhow!("generate browser password salt"))?;
        let password_hash = Argon2::default()
            .hash_password(password, &salt)
            .map_err(|_| anyhow::anyhow!("hash browser password"))?
            .to_string();
        Ok(Self {
            version: 1,
            username: username.to_owned(),
            password_hash,
        })
    }

    pub(super) fn verifies(&self, username: &str, password: &[u8]) -> bool {
        // Hash wrong usernames too, so user discovery does not skip the expensive work.
        let valid = PasswordHash::new(&self.password_hash)
            .ok()
            .is_some_and(|hash| Argon2::default().verify_password(password, &hash).is_ok());
        let same_user = subtle::ConstantTimeEq::ct_eq(
            blake3::hash(username.as_bytes()).as_bytes().as_slice(),
            blake3::hash(self.username.as_bytes()).as_bytes().as_slice(),
        );
        valid && bool::from(same_user)
    }
}

pub fn default_credentials_path() -> Result<PathBuf> {
    Ok(dirs::data_local_dir()
        .context("resolve Nexus data directory")?
        .join("nexus/web-credentials.json"))
}

pub fn read_password_file(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = read_private(path, MAX_PASSWORD_BYTES + 2)?
        .context("browser password file does not exist")?
        .into_bytes();
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    validate_password(&bytes)?;
    Ok(bytes)
}

pub fn configure_credentials(path: &Path, username: &str, password: &[u8]) -> Result<()> {
    validate_username(username)?;
    validate_password(password)?;
    // Existing files must satisfy the same policy before an explicit replacement.
    let _ = Credentials::load(path)?;
    let credentials = Credentials::from_password(username, password)?;
    publish(path, &serde_json::to_vec(&credentials)?)
}

fn publish(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    let file = stage(parent, bytes)?;
    file.persist(path).context("publish browser credentials")?;
    std::fs::File::open(parent)?.sync_all().map_err(Into::into)
}

fn stage(parent: &Path, bytes: &[u8]) -> Result<tempfile::NamedTempFile> {
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    Ok(file)
}

fn validate_username(username: &str) -> Result<()> {
    ensure!(
        !username.is_empty()
            && username.len() <= 64
            && username
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')),
        "browser username must contain 1 to 64 ASCII letters, digits, dots, underscores or hyphens"
    );
    Ok(())
}
fn validate_password(password: &[u8]) -> Result<()> {
    ensure!(
        (12..=MAX_PASSWORD_BYTES).contains(&password.len())
            && std::str::from_utf8(password).is_ok()
            && !password
                .iter()
                .any(|byte| matches!(byte, b'\0' | b'\r' | b'\n')),
        "browser password must contain 12 to 1024 UTF-8 bytes and no NUL or newline"
    );
    Ok(())
}

fn read_private(path: &Path, maximum: usize) -> Result<Option<String>> {
    match OpenedSecretFile::open(path) {
        Ok(file) => file
            .read_utf8_bounded(maximum)
            .map(Some)
            .map_err(Into::into),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("open private browser credentials or password file"),
    }
}
