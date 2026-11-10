// @feature code-map
// @spec docs/features/code-map.md
use super::*;
use anyhow::{ensure, Result};
use std::collections::HashSet;

pub(crate) const MAX_BYTES: usize = 384 * 1024;
pub(crate) const MAX_POINTS: usize = 4096;

fn text(value: &str, limit: usize) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && value.len() <= limit,
        "empty or oversized code map field"
    );
    ensure!(
        !value.chars().any(char::is_control),
        "control character in code map field"
    );
    Ok(())
}

fn dimension(value: &str) -> Result<()> {
    text(value, 64)?;
    ensure!(value != "subject", "subject is a reserved dimension");
    Ok(())
}

pub(crate) fn validate_concern_dimensions(values: &BTreeMap<String, String>) -> Result<()> {
    ensure!(values.len() <= 32, "too many code concern dimensions");
    for (key, value) in values {
        dimension(key)?;
        ensure!(
            !matches!(
                key.as_str(),
                "feature" | "subsystem" | "file" | "directory" | "repository"
            ),
            "use structured concern attributions for built-in code map dimensions"
        );
        text(value, 256)?;
    }
    Ok(())
}

impl MapSnapshot {
    pub(crate) fn validate(&self) -> Result<()> {
        for value in [
            &self.id,
            &self.producer,
            &self.measure,
            &self.title,
            &self.unit,
            &self.revision,
        ] {
            text(value, 160)?;
        }
        ensure!(
            self.samples.len() <= MAX_POINTS,
            "too many code map samples"
        );
        let mut subjects = HashSet::new();
        let mut keys = HashSet::new();
        for sample in &self.samples {
            sample.validate()?;
            ensure!(
                subjects.insert(&sample.subject),
                "duplicate code map subject"
            );
            keys.extend(sample.dimensions.keys());
        }
        ensure!(keys.len() <= 32, "too many code map dimensions");
        Ok(())
    }
}

impl MapSample {
    fn validate(&self) -> Result<()> {
        text(&self.subject, 256)?;
        ensure!(
            self.value.abs() <= 1e100,
            "code map value must be finite and bounded"
        );
        ensure!(
            self.dimensions.len() <= 32,
            "too many dimensions on a code map sample"
        );
        for (key, value) in &self.dimensions {
            dimension(key)?;
            text(value, 256)?;
        }
        if let Some(evidence) = &self.evidence {
            text(evidence, 512)?;
        }
        Ok(())
    }
}

impl MapQuery {
    pub(crate) fn validate(&self) -> Result<()> {
        if let MapMeasure::Published { producer, measure } = &self.measure {
            text(producer, 160)?;
            text(measure, 160)?;
        } else {
            ensure!(
                self.snapshot_id.is_none(),
                "snapshot_id requires a published measure"
            );
        }
        if let Some(group) = &self.group_by {
            text(group, 64)?;
        }
        if let Some(bucket) = &self.bucket {
            text(bucket, 4096)?;
        }
        if let Some(snapshot) = &self.snapshot_id {
            text(snapshot, 160)?;
        }
        Ok(())
    }
}
