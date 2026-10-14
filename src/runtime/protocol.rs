// @feature runtime
// @spec docs/features/runtime.md
// @boundary dynamic-json
use crate::coordination::api::ServiceRequest;
use crate::coordination::client_evidence::ClientFilesystemEvidence;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct WireRequest {
    pub(crate) method: String,
    #[serde(default)]
    pub(crate) params: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) client_evidence: Option<ClientFilesystemEvidence>,
}

impl WireRequest {
    pub(crate) fn from_service(request: &ServiceRequest, project_id: Option<&str>) -> Result<Self> {
        let (method, params) = request.wire_parts()?;
        Ok(Self {
            method: method.to_owned(),
            params,
            project_id: project_id.map(str::to_owned),
            client_evidence: None,
        })
    }

    pub(crate) fn reconciliation(project_id: &str, evidence: ClientFilesystemEvidence) -> Self {
        Self {
            method: "client_reconcile".to_owned(),
            params: Value::Object(Default::default()),
            project_id: Some(project_id.to_owned()),
            client_evidence: Some(evidence),
        }
    }
}
