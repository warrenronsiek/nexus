// @feature evals
// @spec docs/features/evals.md
// @entrypoint EvalRecordInput::validate
mod lifecycle_projection;
mod records;
pub(crate) mod validation;

pub use lifecycle_projection::*;
pub use records::*;
pub use validation::{digest_is_valid, MAX_BLOB_BYTES, MAX_CHUNK_BYTES};
