// @feature installation
// @feature runtime
// @spec docs/features/installation.md
// @spec docs/features/runtime.md
// @boundary update-orchestration: caller-owned configuration and fixed-origin verified assets are retained across managed service transitions.

use super::{
    compose::Docker,
    install::PreparedBinary,
    registry::{Registry, RevisionRelation},
    BuildIdentity, Options, Report,
};
use anyhow::{Context, Result};
use std::path::Path;

pub(super) struct UpdateContext<'a> {
    pub(super) loaded: &'a crate::LoadedConfig,
    pub(super) explicit_config: Option<&'a Path>,
    pub(super) executable: &'a Path,
    pub(super) current: BuildIdentity,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum Decision {
    Current,
    Install,
    AwaitRelease,
    RequireForce,
}
fn decision(relation: RevisionRelation, dirty: bool, force: bool, published: bool) -> Decision {
    if relation == RevisionRelation::Current && !dirty && !force {
        return Decision::Current;
    }
    if !force && (dirty || relation != RevisionRelation::Behind) {
        return Decision::RequireForce;
    }
    if published {
        Decision::Install
    } else {
        Decision::AwaitRelease
    }
}

struct ReleaseCheck {
    report: Report,
    release: Option<super::release::ReleaseManifest>,
    decision: Decision,
}
async fn check_release(
    registry: &Registry,
    context: &UpdateContext<'_>,
    options: &Options,
) -> Result<ReleaseCheck> {
    let revision = registry.main_revision().await?;
    let relation = registry
        .compare(&context.current.revision, &revision)
        .await?;
    let release =
        if relation == RevisionRelation::Current && !context.current.dirty && !options.force {
            None
        } else {
            registry.manifest(&revision).await?
        };
    let decision = decision(
        relation,
        context.current.dirty,
        options.force,
        release.is_some(),
    );
    let report = Report {
        status: match decision {
            Decision::Current => "current",
            Decision::Install => "available",
            Decision::AwaitRelease => "release_pending",
            Decision::RequireForce => "requires_force",
        },
        current: context.current.clone(),
        main_revision: revision,
        available_version: release.as_ref().map(|release| release.version.clone()),
        service: "unchanged",
        notes: Vec::new(),
    };
    Ok(ReleaseCheck {
        report,
        release,
        decision,
    })
}
pub(super) async fn execute(
    registry: &Registry,
    context: UpdateContext<'_>,
    options: &Options,
) -> Result<Report> {
    let check = check_release(registry, &context, options).await?;
    if check.decision != Decision::Install || options.check {
        return Ok(check.report);
    }
    let release = check
        .release
        .context("the main release has not been published")?;
    let (service, warning) = install_release(registry, &context, options, &release).await?;
    let mut report = check.report;
    report.status = "updated";
    report.service = service.status();
    report.notes.extend(warning);
    if let Some(note) = service.note() {
        report.notes.push(note.into())
    }
    report.notes.push("Reconnect existing agent MCP companions to load the new executable; running agents are not restarted.".into());
    Ok(report)
}
async fn install_release(
    registry: &Registry,
    context: &UpdateContext<'_>,
    options: &Options,
    release: &super::release::ReleaseManifest,
) -> Result<(super::service::ServicePlan, Option<String>)> {
    let prepared = stage_binary(registry, context, release).await?;
    let service = super::service::ServicePlan::prepare(context, options, registry, release).await?;
    let warning = transition(prepared, &service, context).await?;
    Ok((service, warning))
}
async fn stage_binary(
    registry: &Registry,
    context: &UpdateContext<'_>,
    release: &super::release::ReleaseManifest,
) -> Result<PreparedBinary> {
    let artifact = release.binary(&context.current.target)?;
    let download = tempfile::NamedTempFile::new()?;
    registry
        .download(&release.revision, artifact, download.path())
        .await?;
    let expected = BuildIdentity {
        version: release.version.clone(),
        revision: release.revision.clone(),
        target: context.current.target.clone(),
        dirty: false,
    };
    PreparedBinary::verify_path(context.executable, download.path(), &expected, artifact)
}
async fn activate_managed(
    prepared: PreparedBinary,
    service: &super::service::ServicePlan,
    context: &UpdateContext<'_>,
) -> Result<super::install::Replacement> {
    service.stop(context).await?;
    match prepared.activate() {
        Ok(replacement) => Ok(replacement),
        Err(error) => {
            service
                .recover(context)
                .await
                .context("previous executable retained but managed service restart failed")?;
            Err(error)
        }
    }
}
async fn apply_assets_and_service(
    service: &super::service::ServicePlan,
    context: &UpdateContext<'_>,
) -> Result<()> {
    refresh_assets(context.executable, context.explicit_config)?;
    service.start(context).await
}
async fn transition(
    prepared: PreparedBinary,
    service: &super::service::ServicePlan,
    context: &UpdateContext<'_>,
) -> Result<Option<String>> {
    let replacement = activate_managed(prepared, service, context).await?;
    if let Err(error) = apply_assets_and_service(service, context).await {
        rollback_update(replacement, service, context).await?;
        return Err(error).context("update failed; previous executable and managed service restored; persisted data was retained");
    }
    Ok(replacement.commit().err().map(|error| format!("{error:#}")))
}
async fn rollback_update(
    replacement: super::install::Replacement,
    service: &super::service::ServicePlan,
    context: &UpdateContext<'_>,
) -> Result<()> {
    replacement
        .rollback()
        .context("update failed and executable rollback failed")?;
    let setup = refresh_assets(context.executable, context.explicit_config);
    let recovery = service.recover(context).await;
    setup.context("previous executable restored but setup asset rollback failed")?;
    recovery.context("previous executable restored but managed service recovery failed")
}

fn refresh_assets(executable: &Path, explicit: Option<&Path>) -> Result<()> {
    let mut command = std::process::Command::new(executable);
    if let Some(path) = explicit {
        command.arg("--config").arg(path);
    }
    command
        .args(["setup", "--refresh"])
        .stdin(std::process::Stdio::null());
    let output =
        crate::git_process::bounded_output(command, std::time::Duration::from_secs(60), 256 * 1024)
            .context("refresh installed Nexus setup assets")?;
    anyhow::ensure!(
        output.status.success(),
        "Nexus setup asset refresh failed; rerun nexus setup to inspect host configuration"
    );
    Ok(())
}

pub(super) fn register(endpoint: &str, compose_file: Option<&Path>) -> Result<()> {
    let docker = Docker::system()?;
    let deployment = docker
        .discover(endpoint, compose_file)?
        .context("no running Compose service exactly matches the selected endpoint")?;
    docker.retain(&deployment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updater::registry::RevisionRelation;
    #[test]
    fn update_decisions_distinguish_publication_delay_and_unrelated_source_history() {
        assert_eq!(
            decision(RevisionRelation::Current, false, false, true),
            Decision::Current
        );
        assert_eq!(
            decision(RevisionRelation::Behind, false, false, true),
            Decision::Install
        );
        assert_eq!(
            decision(RevisionRelation::Behind, false, false, false),
            Decision::AwaitRelease
        );
        assert_eq!(
            decision(RevisionRelation::Ahead, false, false, true),
            Decision::RequireForce
        );
        assert_eq!(
            decision(RevisionRelation::Diverged, false, false, true),
            Decision::RequireForce
        );
        assert_eq!(
            decision(RevisionRelation::Unknown, false, false, true),
            Decision::RequireForce
        );
        assert_eq!(
            decision(RevisionRelation::Current, true, false, true),
            Decision::RequireForce
        );
        assert_eq!(
            decision(RevisionRelation::Diverged, false, true, true),
            Decision::Install
        );
        assert_eq!(
            decision(RevisionRelation::Current, true, true, true),
            Decision::Install
        );
    }
    enum FixtureTransfer {
        Metadata,
        Download,
    }
    fn identity() -> BuildIdentity {
        BuildIdentity {
            version: "0.2.0".into(),
            revision: "a".repeat(40),
            target: "aarch64-apple-darwin".into(),
            dirty: false,
        }
    }
    fn local_configuration(directory: &Path) -> crate::LoadedConfig {
        let mut config = crate::Config::default();
        config.runtime.socket_path = directory.join("inactive.sock");
        config.runtime.lock_path = directory.join("inactive.lock");
        config.storage.database_path = directory.join("nexus.db");
        crate::LoadedConfig {
            config,
            sources: vec![],
            hash: "fixture".into(),
        }
    }
    fn release_server(
        expected: &BuildIdentity,
        binary: &[u8],
        transfer: FixtureTransfer,
    ) -> (String, std::thread::JoinHandle<()>) {
        let mut manifest = crate::updater::release::tests::fixture();
        use sha2::Digest;
        manifest["binaries"][3]["size"] = serde_json::json!(binary.len());
        manifest["binaries"][3]["sha256"] =
            serde_json::json!(format!("{:x}", sha2::Sha256::digest(binary)));
        let main_path = format!(
            "/warrenronsiek/nexus/releases/download/main-{}/manifest.json",
            expected.revision
        );
        let asset_path = format!(
            "/warrenronsiek/nexus/releases/download/main-{}/nexus-{}",
            expected.revision, expected.target
        );
        let compare = format!(
            "/repos/warrenronsiek/nexus/compare/{}...{}?per_page=1&page=2",
            "b".repeat(40),
            expected.revision
        );
        let mut routes = vec![
            (
                "/repos/warrenronsiek/nexus/git/ref/heads/main",
                200,
                serde_json::to_vec(&serde_json::json!({"object":{"sha":expected.revision}}))
                    .unwrap(),
            ),
            (&compare, 200, br#"{"status":"ahead"}"#.to_vec()),
            (&main_path, 200, serde_json::to_vec(&manifest).unwrap()),
        ];
        if matches!(transfer, FixtureTransfer::Download) {
            routes.push((&asset_path, 200, binary.to_vec()));
        }
        crate::updater::registry::tests::serve(routes)
    }
    #[tokio::test]
    async fn an_inactive_local_install_updates_without_a_checkout_or_starting_a_daemon() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("nexus");
        std::fs::write(&executable, b"old executable").unwrap();
        let log = directory.path().join("setup-args");
        let expected = crate::updater::BuildIdentity {
            version: "0.2.0".into(),
            revision: "a".repeat(40),
            target: "aarch64-apple-darwin".into(),
            dirty: false,
        };
        let binary=format!("#!/bin/sh\nif [ \"$1\" = --version ]; then printf '%s\\n' 'nexus {}'; else printf '%s\\n' \"$@\" > '{}'; fi\n",expected.display(),log.display()).into_bytes();
        let (origin, server) = release_server(&expected, &binary, FixtureTransfer::Download);
        let mut config = crate::Config::default();
        config.runtime.socket_path = directory.path().join("inactive.sock");
        config.runtime.lock_path = directory.path().join("inactive.lock");
        config.storage.database_path = directory.path().join("nexus.db");
        let loaded = crate::LoadedConfig {
            config,
            sources: vec![],
            hash: "fixture".into(),
        };
        let explicit = directory.path().join("retained config.toml");
        std::fs::write(&explicit, "schema_version = 1").unwrap();
        let current = crate::updater::BuildIdentity {
            revision: "b".repeat(40),
            ..expected.clone()
        };
        let report = execute(
            &crate::updater::registry::Registry::local(&origin),
            UpdateContext {
                loaded: &loaded,
                explicit_config: Some(&explicit),
                executable: &executable,
                current,
            },
            &crate::updater::Options::default(),
        )
        .await
        .unwrap();
        assert_eq!(report.status, "updated");
        assert_eq!(report.service, "local_inactive");
        assert_eq!(std::fs::read(&executable).unwrap(), binary);
        assert_eq!(
            std::fs::read_to_string(log).unwrap(),
            format!("--config\n{}\nsetup\n--refresh\n", explicit.display())
        );
        assert!(!loaded.config.runtime.socket_path.exists());
        assert!(!loaded.config.storage.database_path.exists());
        server.join().unwrap();
    }
    #[tokio::test]
    async fn check_does_not_download_install_refresh_or_start_services() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("nexus");
        std::fs::write(&executable, b"unchanged").unwrap();
        let expected = identity();
        let (origin, server) =
            release_server(&expected, b"not downloaded", FixtureTransfer::Metadata);
        let loaded = local_configuration(directory.path());
        let context = UpdateContext {
            loaded: &loaded,
            explicit_config: None,
            executable: &executable,
            current: BuildIdentity {
                revision: "b".repeat(40),
                ..expected
            },
        };
        let report = execute(
            &Registry::local(&origin),
            context,
            &Options {
                check: true,
                ..Options::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(report.status, "available");
        assert_eq!(report.service, "unchanged");
        assert_eq!(std::fs::read(executable).unwrap(), b"unchanged");
        assert!(!directory.path().join(".nexus-update.lock").exists());
        assert!(!loaded.config.runtime.socket_path.exists());
        server.join().unwrap();
    }
    #[tokio::test]
    async fn failed_setup_restores_the_previous_executable_and_assets_without_touching_data() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("nexus");
        let restored = directory.path().join("restored");
        let previous =
            format!("#!/bin/sh\nprintf restored > '{}'\n", restored.display()).into_bytes();
        std::fs::write(&executable, &previous).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let expected = identity();
        let binary=format!("#!/bin/sh\nif [ \"$1\" = --version ]; then printf '%s\\n' 'nexus {}'; else exit 7; fi\n",expected.display()).into_bytes();
        let (origin, server) = release_server(&expected, &binary, FixtureTransfer::Download);
        let loaded = local_configuration(directory.path());
        std::fs::write(&loaded.config.storage.database_path, b"durable data").unwrap();
        let context = UpdateContext {
            loaded: &loaded,
            explicit_config: None,
            executable: &executable,
            current: BuildIdentity {
                revision: "b".repeat(40),
                ..expected
            },
        };
        let error = execute(&Registry::local(&origin), context, &Options::default())
            .await
            .err()
            .unwrap();
        assert!(format!("{error:#}").contains("previous executable and managed service restored"));
        assert_eq!(std::fs::read(&executable).unwrap(), previous);
        assert_eq!(std::fs::read(restored).unwrap(), b"restored");
        assert_eq!(
            std::fs::read(&loaded.config.storage.database_path).unwrap(),
            b"durable data"
        );
        assert!(!loaded.config.runtime.socket_path.exists());
        server.join().unwrap();
    }
}
