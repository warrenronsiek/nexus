// @feature coordination
// @spec docs/features/coordination.md
use super::NexusService;
use anyhow::Result;

const MAX_CONFLICT_CLOSURE_PASSES: usize = 32;

impl NexusService {
    /// Ends every stored-open conflict whose claims were released or expired, one bounded
    /// pass per store acquisition so lifecycle hooks can interleave.
    pub(crate) fn end_stale_conflicts(&self) -> Result<usize> {
        let mut ended = 0_usize;
        for _ in 0..MAX_CONFLICT_CLOSURE_PASSES {
            let pass = self
                .store
                .lock()
                .map_err(|_| anyhow::anyhow!("store lock poisoned"))?
                .end_stale_conflicts(None)?;
            ended = ended.saturating_add(pass);
            if pass < crate::persistence::MAX_CONFLICT_CLOSURES {
                break;
            }
        }
        Ok(ended)
    }

    /// Expiry has no write of its own, so closure shares the reconciliation cadence.
    pub(crate) fn conflict_closure_seconds(&self) -> u64 {
        self.loaded.config.coordination.reconcile_seconds.max(1)
    }
}
