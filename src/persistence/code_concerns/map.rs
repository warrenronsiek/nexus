// @feature code-map
// @feature code-concerns
// @spec docs/features/code-map.md
// @spec docs/features/code-concerns.md
use super::counts::Population;
use crate::code_concerns::{ConcernQuery, ConcernQueryStatus};
use crate::code_map::{
    DimensionPlacement, MapEvidence, MapMeasure, MapPoint, MapView, Placement, MAX_POINTS,
};
use crate::persistence::backend::DbConnection;
use crate::persistence::schema::{
    code_concerns as c, concern_areas as a, concern_attributions as l, concern_encounters as e,
};
use anyhow::Result;
use diesel::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

type DimensionContributions = Vec<DimensionPlacement>;

struct Encounter {
    event: String,
    concern: String,
    title: String,
    repository: Option<String>,
    dimensions: String,
}

pub(in crate::persistence) fn map_points(
    connection: &mut DbConnection,
    project: &str,
    measure: &MapMeasure,
    view: &mut MapView,
) -> Result<Vec<MapPoint>> {
    let (title, unit, status) = match measure {
        MapMeasure::OpenConcerns => ("Open concerns", "concerns", Some(ConcernQueryStatus::Open)),
        _ => ("Reported concern encounters", "encounters", None),
    };
    view.title = title.into();
    view.unit = unit.into();
    view.dimensions
        .extend(["directory", "feature", "file", "repository", "subsystem"].map(String::from));

    let mut encounters = encounters(connection, project, status)?;
    view.partial = encounters.len() > MAX_POINTS;
    encounters.truncate(MAX_POINTS);
    let mut attributions = attributions(connection, project, &encounters)?;
    let mut points = BTreeMap::new();
    for encounter in encounters {
        let identity = match measure {
            MapMeasure::OpenConcerns => &encounter.concern,
            _ => &encounter.event,
        };
        let dimensions = attributions
            .remove(&encounter.event)
            .unwrap_or_default()
            .into_iter()
            .chain(reported_dimensions(&encounter)?);
        let point = points
            .entry(identity.clone())
            .or_insert_with(|| encounter_point(&encounter));
        point.include_dimensions(dimensions);
    }
    view.dimensions.extend(
        points
            .values()
            .flat_map(|point| point.dimensions.keys().cloned())
            .collect::<BTreeSet<_>>(),
    );
    Ok(points.into_values().map(place_subject).collect())
}

fn encounters(
    connection: &mut DbConnection,
    project: &str,
    status: Option<ConcernQueryStatus>,
) -> Result<Vec<Encounter>> {
    let query = ConcernQuery {
        status,
        ..ConcernQuery::default()
    };
    let population = Population {
        project,
        query: &query,
        concern: None,
        area: None,
    };
    let rows = e::table
        .inner_join(c::table.on(c::project_id.eq(e::project_id).and(c::id.eq(e::concern_id))))
        .filter(e::project_id.eq(project))
        .filter(e::event_id.eq_any(population.query().select(e::event_id)))
        .order((e::effective_time.desc(), e::event_id.desc()))
        .limit(MAX_POINTS as i64 + 1)
        .select((
            e::event_id,
            e::concern_id,
            c::title,
            e::repository,
            e::map_dimensions_json,
        ))
        .load::<(String, String, String, Option<String>, String)>(connection)?;
    Ok(rows
        .into_iter()
        .map(
            |(event, concern, title, repository, dimensions)| Encounter {
                event,
                concern,
                title,
                repository,
                dimensions,
            },
        )
        .collect())
}

fn attributions(
    connection: &mut DbConnection,
    project: &str,
    encounters: &[Encounter],
) -> Result<BTreeMap<String, BTreeSet<DimensionPlacement>>> {
    // Deduplicate while retaining the 4,096-encounter attribution page; waiting
    // for final MapPoint aggregation would retain repeated repository/directory payloads.
    let mut result = BTreeMap::<String, BTreeSet<DimensionPlacement>>::new();
    for chunk in encounters.chunks(256) {
        let rows = l::table
            .inner_join(a::table.on(a::project_id.eq(l::project_id).and(a::id.eq(l::area_id))))
            .filter(l::project_id.eq(project))
            .filter(l::relation.eq("affected"))
            .filter(l::event_id.eq_any(chunk.iter().map(|encounter| &encounter.event)))
            .select((l::event_id, a::repository, a::kind, a::area_key))
            .load::<(String, String, String, String)>(connection)?;
        for (event, repository, kind, key) in rows {
            result
                .entry(event)
                .or_default()
                .extend(area_dimensions(&repository, &kind, &key));
        }
    }
    Ok(result)
}

fn reported_dimensions(encounter: &Encounter) -> Result<DimensionContributions> {
    let custom: BTreeMap<String, String> = serde_json::from_str(&encounter.dimensions)?;
    Ok(encounter
        .repository
        .iter()
        .map(|repository| DimensionPlacement {
            dimension: "repository".into(),
            placement: Placement::value(repository, repository),
        })
        .chain(
            custom
                .into_iter()
                .map(|(dimension, value)| DimensionPlacement {
                    dimension,
                    placement: Placement::value(&value, &value),
                }),
        )
        .collect())
}

fn area_dimensions(repository: &str, kind: &str, key: &str) -> DimensionContributions {
    let areas = match kind {
        "file" => [
            Some(("file", key)),
            Some((
                "directory",
                key.rsplit_once('/').map_or(".", |(parent, _)| parent),
            )),
        ],
        "feature" | "subsystem" => [Some((kind, key)), None],
        _ => [None, None],
    };
    std::iter::once(DimensionPlacement {
        dimension: "repository".into(),
        placement: Placement::value(repository, repository),
    })
    .chain(areas.into_iter().flatten().map(|(dimension, key)| {
        let identity = crate::identity::stable_id(&["code-map", repository, dimension, key]);
        DimensionPlacement {
            dimension: dimension.into(),
            placement: Placement::value(&identity, key),
        }
    }))
    .collect()
}

fn encounter_point(encounter: &Encounter) -> MapPoint {
    MapPoint {
        subject: Placement::value(&encounter.concern, &encounter.title),
        value: 1.0,
        dimensions: BTreeMap::new(),
        evidence: MapEvidence {
            subject: encounter.title.clone(),
            concern_id: Some(encounter.concern.clone()),
            reference: Some(encounter.event.clone()),
        },
    }
}

fn place_subject(mut point: MapPoint) -> MapPoint {
    if let Some(files) = point
        .dimensions
        .get("file")
        .filter(|files| files.len() == 1)
    {
        point.subject = files.first().expect("one file").clone();
    }
    point
}
