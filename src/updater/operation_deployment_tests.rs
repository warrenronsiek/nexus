// @feature installation
// @spec docs/features/installation.md
// @boundary update-transaction-fixtures: fake executables and loopback artifacts exercise supervised deployment rollback without live state.
use super::{tests::*, *};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

struct DeploymentRun {
    directory: tempfile::TempDir,
    result: Result<Report>,
    calls: Vec<serde_json::Value>,
    binary: Vec<u8>,
}
fn adapter(directory: &Path, outcome: &str) -> (PathBuf, PathBuf) {
    let log = directory.join("calls");
    let command = directory.join("deployment updater");
    let script = format!(
        r#"#!/usr/bin/env python3
import json,sys
from pathlib import Path
r=json.load(sys.stdin)
with open({log:?},'a') as f: f.write(json.dumps(r)+'\n')
if r['operation']=='inspect':
 print(json.dumps(dict(format_version=1,endpoint=r['endpoint'],target='aarch64-unknown-linux-gnu',image_id='sha256:'+'e'*64)))
if r['operation']=='apply':
 assert Path(r['archive']).read_bytes()==b'service archive'
 assert r['image_id']=='sha256:'+'d'*64
 assert r['revision']=='a'*40
 sys.exit(7 if {outcome:?}=='failure' else 0)
"#,
        log = log.to_str().unwrap()
    );
    std::fs::write(&command, script).unwrap();
    std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o755)).unwrap();
    (command, log)
}
async fn perform_update(outcome: &str) -> DeploymentRun {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("nexus");
    std::fs::write(&executable, b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let expected = identity();
    let binary = format!(
        "#!/bin/sh\nif [ \"$1\" = --version ]; then printf '%s\\n' 'nexus {}'; fi\n",
        expected.display()
    )
    .into_bytes();
    let (origin, server) = release_server(&expected, &binary, FixtureTransfer::Deployment);
    let (adapter, log) = adapter(directory.path(), outcome);
    let mut loaded = local_configuration(directory.path());
    loaded.config.client = crate::config::ClientConfig::Remote(crate::config::RemoteClientConfig {
        endpoint: "https://service.example".into(),
        project_id: "fixture".into(),
        project_name: None,
        secret_file: None,
    });
    std::fs::write(&loaded.config.storage.database_path, b"persistent data").unwrap();
    let result = execute(
        &Registry::local(&origin),
        UpdateContext {
            loaded: &loaded,
            explicit_config: None,
            codex_home: None,
            executable: &executable,
            current: BuildIdentity {
                revision: "b".repeat(40),
                ..expected
            },
        },
        &Options {
            service: super::super::ServiceSelection::Updater(adapter),
            ..Options::default()
        },
    )
    .await;
    let calls = std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    server.join().unwrap();
    DeploymentRun {
        directory,
        result,
        calls,
        binary,
    }
}
#[tokio::test]
async fn custom_deployment_success_and_failure_preserve_transaction_order() {
    for outcome in ["success", "failure"] {
        let run = perform_update(outcome).await;
        let operations: Vec<_> = run
            .calls
            .iter()
            .map(|r| r["operation"].as_str().unwrap())
            .collect();
        let installed = std::fs::read(run.directory.path().join("nexus")).unwrap();
        if outcome == "success" {
            assert_eq!(run.result.unwrap().service, "deployment_restarted");
            assert_eq!(operations, ["inspect", "apply"]);
            assert_eq!(installed, run.binary);
        } else {
            assert!(run.result.is_err());
            assert_eq!(operations, ["inspect", "apply", "recover"]);
            assert_eq!(
                run.calls[2]["image_id"],
                format!("sha256:{}", "e".repeat(64))
            );
            assert_eq!(installed, b"#!/bin/sh\nexit 0\n");
        }
        assert_eq!(
            std::fs::read(run.directory.path().join("nexus.db")).unwrap(),
            b"persistent data"
        );
        assert!(!Path::new(run.calls[1]["archive"].as_str().unwrap()).exists());
    }
}
