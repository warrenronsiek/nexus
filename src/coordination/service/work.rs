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

impl NexusService {
    pub(super) fn work_command(
        &self,
        command: WorkCommand,
        binding: &ProjectBinding,
    ) -> ServiceResponse {
        self.apply_work_command(command, binding)
            .map(|response| ServiceResponse::Work(Box::new(response)))
            .unwrap_or_else(|error| {
                let error_code = match error.downcast_ref::<WorkRejection>() {
                    Some(WorkRejection::Invalid(_)) => "invalid_input",
                    Some(WorkRejection::Conflict(_)) => "conflict",
                    None => "unavailable",
                };
                ServiceResponse::WorkError(WorkErrorEnvelope {
                    ok: false,
                    error_code,
                    error: error.to_string(),
                })
            })
    }

    fn apply_work_command(
        &self,
        command: WorkCommand,
        binding: &ProjectBinding,
    ) -> Result<WorkEnvelope> {
        let project = workspace::identify(binding, command.context.project_root.as_deref())?;
        let project_id = command
            .command
            .read_project()
            .map(crate::config::canonical_project_id)
            .transpose()
            .map_err(|_| anyhow::anyhow!("invalid work project_id"))?
            .map(str::to_owned)
            .unwrap_or(project.id);
        let actor = WorkActor {
            agent: command.context.agent,
            session_id: command.context.session_id,
            model_id: command.context.model,
            config_hash: self.loaded.hash.clone(),
        };
        let response = self
            .store
            .lock()
            .ok()
            .context("work history store unavailable")?
            .work_command(&project_id, &actor, &command.command)?;
        Ok(WorkEnvelope { ok: true, response })
    }
}
