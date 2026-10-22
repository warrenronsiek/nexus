// @feature evals
// @spec docs/features/evals.md
use super::records::EvalBody;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "scope", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssessmentSubject {
    Run {
        id: String,
    },
    Trial {
        id: String,
    },
    Attempt {
        id: String,
    },
    Call {
        id: String,
    },
    Component {
        attempt_id: String,
        instance: String,
        #[serde(default)]
        call_ids: Vec<String>,
    },
    Routing {
        trace_id: String,
    },
}

impl EvalBody {
    pub(crate) fn assessment_subject(&self) -> Option<&AssessmentSubject> {
        match self {
            Self::Grade(value) => Some(&value.subject),
            Self::Measurement(value) => Some(&value.subject),
            _ => None,
        }
    }
}
