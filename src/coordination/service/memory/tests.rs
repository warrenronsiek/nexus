// @feature agent-memory
// @feature coordination
// @spec docs/features/agent-memory.md
// @spec docs/features/coordination.md
use super::*;
use crate::config::Config;
use crate::coordination::api::{MemoryAddCommand, MemoryScopeArg, ServiceRequest};
use crate::coordination::domain::HookContext;
use crate::coordination::service::tests::{executable, service_with};
use chrono::{Duration, Utc};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

fn add_pair(service: &NexusService, suffix: &str) {
    for ordinal in ["first", "second"] {
        let response = service.memory_add(MemoryAddCommand {
            scope: MemoryScopeArg::Global,
            content: format!("{ordinal} durable {suffix} memory"),
            context: HookContext {
                session_id: "memory-test".into(),
                project_root: None,
                agent: "codex".into(),
                turn_id: None,
                model: None,
            },
        });
        assert!(response.to_json()["ok"].as_bool().unwrap());
    }
}

#[test]
fn memory_add_tool_hooks_capture_each_callers_provenance_without_shared_session_state() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().to_string_lossy().to_string();
    let service = service_with(Config::default());
    for (session_id, model, content) in [
        ("session-a", "model-a", "Session A durable note"),
        ("session-b", "model-b", "Session B durable note"),
    ] {
        let response = service
            .handle(
                ServiceRequest::decode(
                    "pre_tool_use",
                    json!({
                        "session_id":session_id,
                        "project_root":root,
                        "agent":"codex",
                        "model":model,
                        "tool_use_id":format!("{session_id}-memory"),
                        "tool_name":"mcp__nexus__nexus_memory_add",
                        "tool_input":{"scope":"project","content":content}
                    }),
                )
                .unwrap(),
            )
            .to_json();
        assert_eq!(response["permitted"], true, "{response}");
        assert_eq!(response["recorded"], true, "{response}");
    }

    let duplicate = service
        .handle(
            ServiceRequest::decode(
                "memory_add",
                json!({
                    "scope":"project",
                    "content":"Session A durable note",
                    "session_id":"mcp-process-fallback",
                    "project_root":root,
                    "agent":"codex"
                }),
            )
            .unwrap(),
        )
        .to_json();
    assert_eq!(duplicate["inserted"], false, "{duplicate}");
    assert_eq!(duplicate["entry"]["provenance"]["session_id"], "session-a");

    let memories = service
        .handle(
            ServiceRequest::decode(
                "memory_search",
                json!({"scope":"project","project_root":root,"regex":"durable note"}),
            )
            .unwrap(),
        )
        .to_json();
    let memories = memories["memories"].as_array().unwrap();
    assert_eq!(memories.len(), 2);
    for (content, session_id, model) in [
        ("Session A durable note", "session-a", "model-a"),
        ("Session B durable note", "session-b", "model-b"),
    ] {
        let entry = memories
            .iter()
            .find(|entry| entry["content"] == content)
            .unwrap();
        assert_eq!(entry["provenance"]["session_id"], session_id);
        assert_eq!(entry["provenance"]["model_id"], model);
    }
}

fn provider_config(primary: &std::path::Path, fallback: &std::path::Path) -> Config {
    let mut config = Config::default();
    config.analyst.provider = ModelProvider::Claude;
    config.analyst.claude.command = primary.to_string_lossy().into_owned();
    config.analyst.codex.command = fallback.to_string_lossy().into_owned();
    config
}

#[test]
fn dual_failure_cools_down_and_expired_cooldown_recovers() {
    let temporary = tempfile::tempdir().unwrap();
    let primary = temporary.path().join("failing-claude");
    let fallback = temporary.path().join("failing-codex");
    let failure = "#!/bin/sh\ncat >/dev/null\nprintf '%s' 'secret child memory' >&2\nexit 9\n";
    executable(&primary, failure);
    executable(&fallback, failure);
    let config = provider_config(&primary, &fallback);
    assert_dual_failure_cooldown(config.clone());
    assert_expired_cooldown_recovers(config, &primary);
}

fn assert_dual_failure_cooldown(config: Config) {
    let service = service_with(config);
    add_pair(&service, "failure-a");
    add_pair(&service, "failure-b");

    let failed = service.consolidate_memories().unwrap();
    assert_eq!(failed.attempted, 4);
    assert_eq!(failed.failed, 4);
    assert_eq!(failed.succeeded, 0);
    assert_eq!(failed.fallback_uses, 2);

    let cooling = service.consolidate_memories().unwrap();
    assert_eq!(cooling.attempted, 0, "providers must honor their cooldown");
    let health = service
        .store
        .lock()
        .unwrap()
        .memory_health(None, Utc::now())
        .unwrap();
    assert!(health.degraded);
    assert_eq!(health.cooling_down_attempts, 4);
}

fn assert_expired_cooldown_recovers(config: Config, primary: &std::path::Path) {
    let recovered = service_with(config);
    add_pair(&recovered, "recovery");
    let job = recovered
        .store
        .lock()
        .unwrap()
        .ready_memory_compactions(&["claude", "codex"], Utc::now(), 8)
        .unwrap()
        .remove(0);
    {
        let mut store = recovered.store.lock().unwrap();
        for (provider, was_fallback) in [("claude", false), ("codex", true)] {
            store
                .record_memory_compaction_attempt(
                    &job,
                    &MemoryCompactionAttempt {
                        provider: provider.into(),
                        model_id: None,
                        outcome: MemoryCompactionOutcome::Failed,
                        diagnostic: Some("provider invocation failed".into()),
                        was_fallback,
                        attempted_at: Utc::now() - Duration::seconds(16),
                    },
                )
                .unwrap();
        }
    }
    executable(
        primary,
        &format!(
            "#!/bin/sh\ncat >/dev/null\nprintf '%s' '{{\"result\":\"{{\\\"summaries\\\":[{{\\\"job_id\\\":\\\"{}\\\",\\\"summary\\\":\\\"Recovered durable memory\\\"}}]}}\"}}'\n",
            job.job_id
        ),
    );

    let recovery = recovered.consolidate_memories().unwrap();
    assert_eq!(recovery.attempted, 1);
    assert_eq!(recovery.succeeded, 1);
    assert_eq!(recovery.failed, 0);
    assert_eq!(recovery.fallback_uses, 0);
}

#[test]
fn fallback_handles_jobs_while_the_primary_provider_is_cooling_down() {
    let temporary = tempfile::tempdir().unwrap();
    let primary_marker = temporary.path().join("primary-started");
    let primary = temporary.path().join("cooling-claude");
    executable(
        &primary,
        &format!(
            "#!/bin/sh\n: > '{}'\ncat >/dev/null\nexit 9\n",
            primary_marker.display()
        ),
    );
    let fallback = temporary.path().join("working-codex");
    let service = service_with(provider_config(&primary, &fallback));
    add_pair(&service, "cooldown");
    let job = service
        .store
        .lock()
        .unwrap()
        .ready_memory_compactions(&["claude", "codex"], Utc::now(), 8)
        .unwrap()
        .remove(0);
    service
        .store
        .lock()
        .unwrap()
        .record_memory_compaction_attempt(
            &job,
            &MemoryCompactionAttempt {
                provider: "claude".into(),
                model_id: None,
                outcome: MemoryCompactionOutcome::Failed,
                diagnostic: Some("provider invocation failed".into()),
                was_fallback: false,
                attempted_at: Utc::now(),
            },
        )
        .unwrap();
    executable(
        &fallback,
        &format!(
            "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{{\"item\":{{\"text\":\"{{\\\"summaries\\\":[{{\\\"job_id\\\":\\\"{}\\\",\\\"summary\\\":\\\"Fallback while primary cools\\\"}}]}}\"}}}}'\n",
            job.job_id
        ),
    );

    let report = service.consolidate_memories().unwrap();

    assert_eq!(report.attempted, 1);
    assert_eq!(report.succeeded, 1);
    assert_eq!(report.failed, 0);
    assert_eq!(report.fallback_uses, 1);
    assert!(!primary_marker.exists(), "cooling primary was invoked");
}

#[test]
fn runtime_context_limits_defensively_clamp_to_hard_bounds() {
    let mut config = Config::default();
    config.memory.context_max_items = crate::memory::DEFAULT_MEMORY_MAX_ITEMS + 1;
    config.memory.context_max_bytes = crate::memory::DEFAULT_MEMORY_MAX_BYTES + 1;
    let service = service_with(config);

    let limits = service.memory_context_limits();

    assert_eq!(limits.max_items, crate::memory::DEFAULT_MEMORY_MAX_ITEMS);
    assert_eq!(limits.max_bytes, crate::memory::DEFAULT_MEMORY_MAX_BYTES);
}

#[test]
fn concurrent_consolidation_requests_share_one_service_gate() {
    let temporary = tempfile::tempdir().unwrap();
    let marker = temporary.path().join("provider-started");
    let primary = temporary.path().join("slow-claude");
    let fallback = temporary.path().join("unused-codex");
    executable(&fallback, "#!/bin/sh\ncat >/dev/null\nexit 9\n");
    executable(
        &primary,
        &format!(
            "#!/bin/sh\ncat >/dev/null\n: > '{}'\nsleep 1\nprintf '%s' '{{\"result\":\"{{\\\"summaries\\\":[{{\\\"job_id\\\":\\\"PLACEHOLDER\\\",\\\"summary\\\":\\\"One guarded summary\\\"}}]}}\"}}'\n",
            marker.display()
        ),
    );
    let service = Arc::new(service_with(provider_config(&primary, &fallback)));
    add_pair(&service, "guarded");
    let job_id = service
        .store
        .lock()
        .unwrap()
        .ready_memory_compactions(&["claude", "codex"], Utc::now(), 8)
        .unwrap()[0]
        .job_id
        .clone();
    let contents = std::fs::read_to_string(&primary)
        .unwrap()
        .replace("PLACEHOLDER", &job_id);
    executable(&primary, &contents);
    let worker_service = service.clone();
    let worker = std::thread::spawn(move || worker_service.consolidate_memories().unwrap());
    let deadline = Instant::now() + StdDuration::from_secs(2);
    while !marker.exists() {
        assert!(Instant::now() < deadline, "provider did not start");
        std::thread::sleep(StdDuration::from_millis(10));
    }

    let overlapping = service.consolidate_memories().unwrap();

    assert!(overlapping.ok);
    assert!(overlapping.in_progress);
    assert_eq!(overlapping.attempted, 0);
    let completed = worker.join().unwrap();
    assert!(!completed.in_progress);
    assert_eq!(completed.succeeded, 1);
}

#[test]
fn successful_consolidation_publishes_snapshot_for_lifecycle_activation() {
    let temporary = tempfile::tempdir().unwrap();
    let primary = temporary.path().join("working-claude");
    let fallback = temporary.path().join("unused-codex");
    executable(&fallback, "#!/bin/sh\ncat >/dev/null\nexit 9\n");
    let service = service_with(provider_config(&primary, &fallback));
    add_pair(&service, "published snapshot");
    let job_id = service
        .store
        .lock()
        .unwrap()
        .ready_memory_compactions(&["claude", "codex"], Utc::now(), 8)
        .unwrap()[0]
        .job_id
        .clone();
    executable(
        &primary,
        &format!(
            "#!/bin/sh\ncat >/dev/null\nprintf '%s' '{{\"result\":\"{{\\\"summaries\\\":[{{\\\"job_id\\\":\\\"{job_id}\\\",\\\"summary\\\":\\\"Published durable memory\\\"}}]}}\"}}'\n"
        ),
    );

    let report = service.consolidate_memories().unwrap();
    let activated = activate_global_memory(&service, "published-session");

    assert_eq!(report.succeeded, 1);
    assert_eq!(activated.item_count, 2);
}

#[test]
fn zero_job_consolidation_repairs_a_snapshot_left_stale_after_summary_commit() {
    let temporary = tempfile::tempdir().unwrap();
    let primary = temporary.path().join("unused-claude");
    let fallback = temporary.path().join("unused-codex");
    executable(&primary, "#!/bin/sh\ncat >/dev/null\nexit 9\n");
    executable(&fallback, "#!/bin/sh\ncat >/dev/null\nexit 9\n");
    let service = service_with(provider_config(&primary, &fallback));
    add_pair(&service, "crash recovery");
    let mut store = service.store.lock().unwrap();
    let job = store
        .ready_memory_compactions(&["claude", "codex"], Utc::now(), 8)
        .unwrap()
        .remove(0);
    assert!(matches!(
        store
            .insert_memory_summary(&job, "Recovered committed memory", "claude", None)
            .unwrap(),
        MemorySummaryInsert::Inserted(_)
    ));
    drop(store);

    let report = service.consolidate_memories().unwrap();
    let activated = activate_global_memory(&service, "recovered-session");

    assert_eq!(report.attempted, 0);
    assert_eq!(activated.item_count, 2);
}

fn activate_global_memory(
    service: &NexusService,
    session_id: &str,
) -> crate::memory::MemoryContextStatus {
    service
        .store
        .lock()
        .unwrap()
        .activate_memory_context(
            &MemoryReadScope::Global,
            "codex",
            session_id,
            MemoryContextLimits {
                max_items: crate::memory::DEFAULT_MEMORY_MAX_ITEMS,
                max_bytes: crate::memory::DEFAULT_MEMORY_MAX_BYTES,
            },
        )
        .unwrap()
        .expect("the repaired snapshot should be available to lifecycle activation")
}

#[test]
fn invalid_primary_output_falls_back_and_records_category_provenance() {
    assert_primary_failure_falls_back(
        "#!/bin/sh\ncat >/dev/null\nprintf '%s' '{\"result\":\"not strict summary JSON\"}'\n",
        5,
        MemoryCompactionOutcome::InvalidOutput,
    );
}

#[test]
fn timed_out_primary_falls_back_and_records_category_provenance() {
    assert_primary_failure_falls_back(
        "#!/usr/bin/env python3\nimport sys\nimport time\nsys.stdin.read()\ntime.sleep(5)\n",
        2,
        MemoryCompactionOutcome::TimedOut,
    );
}

fn assert_primary_failure_falls_back(
    primary_contents: &str,
    timeout_seconds: u64,
    expected_outcome: MemoryCompactionOutcome,
) {
    let temporary = tempfile::tempdir().unwrap();
    let primary = temporary.path().join("primary-claude");
    let fallback = temporary.path().join("fallback-codex");
    executable(&primary, primary_contents);
    executable(
        &fallback,
        "#!/bin/sh\ncat >/dev/null\nprintf '%s\\n' '{\"item\":{\"text\":\"{\\\"summaries\\\":[{\\\"job_id\\\":\\\"PLACEHOLDER\\\",\\\"summary\\\":\\\"Fallback durable memory\\\"}]}\"}}'\n",
    );
    let mut config = provider_config(&primary, &fallback);
    config.analyst.timeout_seconds = timeout_seconds;
    config.analyst.claude.model = Some("primary-memory-model".into());
    config.analyst.codex.model = Some("fallback-memory-model".into());
    let service = service_with(config);
    add_pair(&service, "provider category");
    let job_id = service
        .store
        .lock()
        .unwrap()
        .ready_memory_compactions(&["claude", "codex"], Utc::now(), 8)
        .unwrap()[0]
        .job_id
        .clone();
    let contents = std::fs::read_to_string(&fallback)
        .unwrap()
        .replace("PLACEHOLDER", &job_id);
    executable(&fallback, &contents);

    let report = service.consolidate_memories().unwrap();

    assert_eq!(report.attempted, 2);
    assert_eq!(report.failed, 1);
    assert_eq!(report.succeeded, 1);
    assert_eq!(report.fallback_uses, 1);
    assert_provider_health(&service, expected_outcome);
}

fn assert_provider_health(service: &NexusService, expected_outcome: MemoryCompactionOutcome) {
    let health = service
        .store
        .lock()
        .unwrap()
        .memory_health(None, Utc::now())
        .unwrap();
    let primary_health = health
        .providers
        .iter()
        .find(|provider| provider.provider == "claude")
        .unwrap();
    assert_eq!(primary_health.last_outcome, expected_outcome);
    assert_eq!(
        primary_health.model_id.as_deref(),
        Some("primary-memory-model")
    );
    let fallback_health = health
        .providers
        .iter()
        .find(|provider| provider.provider == "codex")
        .unwrap();
    assert_eq!(
        fallback_health.last_outcome,
        MemoryCompactionOutcome::Succeeded
    );
    assert_eq!(
        fallback_health.model_id.as_deref(),
        Some("fallback-memory-model")
    );
}
