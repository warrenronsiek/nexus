// @feature runtime
// @feature agent-memory
// @feature papercuts
// @spec docs/features/runtime.md
// @spec docs/features/agent-memory.md
// @spec docs/features/papercuts.md
// @entrypoint handle
use crate::coordination::api::{MemoryConsolidationReport, ServiceRequest, ServiceResponse};
use crate::coordination::{NexusService, PapercutDispatch};
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
    let (response, review_job) = match request {
        ServiceRequest::Papercuts(command) => {
            let request_service = service.clone();
            match tokio::task::spawn_blocking(move || request_service.dispatch_papercuts(command))
                .await
            {
                Ok(PapercutDispatch {
                    response,
                    review_job,
                }) => (response, review_job),
                Err(error) => (ServiceResponse::error(error), None),
            }
        }
        request => {
            let request_service = service.clone();
            (
                tokio::task::spawn_blocking(move || request_service.handle(request))
                    .await
                    .unwrap_or_else(ServiceResponse::error),
                None,
            )
        }
    };
    if let Some(review_job) = review_job {
        drop(tokio::task::spawn_blocking(move || {
            if let Err(error) = service.review_papercut(&review_job) {
                let _ = service.record_papercut_review_failure(&review_job, &error);
            }
        }));
    }
    response
}
