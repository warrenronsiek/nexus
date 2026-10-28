// @feature installation
// @feature runtime
// @spec docs/features/installation.md
// @spec docs/features/runtime.md
// @boundary local-update-control-json: validate the peer and exact instance before stopping
use super::{ControlRequest, ControlResponse, DaemonSnapshot, METHOD};
use crate::{runtime::protocol::WireRequest, LoadedConfig};
use anyhow::Result;
use serde_json::Value;
use tokio::sync::watch;

#[derive(Clone)]
pub(in crate::runtime) struct DaemonControl {
    snapshot: DaemonSnapshot,
    shutdown: watch::Sender<bool>,
}
impl DaemonControl {
    pub(in crate::runtime) fn new(loaded: &LoadedConfig) -> Result<Self> {
        let (shutdown, _) = watch::channel(false);
        Ok(Self {
            snapshot: DaemonSnapshot::capture(loaded)?,
            shutdown,
        })
    }
    pub(in crate::runtime) fn subscribe(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }
    pub(in crate::runtime) fn response(
        &self,
        request: &WireRequest,
        peer_uid: Option<u32>,
    ) -> Option<Value> {
        if request.method != METHOD {
            return None;
        }
        let response = if peer_uid != Some(unsafe { libc::geteuid() }) {
            ControlResponse::Rejected {
                reason: "update control requires the daemon owner".into(),
            }
        } else {
            self.respond(&request.params)
        };
        Some(serde_json::to_value(response).expect("typed daemon control response serializable"))
    }
    fn respond(&self, params: &Value) -> ControlResponse {
        match serde_json::from_value::<ControlRequest>(params.clone()) {
            Ok(ControlRequest::Inspect) => ControlResponse::Snapshot {
                snapshot: Box::new(self.snapshot.clone()),
            },
            Ok(ControlRequest::Stop { instance_id })
                if instance_id == self.snapshot.instance_id =>
            {
                let _ = self.shutdown.send(true);
                ControlResponse::Stopping { instance_id }
            }
            Ok(ControlRequest::Stop { .. }) => ControlResponse::Rejected {
                reason: "daemon instance changed; no process was stopped".into(),
            },
            Err(_) => ControlResponse::Rejected {
                reason: "invalid daemon control request".into(),
            },
        }
    }
}
