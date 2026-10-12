// @feature papercuts
// @feature coordination
// @feature analyst
// @spec docs/features/papercuts.md
// @spec docs/features/coordination.md
// @spec docs/features/analyst.md
use super::NexusService;
use crate::agents::model::{is_timeout, provider_name};
use crate::agents::papercuts::{self, PapercutReviewResult, PapercutReviewerDecision};
use crate::coordination::api::{
    PapercutCommand, PapercutDumpScope, PapercutResponse, ServiceResponse,
};
use crate::coordination::domain::HookContext;
use crate::coordination::workspace;
use crate::papercuts::{PapercutActor, PapercutReviewAttemptOutcome, PapercutReviewDecision};
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

struct PapercutResult {
    response: PapercutResponse,
    review_job: Option<PapercutReviewJob>,
}

impl From<PapercutResponse> for PapercutResult {
    fn from(response: PapercutResponse) -> Self {
        Self {
            response,
            review_job: None,
        }
    }
}

struct CompletedPapercutReview {
    project_id: String,
    papercut_id: String,
    result: PapercutReviewResult,
}

impl NexusService {
    pub(crate) fn dispatch_papercuts(&self, command: PapercutCommand) -> PapercutDispatch {
        match self.papercuts_result(command) {
            Ok(result) => PapercutDispatch {
                response: ServiceResponse::Papercuts(Box::new(result.response)),
                review_job: result.review_job,
            },
            Err(error) => PapercutDispatch {
                response: ServiceResponse::error(error),
                review_job: None,
            },
        }
    }

    fn papercuts_result(&self, command: PapercutCommand) -> Result<PapercutResult> {
        match command {
            PapercutCommand::List { context } => self.list_papercuts(context).map(Into::into),
            PapercutCommand::Dump { context, scope } => {
                self.dump_papercuts(context, scope).map(Into::into)
            }
            PapercutCommand::Report {
                context,
                title,
                description,
            } => self.report_papercut(context, title, description),
            PapercutCommand::Upvote {
                context,
                papercut_id,
                note,
            } => self
                .upvote_papercut(context, papercut_id, note)
                .map(Into::into),
            PapercutCommand::Remove {
                context,
                papercut_id,
                resolution,
            } => self
                .remove_papercut(context, papercut_id, resolution)
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

    fn list_papercuts(&self, context: HookContext) -> Result<PapercutResponse> {
        let project = workspace::identify(context.project_root.as_deref())?;
        let papercuts = self.papercut_store()?.open_papercuts(&project.id)?;
        Ok(PapercutResponse::Listed {
            ok: true,
            papercuts,
        })
    }

    fn dump_papercuts(
        &self,
        context: HookContext,
        scope: PapercutDumpScope,
    ) -> Result<PapercutResponse> {
        let project_id = match scope {
            PapercutDumpScope::Project => {
                Some(workspace::identify(context.project_root.as_deref())?.id)
            }
            PapercutDumpScope::All => None,
        };
        let registry = self
            .papercut_store()?
            .papercut_registry(project_id.as_deref())?;
        Ok(PapercutResponse::Dumped { ok: true, registry })
    }

    fn report_papercut(
        &self,
        context: HookContext,
        title: String,
        description: String,
    ) -> Result<PapercutResult> {
        let project = workspace::identify(context.project_root.as_deref())?;
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
            review_job,
        })
    }

    fn upvote_papercut(
        &self,
        context: HookContext,
        papercut_id: String,
        note: Option<String>,
    ) -> Result<PapercutResponse> {
        let project = workspace::identify(context.project_root.as_deref())?;
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
    ) -> Result<PapercutResponse> {
        let project = workspace::identify(context.project_root.as_deref())?;
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
        let (papercut, registry) = {
            let mut store = self.papercut_store()?;
            (
                store.papercut_by_id(&job.project_id, &job.papercut_id)?,
                store.papercut_registry(Some(&job.project_id))?,
            )
        };
        let result = papercuts::review(
            &self.loaded.config.analyst,
            &papercut,
            &registry,
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

    fn papercut_store(&self) -> Result<std::sync::MutexGuard<'_, crate::persistence::Store>> {
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
