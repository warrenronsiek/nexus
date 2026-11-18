// @feature audit-registry
// @spec docs/features/audit-registry.md
use crate::audit::{AuditAction, AuditResponse};
use serde::Serialize;

pub type AuditCommand = super::ContextCommand<AuditAction>;

#[derive(Debug, Serialize)]
pub struct AuditEnvelope {
    pub ok: bool,
    #[serde(flatten)]
    pub response: AuditResponse,
}

#[derive(Debug, Serialize)]
pub struct AuditErrorEnvelope {
    pub ok: bool,
    pub error_code: &'static str,
    pub error: String,
}
