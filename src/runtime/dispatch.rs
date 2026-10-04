// @feature runtime
// @spec docs/features/runtime.md
// @entrypoint handle
use crate::coordination::api::{ServiceRequest, ServiceResponse};
use crate::coordination::NexusService;
use std::sync::Arc;

pub(super) async fn handle(service: Arc<NexusService>, request: ServiceRequest) -> ServiceResponse {
    tokio::task::spawn_blocking(move || service.handle(request))
        .await
        .unwrap_or_else(ServiceResponse::error)
}
