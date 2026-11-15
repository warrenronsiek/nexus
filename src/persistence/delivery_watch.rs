// @feature persistence
// @feature delivery-coordination
// @spec docs/features/persistence.md
// @spec docs/features/delivery-coordination.md
use super::{backend::DbConnection, schema::delivery_watches, Store};
use crate::{delivery::*, work_history::WorkActor};
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use diesel::prelude::*;

#[derive(Queryable, Selectable, Insertable)]
#[diesel(table_name = delivery_watches)]
struct WatchRow {
    project_id: String,
    id: String,
    pull_request_id: String,
    registration_json: String,
    actor_json: String,
    generation: String,
    enabled: i32,
    next_poll_at: String,
    last_attempt_at: Option<String>,
    last_success_at: Option<String>,
    last_error: Option<String>,
    last_digest: Option<String>,
    provider_updated_at: Option<String>,
}
impl WatchRow {
    fn registration(&self) -> Result<DeliveryWatchRegistration> {
        let registration: DeliveryWatchRegistration =
            serde_json::from_str(&self.registration_json)?;
        registration.validate()?;
        anyhow::ensure!(
            self.id == registration.id && self.pull_request_id == registration.pull_request_id,
            "watch scalar identity differs from registration"
        );
        anyhow::ensure!(matches!(self.enabled, 0 | 1), "invalid watch enabled state");
        Ok(registration)
    }

    fn decode(self) -> Result<DeliveryWatch> {
        let registration = self.registration()?;
        let provenance: WorkActor = serde_json::from_str(&self.actor_json)?;
        provenance.validate()?;
        let times = WatchTimes::decode(&self)?;
        Ok(DeliveryWatch {
            registration,
            project_id: self.project_id,
            provenance,
            generation: self.generation,
            enabled: self.enabled == 1,
            next_poll_at: times.next_poll_at,
            last_attempt_at: times.last_attempt_at,
            last_success_at: times.last_success_at,
            last_error: self.last_error,
            last_digest: self.last_digest,
            provider_updated_at: times.provider_updated_at,
        })
    }
}

struct WatchTimes {
    next_poll_at: DateTime<Utc>,
    last_attempt_at: Option<DateTime<Utc>>,
    last_success_at: Option<DateTime<Utc>>,
    provider_updated_at: Option<DateTime<Utc>>,
}
impl WatchTimes {
    fn decode(row: &WatchRow) -> Result<Self> {
        Ok(Self {
            next_poll_at: row.next_poll_at.parse()?,
            last_attempt_at: row
                .last_attempt_at
                .as_ref()
                .map(|v| v.parse())
                .transpose()?,
            last_success_at: row
                .last_success_at
                .as_ref()
                .map(|v| v.parse())
                .transpose()?,
            provider_updated_at: row
                .provider_updated_at
                .as_ref()
                .map(|v| v.parse())
                .transpose()?,
        })
    }
}

pub(crate) fn validate_snapshot(connection: &mut DbConnection) -> Result<()> {
    let mut project = String::new();
    let mut id = String::new();
    loop {
        let rows = delivery_watches::table
            .filter(
                delivery_watches::project_id
                    .gt(&project)
                    .or(delivery_watches::project_id
                        .eq(&project)
                        .and(delivery_watches::id.gt(&id))),
            )
            .order((
                delivery_watches::project_id.asc(),
                delivery_watches::id.asc(),
            ))
            .limit(32)
            .select(WatchRow::as_select())
            .load::<WatchRow>(connection)?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            project = row.project_id.clone();
            id = row.id.clone();
            let watch = row.decode()?;
            validate_watch_history(connection, &watch)?;
        }
    }
    validate_registration_coverage(connection)
}

fn validate_registration_coverage(connection: &mut DbConnection) -> Result<()> {
    use super::schema::delivery_observations;
    let mut project = String::new();
    let mut ordinal = 0_i64;
    loop {
        let rows = delivery_observations::table
            .filter(delivery_observations::subject_kind.eq("watch"))
            .filter(
                delivery_observations::project_id.gt(&project).or(
                    delivery_observations::project_id
                        .eq(&project)
                        .and(delivery_observations::ordinal.gt(ordinal)),
                ),
            )
            .order((
                delivery_observations::project_id.asc(),
                delivery_observations::ordinal.asc(),
            ))
            .limit(32)
            .select((
                delivery_observations::project_id,
                delivery_observations::ordinal,
                delivery_observations::payload_json,
            ))
            .load::<(String, i64, String)>(connection)?;
        if rows.is_empty() {
            return Ok(());
        }
        for (next_project, next_ordinal, payload) in rows {
            project = next_project;
            ordinal = next_ordinal;
            let input: DeliveryObservationInput = serde_json::from_str(&payload)?;
            if let DeliveryEvent::WatchRegistered { watch } = input.body {
                let exists = delivery_watches::table
                    .find((&project, &watch.id))
                    .select(delivery_watches::id)
                    .first::<String>(connection)
                    .optional()?
                    .is_some();
                anyhow::ensure!(exists, "watch registration has no scheduling projection");
            }
        }
    }
}

fn validate_watch_history(connection: &mut DbConnection, watch: &DeliveryWatch) -> Result<()> {
    validate_source(connection, &watch.project_id, &watch.registration)?;
    let history = watch_history(connection, watch)?;
    let (registration, actor) = history
        .registration
        .context("watch has no registration history")?;
    anyhow::ensure!(
        history.generation.as_ref() == Some(&watch.generation) && watch.enabled == history.enabled,
        "watch generation differs from latest history"
    );
    anyhow::ensure!(
        serde_json::to_value(registration)? == serde_json::to_value(&watch.registration)?
            && actor == watch.provenance,
        "watch registration differs from retained history"
    );
    anyhow::ensure!(
        watch
            .last_error
            .as_ref()
            .is_none_or(|value| value.len() <= 512),
        "watch error exceeds bounds"
    );
    anyhow::ensure!(
        watch
            .last_digest
            .as_ref()
            .is_none_or(|value| value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())),
        "invalid watch digest"
    );
    Ok(())
}

#[derive(Default)]
struct WatchHistory {
    registration: Option<(DeliveryWatchRegistration, WorkActor)>,
    generation: Option<String>,
    enabled: bool,
}
impl WatchHistory {
    fn observe(&mut self, watch_id: &str, row: WatchHistoryRow) -> Result<()> {
        let (_, event_id, payload, provenance) = row;
        let input: DeliveryObservationInput = serde_json::from_str(&payload)?;
        match input.body {
            DeliveryEvent::WatchRegistered { watch: value } if value.id == watch_id => {
                self.registration = Some((value, serde_json::from_str::<WorkActor>(&provenance)?));
                self.generation = Some(event_id);
                self.enabled = true;
            }
            DeliveryEvent::WatchStopped { watch_id: id } if id == watch_id => {
                self.generation = Some(event_id);
                self.enabled = false;
            }
            _ => {}
        }
        Ok(())
    }
}
type WatchHistoryRow = (i64, String, String, String);
fn watch_history(connection: &mut DbConnection, watch: &DeliveryWatch) -> Result<WatchHistory> {
    use super::schema::delivery_observations;
    let mut after = 0_i64;
    let mut history = WatchHistory::default();
    loop {
        let rows = delivery_observations::table
            .filter(delivery_observations::project_id.eq(&watch.project_id))
            .filter(delivery_observations::subject_kind.eq("watch"))
            .filter(delivery_observations::ordinal.gt(after))
            .order(delivery_observations::ordinal.asc())
            .limit(32)
            .select((
                delivery_observations::ordinal,
                delivery_observations::id,
                delivery_observations::payload_json,
                delivery_observations::provenance_json,
            ))
            .load::<(i64, String, String, String)>(connection)?;
        if rows.is_empty() {
            break;
        }
        for row in rows {
            after = row.0;
            history.observe(&watch.registration.id, row)?;
        }
    }
    Ok(history)
}

pub(crate) fn apply_watch_event(
    connection: &mut DbConnection,
    observation: &DeliveryObservation,
) -> Result<()> {
    let project = &observation.project_id;
    match &observation.input.body {
        DeliveryEvent::WatchRegistered { watch } => {
            watch.validate()?;
            validate_source(connection, project, watch)
                .map_err(|error| DeliveryRejection::Invalid(error.to_string()))?;
            let row = registered_row(observation, watch)?;
            save_registration(connection, &row)?;
        }
        DeliveryEvent::WatchStopped { watch_id } => {
            diesel::update(delivery_watches::table.find((project, watch_id)))
                .set((
                    delivery_watches::enabled.eq(0),
                    delivery_watches::generation.eq(&observation.input.id),
                ))
                .execute(connection)?;
        }
        _ => {}
    }
    Ok(())
}

fn registered_row(
    observation: &DeliveryObservation,
    watch: &DeliveryWatchRegistration,
) -> Result<WatchRow> {
    Ok(WatchRow {
        project_id: observation.project_id.clone(),
        id: watch.id.clone(),
        pull_request_id: watch.pull_request_id.clone(),
        registration_json: serde_json::to_string(watch)?,
        actor_json: serde_json::to_string(&observation.provenance)?,
        generation: observation.input.id.clone(),
        enabled: 1,
        next_poll_at: observation.recorded_at.to_rfc3339(),
        last_attempt_at: None,
        last_success_at: None,
        last_error: None,
        last_digest: None,
        provider_updated_at: None,
    })
}
fn save_registration(connection: &mut DbConnection, row: &WatchRow) -> Result<()> {
    diesel::insert_into(delivery_watches::table)
        .values(row)
        .on_conflict((delivery_watches::project_id, delivery_watches::id))
        .do_update()
        .set((
            delivery_watches::pull_request_id.eq(&row.pull_request_id),
            delivery_watches::registration_json.eq(&row.registration_json),
            delivery_watches::actor_json.eq(&row.actor_json),
            delivery_watches::generation.eq(&row.generation),
            delivery_watches::enabled.eq(1),
            delivery_watches::next_poll_at.eq(&row.next_poll_at),
            delivery_watches::last_attempt_at.eq(None::<String>),
            delivery_watches::last_success_at.eq(None::<String>),
            delivery_watches::last_error.eq(None::<String>),
            delivery_watches::last_digest.eq(None::<String>),
            delivery_watches::provider_updated_at.eq(None::<String>),
        ))
        .execute(connection)?;
    Ok(())
}

fn validate_source(
    connection: &mut DbConnection,
    project: &str,
    watch: &DeliveryWatchRegistration,
) -> Result<()> {
    use super::schema::delivery_records;
    let payload = delivery_records::table
        .find((project, "pull_request", &watch.pull_request_id))
        .select(delivery_records::payload_json)
        .first::<String>(connection)
        .optional()?
        .context("watch requires a registered pull request")?;
    let record: DeliveryRecord = serde_json::from_str(&payload)?;
    let DeliveryBody::PullRequest { pull_request, .. } = record.body else {
        anyhow::bail!("watch requires a pull-request subject");
    };
    anyhow::ensure!(
        pull_request.repository == watch.repository_id
            && pull_request.number == watch.source.number
            && pull_request.provider.eq_ignore_ascii_case("github"),
        "watch source differs from registered pull request"
    );
    let url =
        reqwest::Url::parse(&pull_request.url).context("registered pull-request URL invalid")?;
    let expected = format!(
        "/{}/{}/pull/{}",
        watch.source.owner, watch.source.repository, watch.source.number
    );
    anyhow::ensure!(
        url.path()
            .trim_end_matches('/')
            .eq_ignore_ascii_case(&expected),
        "watch repository differs from registered pull-request URL"
    );
    Ok(())
}

pub(crate) fn list(
    connection: &mut DbConnection,
    project: &str,
) -> Result<(Vec<DeliveryWatch>, bool)> {
    let mut rows = delivery_watches::table
        .filter(delivery_watches::project_id.eq(project))
        .order(delivery_watches::id.asc())
        .limit(129)
        .select(WatchRow::as_select())
        .load::<WatchRow>(connection)?;
    let truncated = rows.len() > 128;
    rows.truncate(128);
    Ok((
        rows.into_iter()
            .map(WatchRow::decode)
            .collect::<Result<_>>()?,
        truncated,
    ))
}

impl Store {
    pub fn claim_delivery_watches(&mut self, now: DateTime<Utc>) -> Result<Vec<DeliveryWatch>> {
        self.transaction(|connection| {
            let now_text = now.to_rfc3339();
            let rows = delivery_watches::table
                .filter(delivery_watches::enabled.eq(1))
                .filter(delivery_watches::next_poll_at.le(&now_text))
                .order((
                    delivery_watches::next_poll_at.asc(),
                    delivery_watches::project_id.asc(),
                    delivery_watches::id.asc(),
                ))
                .limit(8)
                .select(WatchRow::as_select())
                .load::<WatchRow>(connection)?;
            let mut claimed = Vec::new();
            for row in rows {
                let mut watch = row.decode()?;
                // A reservation outlives the complete 30-second network budget.
                let reserved_until = (now + Duration::seconds(90)).to_rfc3339();
                let count = diesel::update(
                    delivery_watches::table
                        .find((&watch.project_id, &watch.registration.id))
                        .filter(delivery_watches::generation.eq(&watch.generation))
                        .filter(delivery_watches::next_poll_at.eq(watch.next_poll_at.to_rfc3339()))
                        .filter(delivery_watches::enabled.eq(1)),
                )
                .set((
                    delivery_watches::last_attempt_at.eq(Some(&now_text)),
                    delivery_watches::next_poll_at.eq(&reserved_until),
                ))
                .execute(connection)?;
                if count == 1 {
                    watch.last_attempt_at = Some(now);
                    watch.next_poll_at = now + Duration::seconds(90);
                    claimed.push(watch);
                }
            }
            Ok(claimed)
        })
    }

    pub fn apply_delivery_watch_poll(
        &mut self,
        watch: &DeliveryWatch,
        outcome: DeliveryWatchOutcome,
    ) -> Result<()> {
        self.transaction(|connection| apply_poll(connection, watch, outcome))
    }
}

fn apply_poll(
    connection: &mut DbConnection,
    watch: &DeliveryWatch,
    outcome: DeliveryWatchOutcome,
) -> Result<()> {
    let Some(current) = delivery_watches::table
        .find((&watch.project_id, &watch.registration.id))
        .filter(delivery_watches::enabled.eq(1))
        .filter(delivery_watches::generation.eq(&watch.generation))
        .filter(delivery_watches::last_attempt_at.eq(watch.last_attempt_at.map(|v| v.to_rfc3339())))
        .select(WatchRow::as_select())
        .first::<WatchRow>(connection)
        .optional()?
    else {
        return Ok(());
    };
    match outcome {
        DeliveryWatchOutcome::Observed {
            snapshot,
            provider_updated_at,
            digest,
        } => finish_success(
            connection,
            watch,
            &current,
            ObservedWatch {
                snapshot: *snapshot,
                provider_updated_at,
                digest,
            },
        ),
        DeliveryWatchOutcome::Failed { reason } => {
            finish_failure(connection, watch, &next_poll(watch), &reason)
        }
    }
}

struct ObservedWatch {
    snapshot: PullRequestSnapshot,
    provider_updated_at: DateTime<Utc>,
    digest: String,
}
fn finish_success(
    connection: &mut DbConnection,
    watch: &DeliveryWatch,
    current: &WatchRow,
    evidence: ObservedWatch,
) -> Result<()> {
    if current
        .provider_updated_at
        .as_ref()
        .is_some_and(|old| old > &evidence.provider_updated_at.to_rfc3339())
    {
        return finish_failure(
            connection,
            watch,
            &next_poll(watch),
            "GitHub observation is older than retained provider evidence",
        );
    }
    record_changed_snapshot(connection, watch, current.last_digest.as_deref(), &evidence)?;
    diesel::update(delivery_watches::table.find((&watch.project_id, &watch.registration.id)))
        .set((
            delivery_watches::next_poll_at.eq(next_poll(watch)),
            delivery_watches::last_success_at.eq(Some(Utc::now().to_rfc3339())),
            delivery_watches::last_error.eq(None::<String>),
            delivery_watches::last_digest.eq(Some(evidence.digest)),
            delivery_watches::provider_updated_at
                .eq(Some(evidence.provider_updated_at.to_rfc3339())),
        ))
        .execute(connection)?;
    Ok(())
}

fn record_changed_snapshot(
    connection: &mut DbConnection,
    watch: &DeliveryWatch,
    previous: Option<&str>,
    evidence: &ObservedWatch,
) -> Result<()> {
    if previous == Some(&evidence.digest) {
        return Ok(());
    }
    let attempt = watch
        .last_attempt_at
        .context("watch poll has no reservation")?;
    let id = blake3::hash(
        format!(
            "{}:{}:{}:{}",
            watch.project_id,
            watch.generation,
            attempt.to_rfc3339(),
            evidence.digest
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string();
    let input = DeliveryObservationInput {
        id: format!("watch:{id}"),
        occurred_at: evidence.snapshot.observed_at,
        body: DeliveryEvent::PullRequestObserved {
            pull_request_id: watch.registration.pull_request_id.clone(),
            snapshot: evidence.snapshot.clone(),
        },
    };
    super::delivery::record(
        connection,
        &watch.project_id,
        &watch.provenance,
        &[input],
        &[],
    )?;
    Ok(())
}
fn next_poll(watch: &DeliveryWatch) -> String {
    (Utc::now() + Duration::seconds(watch.registration.interval_seconds as i64)).to_rfc3339()
}

fn finish_failure(
    connection: &mut DbConnection,
    watch: &DeliveryWatch,
    next: &str,
    reason: &str,
) -> Result<()> {
    let reason = crate::coordination::domain::truncate_utf8(reason.to_owned(), 512);
    diesel::update(delivery_watches::table.find((&watch.project_id, &watch.registration.id)))
        .set((
            delivery_watches::next_poll_at.eq(next),
            delivery_watches::last_error.eq(Some(reason)),
        ))
        .execute(connection)?;
    Ok(())
}

#[cfg(all(test, feature = "sqlite"))]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn watcher_snapshot_requires_a_projection_for_retained_registration() {
        let mut store = Store::open_memory().unwrap();
        let actor = WorkActor {
            agent: "test".into(),
            session_id: "test".into(),
            model_id: None,
            config_hash: "test".into(),
        };
        let revision = |sha: char| json!({"repository":"repo","object_format":"sha1","commit":sha.to_string().repeat(40)});
        let bodies = [
            json!({"event":"pull_request_registered","pull_request":{"id":"pr","repository":"repo","provider":"github","number":42,"url":"https://github.com/example/repo/pull/42","title":"Release","base":revision('a'),"head":revision('b'),"contribution_ids":[]}}),
            json!({"event":"watch_registered","watch":{"id":"watch","pull_request_id":"pr","repository_id":"repo","source":{"owner":"example","repository":"repo","number":42},"interval_seconds":30}}),
        ];
        let observations = bodies.into_iter().enumerate().map(|(n,body)|serde_json::from_value(json!({"id":format!("seed-{n}"),"occurred_at":"2026-11-15T12:00:00Z","body":body})).unwrap()).collect();
        store
            .delivery_command("project", &actor, &DeliveryAction::Record { observations })
            .unwrap();
        diesel::delete(delivery_watches::table)
            .execute(&mut store.connection)
            .unwrap();
        assert!(validate_snapshot(&mut store.connection).is_err());
    }
}
