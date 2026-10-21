// @entrypoint NexusService::analytics_command
// @feature usage-analytics
// @spec docs/features/usage-analytics.md
use super::NexusService;
use crate::analytics::{capture, identity, types::*};
use crate::coordination::client_evidence::ClientDerivedUsage;
use crate::coordination::{
    api::ServiceResponse,
    domain::{ToolCompletion, ToolHookInput},
    workspace::{self, ProjectBinding, ProjectIdentity},
};
use anyhow::Result;

impl NexusService {
    pub(super) fn analytics_command(
        &self,
        command: AnalyticsCommand,
        binding: &ProjectBinding,
    ) -> ServiceResponse {
        match self.run_analytics_command(command, binding) {
            Ok(result) => ServiceResponse::Analytics(result),
            Err(error) => ServiceResponse::error(error),
        }
    }
    fn run_analytics_command(
        &self,
        command: AnalyticsCommand,
        binding: &ProjectBinding,
    ) -> Result<AnalyticsResponse> {
        let mut store = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("analytics store unavailable"))?;
        match command {
            AnalyticsCommand::ReserveClassification { context, target } => self
                .classification_command(
                    &mut store,
                    binding,
                    context,
                    target,
                    ClassificationReport::Reserve,
                ),
            AnalyticsCommand::Classify {
                context,
                target,
                draft,
                model,
            } => self.classification_command(
                &mut store,
                binding,
                context,
                target,
                ClassificationReport::Classified(draft, model),
            ),
            AnalyticsCommand::ClassificationFailed { context, target } => self
                .classification_command(
                    &mut store,
                    binding,
                    context,
                    target,
                    ClassificationReport::Failed,
                ),
            AnalyticsCommand::Inventory {
                context,
                scope,
                resources,
                complete,
            } => {
                let project = workspace::identify(binding, context.project_root.as_deref())?;
                store
                    .record_inventory(&project.id, &context, &scope, &resources, complete)
                    .map(|()| AnalyticsResponse::Write { ok: true })
            }
            AnalyticsCommand::RenameCategory {
                id,
                name,
                definition,
            } => store
                .rename_activity_category(&id, &name, &definition)
                .map(|()| AnalyticsResponse::Write { ok: true }),
            AnalyticsCommand::MergeCategory { source, target } => store
                .merge_activity_categories(&source, &target)
                .map(|()| AnalyticsResponse::Write { ok: true }),
            read => read_analytics(&mut store, binding, read),
        }
    }
    fn classification_command(
        &self,
        store: &mut crate::persistence::Store,
        binding: &ProjectBinding,
        context: crate::coordination::domain::HookContext,
        target: ClassificationTarget,
        report: ClassificationReport,
    ) -> Result<AnalyticsResponse> {
        let project = workspace::identify(binding, context.project_root.as_deref())?;
        let job = store.resolve_classification_target(&project.id, &context, &target)?;
        match report {
            ClassificationReport::Reserve => {
                let granted = self.loaded.config.analytics.classification_enabled
                    && store.reserve_activity_classification(
                        &job,
                        self.loaded.config.analytics.max_classifications_per_day,
                    )?;
                Ok(AnalyticsResponse::Permit { ok: true, granted })
            }
            ClassificationReport::Classified(draft, model) => store
                .apply_activity_classification(&job, &draft, &model)
                .map(|()| AnalyticsResponse::Write { ok: true }),
            ClassificationReport::Failed => store
                .mark_activity_classification_failed(&job)
                .map(|()| AnalyticsResponse::Write { ok: true }),
        }
    }

    pub(super) fn observe_activity(
        &self,
        store: &mut crate::persistence::Store,
        project: &ProjectIdentity,
        input: &ToolHookInput,
        completion: ToolCompletion,
        evidence: Option<&ClientDerivedUsage>,
    ) -> Result<()> {
        let derived;
        let evidence = if let Some(evidence) = evidence {
            evidence
        } else {
            derived = project
                .worktree
                .as_deref()
                .map(|root| {
                    crate::coordination::usage::derive_filesystem_evidence(
                        input,
                        completion,
                        std::path::Path::new(root),
                    )
                })
                .unwrap_or_default();
            &derived
        };
        let mut actions = capture::resolve(input, &evidence.scripts);
        capture::version_scripts(&mut actions, &evidence.script_versions);
        let session = identity(&[&project.id, &input.context.agent, &input.context.session_id]);
        for action in &mut actions {
            if action.kind == ResourceKind::Script {
                action.origin = store.script_origin(&project.id, &session, &action.name)?;
            }
        }
        store.record_activity_call(
            &project.id,
            input,
            &actions,
            match completion {
                ToolCompletion::Succeeded => "succeeded",
                ToolCompletion::Failed => "failed",
            },
        )?;
        Ok(())
    }
}

pub(super) fn local_evidence(
    evidence: Option<ClientDerivedUsage>,
    project: &ProjectIdentity,
    input: &ToolHookInput,
    completion: ToolCompletion,
) -> Option<ClientDerivedUsage> {
    evidence.or_else(|| {
        project.worktree.as_deref().map(|root| {
            crate::coordination::usage::derive_filesystem_evidence(
                input,
                completion,
                std::path::Path::new(root),
            )
        })
    })
}

enum ClassificationReport {
    Reserve,
    Classified(CategoryDraft, String),
    Failed,
}
fn read_analytics(
    store: &mut crate::persistence::Store,
    binding: &ProjectBinding,
    command: AnalyticsCommand,
) -> Result<AnalyticsResponse> {
    match command {
        AnalyticsCommand::Categories => {
            store
                .activity_categories()
                .map(|categories| AnalyticsResponse::Categories {
                    ok: true,
                    categories,
                })
        }
        AnalyticsCommand::Summary { project_id } => store
            .activity_summary(binding.default_read_project(project_id).as_deref())
            .map(AnalyticsResponse::Summary),
        AnalyticsCommand::Catalog {
            project_id,
            offset,
            limit,
        } => store
            .activity_catalog(
                binding.default_read_project(project_id).as_deref(),
                offset,
                limit.unwrap_or(32),
            )
            .map(AnalyticsResponse::Catalog),
        AnalyticsCommand::Opportunities { project_id } => store
            .activity_opportunities(binding.default_read_project(project_id).as_deref())
            .map(AnalyticsResponse::Opportunities),
        AnalyticsCommand::Examples {
            project_id,
            category_id,
            resource_key,
            offset,
        } => store
            .activity_examples(
                binding.default_read_project(project_id).as_deref(),
                category_id.as_deref(),
                resource_key.as_deref(),
                offset,
            )
            .map(AnalyticsResponse::Examples),
        _ => anyhow::bail!("expected an analytics read operation"),
    }
}
