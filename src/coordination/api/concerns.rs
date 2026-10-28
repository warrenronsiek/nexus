// @feature code-concerns
// @feature runtime
// @spec docs/features/code-concerns.md
// @spec docs/features/runtime.md
use crate::code_concerns::ConcernResponse;
use serde::Serialize;

pub type ConcernCommand = super::ContextCommand<crate::code_concerns::ConcernAction>;

#[derive(Debug, Serialize)]
pub struct ConcernEnvelope {
    pub ok: bool,
    #[serde(flatten)]
    pub response: ConcernResponse,
}

#[derive(Debug, Serialize)]
pub struct ConcernErrorEnvelope {
    pub ok: bool,
    pub error_code: &'static str,
    pub error: String,
}
