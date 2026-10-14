// @feature sos
// @feature coordination
// @feature analyst
// @spec docs/features/sos.md
// @spec docs/features/coordination.md
// @spec docs/features/analyst.md
use super::feature_ops::{
    background_dispatch_adapter, dispatch_background, dump_registry, registry_dump_adapter,
    BackgroundCommand, BackgroundWorkResult, RegistryDump,
};
use super::NexusService;
use crate::agents::model::{is_timeout, provider_name};
use crate::agents::sos::{self, SosTriageResult};
use crate::coordination::api::{ServiceResponse, SosCommand, SosDumpScope, SosResponse};
use crate::coordination::domain::HookContext;
use crate::coordination::workspace::{self, ProjectBinding};
use crate::persistence::{SosTriageApply, SosTriageFailure, Store};
use crate::sos::{SosActor, SosIssue, SosRegistry, SosReportDraft};
use anyhow::{Context, Result};
use std::path::PathBuf;

pub(crate) struct SosDispatch {
    pub(crate) response: ServiceResponse,
    pub(crate) triage_job: Option<SosTriageJob>,
}

#[derive(Clone)]
pub(crate) struct SosTriageJob {
    pub(crate) project_id: String,
    pub(crate) report_id: String,
    pub(crate) reviewer_worktree: Option<PathBuf>,
}

type SosResult = BackgroundWorkResult<SosResponse, SosTriageJob>;

background_dispatch_adapter!(
    command = SosCommand,
    response = SosResponse => ServiceResponse::Sos,
    job = SosTriageJob,
    dispatch = SosDispatch { triage_job },
    result = sos_result,
    method = dispatch_sos
);

registry_dump_adapter!(
    scope = SosDumpScope,
    registry = SosRegistry,
    response = SosResponse::Dumped,
    store = sos_store,
    load = sos_registry
);

enum SosClosure {
    Resolved(String),
    Deprecated(String),
}

impl SosClosure {
    fn apply(
        &self,
        store: &mut Store,
        project_id: &str,
        issue_id: &str,
        actor: &SosActor,
    ) -> Result<SosIssue> {
        match self {
            Self::Resolved(reason) => store.resolve_sos_issue(project_id, issue_id, reason, actor),
            Self::Deprecated(reason) => {
                store.deprecate_sos_issue(project_id, issue_id, reason, actor)
            }
        }
    }

    fn response(self, issue: SosIssue) -> SosResponse {
        match self {
            Self::Resolved(_) => SosResponse::Resolved { ok: true, issue },
            Self::Deprecated(_) => SosResponse::Deprecated { ok: true, issue },
        }
    }
}

impl NexusService {
    fn sos_result(&self, command: SosCommand, binding: &ProjectBinding) -> Result<SosResult> {
        match command {
            SosCommand::Report {
                context,
                message,
                kind,
                environment,
                version,
                trace,
                metadata,
            } => self.report_sos(
                context,
                SosReportDraft {
                    message,
                    kind,
                    environment,
                    version,
                    model: String::new(),
                    trace,
                    metadata,
                },
                binding,
            ),
            SosCommand::List { context } => self.list_sos(context, binding).map(Into::into),
            SosCommand::Get { context, issue_id } => {
                self.get_sos(context, issue_id, binding).map(Into::into)
            }
            SosCommand::Dump { context, scope } => {
                dump_registry(self, context, scope, binding).map(Into::into)
            }
            SosCommand::Resolve {
                context,
                issue_id,
                resolution,
            } => self
                .close_sos(context, issue_id, SosClosure::Resolved(resolution), binding)
                .map(Into::into),
            SosCommand::Deprecate {
                context,
                issue_id,
                reason,
            } => self
                .close_sos(context, issue_id, SosClosure::Deprecated(reason), binding)
                .map(Into::into),
        }
    }

    fn report_sos(
        &self,
        context: HookContext,
        mut draft: SosReportDraft,
        binding: &ProjectBinding,
    ) -> Result<SosResult> {
        let project = workspace::identify(binding, context.project_root.as_deref())?;
        draft.model = context.model.clone().context("SOS reports require model")?;
        let actor = self.sos_actor(context);
        let report = self
            .sos_store()?
            .record_sos_report(&project.id, draft, &actor)?;
        let triage_job = SosTriageJob {
            project_id: project.id,
            report_id: report.id.clone(),
            reviewer_worktree: project.worktree.map(PathBuf::from),
        };
        Ok(SosResult {
            response: SosResponse::Reported {
                ok: true,
                report,
                triage_requested: true,
            },
            job: Some(triage_job),
        })
    }

    fn list_sos(&self, context: HookContext, binding: &ProjectBinding) -> Result<SosResponse> {
        let project = workspace::identify(binding, context.project_root.as_deref())?;
        let window = self.sos_store()?.open_sos_issues_window(&project.id)?;
        Ok(SosResponse::Listed {
            ok: true,
            issues: window.issues,
            untriaged_report_count: window.untriaged_report_count,
            truncated: window.truncated,
        })
    }

    fn get_sos(
        &self,
        context: HookContext,
        issue_id: String,
        binding: &ProjectBinding,
    ) -> Result<SosResponse> {
        let project = workspace::identify(binding, context.project_root.as_deref())?;
        let detail = self.sos_store()?.sos_issue_detail(&project.id, &issue_id)?;
        Ok(SosResponse::Detailed { ok: true, detail })
    }

    fn close_sos(
        &self,
        context: HookContext,
        issue_id: String,
        closure: SosClosure,
        binding: &ProjectBinding,
    ) -> Result<SosResponse> {
        let project = workspace::identify(binding, context.project_root.as_deref())?;
        let actor = self.sos_actor(context);
        let mut store = self.sos_store()?;
        let issue = closure.apply(&mut store, &project.id, &issue_id, &actor)?;
        drop(store);
        Ok(closure.response(issue))
    }

    pub(crate) fn triage_sos(&self, job: &SosTriageJob) -> Result<()> {
        let (report, candidates) = {
            let mut store = self.sos_store()?;
            (
                store.sos_report(&job.project_id, &job.report_id)?,
                store.sos_triage_candidates(&job.project_id)?,
            )
        };
        if report.issue_id.is_some() {
            return Ok(());
        }
        let result = sos::review(
            &self.loaded.config.analyst,
            &report,
            &candidates,
            job.reviewer_worktree.as_deref(),
        )?;
        self.persist_sos_triage(job, result)
    }

    fn persist_sos_triage(&self, job: &SosTriageJob, result: SosTriageResult) -> Result<()> {
        self.sos_store()?.apply_sos_triage(SosTriageApply {
            project_id: &job.project_id,
            report_id: &job.report_id,
            decision: &result.decision,
            provider: provider_name(result.provider),
            model_id: result.model.as_deref(),
            config_hash: &self.loaded.hash,
        })?;
        Ok(())
    }

    pub(crate) fn record_sos_triage_failure(
        &self,
        job: &SosTriageJob,
        error: &anyhow::Error,
    ) -> Result<()> {
        let provider = self.loaded.config.analyst.provider;
        let model = self.loaded.config.analyst.model(provider).model.clone();
        self.sos_store()?
            .record_sos_triage_failure(SosTriageFailure {
                project_id: &job.project_id,
                report_id: &job.report_id,
                provider: provider_name(provider),
                model_id: model.as_deref(),
                rationale: sanitized_triage_failure(error),
                config_hash: &self.loaded.hash,
            })
    }

    pub(crate) fn ready_sos_triage_jobs(&self) -> Result<Vec<SosTriageJob>> {
        Ok(self
            .sos_store()?
            .ready_sos_reports()?
            .into_iter()
            .map(|(project_id, report_id)| SosTriageJob {
                project_id,
                report_id,
                reviewer_worktree: None,
            })
            .collect())
    }

    fn sos_actor(&self, context: HookContext) -> SosActor {
        SosActor {
            agent: context.agent,
            session_id: context.session_id,
            model_id: context.model,
            config_hash: self.loaded.hash.clone(),
        }
    }

    fn sos_store(&self) -> Result<super::TrackedStoreGuard<'_>> {
        self.store
            .lock()
            .map_err(|_| anyhow::anyhow!("store lock poisoned"))
    }
}

fn sanitized_triage_failure(error: &anyhow::Error) -> &'static str {
    if is_timeout(error) {
        "provider invocation timed out"
    } else {
        "SOS triage workflow failed"
    }
}
