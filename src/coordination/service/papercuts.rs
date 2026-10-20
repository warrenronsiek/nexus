// @feature papercuts
// @feature coordination
// @feature analyst
// @spec docs/features/papercuts.md
// @spec docs/features/coordination.md
// @spec docs/features/analyst.md
use super::feature_ops::{
    background_dispatch_adapter, dispatch_background, dump_registry, registry_dump_adapter,
    BackgroundCommand, BackgroundWorkResult, RegistryDump,
};
use super::NexusService;
use crate::agents::model::{is_timeout, provider_name};
use crate::agents::papercuts::{self, PapercutReviewResult, PapercutReviewerDecision};
use crate::coordination::api::{
    PapercutCommand, PapercutDumpScope, PapercutResponse, ServiceResponse,
};
use crate::coordination::domain::HookContext;
use crate::coordination::workspace;
use crate::coordination::workspace::ProjectBinding;
use crate::papercuts::{
    PapercutActor, PapercutRegistry, PapercutReviewAttemptOutcome, PapercutReviewDecision,
};
use crate::persistence::{PapercutDuplicateProposal, PapercutReviewAttemptRecord};
use anyhow::Result;
use std::path::PathBuf;

pub(crate) struct PapercutDispatch {
    pub(crate) response: ServiceResponse,
    pub(crate) review_job: Option<PapercutReviewJob>,
}

pub(crate) struct PapercutReviewJob {
    pub(crate) project_id: String,
    pub(crate) papercut_id: String,
    pub(crate) reviewer_worktree: Option<PathBuf>,
}

type PapercutResult = BackgroundWorkResult<PapercutResponse, PapercutReviewJob>;

background_dispatch_adapter!(
    command = PapercutCommand,
    response = PapercutResponse => ServiceResponse::Papercuts,
    job = PapercutReviewJob,
    dispatch = PapercutDispatch { review_job },
    result = papercuts_result,
    method = dispatch_papercuts
);

registry_dump_adapter!(
    scope = PapercutDumpScope,
    registry = PapercutRegistry,
    response = PapercutResponse::Dumped,
    store = papercut_store,
    load = papercut_registry
);

struct CompletedPapercutReview {
    project_id: String,
    papercut_id: String,
    result: PapercutReviewResult,
}

impl NexusService {
    fn papercuts_result(
        &self,
        command: PapercutCommand,
        binding: &ProjectBinding,
    ) -> Result<PapercutResult> {
        match command {
            PapercutCommand::List {
                context,
                project_id,
            } => self
                .list_papercuts(context, project_id, binding)
                .map(Into::into),
            PapercutCommand::Dump { context, scope } => {
                dump_registry(self, context, scope, binding).map(Into::into)
            }
            PapercutCommand::Report {
                context,
                title,
                description,
            } => self.report_papercut(context, title, description, binding),
            PapercutCommand::Upvote {
                context,
                papercut_id,
                note,
            } => self
                .upvote_papercut(context, papercut_id, note, binding)
                .map(Into::into),
            PapercutCommand::Remove {
                context,
                papercut_id,
                resolution,
            } => self
                .remove_papercut(context, papercut_id, resolution, binding)
                .map(Into::into),
            PapercutCommand::Respond {
                context,
                review_id,
                reporter_token,
                decision,
                response,
            } => self
                .respond_to_papercut_review(context, review_id, reporter_token, decision, response)
                .map(Into::into),
        }
    }

    fn list_papercuts(
        &self,
        context: HookContext,
        project_id: Option<String>,
        binding: &ProjectBinding,
    ) -> Result<PapercutResponse> {
        let project_id = match project_id {
            Some(id) => id,
            None => workspace::identify(binding, context.project_root.as_deref())?.id,
        };
        let window = self.papercut_store()?.open_papercuts_window(&project_id)?;
        Ok(PapercutResponse::Listed {
            ok: true,
            papercuts: window.items,
            truncated: window.truncated,
        })
    }

    fn report_papercut(
        &self,
        context: HookContext,
        title: String,
        description: String,
        binding: &ProjectBinding,
    ) -> Result<PapercutResult> {
        let project = workspace::identify(binding, context.project_root.as_deref())?;
        let actor = self.papercut_actor(context);
        let result =
            self.papercut_store()?
                .report_papercut(&project.id, &title, &description, &actor)?;
        let review_requested = result.created && self.loaded.config.analyst.enabled;
        let review_job = review_requested.then(|| PapercutReviewJob {
            project_id: project.id,
            papercut_id: result.papercut.id.clone(),
            reviewer_worktree: project.worktree.map(PathBuf::from),
        });
        Ok(PapercutResult {
            response: PapercutResponse::Reported {
                ok: true,
                created: result.created,
                reporter_token: result.reporter_token,
                review_requested,
                papercut: result.papercut,
            },
            job: review_job,
        })
    }

    fn upvote_papercut(
        &self,
        context: HookContext,
        papercut_id: String,
        note: Option<String>,
        binding: &ProjectBinding,
    ) -> Result<PapercutResponse> {
        let project = workspace::identify(binding, context.project_root.as_deref())?;
        let actor = self.papercut_actor(context);
        let (papercut, upvote) = self.papercut_store()?.upvote_papercut(
            &project.id,
            &papercut_id,
            note.as_deref(),
            &actor,
        )?;
        Ok(PapercutResponse::Upvoted {
            ok: true,
            papercut: Box::new(papercut),
            upvote,
        })
    }

    fn remove_papercut(
        &self,
        context: HookContext,
        papercut_id: String,
        resolution: String,
        binding: &ProjectBinding,
    ) -> Result<PapercutResponse> {
        let project = workspace::identify(binding, context.project_root.as_deref())?;
        let actor = self.papercut_actor(context);
        let papercut = self.papercut_store()?.remove_papercut(
            &project.id,
            &papercut_id,
            &resolution,
            &actor,
        )?;
        Ok(PapercutResponse::Removed { ok: true, papercut })
    }

    fn respond_to_papercut_review(
        &self,
        context: HookContext,
        review_id: String,
        reporter_token: String,
        decision: PapercutReviewDecision,
        response: String,
    ) -> Result<PapercutResponse> {
        let actor = self.papercut_actor(context);
        let review = self.papercut_store()?.respond_to_papercut_review(
            &review_id,
            decision,
            &response,
            &reporter_token,
            &actor,
        )?;
        Ok(PapercutResponse::Responded { ok: true, review })
    }

    pub(crate) fn review_papercut(&self, job: &PapercutReviewJob) -> Result<()> {
        let completed = self.invoke_papercut_reviewer(job)?;
        self.persist_papercut_review(completed)
    }

    fn invoke_papercut_reviewer(&self, job: &PapercutReviewJob) -> Result<CompletedPapercutReview> {
        let (papercut, candidates) = {
            let mut store = self.papercut_store()?;
            (
                store.papercut_by_id(&job.project_id, &job.papercut_id)?,
                store.papercut_review_candidates(&job.project_id, &job.papercut_id)?,
            )
        };
        let result = papercuts::review(
            &self.loaded.config.analyst,
            &papercut,
            &candidates,
            job.reviewer_worktree.as_deref(),
        )?;
        Ok(CompletedPapercutReview {
            project_id: job.project_id.clone(),
            papercut_id: job.papercut_id.clone(),
            result,
        })
    }

    fn persist_papercut_review(&self, completed: CompletedPapercutReview) -> Result<()> {
        let CompletedPapercutReview {
            project_id,
            papercut_id,
            result,
        } = completed;
        let provider = provider_name(result.provider);
        match result.decision {
            PapercutReviewerDecision::Distinct { rationale } => self
                .papercut_store()?
                .record_papercut_review_event(PapercutReviewAttemptRecord {
                    project_id: &project_id,
                    papercut_id: &papercut_id,
                    outcome: PapercutReviewAttemptOutcome::Distinct,
                    provider,
                    model_id: result.model.as_deref(),
                    detail: &rationale,
                    config_hash: &self.loaded.hash,
                }),
            PapercutReviewerDecision::PossibleDuplicate {
                canonical_id,
                rationale,
                question,
            } => {
                let reviewer = PapercutActor {
                    agent: format!("nexus-{provider}-reviewer"),
                    session_id: format!("papercut-review-{papercut_id}"),
                    model_id: result.model.clone(),
                    config_hash: self.loaded.hash.clone(),
                };
                self.papercut_store()?
                    .propose_papercut_duplicate(PapercutDuplicateProposal {
                        project_id: &project_id,
                        papercut_id: &papercut_id,
                        canonical_id: &canonical_id,
                        rationale: &rationale,
                        question: &question,
                        reviewer: &reviewer,
                        provider: Some(provider),
                        model_id: result.model.as_deref(),
                    })?;
                Ok(())
            }
        }
    }

    pub(crate) fn record_papercut_review_failure(
        &self,
        job: &PapercutReviewJob,
        error: &anyhow::Error,
    ) -> Result<()> {
        let provider = self.loaded.config.analyst.provider;
        let model = self.loaded.config.analyst.model(provider).model.clone();
        let detail = sanitized_review_failure(error);
        self.papercut_store()?
            .record_papercut_review_event(PapercutReviewAttemptRecord {
                project_id: &job.project_id,
                papercut_id: &job.papercut_id,
                outcome: PapercutReviewAttemptOutcome::Failed,
                provider: provider_name(provider),
                model_id: model.as_deref(),
                detail,
                config_hash: &self.loaded.hash,
            })
    }

    fn papercut_actor(&self, context: HookContext) -> PapercutActor {
        PapercutActor {
            agent: context.agent,
            session_id: context.session_id,
            model_id: context.model,
            config_hash: self.loaded.hash.clone(),
        }
    }

    fn papercut_store(&self) -> Result<super::TrackedStoreGuard<'_>> {
        self.store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))
    }
}

fn sanitized_review_failure(error: &anyhow::Error) -> &'static str {
    if is_timeout(error) {
        "provider invocation timed out"
    } else {
        "review workflow failed"
    }
}
