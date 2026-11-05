// @feature runtime
// @spec docs/features/runtime.md
// @boundary dynamic-json
use crate::coordination::api::ServiceRequest;
use crate::coordination::client_evidence::ClientFilesystemEvidence;
use crate::coordination::ProjectBinding;
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
    /// Client-reported label for `project_id`; older servers ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) project_name: Option<String>,
    /// Browser inspection reads a selected project without recording activity there.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) inspection: bool,
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
            project_name: None,
            inspection: false,
            client_evidence: None,
        })
    }

    pub(crate) fn named(mut self, project_name: Option<&str>) -> Self {
        self.project_name = project_name.map(str::to_owned);
        self
    }

    pub(crate) fn inspecting(mut self) -> Self {
        self.inspection = true;
        self
    }

    /// Resolves who the request speaks for once the server has validated `project_id`.
    pub(crate) fn binding(&self, project_id: Option<String>) -> ProjectBinding {
        match (self.inspection, project_id) {
            (true, project_id) => ProjectBinding::Inspect { project_id },
            (false, Some(project_id)) => {
                ProjectBinding::remote(project_id, self.project_name.clone())
            }
            (false, None) => ProjectBinding::Local,
        }
    }

    pub(crate) fn reconciliation(project_id: &str, evidence: ClientFilesystemEvidence) -> Self {
        Self {
            method: "client_reconcile".to_owned(),
            params: Value::Object(Default::default()),
            project_id: Some(project_id.to_owned()),
            project_name: None,
            inspection: false,
            client_evidence: Some(evidence),
        }
    }
}
