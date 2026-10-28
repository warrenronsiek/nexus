// @feature installation
// @feature runtime
// @spec docs/features/installation.md
// @spec docs/features/runtime.md
// @boundary compose-runtime-fixtures: controlled Docker inspection and normalized Compose models.
use super::*;
use serde_json::{json, Value};

fn evidence() -> (Value, Value) {
    let container = json!({"Name":"/project-nexus-1","Config":{"Env":["SECRET=kept"],"Labels":{"com.docker.compose.service":"nexus"}},"HostConfig":{"ReadonlyRootfs":true},"Mounts":[{"Type":"volume","Name":"physical-state","Source":"/docker/volumes/physical-state/_data","Destination":"/var/lib/nexus","RW":true},{"Type":"bind","Source":"/absolute/config","Destination":"/etc/nexus","RW":false}],"NetworkSettings":{"Networks":{"physical-network":{"NetworkID":"old-network-id","IPAddress":"172.0.0.2","Aliases":["project-nexus-1","nexus","agents"]}}}});
    let model = json!({"services":{"nexus":{"volumes":[{"type":"volume","source":"state","target":"/var/lib/nexus"},{"type":"bind","source":"/absolute/config","target":"/etc/nexus","read_only":true}],"networks":{"default":{"aliases":["agents"]}}}},"volumes":{"state":{"name":"physical-state"}},"networks":{"default":{"name":"physical-network"}}});
    (container, model)
}

#[test]
fn physical_mounts_must_match_the_resolved_resources_before_recreation() {
    let (container, model) = evidence();
    verify_resources(&container, &model).unwrap();
    for changed in [
        {
            let mut value = model.clone();
            value["volumes"]["state"]["name"] = json!("different-state");
            value
        },
        {
            let mut value = model.clone();
            value["services"]["nexus"]["volumes"][1]["source"] = json!("/different/config");
            value
        },
        {
            let mut value = model.clone();
            value["services"]["nexus"]["volumes"][1]["read_only"] = json!(false);
            value
        },
    ] {
        assert!(
            verify_resources(&container, &changed).is_err(),
            "changed physical resources were accepted"
        );
    }
}

#[test]
fn every_running_network_and_alias_must_survive_the_retained_model() {
    let (container, model) = evidence();
    for changed in [
        {
            let mut value = model.clone();
            value["networks"]["default"]["name"] = json!("different-network");
            value
        },
        {
            let mut value = model.clone();
            value["services"]["nexus"]["networks"]["default"]["aliases"] = json!([]);
            value
        },
    ] {
        assert!(verify_resources(&container, &changed).is_err());
    }
    let mut extra = container.clone();
    extra["NetworkSettings"]["Networks"]["other"] = json!({"Aliases":["nexus"]});
    assert!(verify_resources(&extra, &model).is_err());
    let mut config = model.clone();
    config["services"]["nexus"]["configs"] = json!([{"source":"runtime-secret"}]);
    assert!(verify_resources(&container, &config).is_err());
}

#[test]
fn retained_fingerprint_binds_runtime_configuration_without_transient_network_addresses() {
    let (container, _) = evidence();
    let original = runtime_fingerprint(&container).unwrap();
    let mut dynamic = container.clone();
    dynamic["NetworkSettings"]["Networks"]["physical-network"]["NetworkID"] =
        json!("new-network-id");
    dynamic["NetworkSettings"]["Networks"]["physical-network"]["IPAddress"] = json!("172.0.0.9");
    dynamic["NetworkSettings"]["Networks"]["physical-network"]["Aliases"] =
        json!(["agents", "nexus", "project-nexus-1"]);
    assert_eq!(runtime_fingerprint(&dynamic).unwrap(), original);
    dynamic["Config"]["Env"] = json!(["SECRET=changed"]);
    assert_ne!(runtime_fingerprint(&dynamic).unwrap(), original);
    let mut security = container.clone();
    security["HostConfig"]["ReadonlyRootfs"] = json!(false);
    assert_ne!(runtime_fingerprint(&security).unwrap(), original);
    assert!(runtime_fingerprint(&json!({})).is_err());
}
