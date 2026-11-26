// @feature coordination
// @feature persistence
// @spec docs/features/coordination.md
// @spec docs/features/persistence.md
// @feature agent-traces
// @spec docs/features/agent-traces.md
use super::DatabaseOperation::{self, History, Interactive, Lifecycle, Write};
use crate::analytics::types::AnalyticsCommand;
use crate::audit::AuditAction;
use crate::code_concerns::ConcernAction;
use crate::coordination::api::{EvalAction, PapercutCommand, ServiceRequest, SosCommand};
use crate::delivery::DeliveryAction;
use crate::work_history::WorkAction;

pub(super) trait StorageOperation {
    fn database_operation(&self) -> DatabaseOperation;
}

impl ServiceRequest {
    pub(crate) fn database_operation(&self) -> Option<DatabaseOperation> {
        Some(match self {
            Self::UserPrompt(_)
            | Self::ToolHook { .. }
            | Self::SkillUse(_)
            | Self::ScriptUse(_)
            | Self::SessionStop(_) => Lifecycle,
            Self::Resolve(_)
            | Self::Release(_)
            | Self::MemoryAdd(_)
            | Self::MemoryInvalidate(_) => Write,
            Self::Papercuts(command) => command.database_operation(),
            Self::Sos(command) => command.database_operation(),
            Self::Analytics(command) => command.database_operation(),
            Self::Concerns(command) => command.command.database_operation(),
            Self::Evals(command) => command.command.database_operation(),
            Self::Work(command) => command.command.database_operation(),
            Self::Delivery(command) => command.command.database_operation(),
            Self::Audit(command) => command.command.database_operation(),
            Self::Traces(_) => History,
            Self::Config | Self::UiPreferences | Self::Analyze(_) | Self::MemoryConsolidate => {
                return None
            }
            _ => Interactive,
        })
    }
}

impl StorageOperation for EvalAction {
    fn database_operation(&self) -> DatabaseOperation {
        match self {
            Self::Record { .. }
            | Self::BlobBegin { .. }
            | Self::BlobPut { .. }
            | Self::BlobAbort { .. } => Write,
            Self::Dashboard { .. } | Self::Why { .. } | Self::BlobSeal { .. } => History,
            _ => Interactive,
        }
    }
}

impl StorageOperation for ConcernAction {
    fn database_operation(&self) -> DatabaseOperation {
        match self {
            Self::Record { .. }
            | Self::Import { .. }
            | Self::Respond { .. }
            | Self::PublishMeasure { .. } => Write,
            Self::Map { .. } | Self::Dashboard { .. } | Self::Dump { .. } | Self::Export { .. } => {
                History
            }
            _ => Interactive,
        }
    }
}

impl StorageOperation for WorkAction {
    fn database_operation(&self) -> DatabaseOperation {
        match self {
            Self::Record { .. } | Self::Import { .. } => Write,
            Self::Dashboard { .. } | Self::Export { .. } => History,
            _ => Interactive,
        }
    }
}

impl StorageOperation for DeliveryAction {
    fn database_operation(&self) -> DatabaseOperation {
        match self {
            Self::Record { .. }
            | Self::Import { .. }
            | Self::Remediate { .. }
            | Self::Acknowledge { .. } => Write,
            Self::Export { .. } => History,
            _ => Interactive,
        }
    }
}

impl StorageOperation for AuditAction {
    fn database_operation(&self) -> DatabaseOperation {
        match self {
            Self::Append { .. } => Write,
            Self::Journal { .. } => History,
            _ => Interactive,
        }
    }
}

impl StorageOperation for AnalyticsCommand {
    fn database_operation(&self) -> DatabaseOperation {
        match self {
            Self::ReserveClassification { .. }
            | Self::Classify { .. }
            | Self::ClassificationFailed { .. }
            | Self::Inventory { .. }
            | Self::RenameCategory { .. }
            | Self::MergeCategory { .. } => Write,
            Self::Summary { .. }
            | Self::Catalog { .. }
            | Self::Opportunities { .. }
            | Self::Examples { .. }
            | Self::Commands { .. } => History,
            _ => Interactive,
        }
    }
}

impl StorageOperation for PapercutCommand {
    fn database_operation(&self) -> DatabaseOperation {
        match self {
            Self::List { .. } | Self::Get { .. } => Interactive,
            Self::Dump { .. } => History,
            _ => Write,
        }
    }
}

impl StorageOperation for SosCommand {
    fn database_operation(&self) -> DatabaseOperation {
        match self {
            Self::List { .. } | Self::Get { .. } => Interactive,
            Self::Dump { .. } => History,
            _ => Write,
        }
    }
}
