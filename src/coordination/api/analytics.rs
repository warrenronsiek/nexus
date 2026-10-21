// @feature usage-analytics
// @spec docs/features/usage-analytics.md
use super::*;
use crate::analytics::{
    classification::{valid_slug, validate_draft},
    inventory,
    types::*,
};

pub(super) fn decode_analytics(params: Value) -> Result<ServiceRequest> {
    let mut command: AnalyticsCommand = decode(params)?;
    validate_analytics(&mut command)?;
    validate_pagination(&command)?;
    Ok(ServiceRequest::Analytics(command))
}
fn validate_target(target: &ClassificationTarget) -> Result<()> {
    let pattern = match target {
        ClassificationTarget::Action {
            invocation_id,
            index,
            pattern_key,
        } => {
            validate_required_field("invocation_id", invocation_id, 512)?;
            if *index >= MAX_ACTIONS as u32 {
                bail!("classification action index exceeds bounds");
            }
            pattern_key
        }
        ClassificationTarget::Resource {
            name, pattern_key, ..
        } => {
            validate_required_field("resource name", name, 512)?;
            pattern_key
        }
    };
    if pattern.len() != 64 || !pattern.bytes().all(|c| c.is_ascii_hexdigit()) {
        bail!("classification reference must contain a stable fingerprint");
    }
    Ok(())
}

fn validate_pagination(command: &AnalyticsCommand) -> Result<()> {
    if let AnalyticsCommand::Examples {
        category_id,
        resource_key,
        offset,
        ..
    } = command
    {
        if *offset > 1_000_000
            || category_id.as_ref().is_some_and(|id| !valid_slug(id))
            || resource_key
                .as_ref()
                .is_some_and(|key| key.len() > 1024 || key.chars().any(char::is_control))
        {
            bail!("invalid example filter");
        }
    }
    if let AnalyticsCommand::Catalog { offset, limit, .. } = command {
        if *offset > 1_000_000 || limit.is_some_and(|limit| limit == 0 || limit > 64) {
            bail!("catalog pagination exceeds bounds");
        }
    }
    Ok(())
}

fn validate_classification_context(
    context: &mut HookContext,
    target: &ClassificationTarget,
) -> Result<()> {
    validate_hook_context(context)?;
    validate_target(target)
}
fn validate_report(draft: &CategoryDraft, model: &str) -> Result<()> {
    validate_draft(draft)?;
    validate_required_field("classifier_model", model, 128)
}

fn validate_analytics(command: &mut AnalyticsCommand) -> Result<()> {
    match command {
        AnalyticsCommand::Summary { project_id }
        | AnalyticsCommand::Catalog { project_id, .. }
        | AnalyticsCommand::Opportunities { project_id }
        | AnalyticsCommand::Examples { project_id, .. } => normalize_read_project(project_id),
        AnalyticsCommand::Inventory {
            context,
            scope,
            resources,
            ..
        } => {
            validate_hook_context(context)?;
            inventory::validate(scope, resources)
        }
        AnalyticsCommand::ReserveClassification { context, target }
        | AnalyticsCommand::ClassificationFailed { context, target } => {
            validate_classification_context(context, target)
        }
        AnalyticsCommand::Classify {
            context,
            target,
            draft,
            model,
        } => {
            validate_classification_context(context, target)?;
            validate_report(draft, model)
        }
        AnalyticsCommand::RenameCategory {
            id,
            name,
            definition,
        } => validate_draft(&CategoryDraft {
            category_id: id.clone(),
            name: name.clone(),
            definition: definition.clone(),
            description: name.clone(),
        }),
        AnalyticsCommand::MergeCategory { source, target } => {
            if !valid_slug(source) || !valid_slug(target) {
                bail!("category identifiers must be bounded slugs");
            }
            Ok(())
        }
        AnalyticsCommand::Categories => Ok(()),
    }
}
