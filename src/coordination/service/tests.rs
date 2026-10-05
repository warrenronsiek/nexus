// @feature coordination
// @feature usage-analytics
// @feature agent-memory
// @spec docs/features/coordination.md
// @spec docs/features/usage-analytics.md
// @spec docs/features/agent-memory.md
use super::*;
use crate::config::Config;
use crate::coordination::api::MemorySummaryQuery;
use serde_json::json;
use std::os::unix::fs::PermissionsExt;

fn service() -> NexusService {
    service_with(Config::default())
}

pub(super) fn service_with(config: Config) -> NexusService {
    let encoded = toml::to_string(&config).unwrap();
    NexusService::from_store(
        LoadedConfig {
            config,
            sources: vec![],
            hash: blake3::hash(encoded.as_bytes()).to_hex().to_string(),
        },
        Store::open_memory().unwrap(),
    )
}

pub(super) fn executable(path: &std::path::Path, contents: &str) {
    std::fs::write(path, contents).unwrap();
    let mut permissions = std::fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).unwrap();
}

fn edit(
    session_id: &str,
    tool_use_id: &str,
    path: &str,
    line_start: u32,
    line_end: u32,
) -> ServiceRequest {
    ServiceRequest::decode(
        "pre_tool_use",
        json!({
            "session_id": session_id,
            "agent": "test",
            "project_root": "/tmp",
            "tool_use_id": tool_use_id,
            "tool_name": "Edit",
            "tool_input": {
                "file_path": path,
                "line_start": line_start,
                "line_end": line_end
            }
        }),
    )
    .unwrap()
}

#[test]
fn second_session_gets_overlap_advisory() {
    let service = service();
    let one = json!({"session_id":"one","agent":"codex","project_root":"/tmp","tool_use_id":"t1","tool_name":"Edit","tool_input":{"file_path":"a.rs","line_start":10,"line_end":20}});
    let two = json!({"session_id":"two","agent":"claude","project_root":"/tmp","tool_use_id":"t2","tool_name":"Edit","tool_input":{"file_path":"a.rs","line_start":15,"line_end":18}});
    let one = ServiceRequest::decode("pre_tool_use", one).unwrap();
    let two = ServiceRequest::decode("pre_tool_use", two).unwrap();
    assert_eq!(
        service.handle(one).to_json()["advisories"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let response = service.handle(two).to_json();
    assert_eq!(response["permitted"], true, "{response}");
    assert_eq!(
        response["advisories"][0]["kind"], "hunk_overlap",
        "{response}"
    );
}

#[test]
fn lifecycle_hooks_fail_open_without_waiting_for_the_store() {
    let service = std::sync::Arc::new(service());
    let store_guard = service.store.lock().unwrap();
    let request = edit("one", "t1", "a.rs", 1, 5);
    let worker_service = service.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        sender
            .send(worker_service.handle(request).to_json())
            .unwrap();
    });

    let response = receiver.recv_timeout(std::time::Duration::from_millis(100));
    drop(store_guard);
    worker.join().unwrap();

    let response = response.expect("lifecycle hook waited for the busy store");
    assert_eq!(response["permitted"], true);
    assert_eq!(response["recorded"], false);
    assert!(response["diagnostic"].as_str().unwrap().contains("store"));
}

#[test]
fn memory_add_resolves_project_scope_and_records_caller_provenance() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().to_string_lossy();
    let service = service();
    let request = ServiceRequest::decode(
        "memory_add",
        json!({
            "scope":"project",
            "content":"Prefer typed request boundaries",
            "session_id":"session-memory",
            "project_root":root,
            "agent":"codex",
            "model":"gpt-test"
        }),
    )
    .unwrap();

    let response = service.handle(request).to_json();

    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(response["inserted"], true, "{response}");
    assert_eq!(response["entry"]["scope"]["scope"], "project");
    assert_eq!(response["entry"]["provenance"]["agent"], "codex");
    assert_eq!(
        response["entry"]["provenance"]["session_id"],
        "session-memory"
    );
    assert_eq!(response["entry"]["provenance"]["model_id"], "gpt-test");
}

#[test]
fn memory_search_resolves_layered_scope_and_returns_raw_entries() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().to_string_lossy().to_string();
    let service = service();
    for (scope, content) in [
        ("global", "Prefer typed global APIs"),
        ("project", "Project uses TYPED boundaries"),
    ] {
        service.handle(
            ServiceRequest::decode(
                "memory_add",
                json!({
                    "scope":scope,
                    "content":content,
                    "session_id":"session-memory",
                    "project_root":root,
                    "agent":"codex"
                }),
            )
            .unwrap(),
        );
    }

    let response = service
        .handle(
            ServiceRequest::decode(
                "memory_search",
                json!({"scope":"layered","project_root":root,"regex":"typed"}),
            )
            .unwrap(),
        )
        .to_json();

    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(response["memories"].as_array().unwrap().len(), 2);
    assert!(response["memories"]
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| entry["content"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("typed")));
}

#[test]
fn memory_context_uses_configured_bounds() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().to_string_lossy().to_string();
    let mut config = Config::default();
    config.memory.context_max_items = 1;
    config.memory.context_max_bytes = 512;
    let service = service_with(config);
    for content in ["Older project memory", "Newest project memory"] {
        service.handle(
            ServiceRequest::decode(
                "memory_add",
                json!({
                    "scope":"project",
                    "content":content,
                    "session_id":"session-memory",
                    "project_root":root,
                    "agent":"codex"
                }),
            )
            .unwrap(),
        );
    }

    let response = service
        .handle(
            ServiceRequest::decode(
                "memory_context",
                json!({"scope":"project","project_root":root}),
            )
            .unwrap(),
        )
        .to_json();

    assert_eq!(response["ok"], true, "{response}");
    assert!(response["context"]["item_count"].as_u64().unwrap() <= 1);
    assert!(response["context"]["byte_count"].as_u64().unwrap() <= 512);
}

#[test]
fn memory_expand_and_invalidate_operate_only_on_derived_summaries() {
    let service = service();
    for content in ["First durable note", "Second durable note"] {
        service.handle(
            ServiceRequest::decode(
                "memory_add",
                json!({
                    "scope":"global",
                    "content":content,
                    "session_id":"session-memory",
                    "agent":"codex"
                }),
            )
            .unwrap(),
        );
    }
    let summary_id = {
        let mut store = service.store.lock().unwrap();
        let job = store
            .ready_memory_compactions(&["codex"], chrono::Utc::now(), 8)
            .unwrap()
            .remove(0);
        match store
            .insert_memory_summary(&job, "Combined durable note", "codex", None)
            .unwrap()
        {
            crate::memory::MemorySummaryInsert::Inserted(summary) => summary.id,
            other => panic!("expected inserted summary, got {other:?}"),
        }
    };

    let expanded = service
        .handle(ServiceRequest::MemoryExpand(MemorySummaryQuery {
            summary_id: summary_id.clone(),
        }))
        .to_json();
    assert_eq!(expanded["ok"], true, "{expanded}");
    assert_eq!(expanded["nodes"].as_array().unwrap().len(), 2);

    let invalidated = service
        .handle(ServiceRequest::MemoryInvalidate(MemorySummaryQuery {
            summary_id,
        }))
        .to_json();
    assert_eq!(invalidated["ok"], true, "{invalidated}");
    assert_eq!(invalidated["invalidated_summaries"], 1);
    let raw = service
        .handle(
            ServiceRequest::decode("memory_search", json!({"scope":"global","regex":"durable"}))
                .unwrap(),
        )
        .to_json();
    assert_eq!(raw["memories"].as_array().unwrap().len(), 2);
}

#[test]
fn first_user_prompt_activates_memory_once_per_agent_session() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().to_string_lossy().to_string();
    let service = service();
    service.handle(
        ServiceRequest::decode(
            "memory_add",
            json!({
                "scope":"global",
                "content":"Use deterministic validation",
                "session_id":"author-session",
                "agent":"codex"
            }),
        )
        .unwrap(),
    );
    let prompt = || {
        ServiceRequest::decode(
            "user_prompt",
            json!({
                "session_id":"reader-session",
                "project_root":root,
                "agent":"codex",
                "prompt":"Start the task"
            }),
        )
        .unwrap()
    };

    let first = service.handle(prompt()).to_json();
    let second = service.handle(prompt()).to_json();

    assert_eq!(first["memory_context"]["item_count"], 1, "{first}");
    assert!(second.get("memory_context").is_none(), "{second}");
}

#[test]
fn memory_status_reports_project_and_global_health() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().to_string_lossy().to_string();
    let service = service();
    for scope in ["global", "project"] {
        service.handle(
            ServiceRequest::decode(
                "memory_add",
                json!({
                    "scope":scope,
                    "content":format!("{scope} durable note"),
                    "session_id":"session-memory",
                    "project_root":root,
                    "agent":"codex"
                }),
            )
            .unwrap(),
        );
    }

    let response = service
        .handle(ServiceRequest::decode("memory_status", json!({"project_root":root})).unwrap())
        .to_json();

    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(response["health"]["scopes"].as_array().unwrap().len(), 2);
    assert_eq!(response["health"]["pending_summaries"], 0);
    assert_eq!(response["health"]["degraded"], false);
}

#[test]
fn memory_consolidation_uses_analyst_provider_while_conflict_analysis_is_disabled() {
    let temporary = tempfile::tempdir().unwrap();
    let provider = temporary.path().join("fake-claude");
    executable(
            &provider,
            "#!/bin/sh\ncat >/dev/null\nprintf '%s' '{\"result\":\"{\\\"summaries\\\":[{\\\"job_id\\\":\\\"PLACEHOLDER\\\",\\\"summary\\\":\\\"Combined durable memory\\\"}]}\"}'\n",
        );
    let mut config = Config::default();
    assert!(!config.analyst.enabled);
    config.analyst.provider = crate::config::ModelProvider::Claude;
    config.analyst.claude.command = provider.to_string_lossy().into_owned();
    let service = service_with(config);
    for content in ["First durable memory", "Second durable memory"] {
        service.handle(
            ServiceRequest::decode(
                "memory_add",
                json!({
                    "scope":"global",
                    "content":content,
                    "session_id":"session-memory",
                    "agent":"codex"
                }),
            )
            .unwrap(),
        );
    }
    let job_id = service
        .store
        .lock()
        .unwrap()
        .ready_memory_compactions(&["claude"], chrono::Utc::now(), 8)
        .unwrap()[0]
        .job_id
        .clone();
    let contents = std::fs::read_to_string(&provider)
        .unwrap()
        .replace("PLACEHOLDER", &job_id);
    executable(&provider, &contents);

    let response = service.handle(ServiceRequest::MemoryConsolidate).to_json();

    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(response["attempted"], 1);
    assert_eq!(response["succeeded"], 1);
    assert_eq!(response["failed"], 0);
    assert_eq!(response["fallback_uses"], 0);
}

#[test]
fn memory_consolidation_falls_back_after_primary_failure() {
    let temporary = tempfile::tempdir().unwrap();
    let primary = temporary.path().join("failing-claude");
    executable(
            &primary,
            "#!/bin/sh\ncat >/dev/null\nprintf '%s' 'sensitive memory echoed by provider' >&2\nexit 7\n",
        );
    let fallback = temporary.path().join("working-codex");
    executable(
            &fallback,
            "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{\"item\":{\"text\":\"{\\\"summaries\\\":[{\\\"job_id\\\":\\\"PLACEHOLDER\\\",\\\"summary\\\":\\\"Fallback durable memory\\\"}]}\"}}'\n",
        );
    let mut config = Config::default();
    config.analyst.provider = crate::config::ModelProvider::Claude;
    config.analyst.claude.command = primary.to_string_lossy().into_owned();
    config.analyst.codex.command = fallback.to_string_lossy().into_owned();
    let service = service_with(config);
    for content in ["First fallback memory", "Second fallback memory"] {
        service.handle(
            ServiceRequest::decode(
                "memory_add",
                json!({
                    "scope":"global",
                    "content":content,
                    "session_id":"session-memory",
                    "agent":"codex"
                }),
            )
            .unwrap(),
        );
    }
    let job_id = service
        .store
        .lock()
        .unwrap()
        .ready_memory_compactions(&["claude", "codex"], chrono::Utc::now(), 8)
        .unwrap()[0]
        .job_id
        .clone();
    let contents = std::fs::read_to_string(&fallback)
        .unwrap()
        .replace("PLACEHOLDER", &job_id);
    executable(&fallback, &contents);

    let response = service.handle(ServiceRequest::MemoryConsolidate).to_json();

    assert_eq!(response["ok"], true, "{response}");
    assert_eq!(response["attempted"], 2);
    assert_eq!(response["succeeded"], 1);
    assert_eq!(response["failed"], 1);
    assert_eq!(response["fallback_uses"], 1);
    assert!(!response.to_string().contains("sensitive memory"));
}

#[test]
fn weaker_observations_do_not_downgrade_or_reopen_a_conflict() {
    let service = service();
    for (tool_use_id, line_start, line_end) in [("t1", 1, 5), ("t2", 10, 20), ("t3", 15, 25)] {
        service.handle(edit("one", tool_use_id, "a.rs", line_start, line_end));
    }

    let critical = service.handle(edit("two", "t4", "a.rs", 16, 18)).to_json();
    assert_eq!(critical["advisories"].as_array().unwrap().len(), 1);
    assert_eq!(critical["advisories"][0]["kind"], "hunk_overlap");
    let weaker = service
        .handle(edit("two", "t5", "a.rs", 100, 110))
        .to_json();
    assert_eq!(weaker["advisories"].as_array().unwrap().len(), 1);
    assert_eq!(weaker["advisories"][0]["severity"], "info");

    let current = service
        .handle(ServiceRequest::Conflicts(ConflictQuery::default()))
        .to_json();
    assert_eq!(current["conflicts"].as_array().unwrap().len(), 1);
    assert_eq!(current["conflicts"][0]["severity"], "critical");
    assert_eq!(current["conflicts"][0]["kind"], "hunk_overlap");
    let conflict_id = current["conflicts"][0]["id"].as_str().unwrap().to_owned();
    service.handle(ServiceRequest::Resolve(ResolveCommand {
        conflict_id,
        resolution: "coordinated".into(),
    }));
    service.handle(edit("two", "t6", "a.rs", 120, 130));
    let open = service
        .handle(ServiceRequest::Conflicts(ConflictQuery::default()))
        .to_json();
    assert!(open["conflicts"].as_array().unwrap().is_empty());
}

#[test]
fn stronger_observation_reopens_a_resolved_conflict() {
    let service = service();
    service.handle(edit("one", "t7", "b.rs", 1, 5));
    service.handle(edit("two", "t8", "b.rs", 100, 110));
    let all = service
        .handle(ServiceRequest::Conflicts(ConflictQuery {
            project_id: None,
            scope: crate::coordination::domain::ConflictScope::All,
        }))
        .to_json();
    let minor_id = all["conflicts"][0]["id"].as_str().unwrap().to_owned();
    service.handle(ServiceRequest::Resolve(ResolveCommand {
        conflict_id: minor_id,
        resolution: "separate hunks".into(),
    }));
    service.handle(edit("two", "t9", "b.rs", 2, 4));
    let reopened = service
        .handle(ServiceRequest::Conflicts(ConflictQuery::default()))
        .to_json();
    assert_eq!(reopened["conflicts"].as_array().unwrap().len(), 1);
    assert_eq!(reopened["conflicts"][0]["severity"], "critical");
}

#[test]
fn dashboard_is_scoped_and_bounded_while_projects_remain_global() {
    let temporary = tempfile::tempdir().unwrap();
    let first_root = temporary.path().join("first");
    let second_root = temporary.path().join("second");
    std::fs::create_dir_all(&first_root).unwrap();
    std::fs::create_dir_all(&second_root).unwrap();
    let mut config = Config::default();
    config.ui.recent_event_limit = 2;
    config.ui.recent_record_limit = 1;
    let service = service_with(config);

    let prompt = |session_id: &str, root: &std::path::Path| {
        ServiceRequest::decode(
            "user_prompt",
            json!({
                "session_id":session_id,
                "agent":"test",
                "project_root":root,
                "prompt":"work on the dashboard"
            }),
        )
        .unwrap()
    };
    let first_project = service.handle(prompt("one", &first_root)).to_json()["project_id"]
        .as_str()
        .unwrap()
        .to_owned();
    service.handle(prompt("two", &first_root));
    service.handle(prompt("three", &second_root));

    let projects = service.handle(ServiceRequest::Projects).to_json();
    assert_eq!(projects["projects"].as_array().unwrap().len(), 2);
    assert_eq!(projects["projects"][0]["agents"][0], "test");

    let dashboard = service
        .handle(ServiceRequest::Dashboard(DashboardQuery {
            project_id: Some(first_project.clone()),
        }))
        .to_json();
    assert_eq!(dashboard["project_id"], first_project);
    assert_eq!(dashboard["counts"]["active_sessions"], 2);
    assert_eq!(dashboard["events"]["items"].as_array().unwrap().len(), 2);
    assert_eq!(dashboard["events"]["truncated"], true);
    assert_eq!(dashboard["sessions"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(dashboard["sessions"]["truncated"], true);
    assert_eq!(dashboard["claims"]["truncated"], false);
    assert_eq!(dashboard["conflicts"]["truncated"], false);
    assert_eq!(dashboard["refresh_interval_ms"], 2_000);
}
