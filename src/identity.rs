// @feature work-history
// @feature usage-analytics
// @spec docs/features/work-history.md
// @spec docs/features/usage-analytics.md
/// Length prefixes keep opaque identities unambiguous without retaining source text.
pub(crate) fn stable_id(parts: &[&str]) -> String {
    let mut hash = blake3::Hasher::new();
    for part in parts {
        hash.update(&(part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    hash.finalize().to_hex().to_string()
}
