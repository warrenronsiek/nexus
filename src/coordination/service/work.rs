// @feature work-history
// @spec docs/features/work-history.md
// @entrypoint NexusService::work_command
use super::NexusService;
use crate::coordination::{
    api::{ServiceResponse, WorkCommand, WorkEnvelope, WorkErrorEnvelope},
    workspace::{self, ProjectBinding},
};
use crate::work_history::{WorkActor, WorkRejection};
use anyhow::{Context, Result};

use super::history_ops::history_service_adapter;

history_service_adapter!(
    command = WorkCommand, actor = WorkActor, rejection = WorkRejection,
    envelope = WorkEnvelope => Work,
    error = WorkErrorEnvelope => WorkError,
    method = work_command, apply = apply_work_command, label = "work history"
);
