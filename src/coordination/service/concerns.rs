// @feature code-concerns
// @spec docs/features/code-concerns.md
// @entrypoint NexusService::concern_command
use super::NexusService;
use crate::code_concerns::{ConcernActor, ConcernRejection};
use crate::coordination::{
    api::{ConcernCommand, ConcernEnvelope, ConcernErrorEnvelope, ServiceResponse},
    workspace::{self, ProjectBinding},
};
use anyhow::{Context, Result};

use super::history_ops::history_service_adapter;

history_service_adapter!(
    command = ConcernCommand, actor = ConcernActor, rejection = ConcernRejection,
    envelope = ConcernEnvelope => Concerns,
    error = ConcernErrorEnvelope => ConcernError,
    method = concern_command, apply = apply_concern_command, label = "code concern"
);
