// @feature agent-memory
// @feature runtime
// @feature persistence
// @spec docs/features/agent-memory.md
// @spec docs/features/runtime.md
// @spec docs/features/persistence.md
// @boundary mock-llm-process
use super::{memory_cli, support::write_executable};
use diesel::dsl::count_star;
use diesel::prelude::*;
use diesel::sqlite::SqliteConnection;
use std::path::Path;
use std::time::{Duration, Instant};

mod schema {
    diesel::table! {
        memory_spaces (id) {
            id -> Text,
            scope -> Text,
            next_ordinal -> BigInt,
        }
    }

    diesel::table! {
        memory_entries (id) {
            id -> Text,
            space_id -> Text,
            ordinal -> BigInt,
            content -> Text,
            agent -> Text,
            session_id -> Text,
        }
    }

    diesel::table! {
        memory_summaries (id) {
            id -> Text,
            start_ordinal -> BigInt,
            end_ordinal -> BigInt,
            content -> Text,
            provider -> Text,
        }
    }

    diesel::table! {
        memory_compaction_attempts (id) {
            id -> Text,
            provider -> Text,
            outcome -> Text,
            was_fallback -> Bool,
        }
    }

    diesel::table! {
        memory_compaction_queue (space_id, level, start_ordinal, end_ordinal) {
            space_id -> Text,
            level -> Integer,
            start_ordinal -> BigInt,
            end_ordinal -> BigInt,
        }
    }
}

#[test]
fn cli_memory_round_trip_persists_mock_consolidation_to_sqlite() {
    let provider_temp = tempfile::tempdir().unwrap();
    let provider = provider_temp.path().join("mock-claude");
    write_executable(
        &provider,
        r#"#!/usr/bin/env python3
import json
import re
import sys
prompt = sys.stdin.read()
job_id = re.findall(r'"job_id":"([^"]+)"', prompt)[-1]
summary = {"summaries": [{"job_id": job_id, "summary": "Mock consolidated memory"}]}
print(json.dumps({"result": json.dumps(summary)}))
"#,
    );
    let fallback = provider_temp.path().join("unused-codex");
    write_executable(&fallback, "#!/bin/sh\ncat >/dev/null\nexit 9\n");
    let mut harness =
        super::support::Harness::start_with_memory_analysts(&provider, &fallback, 3_600, 10);
    let config = harness._temp.path().join("config.toml");

    for note in ["Prefer typed boundaries", "Keep lifecycle hooks fail open"] {
        let result = memory_cli(&config, &harness.root, &["add", "--scope", "global", note]);
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["inserted"], true, "{result}");
    }
    let requested = memory_cli(&config, &harness.root, &["consolidate"]);
    assert_eq!(requested["ok"], true, "{requested}");
    assert_eq!(requested["in_progress"], true, "{requested}");

    wait_for_consolidation(&config, &harness.root, &mut harness.daemon);
    harness.daemon.kill().unwrap();
    harness.daemon.wait().unwrap();
    assert_raw_memory_state(&harness.database);
    assert_consolidation_state(&harness.database);
}

fn wait_for_consolidation(config: &Path, root: &Path, daemon: &mut std::process::Child) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(daemon.try_wait().unwrap().is_none(), "daemon exited early");
        let status = memory_cli(config, root, &["status"]);
        let health = &status["health"];
        let provider_succeeded = health["providers"].as_array().is_some_and(|providers| {
            providers.iter().any(|provider| {
                provider["provider"] == "claude" && provider["last_outcome"] == "succeeded"
            })
        });
        if health["pending_summaries"] == 0 && provider_succeeded {
            return;
        }
        assert!(Instant::now() < deadline, "mock summary was not persisted");
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn assert_raw_memory_state(database: &Path) {
    let mut connection = connection(database);
    let spaces = schema::memory_spaces::table
        .select((
            schema::memory_spaces::scope,
            schema::memory_spaces::next_ordinal,
        ))
        .load::<(String, i64)>(&mut connection)
        .unwrap();
    assert_eq!(spaces, vec![("global".into(), 2)]);

    let entries = schema::memory_entries::table
        .order(schema::memory_entries::ordinal.asc())
        .select((
            schema::memory_entries::ordinal,
            schema::memory_entries::content,
            schema::memory_entries::agent,
            schema::memory_entries::session_id,
        ))
        .load::<(i64, String, String, String)>(&mut connection)
        .unwrap();
    assert_eq!(
        entries,
        vec![
            (
                0,
                "Prefer typed boundaries".into(),
                "user".into(),
                "cli".into()
            ),
            (
                1,
                "Keep lifecycle hooks fail open".into(),
                "user".into(),
                "cli".into()
            ),
        ]
    );
}

fn assert_consolidation_state(database: &Path) {
    let mut connection = connection(database);
    let summaries = schema::memory_summaries::table
        .select((
            schema::memory_summaries::start_ordinal,
            schema::memory_summaries::end_ordinal,
            schema::memory_summaries::content,
            schema::memory_summaries::provider,
        ))
        .load::<(i64, i64, String, String)>(&mut connection)
        .unwrap();
    assert_eq!(
        summaries,
        vec![(0, 2, "Mock consolidated memory".into(), "claude".into())]
    );

    let attempts = schema::memory_compaction_attempts::table
        .select((
            schema::memory_compaction_attempts::provider,
            schema::memory_compaction_attempts::outcome,
            schema::memory_compaction_attempts::was_fallback,
        ))
        .load::<(String, String, bool)>(&mut connection)
        .unwrap();
    assert_eq!(attempts, vec![("claude".into(), "succeeded".into(), false)]);

    let queued = schema::memory_compaction_queue::table
        .select(count_star())
        .first::<i64>(&mut connection)
        .unwrap();
    assert_eq!(queued, 0);
}

fn connection(database: &Path) -> SqliteConnection {
    SqliteConnection::establish(database.to_str().unwrap()).unwrap()
}
