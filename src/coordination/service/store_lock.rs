// @feature coordination
// @feature persistence
// @spec docs/features/coordination.md
// @spec docs/features/persistence.md
use crate::persistence::Store;
use std::ops::{Deref, DerefMut};
use std::sync::{LockResult, Mutex, MutexGuard, PoisonError, TryLockError, TryLockResult};
use std::time::{Duration, Instant};

pub(in crate::coordination) struct TrackedStoreMutex {
    inner: Mutex<Store>,
    held_since: Mutex<Option<Instant>>,
}

impl TrackedStoreMutex {
    pub(super) fn new(store: Store) -> Self {
        Self {
            inner: Mutex::new(store),
            held_since: Mutex::new(None),
        }
    }

    pub(in crate::coordination) fn lock(&self) -> LockResult<TrackedStoreGuard<'_>> {
        match self.inner.lock() {
            Ok(store) => Ok(self.track(store)),
            Err(error) => Err(PoisonError::new(self.track(error.into_inner()))),
        }
    }

    pub(super) fn try_lock(&self) -> TryLockResult<TrackedStoreGuard<'_>> {
        match self.inner.try_lock() {
            Ok(store) => Ok(self.track(store)),
            Err(TryLockError::WouldBlock) => Err(TryLockError::WouldBlock),
            Err(TryLockError::Poisoned(error)) => Err(TryLockError::Poisoned(PoisonError::new(
                self.track(error.into_inner()),
            ))),
        }
    }

    pub(super) fn try_lock_for(
        &self,
        timeout: Duration,
        retry_interval: Duration,
    ) -> TryLockResult<TrackedStoreGuard<'_>> {
        let deadline = Instant::now() + timeout;
        self.try_lock_with_retry(|| {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(retry_interval);
            true
        })
    }

    pub(super) fn try_lock_with_retry(
        &self,
        mut retry: impl FnMut() -> bool,
    ) -> TryLockResult<TrackedStoreGuard<'_>> {
        loop {
            match self.try_lock() {
                Err(TryLockError::WouldBlock) if retry() => {}
                result => return result,
            }
        }
    }

    #[cfg(feature = "postgres")]
    pub(super) fn held_for_at_least(&self, threshold: Duration) -> bool {
        self.held_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|started| started.elapsed() >= threshold)
    }

    #[cfg(feature = "postgres")]
    pub(super) fn is_poisoned(&self) -> bool {
        self.inner.is_poisoned()
    }

    fn track<'a>(&'a self, store: MutexGuard<'a, Store>) -> TrackedStoreGuard<'a> {
        *self
            .held_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
        TrackedStoreGuard {
            store,
            held_since: &self.held_since,
        }
    }
}

pub(in crate::coordination) struct TrackedStoreGuard<'a> {
    store: MutexGuard<'a, Store>,
    held_since: &'a Mutex<Option<Instant>>,
}

impl Deref for TrackedStoreGuard<'_> {
    type Target = Store;

    fn deref(&self) -> &Self::Target {
        &self.store
    }
}

impl DerefMut for TrackedStoreGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.store
    }
}

impl Drop for TrackedStoreGuard<'_> {
    fn drop(&mut self) {
        *self
            .held_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
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
