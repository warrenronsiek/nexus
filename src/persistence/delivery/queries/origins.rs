// @feature persistence
// @feature delivery-coordination
// @spec docs/features/persistence.md
// @spec docs/features/delivery-coordination.md
use super::*;
use crate::persistence::schema::{delivery_features, delivery_origins};
use std::collections::HashSet;
#[derive(Default)]
struct Candidates {
    ids: Vec<String>,
    mapping_ids: HashSet<(String, String)>,
    gaps: Vec<String>,
}
struct Resolution {
    matches: Vec<OriginMatch>,
    gaps: Vec<String>,
    budget: WindowBudget,
}
pub(super) fn resolve(
    connection: &mut DbConnection,
    project: &str,
    query: &OriginQuery,
) -> Result<DeliveryResponse> {
    query
        .validate()
        .map_err(|e| DeliveryRejection::Invalid(e.to_string()))?;
    let candidates = candidates(connection, project, query)?;
    let mut result = Resolution {
        gaps: candidates.gaps.clone(),
        matches: vec![],
        budget: WindowBudget::new(limit(query.limit) as usize, 512 * 1024),
    };
    for id in &candidates.ids {
        for item in contribution_matches(connection, project, query, id, &candidates)? {
            result.push(item)?;
        }
    }
    qualify_fingerprint(query, &mut result.matches);
    qualify_overlap(&mut result.matches);
    let qualification = qualification(&result);
    result.gaps.sort();
    result.gaps.dedup();
    Ok(DeliveryResponse::Origins {
        matches: result.matches,
        qualification,
        gaps: result.gaps,
    })
}
impl Resolution {
    fn push(&mut self, item: OriginMatch) -> Result<()> {
        if self.budget.include(&item)? {
            self.matches.push(item);
        } else {
            self.gaps.push("origin_result_limit".into());
        }
        Ok(())
    }
}

fn candidates(
    connection: &mut DbConnection,
    project: &str,
    query: &OriginQuery,
) -> Result<Candidates> {
    let composition = selected_composition(connection, project, query)?;
    let mut result = if query.revision.is_some() || query.fingerprint.is_some() {
        let mut result = revision_candidates(connection, project, query)?;
        if let Some(composition) = composition {
            let ids = composition.ids.into_iter().collect::<HashSet<_>>();
            result.ids.retain(|id| ids.contains(id));
            result.gaps.extend(composition.gaps);
        }
        result
    } else if let Some(composition) = composition {
        composition
    } else {
        repository_candidates(connection, project, query)?
    };
    result.ids.sort();
    result.ids.dedup();
    Ok(result)
}
fn revision_candidates(
    connection: &mut DbConnection,
    project: &str,
    query: &OriginQuery,
) -> Result<Candidates> {
    let repository = query
        .revision
        .as_ref()
        .map(|r| r.repository.as_str())
        .or(query.repository.as_deref())
        .expect("validated repository");
    let mut selection = delivery_origins::table
        .filter(delivery_origins::project_id.eq(project))
        .filter(delivery_origins::repository.eq(repository))
        .into_boxed();
    if let Some(revision) = &query.revision {
        selection = selection.filter(delivery_origins::revision.eq(&revision.commit));
    }
    if let Some(fingerprint) = &query.fingerprint {
        selection = selection.filter(delivery_origins::fingerprint.eq(fingerprint));
    }
    if let Some(path) = &query.path {
        selection = selection.filter(delivery_origins::path.eq(path));
    }
    if let Some(start) = query.line_start {
        selection = selection
            .filter(delivery_origins::new_start.le(i64::from(query.line_end.unwrap_or(start))))
            .filter(
                (delivery_origins::new_start + delivery_origins::new_lines).gt(i64::from(start)),
            );
    }
    let rows = selection
        .order((
            delivery_origins::contribution_id.asc(),
            delivery_origins::mapping_id.asc(),
            delivery_origins::hunk_index.asc(),
        ))
        .limit(1025)
        .select(OriginRow::as_select())
        .load::<OriginRow>(connection)?;
    Ok(Candidates::from_index(rows))
}
impl Candidates {
    fn from_index(rows: Vec<OriginRow>) -> Self {
        let mut result = Self::default();
        if rows.len() > 1024 {
            result.gaps.push("origin_index_limit".into());
        }
        for row in rows.into_iter().take(1024) {
            result
                .mapping_ids
                .insert((row.contribution_id.clone(), row.mapping_id));
            result.ids.push(row.contribution_id);
        }
        result
    }
}

fn repository_candidates(
    connection: &mut DbConnection,
    project: &str,
    query: &OriginQuery,
) -> Result<Candidates> {
    let mut selection = delivery_records::table
        .filter(delivery_records::project_id.eq(project))
        .filter(delivery_records::kind.eq("contribution"))
        .into_boxed();
    if let Some(repo) = &query.repository {
        selection = selection.filter(delivery_records::repository.eq(repo));
    }
    if let Some(feature) = &query.feature {
        selection = selection.filter(
            delivery_records::id.eq_any(
                delivery_features::table
                    .filter(delivery_features::project_id.eq(project))
                    .filter(delivery_features::feature.eq(feature))
                    .select(delivery_features::contribution_id),
            ),
        );
    }
    Ok(Candidates {
        ids: selection
            .select(delivery_records::id)
            .order(delivery_records::id.asc())
            .limit(limit(query.limit) + 1)
            .load(connection)?,
        ..Default::default()
    })
}

fn contribution_matches(
    connection: &mut DbConnection,
    project: &str,
    query: &OriginQuery,
    id: &str,
    candidates: &Candidates,
) -> Result<Vec<OriginMatch>> {
    let Some(record) = projection::load(
        connection,
        project,
        &DeliverySubject {
            kind: DeliveryKind::Contribution,
            id: id.into(),
        },
    )?
    else {
        return Ok(vec![]);
    };
    let DeliveryBody::Contribution {
        contribution,
        mappings,
    } = record.body
    else {
        unreachable!("contribution key")
    };
    if !matches_contribution(query, &contribution) {
        return Ok(vec![]);
    }
    let mappings = mappings
        .into_iter()
        .filter(|m| {
            candidates
                .mapping_ids
                .contains(&(contribution.id.clone(), m.id.clone()))
        })
        .collect::<Vec<_>>();
    let selected = if query.revision.is_some() || query.fingerprint.is_some() {
        mappings.into_iter().map(Some).collect()
    } else {
        vec![None]
    };
    let availability = presence(connection, project, &contribution.origin)?;
    Ok(selected
        .into_iter()
        .map(|mapping| origin_match(query, &contribution, mapping, availability.clone()))
        .collect())
}
fn matches_contribution(query: &OriginQuery, c: &Contribution) -> bool {
    query.repository.as_ref().is_none_or(|r| r == &c.repository)
        && query
            .feature
            .as_ref()
            .is_none_or(|f| c.features.contains(f))
}
fn matches_hunk(query: &OriginQuery, hunk: &ChangedHunk) -> bool {
    query
        .fingerprint
        .as_ref()
        .is_none_or(|digest| hunk.fingerprint.as_ref() == Some(digest))
        && query.path.as_ref().is_none_or(|path| path == &hunk.path)
        && query.line_start.is_none_or(|start| {
            hunk.new_start <= query.line_end.unwrap_or(start)
                && hunk.new_start.saturating_add(hunk.new_lines) > start
        })
}
fn origin_match(
    query: &OriginQuery,
    contribution: &Contribution,
    mapping: Option<RevisionMapping>,
    presence: Option<AgentPresence>,
) -> OriginMatch {
    let qualification = if mapping
        .as_ref()
        .is_some_and(|m| m.verification == MappingVerification::MechanicallyVerified)
    {
        OriginQualification::Exact
    } else {
        OriginQualification::Candidates
    };
    let matched_hunks = mapping
        .as_ref()
        .map(|m| {
            m.hunks
                .iter()
                .filter(|h| matches_hunk(query, h))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    OriginMatch {
        contribution: contribution.clone(),
        mapping,
        matched_hunks,
        qualification,
        presence,
    }
}
fn overlaps(a: &OriginMatch, b: &OriginMatch) -> bool {
    a.contribution.id != b.contribution.id
        && a.matched_hunks.iter().any(|ha| {
            b.matched_hunks.iter().any(|hb| {
                ha.path == hb.path
                    && ha.new_start < hb.new_start.saturating_add(hb.new_lines)
                    && hb.new_start < ha.new_start.saturating_add(ha.new_lines)
            })
        })
}
fn qualify_overlap(matches: &mut [OriginMatch]) {
    let ambiguous = matches
        .iter()
        .enumerate()
        .flat_map(|(left, a)| {
            matches
                .iter()
                .enumerate()
                .skip(left + 1)
                .filter_map(move |(right, b)| overlaps(a, b).then_some([left, right]))
        })
        .flatten()
        .collect::<HashSet<_>>();
    for index in ambiguous {
        matches[index].qualification = OriginQualification::Ambiguous;
    }
}
fn qualification(result: &Resolution) -> OriginQualification {
    if result.matches.is_empty() {
        OriginQualification::Unknown
    } else if !result.gaps.is_empty()
        || result
            .matches
            .iter()
            .any(|m| m.qualification == OriginQualification::Ambiguous)
    {
        OriginQualification::Ambiguous
    } else if result
        .matches
        .iter()
        .all(|m| m.qualification == OriginQualification::Exact)
    {
        OriginQualification::Exact
    } else {
        OriginQualification::Candidates
    }
}
#[derive(Default)]
struct Composition {
    ids: Vec<String>,
    seen: HashSet<DeliverySubject>,
    gaps: Vec<String>,
}
impl Composition {
    fn collect(
        &mut self,
        connection: &mut DbConnection,
        project: &str,
        subject: &DeliverySubject,
        depth: usize,
    ) -> Result<()> {
        if depth >= 8 || self.seen.len() >= 256 {
            self.gaps.push("composition_limit".into());
            return Ok(());
        }
        if !self.seen.insert(subject.clone()) {
            return Ok(());
        }
        match subject.kind {
            DeliveryKind::Contribution => self.ids.push(subject.id.clone()),
            DeliveryKind::Rollout => self.rollout_sources(connection, project, subject, depth)?,
            _ => self.descendants(connection, project, subject, depth)?,
        }
        Ok(())
    }
    fn rollout_sources(
        &mut self,
        connection: &mut DbConnection,
        project: &str,
        subject: &DeliverySubject,
        depth: usize,
    ) -> Result<()> {
        let manifest =
            projection::load(connection, project, subject)?.and_then(|record| match record.body {
                DeliveryBody::Rollout { rollout } if !rollout.contribution_ids.is_empty() => {
                    Some(rollout.contribution_ids)
                }
                _ => None,
            });
        match manifest {
            Some(ids) => {
                for id in ids {
                    self.collect(
                        connection,
                        project,
                        &DeliverySubject {
                            kind: DeliveryKind::Contribution,
                            id,
                        },
                        depth + 1,
                    )?;
                }
            }
            None => {
                self.gaps.push("rollout_manifest_missing".into());
                self.descendants(connection, project, subject, depth)?;
            }
        }
        Ok(())
    }
    fn descendants(
        &mut self,
        connection: &mut DbConnection,
        project: &str,
        subject: &DeliverySubject,
        depth: usize,
    ) -> Result<()> {
        let rows = delivery_relations::table
            .filter(delivery_relations::project_id.eq(project))
            .filter(delivery_relations::source_kind.eq(subject.kind.as_str()))
            .filter(delivery_relations::source_id.eq(&subject.id))
            .select((
                delivery_relations::target_kind,
                delivery_relations::target_id,
            ))
            .order((
                delivery_relations::target_kind.asc(),
                delivery_relations::target_id.asc(),
            ))
            .limit(257)
            .load::<(String, String)>(connection)?;
        if rows.len() > 256 {
            self.gaps.push("composition_limit".into());
        }
        for (kind, id) in rows.into_iter().take(256) {
            self.collect(
                connection,
                project,
                &DeliverySubject {
                    kind: DeliveryKind::deserialize(kind.into_deserializer())
                        .map_err(|_: LabelError| anyhow::anyhow!("invalid stored delivery kind"))?,
                    id,
                },
                depth + 1,
            )?;
        }
        Ok(())
    }
}

fn selected_composition(
    connection: &mut DbConnection,
    project: &str,
    query: &OriginQuery,
) -> Result<Option<Candidates>> {
    let mut selected: Option<Candidates> = None;
    for (kind, id) in [
        (DeliveryKind::PullRequest, &query.pull_request_id),
        (DeliveryKind::Rollout, &query.rollout_id),
    ] {
        if let Some(id) = id {
            let mut composition = Composition::default();
            composition.collect(
                connection,
                project,
                &DeliverySubject {
                    kind,
                    id: id.clone(),
                },
                0,
            )?;
            match &mut selected {
                Some(previous) => {
                    let ids = composition.ids.into_iter().collect::<HashSet<_>>();
                    previous.ids.retain(|id| ids.contains(id));
                    previous.gaps.extend(composition.gaps);
                }
                None => {
                    selected = Some(Candidates {
                        ids: composition.ids,
                        gaps: composition.gaps,
                        ..Default::default()
                    })
                }
            }
        }
    }
    Ok(selected)
}

fn qualify_fingerprint(query: &OriginQuery, matches: &mut [OriginMatch]) {
    if query.fingerprint.is_none() || query.revision.is_some() {
        return;
    }
    let contributors = matches
        .iter()
        .map(|m| m.contribution.id.as_str())
        .collect::<HashSet<_>>()
        .len();
    if contributors > 1 {
        for item in matches {
            item.qualification = OriginQualification::Ambiguous;
        }
    }
}
