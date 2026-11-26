// @feature agent-traces
// @spec docs/features/agent-traces.md
// @feature persistence
// @feature coordination
// @spec docs/features/persistence.md
// @spec docs/features/coordination.md
use super::store_lock::{LifecycleStoreBusy, SingleStoreGuard, StoreMutex};
use crate::persistence::Store;
use anyhow::Result;
use std::ops::{Deref, DerefMut};
use std::sync::TryLockError;
use std::time::Duration;

#[derive(Clone, Copy)]
#[repr(usize)]
pub(crate) enum DatabaseOperation {
    Interactive,
    History,
    Write,
    Lifecycle,
    Maintenance,
    TraceIngest,
    TraceProjection,
}

impl DatabaseOperation {
    pub(crate) const ALL: [Self; 7] = [
        Self::Interactive,
        Self::History,
        Self::Write,
        Self::Lifecycle,
        Self::Maintenance,
        Self::TraceIngest,
        Self::TraceProjection,
    ];

    pub(crate) fn capacity(self) -> usize {
        match self {
            Self::Interactive => 3,
            Self::History => 2,
            Self::TraceIngest => 4,
            Self::Write | Self::Lifecycle | Self::Maintenance | Self::TraceProjection => 1,
        }
    }

    #[cfg(feature = "postgres")]
    fn baseline_connections() -> u32 {
        Self::ALL
            .into_iter()
            .filter(|operation| !matches!(operation, Self::TraceIngest | Self::TraceProjection))
            .map(Self::capacity)
            .sum::<usize>() as u32
    }

    pub(crate) fn wait(self) -> Duration {
        match self {
            Self::Lifecycle => Duration::from_millis(100),
            _ => Duration::from_secs(5),
        }
    }
}

pub(super) enum Database {
    Single(StoreMutex),
    #[cfg(feature = "postgres")]
    Postgres(Box<postgres::PostgresDatabase>),
}

impl Database {
    pub(super) fn is_pooled(&self) -> bool {
        match self {
            Self::Single(_) => false,
            #[cfg(feature = "postgres")]
            Self::Postgres(_) => true,
        }
    }

    pub(super) fn single(store: Store) -> Self {
        Self::Single(StoreMutex::new(store))
    }

    #[cfg(test)]
    pub(super) fn single_for_test(&self) -> &StoreMutex {
        match self {
            Self::Single(store) => store,
            #[cfg(feature = "postgres")]
            Self::Postgres(_) => panic!("fixture requires the single-connection store"),
        }
    }

    pub(super) fn connect(url: &str) -> Result<Self> {
        #[cfg(feature = "postgres")]
        return Ok(Self::Postgres(Box::new(
            postgres::PostgresDatabase::connect(url)?,
        )));
        #[cfg(feature = "sqlite")]
        Ok(Self::single(Store::connect(url)?))
    }

    pub(super) fn acquire(&self, operation: DatabaseOperation) -> Result<DatabaseLease<'_>> {
        match self {
            Self::Single(store) => match store.try_lock_for(operation.wait()) {
                Ok(guard) => Ok(DatabaseLease::Single(guard)),
                Err(TryLockError::WouldBlock) => Err(at_capacity(operation)),
                Err(TryLockError::Poisoned(_)) => anyhow::bail!("store lock poisoned"),
            },
            #[cfg(feature = "postgres")]
            Self::Postgres(database) => database.acquire(operation).map(DatabaseLease::Postgres),
        }
    }

    pub(super) fn readiness(&self, window: i64, stall: Duration) -> Result<&'static str> {
        #[cfg(not(feature = "postgres"))]
        let _ = stall;
        match self {
            Self::Single(store) => match store.try_lock() {
                Ok(mut store) => {
                    store.counts(window)?;
                    Ok("ready")
                }
                Err(TryLockError::WouldBlock) => Ok("busy"),
                Err(TryLockError::Poisoned(_)) => anyhow::bail!("store lock poisoned"),
            },
            #[cfg(feature = "postgres")]
            Self::Postgres(database) => database.readiness(stall),
        }
    }
}

impl super::NexusService {
    pub(in crate::coordination) fn lifecycle_store(&self) -> Result<DatabaseLease<'_>> {
        self.store.acquire(DatabaseOperation::Lifecycle)
    }

    pub(in crate::coordination) fn read_store(&self) -> Result<DatabaseLease<'_>> {
        self.store.acquire(DatabaseOperation::Interactive)
    }

    pub(super) fn history_store(&self) -> Result<DatabaseLease<'_>> {
        self.store.acquire(DatabaseOperation::History)
    }

    pub(super) fn write_store(&self) -> Result<DatabaseLease<'_>> {
        self.store.acquire(DatabaseOperation::Write)
    }

    pub(crate) fn admission(
        &self,
        request: &crate::coordination::api::ServiceRequest,
    ) -> Option<DatabaseOperation> {
        self.store
            .is_pooled()
            .then(|| request.database_operation())
            .flatten()
    }

    pub(crate) fn append_trace(
        &self,
        project: &str,
        batch: crate::agent_traces::TraceBatch,
    ) -> Result<crate::agent_traces::TraceReceipt> {
        self.store
            .acquire(DatabaseOperation::TraceIngest)?
            .trace_append_validated(project, batch)
    }

    pub(crate) fn project_traces(&self, limit: usize) -> Result<usize> {
        self.store
            .acquire(DatabaseOperation::TraceProjection)?
            .trace_project_pending(limit)
    }

    pub(crate) fn expire_trace_payloads(&self, cutoff: &str, limit: usize) -> Result<usize> {
        self.store
            .acquire(DatabaseOperation::TraceProjection)?
            .trace_expire_payloads(cutoff, limit)
    }

    pub(super) fn maintenance_store(&self) -> Result<DatabaseLease<'_>> {
        self.store.acquire(DatabaseOperation::Maintenance)
    }
}

fn at_capacity(operation: DatabaseOperation) -> anyhow::Error {
    match operation {
        DatabaseOperation::Lifecycle => LifecycleStoreBusy.into(),
        _ => anyhow::anyhow!("database operation capacity is exhausted"),
    }
}

pub(in crate::coordination) enum DatabaseLease<'a> {
    Single(SingleStoreGuard<'a>),
    #[cfg(feature = "postgres")]
    Postgres(postgres::PostgresLease<'a>),
}

impl Deref for DatabaseLease<'_> {
    type Target = Store;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Single(store) => store,
            #[cfg(feature = "postgres")]
            Self::Postgres(store) => &store.connection,
        }
    }
}

impl DerefMut for DatabaseLease<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Single(store) => store,
            #[cfg(feature = "postgres")]
            Self::Postgres(store) => &mut store.connection,
        }
    }
}

#[cfg(feature = "postgres")]
mod postgres {
    use super::{at_capacity, DatabaseOperation};
    use crate::persistence::{DatabaseReadinessProbe, Store};
    use anyhow::{Context, Result};
    use diesel::pg::PgConnection;
    use diesel::r2d2::{ConnectionManager, ManageConnection, Pool, PooledConnection};
    use std::sync::{Condvar, Mutex};
    use std::time::{Duration, Instant};

    pub(in crate::coordination::service) struct PostgresDatabase {
        pool: Pool<StoreManager>,
        lanes: [Lane; DatabaseOperation::ALL.len()],
        pub(in crate::coordination::service) probe: Mutex<DatabaseReadinessProbe>,
    }

    pub(super) struct StoreManager(ConnectionManager<PgConnection>);

    impl ManageConnection for StoreManager {
        type Connection = Store;
        type Error = diesel::r2d2::Error;

        fn connect(&self) -> Result<Store, Self::Error> {
            self.0.connect().map(Store::from_postgres)
        }

        fn is_valid(&self, store: &mut Store) -> Result<(), Self::Error> {
            self.0.is_valid(store.postgres_connection())
        }

        fn has_broken(&self, store: &mut Store) -> bool {
            self.0.has_broken(store.postgres_connection())
        }
    }

    impl PostgresDatabase {
        pub(super) fn connect(url: &str) -> Result<Self> {
            Store::migrate_postgres(url)?;
            let size: usize = DatabaseOperation::ALL
                .iter()
                .map(|operation| operation.capacity())
                .sum();
            let pool = Pool::builder()
                .max_size(size as u32)
                .min_idle(Some(DatabaseOperation::baseline_connections()))
                .connection_timeout(Duration::from_secs(5))
                .build(StoreManager(ConnectionManager::new(url)))
                .context("open PostgreSQL connection pool")?;
            Ok(Self {
                pool,
                lanes: DatabaseOperation::ALL.map(|operation| Lane::new(operation.capacity())),
                probe: Mutex::new(DatabaseReadinessProbe::connect(url)?),
            })
        }

        pub(super) fn acquire(&self, operation: DatabaseOperation) -> Result<PostgresLease<'_>> {
            let deadline = Instant::now() + operation.wait();
            let slot = self.lanes[operation as usize]
                .acquire(deadline)
                .ok_or_else(|| at_capacity(operation))?;
            let connection = self
                .pool
                .get_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|_| at_capacity(operation))?;
            Ok(PostgresLease {
                connection,
                _slot: slot,
            })
        }

        pub(super) fn readiness(&self, stall: Duration) -> Result<&'static str> {
            anyhow::ensure!(
                !self.lanes.iter().any(|lane| lane.stalled(stall)),
                "store stalled beyond the {}ms readiness threshold",
                stall.as_millis(),
            );
            let mut probe = self
                .probe
                .try_lock()
                .map_err(|error| anyhow::anyhow!("readiness probe unavailable: {error}"))?;
            probe.check()?;
            Ok("ready")
        }
    }

    pub(in crate::coordination) struct PostgresLease<'a> {
        pub(super) connection: PooledConnection<StoreManager>,
        _slot: Slot<'a>,
    }

    struct Lane {
        started: Mutex<Vec<Option<Instant>>>,
        available: Condvar,
    }

    impl Lane {
        fn new(capacity: usize) -> Self {
            Self {
                started: Mutex::new(vec![None; capacity]),
                available: Condvar::new(),
            }
        }

        fn acquire(&self, deadline: Instant) -> Option<Slot<'_>> {
            let slots = self.started.lock().ok()?;
            let (mut slots, _) = self
                .available
                .wait_timeout_while(
                    slots,
                    deadline.saturating_duration_since(Instant::now()),
                    |slots| slots.iter().all(Option::is_some),
                )
                .ok()?;
            let index = slots.iter().position(Option::is_none)?;
            slots[index] = Some(Instant::now());
            Some(Slot { lane: self, index })
        }

        fn stalled(&self, threshold: Duration) -> bool {
            self.started.lock().map_or(true, |slots| {
                slots
                    .iter()
                    .flatten()
                    .any(|started| started.elapsed() >= threshold)
            })
        }
    }

    struct Slot<'a> {
        lane: &'a Lane,
        index: usize,
    }

    impl Drop for Slot<'_> {
        fn drop(&mut self) {
            if let Ok(mut slots) = self.lane.started.lock() {
                slots[self.index] = None;
                self.lane.available.notify_one();
            }
        }
    }
}
