// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary compatible-cached-work: batch bounded typed observations only when their complete caller contexts match; preserve original cache bytes and reconcile rejected batches individually.
use super::outbox::{single_observation, Pending, PendingPhase};
use super::{
    deliver, disposition, settle, Delivery, HookContext, NexusClient, ServiceRequest, WorkAction,
    WorkCommand, WorkObservationInput,
};
use anyhow::Result;
use std::{path::Path, time::Instant};

const MAX_BATCH_SOURCE_BYTES: usize = 64 * 1024;

enum Progress {
    Settled,
    Deferred,
}

struct CompatibleBatch {
    context: HookContext,
    observations: Vec<WorkObservationInput>,
}

impl CompatibleBatch {
    fn from_pending(pending: &[Pending]) -> Option<Self> {
        let first = pending.first()?;
        if first.phase() != PendingPhase::Record {
            return None;
        }
        let (context, observation) = single_observation(&first.bytes)?;
        let mut batch = Self {
            context,
            observations: vec![observation],
        };
        let mut bytes = first.bytes.len();
        for entry in pending.iter().skip(1) {
            if entry.phase() != PendingPhase::Record
                || bytes + entry.bytes.len() > MAX_BATCH_SOURCE_BYTES
            {
                break;
            }
            let Some((context, observation)) = single_observation(&entry.bytes) else {
                break;
            };
            if batch.context != context {
                break;
            }
            bytes += entry.bytes.len();
            batch.observations.push(observation);
        }
        Some(batch)
    }

    async fn deliver(
        self,
        client: &NexusClient,
        path: &Path,
        entries: &[Pending],
        deadline: Instant,
    ) -> Result<Progress> {
        let request = ServiceRequest::Work(WorkCommand {
            context: self.context,
            command: WorkAction::Record {
                observations: self.observations,
            },
        });
        let response = tokio::time::timeout(
            deadline.saturating_duration_since(Instant::now()),
            client.ensure_and_request(&request),
        )
        .await;
        let Ok(Ok(response)) = response else {
            return Ok(Progress::Deferred);
        };
        match disposition(&response) {
            Delivery::Recorded => {
                for entry in entries {
                    settle(path, entry, Delivery::Recorded)?;
                }
                Ok(Progress::Settled)
            }
            Delivery::Rejected(_) => settle_individually(client, path, entries, deadline).await,
            Delivery::RetryLater => Ok(Progress::Deferred),
        }
    }
}

async fn settle_individually(
    client: &NexusClient,
    path: &Path,
    entries: &[Pending],
    deadline: Instant,
) -> Result<Progress> {
    for entry in entries {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero()
            || matches!(
                deliver(client, path, entry, remaining).await?,
                Delivery::RetryLater
            )
        {
            return Ok(Progress::Deferred);
        }
    }
    Ok(Progress::Settled)
}

pub(super) async fn deliver_page(
    client: &NexusClient,
    path: &Path,
    pending: &[Pending],
    deadline: Instant,
) -> Result<()> {
    let mut index = 0;
    while index < pending.len() && Instant::now() < deadline {
        let batch = CompatibleBatch::from_pending(&pending[index..]);
        let count = batch.as_ref().map_or(1, |batch| batch.observations.len());
        let progress = match batch.filter(|batch| batch.observations.len() > 1) {
            Some(batch) => {
                batch
                    .deliver(client, path, &pending[index..index + count], deadline)
                    .await?
            }
            None => settle_individually(client, path, &pending[index..=index], deadline).await?,
        };
        if matches!(progress, Progress::Deferred) {
            break;
        }
        index += count;
    }
    Ok(())
}
