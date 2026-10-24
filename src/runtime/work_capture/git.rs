// @feature work-history
// @feature runtime
// @spec docs/features/work-history.md
// @spec docs/features/runtime.md
use super::*;
use crate::work_history::{
    GitRevision, WorkContext, WorkEvent, WorkReference, WorkRelation, WorkSubject,
};
use std::process::Command;

pub fn git_hook(loaded: &LoadedConfig, root: &Path, phase: &str) -> Result<()> {
    let Ok(attempt) = std::env::var("NEXUS_COMMIT_OPERATION_ID") else {
        return Ok(());
    };
    let context = CaptureContext::from_environment(root);
    let mut git = revision_context(root).context("Git metadata unavailable")?;
    let subject = WorkSubject::Activity { id: attempt };
    let mut events = Vec::new();
    match phase {
        "pre-commit" => {
            if let Some(tree) = git_value(root, &["write-tree"]) {
                git.labels.insert("staged_tree".into(), tree);
            }
            events.push(lifecycle::event(
                subject,
                "git-candidate",
                WorkEvent::Milestone {
                    name: "Commit candidate observed".into(),
                    context: git,
                },
            ));
        }
        "post-commit" => {
            let revision = git.revision.clone().context("created commit unavailable")?;
            events.push(lifecycle::event(
                subject.clone(),
                "git-produced",
                WorkEvent::Linked {
                    relation: WorkRelation::Produced,
                    target: WorkReference::Revision {
                        repository: revision.repository,
                        object_format: revision.object_format,
                        commit: revision.commit,
                    },
                },
            ));
            events.push(lifecycle::event(
                subject,
                "git-created",
                WorkEvent::Milestone {
                    name: "Commit created".into(),
                    context: git,
                },
            ));
        }
        _ => anyhow::bail!("unsupported Git hook phase"),
    }
    for event in &mut events {
        event.method = format!("git_{phase}_callback");
    }
    enqueue(loaded, &context.hook, events)
}

fn revision_context(root: &Path) -> Option<WorkContext> {
    let common = git_value(root, &["rev-parse", "--git-common-dir"])?;
    let common = root.join(common).canonicalize().ok()?;
    let repository = std::env::var("NEXUS_REPOSITORY_ID")
        .unwrap_or_else(|_| stable_id(&[&common.to_string_lossy()]));
    let object_format = git_value(root, &["rev-parse", "--show-object-format"])?;
    let revision = git_value(root, &["rev-parse", "--verify", "HEAD"]).map(|commit| GitRevision {
        repository: repository.clone(),
        object_format,
        commit,
    });
    Some(WorkContext {
        repository: Some(repository),
        worktree: Some(root.to_string_lossy().into_owned()),
        revision,
        ..Default::default()
    })
}

fn git_value(root: &Path, arguments: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    command.current_dir(root).args(arguments);
    let output =
        crate::git_process::bounded_output(command, Duration::from_millis(150), 4096).ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
        .map(|value| value.trim().to_owned())
}
