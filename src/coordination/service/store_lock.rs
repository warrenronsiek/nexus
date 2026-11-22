// @feature coordination
// @feature persistence
// @spec docs/features/coordination.md
// @spec docs/features/persistence.md
use crate::persistence::Store;
use std::sync::{Mutex, MutexGuard, TryLockError, TryLockResult};
use std::time::{Duration, Instant};

pub(super) struct StoreMutex(Mutex<Store>);
pub(super) type SingleStoreGuard<'a> = MutexGuard<'a, Store>;

impl StoreMutex {
    pub(super) fn new(store: Store) -> Self {
        Self(Mutex::new(store))
    }

    pub(super) fn try_lock(&self) -> TryLockResult<SingleStoreGuard<'_>> {
        self.0.try_lock()
    }

    pub(super) fn try_lock_for(&self, timeout: Duration) -> TryLockResult<SingleStoreGuard<'_>> {
        let deadline = Instant::now() + timeout;
        self.try_lock_with_retry(|| {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
            true
        })
    }

    pub(super) fn try_lock_with_retry(
        &self,
        mut retry: impl FnMut() -> bool,
    ) -> TryLockResult<SingleStoreGuard<'_>> {
        loop {
            match self.try_lock() {
                Err(TryLockError::WouldBlock) if retry() => {}
                result => return result,
            }
        }
    }
}

/// A failed acquisition is safe to retry because no lifecycle writes began.
#[derive(Debug)]
pub(super) struct LifecycleStoreBusy;

impl std::fmt::Display for LifecycleStoreBusy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("lifecycle store remained busy before acquisition")
    }
}

impl std::error::Error for LifecycleStoreBusy {}
