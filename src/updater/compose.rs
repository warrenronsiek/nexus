// @feature installation
// @feature runtime
// @spec docs/features/installation.md
// @spec docs/features/runtime.md
// @boundary docker-compose-management: verifies engine identity, exact container endpoint, and private resolved Compose snapshots before replacing only the Nexus service image.

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(super) struct ManagementConflict(&'static str);

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ComposeDeployment {
    pub(super) endpoint: String,
    pub(super) project: String,
    pub(super) previous_image: String,
    model: Value,
    runtime_fingerprint: String,
}

impl ComposeDeployment {
    fn from_running(endpoint: &str, container: &Value, mut model: Value) -> Result<Self> {
        let identity = ContainerIdentity::observe(endpoint, container)?;
        super::compose_evidence::verify_resources(container, &model).map_err(|_|ManagementConflict("resolved Compose resources differ from the running service; exact preservation cannot be proven"))?;
        let runtime_fingerprint = super::compose_evidence::runtime_fingerprint(container)?;
        let environment = running_environment(container)?;
        scope_model(&mut model, &identity, environment)?;
        Ok(Self {
            endpoint: endpoint.into(),
            project: identity.project,
            previous_image: identity.image,
            model,
            runtime_fingerprint,
        })
    }
    pub(super) fn with_image(&self, image: &str) -> Result<Value> {
        ensure!(
            image.starts_with("sha256:"),
            "service image must be pinned by ID"
        );
        let mut model = self.model.clone();
        model["services"]["nexus"]["image"] = Value::String(image.into());
        Ok(model)
    }
    pub(super) fn save(&self, directory: &Path) -> Result<()> {
        let path = descriptor_path(directory, &self.endpoint);
        crate::private_state::write(&path, &serde_json::to_vec(self)?)
    }
    fn retained(directory: &Path, endpoint: &str) -> Result<Option<Self>> {
        let path = descriptor_path(directory, endpoint);
        let Some(bytes) = crate::private_state::read(&path, 2 * 1024 * 1024)? else {
            return Ok(None);
        };
        let deployment: Self =
            serde_json::from_slice(&bytes).context("invalid managed Compose descriptor")?;
        ensure!(
            endpoint_identity(&deployment.endpoint) == endpoint_identity(endpoint),
            "managed service endpoint mismatch"
        );
        Ok(Some(deployment))
    }
}

struct ContainerIdentity {
    project: String,
    image: String,
}
impl ContainerIdentity {
    fn observe(endpoint: &str, container: &Value) -> Result<Self> {
        let labels = container
            .pointer("/Config/Labels")
            .context("container has no Compose identity")?;
        validate_container(endpoint, container, labels)?;
        let project = field(labels, "com.docker.compose.project")?.to_owned();
        ensure!(
            !project.is_empty() && project.len() <= 256,
            "invalid Compose project"
        );
        Ok(Self {
            project,
            image: field(container, "Image")?.into(),
        })
    }
}
fn validate_container(endpoint: &str, container: &Value, labels: &Value) -> Result<()> {
    ensure!(
        matches_endpoint(endpoint, container)?,
        "Compose container does not serve the configured local endpoint"
    );
    ensure!(
        container.pointer("/State/Running").and_then(Value::as_bool) == Some(true),
        "Compose service is not running"
    );
    ensure!(
        field(labels, "com.docker.compose.service")? == "nexus",
        "unrecognized Compose service"
    );
    let entrypoint = container
        .pointer("/Config/Entrypoint")
        .and_then(Value::as_array)
        .context("unrecognized service entrypoint")?;
    ensure!(
        entrypoint.iter().any(|value| matches!(
            value.as_str(),
            Some("nexus-server" | "/usr/local/bin/nexus-server")
        )),
        "unrecognized Nexus service entrypoint"
    );
    Ok(())
}
fn scope_model(model: &mut Value, identity: &ContainerIdentity, environment: Value) -> Result<()> {
    let services = model
        .get_mut("services")
        .and_then(Value::as_object_mut)
        .context("Compose model has no services")?;
    let mut nexus = services
        .remove("nexus")
        .context("Compose model has no Nexus service")?;
    let object = nexus
        .as_object_mut()
        .context("invalid Nexus service model")?;
    object.remove("build");
    object.remove("depends_on");
    object.remove("pull_policy");
    object.insert("image".into(), Value::String(identity.image.clone()));
    object.insert("environment".into(), environment);
    *services = Map::from_iter([("nexus".into(), nexus)]);
    external_resources(model)?;
    model["name"] = Value::String(identity.project.clone());
    Ok(())
}

fn external_resources(model: &mut Value) -> Result<()> {
    let object = model
        .as_object_mut()
        .context("invalid Compose project model")?;
    object.retain(|key, _| matches!(key.as_str(), "name" | "services" | "volumes" | "networks"));
    for kind in ["volumes", "networks"] {
        if let Some(resources) = object.get_mut(kind).and_then(Value::as_object_mut) {
            for definition in resources.values_mut() {
                let name = field(definition, "name")?.to_owned();
                *definition = serde_json::json!({"name":name,"external":true});
            }
        }
    }
    Ok(())
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .context("required Docker identity is missing")
}
fn running_environment(container: &Value) -> Result<Value> {
    let mut environment = Map::new();
    for entry in container
        .pointer("/Config/Env")
        .and_then(Value::as_array)
        .context("running service environment is unavailable")?
    {
        let (key, value) = entry
            .as_str()
            .and_then(|entry| entry.split_once('='))
            .context("invalid running service environment")?;
        environment.insert(key.into(), Value::String(value.into()));
    }
    Ok(Value::Object(environment))
}
fn matches_endpoint(endpoint: &str, container: &Value) -> Result<bool> {
    let url = reqwest::Url::parse(endpoint)?;
    ensure!(
        url.scheme() == "http"
            && matches!(
                url.host_str(),
                Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
            ),
        "only exact local HTTP Compose services can be managed"
    );
    ensure!(
        url.path() == "/" || url.path().is_empty(),
        "service endpoint must be its root URL"
    );
    let port = url
        .port_or_known_default()
        .context("service endpoint has no port")?
        .to_string();
    let bindings = container
        .pointer("/NetworkSettings/Ports/7337~1tcp")
        .and_then(Value::as_array)
        .context("Nexus service port is not published")?;
    Ok(bindings.iter().any(|binding| {
        binding.get("HostPort").and_then(Value::as_str) == Some(&port)
            && matches!(
                binding.get("HostIp").and_then(Value::as_str),
                Some("127.0.0.1" | "0.0.0.0" | "::" | "::1")
            )
    }))
}
fn descriptor_path(directory: &Path, endpoint: &str) -> PathBuf {
    directory.join(format!(
        "compose-{}.json",
        blake3::hash(endpoint_identity(endpoint).as_bytes()).to_hex()
    ))
}
pub(super) fn docker_target(os: &str, architecture: &str) -> Result<&'static str> {
    match (os, architecture) {
        ("linux", "amd64" | "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("linux", "arm64" | "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        _ => bail!("no Nexus release image supports this Docker server platform"),
    }
}

pub(super) struct Docker {
    executable: PathBuf,
    state_directory: PathBuf,
}
impl Docker {
    pub(super) fn system() -> Result<Self> {
        Ok(Self {
            executable: "docker".into(),
            state_directory: super::state_directory()?,
        })
    }
    fn command(
        &self,
        args: &[&std::ffi::OsStr],
        timeout: Duration,
        limit: usize,
    ) -> Result<Vec<u8>> {
        let mut command = std::process::Command::new(&self.executable);
        command.args(args);
        let output = crate::git_process::bounded_output(command, timeout, limit)
            .context("Docker command did not complete")?;
        ensure!(
            output.status.success(),
            "Docker command failed; inspect the managed service locally"
        );
        Ok(output.stdout)
    }
    fn json(&self, args: &[&str]) -> Result<Value> {
        let args: Vec<_> = args.iter().map(std::ffi::OsStr::new).collect();
        serde_json::from_slice(&self.command(&args, Duration::from_secs(20), 2 * 1024 * 1024)?)
            .context("Docker returned invalid management metadata")
    }
    fn require_local_engine(&self) -> Result<()> {
        ensure!(
            std::env::var("DOCKER_HOST")
                .ok()
                .is_none_or(|host| host.starts_with("unix://")),
            "remote Docker hosts are externally managed"
        );
        let context = self.json(&[
            "context",
            "inspect",
            "--format",
            "{{json .Endpoints.docker.Host}}",
        ])?;
        ensure!(
            context
                .as_str()
                .is_some_and(|host| host.starts_with("unix://")),
            "remote Docker contexts are externally managed"
        );
        Ok(())
    }
    pub(super) fn target(&self) -> Result<&'static str> {
        self.require_local_engine()?;
        let server = self.json(&["version", "--format", "{{json .Server}}"])?;
        docker_target(field(&server, "Os")?, field(&server, "Arch")?)
    }
    fn running(&self) -> Result<Vec<Value>> {
        let bytes = self.command(
            &[
                "ps".as_ref(),
                "--filter".as_ref(),
                "label=com.docker.compose.service=nexus".as_ref(),
                "--format".as_ref(),
                "{{.ID}}".as_ref(),
            ],
            Duration::from_secs(10),
            64 * 1024,
        )?;
        let ids = std::str::from_utf8(&bytes)?;
        let mut containers = Vec::new();
        ensure!(
            ids.lines().count() <= 64,
            "too many candidate Compose services"
        );
        for id in ids.lines().filter(|id| !id.is_empty()) {
            let inspected = self.json(&["inspect", id])?;
            containers.push(
                inspected
                    .as_array()
                    .and_then(|values| values.first())
                    .context("Docker container disappeared")?
                    .clone(),
            );
        }
        Ok(containers)
    }
    pub(super) fn discover(
        &self,
        endpoint: &str,
        explicit: Option<&Path>,
    ) -> Result<Option<ComposeDeployment>> {
        let Some(container) = self.find_container(endpoint, explicit)? else {
            return Ok(None);
        };
        let model = self.resolve_model(endpoint, &container, explicit)?;
        Ok(Some(ComposeDeployment::from_running(
            endpoint, &container, model,
        )?))
    }
    fn find_container(&self, endpoint: &str, explicit: Option<&Path>) -> Result<Option<Value>> {
        self.require_local_engine()?;
        let candidates: Vec<_> = self
            .running()?
            .into_iter()
            .filter(|container| matches_endpoint(endpoint, container).unwrap_or(false))
            .collect();
        select_container(candidates, explicit)
    }

    fn resolve_model(
        &self,
        endpoint: &str,
        container: &Value,
        explicit: Option<&Path>,
    ) -> Result<Value> {
        let labels = container
            .pointer("/Config/Labels")
            .context("Compose labels missing")?;
        let project = field(labels, "com.docker.compose.project")?;
        if explicit.is_none() {
            if let Some(model) = self.retained_model(endpoint, container, project)? {
                return Ok(model);
            }
        }
        let files = compose_files(labels, explicit)?;
        self.source_model(project, &files, container)
    }
    fn retained_model(
        &self,
        endpoint: &str,
        container: &Value,
        project: &str,
    ) -> Result<Option<Value>> {
        let Some(retained) = ComposeDeployment::retained(&self.state_directory, endpoint)? else {
            return Ok(None);
        };
        ensure!(
            retained.project == project,
            ManagementConflict(
                "managed Compose project changed; explicit verified adoption is required"
            )
        );
        ensure!(retained.runtime_fingerprint==super::compose_evidence::runtime_fingerprint(container)?,ManagementConflict("running Compose configuration differs from the retained management evidence; service update refused"));
        Ok(Some(retained.model))
    }
    fn source_model(&self, project: &str, files: &[PathBuf], container: &Value) -> Result<Value> {
        let mut model = self.compose_config(project, files)?;
        hydrate_declared_environment(&mut model, container)?;
        self.verify_service_hash(project, &model, container)?;
        Ok(model)
    }
    fn verify_service_hash(&self, project: &str, model: &Value, container: &Value) -> Result<()> {
        let expected=container.pointer("/Config/Labels/com.docker.compose.config-hash").and_then(Value::as_str).context("running Compose service has no configuration hash; exact settings cannot be proven")?;
        ensure!(
            super::release::hex(expected, 64),
            "running Compose service has an invalid configuration hash"
        );
        let observed = self.service_hash(project, model)?;
        ensure!(observed==expected,ManagementConflict("resolved Compose settings differ from the running service; restore matching settings before adoption"));
        Ok(())
    }
    fn service_hash(&self, project: &str, model: &Value) -> Result<String> {
        let bytes = super::compose_format::encode(model)?;
        let file = crate::private_state::temporary_in(&self.state_directory, &bytes)?;
        let bytes = self.command(
            &[
                "compose".as_ref(),
                "--project-name".as_ref(),
                project.as_ref(),
                "--file".as_ref(),
                file.path().as_os_str(),
                "config".as_ref(),
                "--hash".as_ref(),
                "nexus".as_ref(),
            ],
            Duration::from_secs(20),
            4096,
        )?;
        decode_service_hash(&bytes)
    }
    fn compose_config(&self, project: &str, files: &[PathBuf]) -> Result<Value> {
        let mut args = vec![
            "compose".as_ref(),
            "--project-name".as_ref(),
            project.as_ref(),
        ];
        for file in files {
            args.push("--file".as_ref());
            args.push(file.as_os_str());
        }
        args.extend([
            std::ffi::OsStr::new("config"),
            std::ffi::OsStr::new("--format"),
            std::ffi::OsStr::new("json"),
        ]);
        super::compose_format::decode(&self.command(
            &args,
            Duration::from_secs(20),
            2 * 1024 * 1024,
        )?)
        .context("Compose configuration could not be resolved")
    }
    pub(super) fn load_image(&self, path: &Path, expected: &str) -> Result<()> {
        self.command(
            &[
                "image".as_ref(),
                "load".as_ref(),
                "--input".as_ref(),
                path.as_os_str(),
            ],
            Duration::from_secs(180),
            64 * 1024,
        )?;
        let image = self.json(&["image", "inspect", expected])?;
        ensure!(
            image
                .as_array()
                .and_then(|values| values.first())
                .and_then(|value| value.get("Id"))
                .and_then(Value::as_str)
                == Some(expected),
            "loaded service image identity differs from the release manifest"
        );
        Ok(())
    }
    pub(super) fn replace(&self, deployment: &ComposeDeployment, image: &str) -> Result<()> {
        let model = deployment.with_image(image)?;
        self.recreate(&deployment.project, &model)?;
        let container = self.wait_ready(deployment, image)?;
        let updated = ComposeDeployment::from_running(&deployment.endpoint, &container, model)?;
        self.retain(&updated)
    }
    fn recreate(&self, project: &str, model: &Value) -> Result<()> {
        let file = crate::private_state::temporary_in(
            &self.state_directory,
            &super::compose_format::encode(model)?,
        )?;
        self.command(
            &[
                "compose".as_ref(),
                "--project-name".as_ref(),
                project.as_ref(),
                "--file".as_ref(),
                file.path().as_os_str(),
                "up".as_ref(),
                "--detach".as_ref(),
                "--no-build".as_ref(),
                "--no-deps".as_ref(),
                "--pull".as_ref(),
                "never".as_ref(),
                "nexus".as_ref(),
            ],
            Duration::from_secs(120),
            64 * 1024,
        )
        .map(|_| ())
    }
    pub(super) fn retain(&self, deployment: &ComposeDeployment) -> Result<()> {
        deployment.save(&self.state_directory)
    }
    fn wait_ready(&self, deployment: &ComposeDeployment, image: &str) -> Result<Value> {
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            let ready = self.running()?.into_iter().find(|container| {
                container
                    .pointer("/Config/Labels/com.docker.compose.project")
                    .and_then(Value::as_str)
                    == Some(&deployment.project)
                    && container.get("Image").and_then(Value::as_str) == Some(image)
                    && container.pointer("/State/Running").and_then(Value::as_bool) == Some(true)
                    && container
                        .pointer("/State/Health/Status")
                        .and_then(Value::as_str)
                        == Some("healthy")
            });
            if let Some(container) = ready {
                return Ok(container);
            }
            ensure!(
                std::time::Instant::now() < deadline,
                "updated Compose service did not become healthy"
            );
            std::thread::sleep(Duration::from_millis(250));
        }
    }
}
fn endpoint_identity(endpoint: &str) -> String {
    reqwest::Url::parse(endpoint)
        .map(|url| url.to_string())
        .unwrap_or_else(|_| endpoint.into())
}
fn decode_service_hash(bytes: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(bytes)?;
    let parts: Vec<_> = text.split_whitespace().collect();
    ensure!(
        parts.len() == 2 && parts[0] == "nexus" && super::release::hex(parts[1], 64),
        "Compose returned an invalid service configuration hash"
    );
    Ok(parts[1].into())
}
fn hydrate_declared_environment(model: &mut Value, container: &Value) -> Result<()> {
    let running = running_environment(container)?;
    if let Some(declared) = model
        .pointer_mut("/services/nexus/environment")
        .and_then(Value::as_object_mut)
    {
        for (key, value) in declared {
            match running.get(key) {
                Some(observed) => *value = observed.clone(),
                None => ensure!(
                    value.is_null(),
                    "declared Compose environment differs from the running service"
                ),
            }
        }
    }
    Ok(())
}

fn select_container(mut candidates: Vec<Value>, explicit: Option<&Path>) -> Result<Option<Value>> {
    if let Some(explicit) = explicit {
        let expected = explicit
            .canonicalize()
            .context("explicit Compose file is unavailable")?;
        candidates.retain(|container| {
            container
                .pointer("/Config/Labels/com.docker.compose.project.config_files")
                .and_then(Value::as_str)
                .is_some_and(|files| {
                    files
                        .split(',')
                        .any(|file| Path::new(file).canonicalize().ok().as_ref() == Some(&expected))
                })
        });
        ensure!(
            !candidates.is_empty(),
            "explicit Compose file does not own the configured running service"
        );
    }
    ensure!(
        candidates.len() <= 1,
        "multiple Compose services match; supply an exact --compose-file"
    );
    Ok(candidates.into_iter().next())
}
fn compose_files(labels: &Value, explicit: Option<&Path>) -> Result<Vec<PathBuf>> {
    let recorded = field(labels, "com.docker.compose.project.config_files")?;
    let files: Vec<PathBuf> = recorded.split(',').map(PathBuf::from).collect();
    ensure!(
        !files.is_empty() && files.len() <= 16 && files.iter().all(|path| path.is_absolute()),
        "Compose source files cannot be identified"
    );
    if let Some(explicit) = explicit {
        let explicit = explicit
            .canonicalize()
            .context("explicit Compose file is unavailable")?;
        ensure!(
            files
                .iter()
                .any(|path| path.canonicalize().ok().as_ref() == Some(&explicit)),
            "explicit Compose file does not own the configured running service"
        );
    } else {
        ensure!(
            files.iter().all(|file| file
                .file_name()
                .is_some_and(|name| name == "docker-compose.yml")
                && file.is_file()),
            "legacy Compose deployment needs --compose-file to establish management"
        );
    }
    Ok(files)
}

#[cfg(test)]
#[path = "compose/tests.rs"]
mod tests;
