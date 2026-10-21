// @feature usage-analytics
// @spec docs/features/usage-analytics.md
use crate::coordination::domain::HookContext;
use serde::{Deserialize, Serialize};

pub const POLICY_VERSION: &str = "activity-v1";
pub const MAX_RESOURCES: usize = 256;
pub const MAX_ACTIONS: usize = 64;
pub const MAX_CLASSIFIER_BYTES: usize = 8 * 1024;

macro_rules! labels {
    ($name:ident { $($variant:ident => $label:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name { $(#[serde(rename=$label)] $variant),+ }
        impl $name { pub fn as_str(self) -> &'static str { match self { $(Self::$variant => $label),+ } } }
    }
}
labels!(ResourceKind { Tool=>"tool", Script=>"script", Cli=>"cli", Inline=>"inline", Skill=>"skill" });
labels!(Origin { HostTool=>"host_tool", Repository=>"repository", StandardCli=>"standard_cli", AgentAuthored=>"agent_authored", Unknown=>"unknown", Internal=>"internal" });
labels!(Exposure { Declared=>"declared", Discoverable=>"discoverable", Exposed=>"exposed" });

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub name: String,
    pub kind: ResourceKind,
    pub origin: Origin,
    pub exposure: Exposure,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub activities: Vec<String>,
    #[serde(default)]
    pub version: Option<String>,
}
impl Resource {
    pub fn key(&self, agent: &str) -> String {
        resource_key(self.kind, &self.name, agent)
    }
}
pub fn resource_key(kind: ResourceKind, name: &str, agent: &str) -> String {
    if kind == ResourceKind::Tool {
        format!("tool:{}:{agent}:{name}", agent.len())
    } else {
        format!("{}:{name}", kind.as_str())
    }
}
pub fn native_tool_key(key: &str) -> Option<(&str, &str)> {
    let (length, rest) = key.strip_prefix("tool:")?.split_once(':')?;
    let length = length.parse::<usize>().ok()?;
    let (agent, name) = rest.split_at_checked(length)?;
    Some((agent, name.strip_prefix(':')?))
}
pub fn resource_pattern(resource: &Resource) -> String {
    super::identity(&[
        POLICY_VERSION,
        resource.kind.as_str(),
        &resource.name,
        &resource.description,
        resource.version.as_deref().unwrap_or(""),
    ])
}
fn complete_default() -> bool {
    true
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnalyticsCommand {
    Categories,
    ReserveClassification {
        #[serde(flatten)]
        context: HookContext,
        target: ClassificationTarget,
    },
    Classify {
        #[serde(flatten)]
        context: HookContext,
        target: ClassificationTarget,
        draft: CategoryDraft,
        #[serde(rename = "classifier_model")]
        model: String,
    },
    ClassificationFailed {
        #[serde(flatten)]
        context: HookContext,
        target: ClassificationTarget,
    },
    Summary {
        #[serde(default)]
        project_id: Option<String>,
    },
    Catalog {
        #[serde(default)]
        project_id: Option<String>,
        #[serde(default)]
        offset: u32,
        #[serde(default)]
        limit: Option<u32>,
    },
    Opportunities {
        #[serde(default)]
        project_id: Option<String>,
    },
    Examples {
        #[serde(default)]
        project_id: Option<String>,
        #[serde(default)]
        category_id: Option<String>,
        #[serde(default)]
        resource_key: Option<String>,
        #[serde(default)]
        offset: u32,
    },
    Inventory {
        #[serde(flatten)]
        context: HookContext,
        scope: String,
        resources: Vec<Resource>,
        #[serde(default = "complete_default")]
        complete: bool,
    },
    RenameCategory {
        id: String,
        name: String,
        definition: String,
    },
    MergeCategory {
        source: String,
        target: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClassificationTarget {
    Action {
        invocation_id: String,
        index: u32,
        pattern_key: String,
    },
    Resource {
        name: String,
        resource_kind: ResourceKind,
        pattern_key: String,
    },
}
impl ClassificationTarget {
    pub fn pattern_key(&self) -> &str {
        match self {
            Self::Action { pattern_key, .. } | Self::Resource { pattern_key, .. } => pattern_key,
        }
    }
}
#[derive(Clone, Debug)]
pub enum ClassificationRecord {
    Action(String),
    Resource(String),
}

#[derive(Clone, Debug)]
pub struct Action {
    pub name: String,
    pub kind: ResourceKind,
    pub origin: Origin,
    pub category_id: String,
    pub description: String,
    pub pattern_key: String,
    pub model_input: Option<String>,
}
impl Action {
    pub fn resource(&self, category: &str, description: &str) -> Resource {
        Resource {
            name: self.name.clone(),
            kind: self.kind,
            origin: self.origin,
            exposure: Exposure::Declared,
            description: description.into(),
            activities: vec![category.into()],
            version: Some(self.pattern_key.clone()),
        }
    }
}
#[derive(Clone, Debug)]
pub struct ClassificationJob {
    pub project_id: String,
    pub record: ClassificationRecord,
    pub pattern_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategoryDraft {
    pub category_id: String,
    pub name: String,
    pub definition: String,
    pub description: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub definition: String,
}
#[derive(Debug, Serialize)]
pub struct ActivityCount {
    pub category_id: String,
    pub name: String,
    pub calls: i64,
    pub actions: i64,
    pub sessions: i64,
    pub improvised_actions: i64,
}
#[derive(Debug, Serialize)]
pub struct ActivitySummary {
    pub ok: bool,
    pub observed_calls: i64,
    pub identified_actions: i64,
    pub classified_actions: i64,
    pub unclassified_actions: i64,
    pub activities: Vec<ActivityCount>,
    pub truncated: bool,
    pub window_started_at: String,
    pub window_ended_at: String,
    pub counting: &'static str,
}
#[derive(Debug, Serialize)]
pub struct CapabilityCount {
    pub resource_key: String,
    pub name: String,
    pub kind: String,
    pub origin: String,
    pub calls: i64,
    pub offered_sessions: i64,
    pub discoverable_sessions: i64,
    pub availability: &'static str,
    pub description: String,
}
#[derive(Debug, Serialize)]
pub struct CatalogPage {
    pub ok: bool,
    pub capabilities: Vec<CapabilityCount>,
    pub next_offset: Option<u32>,
}
#[derive(Debug, Serialize)]
pub struct Opportunity {
    pub category_id: String,
    pub name: String,
    pub calls: i64,
    pub sessions: i64,
    pub supplied_alternatives: Vec<String>,
    pub evidence: &'static str,
}
#[derive(Debug, Serialize)]
pub struct Opportunities {
    pub ok: bool,
    pub opportunities: Vec<Opportunity>,
    pub truncated: bool,
}
#[derive(Debug, Serialize, diesel::Queryable)]
pub struct ActivityExample {
    pub name: String,
    pub description: String,
    pub origin: String,
    pub evidence: String,
    pub tool_name: String,
    pub call_outcome: String,
    pub model_id: Option<String>,
    pub observed_at: String,
    pub category_id: String,
}
#[derive(Debug, Serialize)]
pub struct ExamplesPage {
    pub ok: bool,
    pub examples: Vec<ActivityExample>,
    pub next_offset: Option<u32>,
}
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum AnalyticsResponse {
    Summary(ActivitySummary),
    Catalog(CatalogPage),
    Examples(ExamplesPage),
    Opportunities(Opportunities),
    Categories { ok: bool, categories: Vec<Category> },
    Permit { ok: bool, granted: bool },
    Write { ok: bool },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryInventory {
    pub resources: Vec<Resource>,
    pub complete: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tool_identity_does_not_confuse_namespace_with_name_delimiters() {
        assert_ne!(
            resource_key(ResourceKind::Tool, "c", "a:b"),
            resource_key(ResourceKind::Tool, "b:c", "a")
        );
    }
}
