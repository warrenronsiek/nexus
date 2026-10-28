// @feature work-history
// @feature code-concerns
// @spec docs/features/work-history.md
// @spec docs/features/code-concerns.md
// The two histories share bound recording/query transport, not domain lifecycles.
// Types, rejection variants, projections and persistence remain feature-owned.
macro_rules! history_service_adapter {
    (
        command = $command:ident, actor = $actor:ident, rejection = $rejection:ident,
        envelope = $envelope:ident => $success:ident,
        error = $error:ident => $failure:ident,
        method = $method:ident, apply = $apply:ident, label = $label:literal
    ) => {
        impl NexusService {
            pub(super) fn $method(
                &self,
                command: $command,
                binding: &ProjectBinding,
            ) -> ServiceResponse {
                self.$apply(command, binding)
                    .map(|response| ServiceResponse::$success(Box::new(response)))
                    .unwrap_or_else(|error| {
                        let error_code = match error.downcast_ref::<$rejection>() {
                            Some($rejection::Invalid(_)) => "invalid_input",
                            Some($rejection::Conflict(_)) => "conflict",
                            None => "unavailable",
                        };
                        ServiceResponse::$failure($error {
                            ok: false,
                            error_code,
                            error: error.to_string(),
                        })
                    })
            }
            fn $apply(&self, command: $command, binding: &ProjectBinding) -> Result<$envelope> {
                let project =
                    workspace::identify(binding, command.context.project_root.as_deref())?;
                let project_id = command
                    .command
                    .read_project()
                    .map(crate::config::canonical_project_id)
                    .transpose()
                    .map_err(|_| {
                        $rejection::Invalid(concat!("invalid ", $label, " project_id").into())
                    })?
                    .map(str::to_owned)
                    .unwrap_or(project.id);
                let actor = $actor {
                    agent: command.context.agent,
                    session_id: command.context.session_id,
                    model_id: command.context.model,
                    config_hash: self.loaded.hash.clone(),
                };
                let response = self
                    .store
                    .lock()
                    .ok()
                    .context(concat!($label, " store unavailable"))?
                    .$method(&project_id, &actor, &command.command)?;
                Ok($envelope { ok: true, response })
            }
        }
    };
}
pub(super) use history_service_adapter;
