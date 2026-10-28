// @feature installation
// @feature runtime
// @spec docs/features/installation.md
// @spec docs/features/runtime.md
// @boundary isolated-compose-fixtures: controlled Docker metadata and commands prove update preservation without live services.
use super::*;
use serde_json::json;

fn container() -> serde_json::Value {
    json!({"Id":"container1","Name":"/existing-project-nexus-1","HostConfig":{},"Mounts":[{"Type":"volume","Name":"existing-state","Source":"/docker/existing-state","Destination":"/var/lib/nexus","RW":true}],"Image":"sha256:old","Config":{"Entrypoint":["nexus-server"],"Env":["NEXUS_DATABASE_URL=postgres://kept:secret@postgres/nexus","NEXUS_SECRET_FILE=/var/lib/nexus/service.secret","NEXUS_SECRET=existing$$token${VALUE}"],"Labels":{
        "com.docker.compose.project":"existing-project", "com.docker.compose.service":"nexus", "com.docker.compose.project.working_dir":"/old/source", "com.docker.compose.project.config_files":"/old/source/docker-compose.yml"}},
        "NetworkSettings":{"Ports":{"7337/tcp":[{"HostIp":"127.0.0.1","HostPort":"7337"}]},"Networks":{"existing-project_default":{"Aliases":["existing-project-nexus-1","nexus"]}}},"State":{"Running":true,"Health":{"Status":"healthy"}}})
}
#[test]
fn compose_snapshot_keeps_existing_secrets_storage_and_service_settings() {
    let model = json!({"name":"existing-project","services":{"nexus":{"networks":{"default":null},"build":{"context":"/old/source"},"environment":{"NEXUS_DATABASE_URL":"wrong-current-default"},"depends_on":{"postgres":{"condition":"service_healthy"}},"healthcheck":{"test":["CMD","check"]},"volumes":[{"type":"volume","source":"state","target":"/var/lib/nexus"}],"ports":[{"target":7337,"published":"7337","host_ip":"127.0.0.1"}]},"postgres":{"image":"postgres:17"}},"volumes":{"state":{"name":"existing-state"}},"networks":{"default":{"name":"existing-project_default"}}});
    let deployment =
        ComposeDeployment::from_running("http://127.0.0.1:7337", &container(), model).unwrap();
    let updated = deployment.with_image("sha256:new").unwrap();
    assert_eq!(updated["name"], "existing-project");
    assert_eq!(updated["services"]["nexus"]["image"], "sha256:new");
    assert!(updated["services"]["nexus"].get("build").is_none());
    assert!(updated["services"]["nexus"].get("depends_on").is_none());
    assert!(updated["services"].get("postgres").is_none());
    assert_eq!(
        updated["services"]["nexus"]["environment"]["NEXUS_DATABASE_URL"],
        "postgres://kept:secret@postgres/nexus"
    );
    assert_eq!(
        updated["services"]["nexus"]["healthcheck"]["test"][1],
        "check"
    );
    assert_eq!(updated["volumes"]["state"]["name"], "existing-state");
    assert_eq!(deployment.project, "existing-project");
    assert_eq!(deployment.previous_image, "sha256:old");
}
#[test]
fn compose_adoption_requires_the_exact_running_local_endpoint_and_known_service() {
    let model = json!({"services":{"nexus":{}},"volumes":{}});
    assert!(ComposeDeployment::from_running(
        "https://other.example:7337",
        &container(),
        model.clone()
    )
    .is_err());
    assert!(
        ComposeDeployment::from_running("http://127.0.0.1:9999", &container(), model.clone())
            .is_err()
    );
    let mut wrong = container();
    wrong["Config"]["Entrypoint"] = json!(["unrelated"]);
    assert!(
        ComposeDeployment::from_running("http://127.0.0.1:7337", &wrong, model.clone()).is_err()
    );
    assert_eq!(
        docker_target("linux", "amd64").unwrap(),
        "x86_64-unknown-linux-gnu"
    );
    assert_eq!(
        docker_target("linux", "arm64").unwrap(),
        "aarch64-unknown-linux-gnu"
    );
    assert!(docker_target("darwin", "arm64").is_err());
}
fn docker_fixture(directory: &Path) -> (Docker, PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let executable = directory.join("docker");
    let state = directory.join("private-state");
    let observed = directory.join("observed.json");
    let arguments = directory.join("arguments");
    let mut ready = container();
    ready["Image"] = json!("sha256:new");
    let metadata = directory.join("container.json");
    std::fs::write(&metadata, serde_json::to_vec(&vec![ready]).unwrap()).unwrap();
    let script = format!(
        r#"#!/bin/sh
case "$1 $2" in
  'context inspect') printf '%s\n' '"unix:///docker-fixture.sock"' ;;
  'version --format') printf '%s\n' '{{"Os":"linux","Arch":"arm64"}}' ;;
  'image load') exit 0 ;;
  'image inspect') printf '%s\n' '[{{"Id":"sha256:new"}}]' ;;
  'compose --project-name') cp "$5" '{}'; printf '%s\n' "$@" > '{}' ;;
  'ps --filter') printf '%s\n' fixture ;;
  'inspect fixture') cat '{}' ;;
  *) exit 9 ;;
esac
"#,
        observed.display(),
        arguments.display(),
        metadata.display()
    );
    std::fs::write(&executable, script).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let docker = Docker {
        executable,
        state_directory: state,
    };
    (docker, observed, arguments)
}
#[test]
fn docker_uses_the_server_platform_and_recreates_only_the_pinned_service() {
    let directory = tempfile::tempdir().unwrap();
    let (docker, observed, arguments) = docker_fixture(directory.path());
    assert_eq!(docker.target().unwrap(), "aarch64-unknown-linux-gnu");
    let model = json!({"services":{"nexus":{"networks":{"default":null},"volumes":[{"source":"state","target":"/var/lib/nexus","type":"volume"}]}},"volumes":{"state":{"name":"existing-state"}},"networks":{"default":{"name":"existing-project_default"}}});
    let deployment =
        ComposeDeployment::from_running("http://127.0.0.1:7337", &container(), model).unwrap();
    docker.load_image(directory.path(), "sha256:new").unwrap();
    docker.replace(&deployment, "sha256:new").unwrap();
    let retained = ComposeDeployment::retained(&docker.state_directory, "http://127.0.0.1:7337")
        .unwrap()
        .unwrap();
    assert_eq!(
        retained.model["services"]["nexus"]["environment"]["NEXUS_SECRET"],
        "existing$$token${VALUE}",
        "Docker metadata retains literal dollars without Compose decoding"
    );
    let model: serde_json::Value =
        serde_json::from_slice(&std::fs::read(observed).unwrap()).unwrap();
    assert_eq!(model["services"]["nexus"]["image"], "sha256:new");
    assert_eq!(
        model["services"]["nexus"]["environment"]["NEXUS_SECRET"], "existing$$$$token$${VALUE}",
        "literal secrets must not be re-interpolated by Compose"
    );
    assert_eq!(model["volumes"]["state"]["name"], "existing-state");
    let arguments = std::fs::read_to_string(arguments).unwrap();
    assert!(arguments.contains("--no-build\n--no-deps\n--pull\nnever\nnexus\n"));
    assert!(!arguments
        .lines()
        .any(|arg| matches!(arg, "down" | "--volumes" | "--build")));
}
#[test]
fn an_explicit_compose_file_selects_its_exact_project_and_refuses_missing_ownership() {
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("first.yml");
    std::fs::write(&first, "services: {}").unwrap();
    let second = directory.path().join("second.yml");
    std::fs::write(&second, "services: {}").unwrap();
    let mut left = container();
    left["Id"] = json!("left");
    left["Config"]["Labels"]["com.docker.compose.project.config_files"] = json!(first);
    let mut right = container();
    right["Id"] = json!("right");
    right["Config"]["Labels"]["com.docker.compose.project.config_files"] = json!(second);
    let selected = select_container(vec![left.clone(), right], Some(&first))
        .unwrap()
        .unwrap();
    assert_eq!(selected["Id"], "left");
    assert!(select_container(vec![left], Some(&second)).is_err());
    assert!(select_container(Vec::new(), Some(&first)).is_err());
}
#[test]
fn edited_source_mounts_are_rejected_instead_of_switching_the_running_dataset() {
    let mut running = container();
    running["Mounts"] =
        json!([{"Type":"volume","Name":"original-state","Destination":"/var/lib/nexus","RW":true}]);
    let edited = json!({"services":{"nexus":{"volumes":[{"type":"volume","source":"state","target":"/var/lib/nexus"}]}},"volumes":{"state":{"name":"edited-state"}}});
    assert!(
        ComposeDeployment::from_running("http://127.0.0.1:7337", &running, edited).is_err(),
        "an edited Compose source must not replace the running data volume"
    );
}

#[test]
fn source_hash_and_retained_runtime_identity_are_required_before_adoption() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("docker");
    std::fs::write(
        &executable,
        format!("#!/bin/sh\nprintf 'nexus %s\\n' '{}'\n", "b".repeat(64)),
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let docker = Docker {
        executable,
        state_directory: directory.path().join("private"),
    };
    let mut running = container();
    running["Config"]["Labels"]["com.docker.compose.config-hash"] = json!("a".repeat(64));
    let model = json!({"services":{"nexus":{"command":["edited"],"networks":{"default":null},"volumes":[{"type":"volume","source":"state","target":"/var/lib/nexus"}]}},"volumes":{"state":{"name":"existing-state"}},"networks":{"default":{"name":"existing-project_default"}}});
    let error = docker
        .verify_service_hash("existing-project", &model, &running)
        .unwrap_err();
    assert!(error.downcast_ref::<ManagementConflict>().is_some());
    std::fs::write(
        &docker.executable,
        format!("#!/bin/sh\nprintf 'nexus %s\\n' '{}'\n", "a".repeat(64)),
    )
    .unwrap();
    docker
        .verify_service_hash("existing-project", &model, &running)
        .unwrap();
    let deployment =
        ComposeDeployment::from_running("http://127.0.0.1:7337", &running, model).unwrap();
    docker.retain(&deployment).unwrap();
    assert!(docker
        .retained_model("http://127.0.0.1:7337/", &running, "existing-project")
        .unwrap()
        .is_some());
    running["Config"]["Env"] = json!(["NEXUS_SECRET=different"]);
    assert!(docker
        .retained_model("http://127.0.0.1:7337", &running, "existing-project")
        .is_err());
}
