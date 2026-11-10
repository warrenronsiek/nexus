// @feature runtime
// @feature observability-ui
// @spec docs/features/runtime.md
// @spec docs/features/observability-ui.md
// @boundary browser-read: closed feature-owned reads map to validated service operations, never arbitrary methods.
use crate::analytics::types::AnalyticsCommand;
use crate::code_concerns::{ConcernAction, ConcernQuery};
use crate::coordination::api::*;
use crate::coordination::domain::{ConflictScope, HookContext, RecordScope};
use crate::evals::EvalQuery;
use crate::work_history::{WorkAction, WorkQuery, WorkSessionOrigin, WorkSubject};
use anyhow::{ensure, Result};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReadEnvelope {
    project_id: Option<String>,
    query: BrowserRead,
}
// Families retain their owning domain's fields and validation. Untagged dispatch preserves
// the flat browser wire contract, while each internally tagged family is closed.
#[derive(Deserialize)]
#[serde(untagged)]
enum BrowserRead {
    Coordination(CoordinationRead),
    Analytics(AnalyticsRead),
    Memory(MemoryRead),
    Issues(IssueRead),
    Evals(EvalRead),
    Work(WorkRead),
    Concerns(ConcernRead),
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum CoordinationRead {
    Projects {},
    Coordination {},
    CoordinationSessions {
        #[serde(default)]
        scope: RecordScope,
    },
    CoordinationClaims {
        #[serde(default)]
        scope: RecordScope,
    },
    CoordinationConflicts {
        #[serde(default)]
        scope: ConflictScope,
    },
    CoordinationEvents {
        limit: Option<usize>,
    },
    CoordinationSessionTimeline {
        session_id: String,
    },
    CoordinationSessionIndex {},
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum AnalyticsRead {
    Usage {},
    AnalyticsSummary {},
    AnalyticsCatalog {
        #[serde(default)]
        offset: u32,
        limit: Option<u32>,
    },
    AnalyticsOpportunities {},
    AnalyticsExamples {
        category_id: Option<String>,
        resource_key: Option<String>,
        #[serde(default)]
        offset: u32,
    },
    AnalyticsCategories {},
    AnalyticsCommands {
        category_id: String,
    },
    SkillGet {
        name: String,
    },
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum MemoryRead {
    #[serde(rename = "memory_status")]
    Status {},
    #[serde(rename = "memory_context")]
    Context {
        #[serde(default)]
        scope: MemoryReadScopeArg,
    },
    #[serde(rename = "memory_search")]
    Search {
        #[serde(default)]
        scope: MemoryReadScopeArg,
        regex: String,
        limit: Option<usize>,
    },
    #[serde(rename = "memory_expand")]
    Expand { summary_id: String },
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum IssueRead {
    Papercuts { after: Option<PapercutListCursor> },
    PapercutGet { papercut_id: String },
    SosList {},
    SosGet { issue_id: String },
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum EvalRead {
    #[serde(rename = "eval_dashboard")]
    Dashboard { group_id: Option<String> },
    #[serde(rename = "eval_list")]
    List {
        #[serde(default)]
        filter: EvalQuery,
    },
    #[serde(rename = "eval_get")]
    Get { id: String },
    #[serde(rename = "eval_why")]
    Why { id: String },
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum WorkRead {
    #[serde(rename = "work_dashboard")]
    Dashboard {
        work_session_id: Option<String>,
        #[serde(default = "host_origin")]
        session_origin: WorkSessionOrigin,
    },
    #[serde(rename = "work_list")]
    List {
        #[serde(default)]
        filter: WorkQuery,
    },
    #[serde(rename = "work_activities")]
    Activities {
        #[serde(default)]
        filter: WorkQuery,
    },
    #[serde(rename = "work_get")]
    Get { subject: WorkSubject },
    #[serde(rename = "work_observation")]
    Observation { id: String },
    #[serde(rename = "work_timeline")]
    Timeline {
        work_session_id: String,
        parent_activity_id: Option<String>,
        after: Option<String>,
        limit: Option<u32>,
    },
    #[serde(rename = "work_events")]
    Events {
        subject: WorkSubject,
        after: Option<String>,
        limit: Option<u32>,
    },
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ConcernRead {
    #[serde(rename = "code_map")]
    Map {
        #[serde(default)]
        filter: crate::code_map::MapQuery,
    },
    #[serde(rename = "concern_dashboard")]
    Dashboard {
        #[serde(default)]
        filter: ConcernQuery,
    },
    #[serde(rename = "concern_list")]
    List {
        #[serde(default)]
        filter: ConcernQuery,
    },
    #[serde(rename = "concern_get")]
    Get { concern_id: String },
    #[serde(rename = "concern_history")]
    History {
        concern_id: String,
        after: Option<String>,
        limit: Option<u32>,
    },
}
fn host_origin() -> WorkSessionOrigin {
    WorkSessionOrigin::Host
}

impl ReadEnvelope {
    pub(super) fn prepare(self) -> Result<(ServiceRequest, Option<String>)> {
        let project = self
            .project_id
            .as_deref()
            .map(crate::config::canonical_project_id)
            .transpose()
            .map_err(|_| anyhow::anyhow!("invalid project_id"))?
            .map(str::to_owned);
        ensure!(
            project.is_some() || self.query.permits_global(),
            "select a project for this read"
        );
        let request = self.query.into_request(project.clone());
        let (method, params) = request.wire_parts()?;
        Ok((ServiceRequest::decode(method, params)?, project))
    }
}
impl BrowserRead {
    fn permits_global(&self) -> bool {
        match self {
            Self::Coordination(read) => read.permits_global(),
            Self::Analytics(_) => true,
            Self::Memory(read) => matches!(
                read,
                MemoryRead::Expand { .. }
                    | MemoryRead::Context {
                        scope: MemoryReadScopeArg::Global
                    }
                    | MemoryRead::Search {
                        scope: MemoryReadScopeArg::Global,
                        ..
                    }
            ),
            _ => false,
        }
    }
    fn into_request(self, project: Option<String>) -> ServiceRequest {
        match self {
            Self::Coordination(read) => read.into_request(project),
            Self::Analytics(read) => read.into_request(project),
            Self::Memory(read) => read.into_request(project),
            Self::Issues(read) => read.into_request(project),
            Self::Evals(read) => ServiceRequest::Evals(bound(read.into_action(project))),
            Self::Work(read) => ServiceRequest::Work(bound(read.into_action(project))),
            Self::Concerns(read) => ServiceRequest::Concerns(bound(read.into_action(project))),
        }
    }
}
impl CoordinationRead {
    /// Session identity is project-qualified, so session history always needs a project.
    fn permits_global(&self) -> bool {
        !matches!(
            self,
            Self::CoordinationSessionTimeline { .. } | Self::CoordinationSessionIndex {}
        )
    }
    fn into_request(self, project_id: Option<String>) -> ServiceRequest {
        match self {
            Self::Projects {} => ServiceRequest::Projects,
            Self::Coordination {} => ServiceRequest::Dashboard(DashboardQuery { project_id }),
            Self::CoordinationSessions { scope } => {
                ServiceRequest::Sessions(SessionQuery { project_id, scope })
            }
            Self::CoordinationClaims { scope } => {
                ServiceRequest::Claims(ClaimQuery { project_id, scope })
            }
            Self::CoordinationConflicts { scope } => {
                ServiceRequest::Conflicts(ConflictQuery { project_id, scope })
            }
            Self::CoordinationEvents { limit } => ServiceRequest::Events(EventQuery {
                project_id,
                limit: limit.unwrap_or(64),
            }),
            Self::CoordinationSessionTimeline { session_id } => {
                ServiceRequest::SessionTimeline(SessionTimelineQuery {
                    project_id,
                    session_id,
                })
            }
            Self::CoordinationSessionIndex {} => {
                ServiceRequest::SessionIndex(SessionIndexQuery { project_id })
            }
        }
    }
}
impl AnalyticsRead {
    fn into_request(self, project_id: Option<String>) -> ServiceRequest {
        if matches!(self, Self::Usage {}) {
            return ServiceRequest::Usage(UsageQuery { project_id });
        }
        let command = match self {
            Self::AnalyticsSummary {} => AnalyticsCommand::Summary { project_id },
            Self::AnalyticsCatalog { offset, limit } => AnalyticsCommand::Catalog {
                project_id,
                offset,
                limit,
            },
            Self::AnalyticsOpportunities {} => AnalyticsCommand::Opportunities { project_id },
            Self::AnalyticsExamples {
                category_id,
                resource_key,
                offset,
            } => AnalyticsCommand::Examples {
                project_id,
                category_id,
                resource_key,
                offset,
            },
            Self::AnalyticsCategories {} => AnalyticsCommand::Categories,
            Self::AnalyticsCommands { category_id } => AnalyticsCommand::Commands {
                project_id,
                category_id,
            },
            Self::SkillGet { name } => AnalyticsCommand::Skill { project_id, name },
            Self::Usage {} => unreachable!("legacy usage is selected above"),
        };
        ServiceRequest::Analytics(command)
    }
}
impl MemoryRead {
    fn into_request(self, project_id: Option<String>) -> ServiceRequest {
        let query = match self {
            Self::Status {} => MemoryInspection::Status {
                project_id: project_id.expect("validated project"),
            },
            Self::Context { scope } => MemoryInspection::Context { project_id, scope },
            Self::Search {
                scope,
                regex,
                limit,
            } => MemoryInspection::Search {
                project_id,
                scope,
                regex,
                limit: limit.unwrap_or(50),
            },
            Self::Expand { summary_id } => {
                return ServiceRequest::MemoryExpand(MemorySummaryQuery { summary_id })
            }
        };
        ServiceRequest::MemoryInspect(query)
    }
}
impl IssueRead {
    fn into_request(self, project_id: Option<String>) -> ServiceRequest {
        match self {
            Self::Papercuts { after } => ServiceRequest::Papercuts(PapercutCommand::List {
                context: context(),
                project_id,
                after,
            }),
            Self::PapercutGet { papercut_id } => ServiceRequest::Papercuts(PapercutCommand::Get {
                context: context(),
                project_id,
                papercut_id,
            }),
            Self::SosList {} => ServiceRequest::Sos(SosCommand::List { context: context() }),
            Self::SosGet { issue_id } => ServiceRequest::Sos(SosCommand::Get {
                context: context(),
                issue_id,
            }),
        }
    }
}
impl EvalRead {
    fn into_action(self, project_id: Option<String>) -> EvalAction {
        match self {
            Self::Dashboard { group_id } => EvalAction::Dashboard {
                project_id,
                group_id,
            },
            Self::List { filter } => EvalAction::List {
                project_id,
                kind: filter.kind,
                repository: filter.repository,
                model: filter.model,
                id_prefix: filter.id_prefix,
                search: filter.search,
                after: filter.after,
                limit: filter.limit,
            },
            Self::Get { id } => EvalAction::Get { project_id, id },
            Self::Why { id } => EvalAction::Why {
                project_id,
                id,
                limit: Some(64),
            },
        }
    }
}
impl WorkRead {
    fn into_action(self, project_id: Option<String>) -> WorkAction {
        match self {
            Self::Dashboard {
                work_session_id,
                session_origin,
            } => WorkAction::Dashboard {
                project_id,
                work_session_id,
                session_origin,
            },
            Self::List { filter } => WorkAction::List {
                project_id,
                query: filter,
            },
            Self::Activities { filter } => WorkAction::Activities {
                project_id,
                query: filter,
            },
            Self::Get { subject } => WorkAction::Get {
                project_id,
                subject,
            },
            Self::Observation { id } => WorkAction::Observation { project_id, id },
            Self::Timeline {
                work_session_id,
                parent_activity_id,
                after,
                limit,
            } => WorkAction::Timeline {
                project_id,
                work_session_id,
                parent_activity_id,
                after,
                limit,
            },
            Self::Events {
                subject,
                after,
                limit,
            } => WorkAction::Events {
                project_id,
                subject,
                after,
                limit,
            },
        }
    }
}
impl ConcernRead {
    fn into_action(self, project_id: Option<String>) -> ConcernAction {
        match self {
            Self::Map { filter } => ConcernAction::Map {
                project_id,
                query: filter,
            },
            Self::Dashboard { filter } => ConcernAction::Dashboard {
                project_id,
                query: filter,
            },
            Self::List { filter } => ConcernAction::List {
                project_id,
                query: filter,
            },
            Self::Get { concern_id } => ConcernAction::Get {
                project_id,
                concern_id,
            },
            Self::History {
                concern_id,
                after,
                limit,
            } => ConcernAction::History {
                project_id,
                concern_id,
                after,
                limit,
            },
        }
    }
}
fn context() -> HookContext {
    HookContext {
        agent: "nexus-browser".into(),
        session_id: "read-only-browser".into(),
        project_root: None,
        turn_id: None,
        model: None,
    }
}
fn bound<T>(command: T) -> ContextCommand<T> {
    ContextCommand {
        context: context(),
        command,
    }
}
