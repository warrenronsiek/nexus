// @feature installation
// @feature runtime
// @spec docs/features/installation.md
// @spec docs/features/runtime.md
// @boundary compose-runtime-evidence: compare resolved resource references with inspected physical resources before retaining a deployment.
use anyhow::{bail, ensure, Context, Result};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum MountSource {
    Volume(String),
    Bind(String),
}
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct MountResource {
    target: String,
    source: MountSource,
    read_only: bool,
}

pub(super) fn verify_resources(container: &Value, model: &Value) -> Result<()> {
    let service = model
        .pointer("/services/nexus")
        .context("resolved Nexus service is missing")?;
    for field in ["configs", "secrets", "tmpfs"] {
        ensure!(
            empty(service.get(field)),
            "running {field} resources cannot be proven by this updater"
        );
    }
    ensure!(
        service.get("network_mode").is_none(),
        "network_mode cannot be proven by this updater"
    );
    ensure!(
        expected_mounts(service, model)? == actual_mounts(container)?,
        "resolved Compose mounts differ from the running physical resources"
    );
    ensure!(
        expected_networks(service, model, container)? == actual_networks(container)?,
        "resolved Compose network names or aliases differ from the running resources"
    );
    Ok(())
}

fn empty(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::Array(values)) => values.is_empty(),
        Some(Value::Object(values)) => values.is_empty(),
        _ => false,
    }
}
fn text(value: &Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .with_context(|| format!("runtime resource {key} is unavailable"))
}
fn expected_mounts(service: &Value, model: &Value) -> Result<BTreeSet<MountResource>> {
    let mut resources = BTreeSet::new();
    let Some(volumes) = service.get("volumes") else {
        return Ok(resources);
    };
    for mount in volumes
        .as_array()
        .context("Compose mounts must use resolved long form")?
    {
        ensure!(
            resources.insert(expected_mount(mount, model)?),
            "duplicate Compose mount identity"
        );
    }
    Ok(resources)
}
fn physical_name(model: &Value, collection: &str, key: &str) -> Result<String> {
    let definition = model
        .get(collection)
        .and_then(|v| v.get(key))
        .with_context(|| format!("physical Compose {collection} identity is unavailable"))?;
    text(definition, "name")
}
fn expected_mount(mount: &Value, model: &Value) -> Result<MountResource> {
    let read_only = mount
        .get("read_only")
        .map(|v| v.as_bool().context("invalid Compose mount read_only"))
        .transpose()?
        .unwrap_or(false);
    Ok(MountResource {
        target: text(mount, "target")?,
        source: expected_source(mount, model)?,
        read_only,
    })
}
fn expected_source(mount: &Value, model: &Value) -> Result<MountSource> {
    let source = text(mount, "source")?;
    match text(mount, "type")?.as_str() {
        "volume" => Ok(MountSource::Volume(physical_name(
            model, "volumes", &source,
        )?)),
        "bind" => {
            ensure!(
                std::path::Path::new(&source).is_absolute(),
                "resolved bind source must be absolute"
            );
            Ok(MountSource::Bind(source))
        }
        _ => bail!("mount type cannot be proven by this updater"),
    }
}
fn actual_mounts(container: &Value) -> Result<BTreeSet<MountResource>> {
    let mut resources = BTreeSet::new();
    for mount in container
        .get("Mounts")
        .and_then(Value::as_array)
        .context("running mounts are unavailable")?
    {
        let source = match text(mount, "Type")?.as_str() {
            "volume" => MountSource::Volume(text(mount, "Name")?),
            "bind" => MountSource::Bind(text(mount, "Source")?),
            _ => bail!("running mount type cannot be preserved by this updater"),
        };
        let read_only = !mount
            .get("RW")
            .and_then(Value::as_bool)
            .context("running mount access is unavailable")?;
        ensure!(
            resources.insert(MountResource {
                target: text(mount, "Destination")?,
                source,
                read_only
            }),
            "duplicate running mount identity"
        );
    }
    Ok(resources)
}
fn aliases(value: Option<&Value>) -> Result<BTreeSet<String>> {
    match value {
        None | Some(Value::Null) => Ok(BTreeSet::new()),
        Some(Value::Array(values)) => values
            .iter()
            .map(|v| {
                v.as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .context("invalid network alias")
            })
            .collect(),
        _ => bail!("network aliases are unavailable"),
    }
}
fn expected_networks(
    service: &Value,
    model: &Value,
    container: &Value,
) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let automatic = automatic_aliases(container)?;
    let mut networks = BTreeMap::new();
    for (key, settings) in service
        .get("networks")
        .and_then(Value::as_object)
        .context("resolved Compose networks are unavailable")?
    {
        let physical = physical_name(model, "networks", key)?;
        let mut names = aliases(settings.get("aliases"))?;
        names.extend(automatic.iter().cloned());
        ensure!(
            networks.insert(physical, names).is_none(),
            "duplicate physical Compose network"
        );
    }
    Ok(networks)
}
fn automatic_aliases(container: &Value) -> Result<BTreeSet<String>> {
    let name = text(container, "Name")?;
    let name = name.strip_prefix('/').unwrap_or(&name).to_owned();
    let service_name = container
        .pointer("/Config/Labels/com.docker.compose.service")
        .and_then(Value::as_str)
        .context("running Compose service identity is unavailable")?;
    ensure!(
        service_name == "nexus",
        "unrecognized running Compose service"
    );
    Ok(BTreeSet::from([name, service_name.to_owned()]))
}
fn actual_networks(container: &Value) -> Result<BTreeMap<String, BTreeSet<String>>> {
    container
        .pointer("/NetworkSettings/Networks")
        .and_then(Value::as_object)
        .context("running network identities are unavailable")?
        .iter()
        .map(|(name, settings)| Ok((name.clone(), aliases(settings.get("Aliases"))?)))
        .collect()
}

pub(super) fn runtime_fingerprint(container: &Value) -> Result<String> {
    let config = container
        .get("Config")
        .filter(|v| v.is_object())
        .context("running configuration is unavailable")?;
    let host = container
        .get("HostConfig")
        .filter(|v| v.is_object())
        .context("running host configuration is unavailable")?;
    let mut mounts = container
        .get("Mounts")
        .and_then(Value::as_array)
        .context("running mounts are unavailable")?
        .clone();
    mounts.sort_by_cached_key(Value::to_string);
    let networks = actual_networks(container)?;
    let evidence =
        serde_json::json!({"config":config,"host":host,"mounts":mounts,"networks":networks});
    Ok(blake3::hash(&serde_json::to_vec(&evidence)?)
        .to_hex()
        .to_string())
}

#[cfg(test)]
#[path = "compose_evidence/tests.rs"]
mod tests;
