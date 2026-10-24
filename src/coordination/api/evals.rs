// @feature evals
// @feature runtime
// @spec docs/features/evals.md
// @spec docs/features/runtime.md
// @boundary generated-command-schema-and-service-json
use crate::evals::{
    EvalBlob, EvalHistory, EvalKind, EvalPage, EvalQuery, EvalReceipt, EvalRecord, EvalRecordInput,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub type EvalCommand = super::ContextCommand<EvalAction>;

/// Recording and retrieval only. Workload execution belongs to the producer.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum EvalAction {
    Record {
        records: Vec<EvalRecordInput>,
    },
    List {
        #[serde(default)]
        project_id: Option<String>,
        #[serde(default)]
        kind: Option<EvalKind>,
        #[serde(default)]
        repository: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        search: Option<String>,
        #[serde(default)]
        after: Option<String>,
        #[serde(default)]
        limit: Option<u32>,
    },
    Get {
        id: String,
        #[serde(default)]
        project_id: Option<String>,
    },
    Why {
        id: String,
        #[serde(default)]
        project_id: Option<String>,
        #[serde(default)]
        limit: Option<u32>,
    },
    BlobBegin {
        digest: String,
        byte_size: u64,
    },
    BlobPut {
        digest: String,
        index: u32,
        data: String,
    },
    BlobSeal {
        digest: String,
    },
    BlobAbort {
        digest: String,
    },
    BlobInfo {
        digest: String,
        #[serde(default)]
        project_id: Option<String>,
    },
    BlobGet {
        digest: String,
        index: u32,
        #[serde(default)]
        project_id: Option<String>,
    },
}

impl EvalAction {
    pub(crate) fn read_project(&self) -> Option<&str> {
        match self {
            Self::List { project_id, .. }
            | Self::Get { project_id, .. }
            | Self::Why { project_id, .. }
            | Self::BlobGet { project_id, .. }
            | Self::BlobInfo { project_id, .. } => project_id.as_deref(),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum EvalResponse {
    Recorded {
        ok: bool,
        #[serde(flatten)]
        receipt: EvalReceipt,
    },
    Listed {
        ok: bool,
        #[serde(flatten)]
        page: EvalPage,
    },
    Detailed {
        ok: bool,
        record: Option<Box<EvalRecord>>,
    },
    History {
        ok: bool,
        #[serde(flatten)]
        history: EvalHistory,
    },
    Blob {
        ok: bool,
        blob: EvalBlob,
    },
    BlobChunk {
        ok: bool,
        data: Option<String>,
    },
    BlobInfo {
        ok: bool,
        blob: Option<EvalBlob>,
    },
    BlobAborted {
        ok: bool,
        removed: bool,
    },
}

pub(crate) fn list_query(action: EvalAction) -> Option<EvalQuery> {
    if let EvalAction::List {
        kind,
        repository,
        model,
        search,
        after,
        limit,
        ..
    } = action
    {
        Some(EvalQuery {
            kind,
            repository,
            model,
            search,
            after,
            limit,
        })
    } else {
        None
    }
}
