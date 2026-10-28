// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
// @boundary checksummed-work-history-exchange
use super::WorkClient;
use crate::history_transfer::{HistoryFormat, HistoryWriter, VerifiedHistory};
use anyhow::Result;
use nexus::work_history::{WorkAction, WorkObservation};
use serde_json::{json, Value};
use std::path::Path;

pub(super) async fn export(client: &WorkClient, output: &Path) -> Result<Value> {
    let mut writer = HistoryWriter::<WorkObservation>::new(output, HistoryFormat::WorkV1)?;
    let mut after = None;
    loop {
        let page = client
            .send(WorkAction::Export {
                project_id: None,
                after,
                limit: Some(32),
            })
            .await?;
        let observations: Vec<WorkObservation> =
            serde_json::from_value(page["observations"].clone())?;
        for observation in observations {
            writer.push(observation)?
        }
        after = page["next_cursor"].as_str().map(str::to_owned);
        if after.is_none() {
            break;
        }
    }
    Ok(json!({"ok":true,"exported":writer.finish()?}))
}

pub(super) async fn import(client: &WorkClient, path: &Path) -> Result<Value> {
    let mut history = VerifiedHistory::<WorkObservation>::open(
        path,
        HistoryFormat::WorkV1,
        WorkObservation::validate,
    )?;
    let mut imported = 0;
    while let Some(observation) = history.next()? {
        client
            .send(WorkAction::Import {
                observations: vec![observation],
            })
            .await?;
        imported += 1;
    }
    Ok(json!({"ok":true,"imported":imported,"validated":history.count}))
}
