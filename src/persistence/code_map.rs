// @feature code-map
// @feature persistence
// @spec docs/features/code-map.md
// @spec docs/features/persistence.md
// @entrypoint Store::code_map
// @entrypoint Store::publish_code_map
use super::{backend::DbConnection, schema::code_map_snapshots as s, Store};
use crate::code_concerns::ConcernRejection;
use crate::code_map::*;
use anyhow::{ensure, Result};
use chrono::SecondsFormat;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Queryable, Selectable, Insertable, Serialize, Deserialize)]
#[diesel(table_name = s)]
pub(super) struct CodeMapSnapshotRow {
    project_id: String,
    id: String,
    producer: String,
    measure: String,
    observed_at: String,
    reporter: String,
    payload_json: String,
}

impl CodeMapSnapshotRow {
    fn new(project: &str, reporter: &str, snapshot: &MapSnapshot) -> Result<Self> {
        snapshot.validate()?;
        let payload_json = serde_json::to_string(snapshot)?;
        ensure!(
            payload_json.len() <= MAX_BYTES,
            "code map snapshot exceeds the byte limit"
        );
        Ok(Self {
            project_id: project.into(),
            reporter: reporter.into(),
            id: snapshot.id.clone(),
            producer: snapshot.producer.clone(),
            measure: snapshot.measure.clone(),
            observed_at: snapshot
                .observed_at
                .to_rfc3339_opts(SecondsFormat::Nanos, true),
            payload_json,
        })
    }

    pub(super) fn validate(&self) -> Result<()> {
        let snapshot: MapSnapshot = serde_json::from_str(&self.payload_json)?;
        let expected = Self::new(&self.project_id, &self.reporter, &snapshot)?;
        ensure!(
            self.id == expected.id
                && self.producer == expected.producer
                && self.measure == expected.measure
                && self.observed_at == expected.observed_at,
            "code map snapshot indexes disagree with its payload"
        );
        Ok(())
    }
}

impl Store {
    pub fn publish_code_map(
        &mut self,
        project: &str,
        reporter: &str,
        snapshot: &MapSnapshot,
    ) -> Result<bool> {
        let row = CodeMapSnapshotRow::new(project, reporter, snapshot)
            .map_err(|error| ConcernRejection::Invalid(error.to_string()))?;
        let inserted = diesel::insert_into(s::table)
            .values(&row)
            .on_conflict((s::project_id, s::id))
            .do_nothing()
            .execute(&mut self.connection)?;
        if inserted == 1 {
            return Ok(true);
        }
        let saved = s::table
            .find((project, &snapshot.id))
            .select(s::payload_json)
            .first::<String>(&mut self.connection)?;
        if saved != row.payload_json {
            return Err(
                ConcernRejection::Conflict("code map snapshot ID already exists".into()).into(),
            );
        }
        Ok(false)
    }

    pub fn code_map(&mut self, project: &str, query: &MapQuery) -> Result<MapView> {
        query
            .validate()
            .map_err(|error| ConcernRejection::Invalid(error.to_string()))?;
        let connection = &mut self.connection;
        let mut view = catalog(connection, project)?;
        view.measure = query.measure.clone();
        let points = match &query.measure {
            MapMeasure::Published { producer, measure } => {
                published(connection, project, query, (producer, measure), &mut view)?
            }
            _ => super::code_concerns::map_points(connection, project, query, &mut view)?,
        };
        select_dimension(&mut view, query.group_by.as_deref(), &points);
        project_points(&mut view, &points, query.bucket.as_deref())?;
        Ok(view)
    }
}

fn select_dimension(view: &mut MapView, preferred: Option<&str>, points: &[MapPoint]) {
    view.dimensions.sort();
    view.dimensions.dedup();
    let dimension = preferred
        .filter(|key| view.dimensions.iter().any(|dimension| dimension == key))
        .unwrap_or_else(|| default_dimension(view, points));
    view.group_by = dimension.into();
    if view.dimensions.len() > 64 {
        view.partial = true;
        view.dimensions.truncate(64);
        if !view.dimensions.contains(&view.group_by) {
            view.dimensions[63] = view.group_by.clone();
        }
    }
}

fn default_dimension<'a>(view: &'a MapView, points: &[MapPoint]) -> &'a str {
    if view.source.is_none() {
        let attributed = points
            .iter()
            .filter(|point| point.dimensions.contains_key("file"))
            .count();
        return if attributed * 2 < points.len() {
            "subject"
        } else {
            "file"
        };
    }
    view.dimensions
        .iter()
        .find(|dimension| *dimension != "subject")
        .map(String::as_str)
        .unwrap_or("subject")
}

fn catalog(connection: &mut DbConnection, project: &str) -> Result<MapView> {
    let mut keys: Vec<(String, String)> = s::table
        .filter(s::project_id.eq(project))
        .select((s::producer, s::measure))
        .distinct()
        .order((s::producer.asc(), s::measure.asc()))
        .limit(65)
        .load(connection)?;
    let measures_partial = keys.len() > 64;
    keys.truncate(64);
    let mut measures = vec![MapMeasure::Encounters, MapMeasure::OpenConcerns];
    measures.extend(
        keys.into_iter()
            .map(|(producer, measure)| MapMeasure::Published { producer, measure }),
    );
    Ok(MapView {
        measure: MapMeasure::default(),
        title: String::new(),
        unit: String::new(),
        reducer: MapReducer::Sum,
        size_by_samples: false,
        source: None,
        measures,
        measures_partial,
        dimensions: vec!["subject".into()],
        group_by: String::new(),
        bucket: None,
        bucket_label: None,
        groups: Vec::new(),
        partial: false,
    })
}

fn published(
    connection: &mut DbConnection,
    project: &str,
    query: &MapQuery,
    (producer, measure): (&str, &str),
    view: &mut MapView,
) -> Result<Vec<MapPoint>> {
    let retained = published_snapshot(connection, project, query, (producer, measure))?;
    let Some((reporter, payload)) = retained else {
        if query.snapshot_id.is_none()
            && (query.occurred_after.is_some() || query.occurred_before.is_some())
            && known_measure(connection, project, (producer, measure))?
        {
            view.title = measure.into();
            if !view.measures.contains(&query.measure) {
                view.measures.push(query.measure.clone());
            }
            return Ok(Vec::new());
        }
        return Err(ConcernRejection::Invalid(
            "code map snapshot was not found in this project".into(),
        )
        .into());
    };
    let snapshot: MapSnapshot = serde_json::from_str(&payload)?;
    if !view.measures.contains(&query.measure) {
        view.measures.push(query.measure.clone());
    }
    view.title = snapshot.title;
    view.unit = snapshot.unit;
    view.reducer = snapshot.reducer;
    view.source = Some(MapSource {
        id: snapshot.id,
        revision: snapshot.revision,
        observed_at: snapshot.observed_at,
        reporter,
    });
    view.dimensions.extend(
        snapshot
            .samples
            .iter()
            .flat_map(|sample| sample.dimensions.keys().cloned()),
    );
    Ok(snapshot.samples.into_iter().map(MapPoint::from).collect())
}

fn known_measure(
    connection: &mut DbConnection,
    project: &str,
    (producer, measure): (&str, &str),
) -> Result<bool> {
    Ok(diesel::select(diesel::dsl::exists(
        s::table
            .filter(s::project_id.eq(project))
            .filter(s::producer.eq(producer))
            .filter(s::measure.eq(measure)),
    ))
    .get_result(connection)?)
}

fn published_snapshot(
    connection: &mut DbConnection,
    project: &str,
    query: &MapQuery,
    (producer, measure): (&str, &str),
) -> Result<Option<(String, String)>> {
    let mut select = s::table
        .filter(s::project_id.eq(project))
        .filter(s::producer.eq(producer))
        .filter(s::measure.eq(measure))
        .into_boxed();
    if let Some(id) = &query.snapshot_id {
        select = select.filter(s::id.eq(id));
    }
    if let Some(time) = query.occurred_after {
        select = select.filter(s::observed_at.ge(time.to_rfc3339_opts(SecondsFormat::Nanos, true)));
    }
    if let Some(time) = query.occurred_before {
        select = select.filter(s::observed_at.lt(time.to_rfc3339_opts(SecondsFormat::Nanos, true)));
    }
    let retained = select
        .order((s::observed_at.desc(), s::id.desc()))
        .select((s::reporter, s::payload_json))
        .first::<(String, String)>(connection)
        .optional()?;
    Ok(retained)
}

use crate::code_map::project as project_points;
