// @feature runtime
// @feature persistence
// @spec docs/features/runtime.md
// @spec docs/features/persistence.md
use crate::coordination::DatabaseOperation;
use std::{sync::Arc, time::Duration};
use thiserror::Error;
use tokio::sync::Semaphore;

#[derive(Clone)]
pub(in crate::runtime) struct BlockingExecutionGate {
    slots: Arc<Semaphore>,
    queue_timeout: Duration,
    database_slots: [Arc<Semaphore>; DatabaseOperation::ALL.len()],
}

#[derive(Debug, Error)]
pub(in crate::runtime) enum BlockingExecutionError {
    #[error("Nexus blocking execution capacity is temporarily exhausted")]
    AtCapacity,
    #[error("Nexus blocking worker failed: {0}")]
    Worker(#[from] tokio::task::JoinError),
}

impl BlockingExecutionGate {
    pub(in crate::runtime) fn new(capacity: usize, queue_timeout: Duration) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(capacity)),
            queue_timeout,
            database_slots: DatabaseOperation::ALL
                .map(|operation| Arc::new(Semaphore::new(operation.capacity()))),
        }
    }

    pub(in crate::runtime) async fn run<F, T>(&self, work: F) -> Result<T, BlockingExecutionError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        self.run_classified(None, work).await
    }

    pub(in crate::runtime) async fn run_classified<F, T>(
        &self,
        operation: Option<DatabaseOperation>,
        work: F,
    ) -> Result<T, BlockingExecutionError>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let database_permit = match operation {
            Some(operation) => Some(
                tokio::time::timeout(
                    self.queue_timeout.min(operation.wait()),
                    self.database_slots[operation as usize]
                        .clone()
                        .acquire_owned(),
                )
                .await
                .map_err(|_| BlockingExecutionError::AtCapacity)?
                .map_err(|_| BlockingExecutionError::AtCapacity)?,
            ),
            None => None,
        };
        let permit = tokio::time::timeout(self.queue_timeout, self.slots.clone().acquire_owned())
            .await
            .map_err(|_| BlockingExecutionError::AtCapacity)?
            .map_err(|_| BlockingExecutionError::AtCapacity)?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let _database_permit = database_permit;
            work()
        })
        .await
        .map_err(BlockingExecutionError::from)
    }
}
