// @feature delivery-coordination
// @feature runtime
// @spec docs/features/delivery-coordination.md
// @spec docs/features/runtime.md
use super::NexusService;
use crate::delivery::{DeliveryWatch, DeliveryWatchOutcome};
use crate::runtime::delivery_watch::GithubPoll;
use anyhow::{Context, Result};
use chrono::Utc;

impl NexusService {
    pub(crate) fn claim_delivery_watches(&self) -> Result<Vec<DeliveryWatch>> {
        self.maintenance_store()
            .context("delivery watch store unavailable")?
            .claim_delivery_watches(Utc::now())
    }

    pub(crate) fn apply_delivery_watch_poll(
        &self,
        watch: &DeliveryWatch,
        poll: Result<GithubPoll>,
    ) -> Result<()> {
        let outcome = match poll {
            Ok(poll) => DeliveryWatchOutcome::Observed {
                snapshot: Box::new(poll.snapshot),
                provider_updated_at: poll.provider_updated_at,
                digest: poll.digest,
            },
            Err(error) => DeliveryWatchOutcome::Failed {
                reason: error.to_string(),
            },
        };
        self.maintenance_store()
            .context("delivery watch store unavailable")?
            .apply_delivery_watch_poll(watch, outcome)
    }
}
