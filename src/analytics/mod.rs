// @feature usage-analytics
// @spec docs/features/usage-analytics.md
pub(crate) mod capture;
pub(crate) mod classification;
pub(crate) mod inventory;
pub(crate) mod types;

pub(crate) fn identity(parts: &[&str]) -> String {
    let mut hash = blake3::Hasher::new();
    for part in parts {
        hash.update(&(part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    hash.finalize().to_hex().to_string()
}
