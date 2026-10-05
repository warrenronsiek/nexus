// @feature runtime
// @feature agent-memory
// @spec docs/features/runtime.md
// @spec docs/features/agent-memory.md
// @entrypoint handle
use crate::coordination::api::{MemoryConsolidationReport, ServiceRequest, ServiceResponse};
use crate::coordination::NexusService;
use std::sync::Arc;

pub(super) async fn handle(service: Arc<NexusService>, request: ServiceRequest) -> ServiceResponse {
    if matches!(&request, ServiceRequest::MemoryConsolidate) {
        drop(tokio::task::spawn_blocking(move || {
            service.handle(ServiceRequest::MemoryConsolidate)
        }));
        return ServiceResponse::MemoryConsolidation(MemoryConsolidationReport {
            ok: true,
            in_progress: true,
            ..MemoryConsolidationReport::default()
        });
    }
    tokio::task::spawn_blocking(move || service.handle(request))
        .await
        .unwrap_or_else(ServiceResponse::error)
}
