// @feature audit-registry
// @spec docs/features/audit-registry.md
use super::{history_ops::history_service_adapter, NexusService};
use crate::audit::AuditRejection;
use crate::coordination::{
    api::{AuditCommand, AuditEnvelope, AuditErrorEnvelope, ServiceResponse},
    workspace::ProjectBinding,
};
use crate::work_history::WorkActor;
use anyhow::{Context, Result};

history_service_adapter!(
    command = AuditCommand, actor = WorkActor, rejection = AuditRejection,
    envelope = AuditEnvelope => Audit,
    error = AuditErrorEnvelope => AuditError,
    method = audit_command, apply = apply_audit_command, label = "audit registry"
);
