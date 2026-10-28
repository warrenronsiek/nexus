// @feature code-concerns
// @feature runtime
// @spec docs/features/code-concerns.md
// @spec docs/features/runtime.md
// @boundary concern-history-exchange: validate typed source observations before resumable ingestion
use super::ConcernClient;
use crate::history_transfer::{HistoryFormat, HistoryWriter, VerifiedHistory};
use anyhow::{Context, Result};
use nexus::code_concerns::{ConcernAction, ConcernObservation};
use serde_json::{json, Value};
use std::path::Path;

pub(super) async fn export(client: &ConcernClient, output: &Path) -> Result<Value> {
    let mut writer =
        HistoryWriter::<ConcernObservation>::new(output, HistoryFormat::CodeConcernsV1)?;
    let mut after = None;
    loop {
        let page = client
            .send(ConcernAction::Export {
                project_id: None,
                after,
                limit: Some(32),
            })
            .await?;
        let observations: Vec<ConcernObservation> =
            serde_json::from_value(page["observations"].clone())?;
        for observation in observations {
            writer.push(observation)?
        }
        after = page["next_cursor"].as_str().map(str::to_owned);
        if after.is_none() {
            break;
        }
    }
    let count = writer.finish()?;
    Ok(json!({"ok":true,"exported":count,"artifact_dependencies":"external_references"}))
}

pub(super) async fn import(client: &ConcernClient, path: &Path) -> Result<Value> {
    let mut history = VerifiedHistory::<ConcernObservation>::open(
        path,
        HistoryFormat::CodeConcernsV1,
        ConcernObservation::validate,
    )?;
    let mut imported = 0;
    while let Some(observation) = history.next()? {
        client
            .send(ConcernAction::Import {
                observations: vec![observation],
            })
            .await
            .with_context(|| {
                format!(
                    "import stopped near observation {}; replay this file to resume",
                    imported + 1
                )
            })?;
        imported += 1;
    }
    Ok(
        json!({"ok":true,"imported":imported,"validated":history.count,"artifact_dependencies":"external_references"}),
    )
}
