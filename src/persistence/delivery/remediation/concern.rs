// @feature persistence
// @feature delivery-coordination
// @feature code-concerns
// @spec docs/features/persistence.md
// @spec docs/features/delivery-coordination.md
// @spec docs/features/code-concerns.md
use super::*;
use crate::code_concerns::{
    ConcernAreaKind, ConcernDetail, ConcernEvent, ConcernLocation, ConcernResponse,
};
use crate::delivery::validation::text;
use crate::work_history::GitRevision;

pub(super) fn origin(
    connection: &mut DbConnection,
    project: &str,
    draft: &RemediationDraft,
) -> Result<OriginQuery> {
    let query = &draft.origin;
    let Some(id) = &draft.concern_id else {
        return Ok(query.clone());
    };
    text(id, 256)?;
    let ConcernResponse::Detail {
        concern: Some(detail),
    } = crate::persistence::code_concerns::concern_detail(connection, project, id)?
    else {
        bail!(DeliveryRejection::Invalid("code concern not found".into()));
    };
    if query.has_selector() {
        return Ok(query.clone());
    }
    selector(query, &detail)
}

fn selector(origin: &OriginQuery, detail: &ConcernDetail) -> Result<OriginQuery> {
    let mut query = feature_query(origin, detail);
    if let Some(location) = pinned_location(detail, query.repository.as_deref()) {
        query.repository = Some(location.revision.repository.clone());
        query.revision = Some(location.revision);
        query.path = location.path;
        query.line_start = location.line_start;
        query.line_end = location.line_end;
    }
    if !query.has_selector() {
        bail!(DeliveryRejection::Invalid(
            "concern has no unique feature or pinned location; specify an origin selector".into()
        ));
    }
    Ok(query)
}

fn feature_query(query: &OriginQuery, detail: &ConcernDetail) -> OriginQuery {
    let mut features = detail.summary.areas.iter().filter(|area| {
        area.kind == ConcernAreaKind::Feature
            && query
                .repository
                .as_ref()
                .is_none_or(|repository| repository == &area.repository)
    });
    let (repository, feature) = match (features.next(), features.next()) {
        (Some(area), None) if !detail.summary.areas_partial => {
            (Some(area.repository.clone()), Some(area.key.clone()))
        }
        _ => (query.repository.clone(), None),
    };
    OriginQuery {
        repository,
        feature,
        fallback: true,
        ..query.clone()
    }
}

#[derive(PartialEq)]
struct PinnedLocation {
    revision: GitRevision,
    path: Option<String>,
    line_start: Option<u32>,
    line_end: Option<u32>,
}

impl PinnedLocation {
    fn from_location(location: &ConcernLocation) -> Option<Self> {
        Some(Self {
            revision: GitRevision {
                repository: location.repository.clone(),
                object_format: location.object_format.clone()?,
                commit: location.commit.clone()?,
            },
            path: location.path.clone(),
            line_start: location.line_start,
            line_end: location.line_end,
        })
    }
}

fn pinned_location(detail: &ConcernDetail, repository: Option<&str>) -> Option<PinnedLocation> {
    if detail.partial {
        return None;
    }
    let locations = detail
        .history
        .iter()
        .filter_map(|observation| match &observation.input.body {
            ConcernEvent::Reported { encounter, .. }
            | ConcernEvent::Encountered { encounter, .. } => Some(encounter),
            _ => None,
        })
        .flat_map(|encounter| &encounter.areas)
        .filter_map(|entry| entry.location.as_ref())
        .filter(|location| {
            location.dirty != Some(true)
                && repository.is_none_or(|repo| repo == location.repository)
        })
        .filter_map(PinnedLocation::from_location);
    let mut locations = locations;
    locations
        .next()
        .filter(|first| locations.all(|other| &other == first))
}
