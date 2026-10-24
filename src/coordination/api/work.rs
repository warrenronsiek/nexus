// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
use crate::work_history::{WorkAction, WorkResponse};
use serde::Serialize;

pub type WorkCommand = super::ContextCommand<WorkAction>;

#[derive(Debug, Serialize)]
pub struct WorkEnvelope {
    pub ok: bool,
    #[serde(flatten)]
    pub response: WorkResponse,
}

#[derive(Debug, Serialize)]
pub struct WorkErrorEnvelope {
    pub ok: bool,
    pub error_code: &'static str,
    pub error: String,
}
