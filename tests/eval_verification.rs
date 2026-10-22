// @feature evals
// @spec docs/features/evals.md
// @boundary typed-eval-verification-fixtures
#[path = "support/eval_record.rs"]
mod eval_record;
use eval_record::record;
use nexus::evals::{EvalProvenance, EvalRecordInput};
use nexus::persistence::Store;
use serde_json::json;
use sha2::{Digest, Sha256};

fn actor() -> EvalProvenance {
    EvalProvenance {
        agent: "test".into(),
        session_id: "session".into(),
        model_id: None,
        config_hash: "config".into(),
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn artifact(byte_size: u64) -> EvalRecordInput {
    record(
        "artifact",
        json!({"kind":"artifact","media_type":"text/plain","byte_size":byte_size,"digest":digest(b"abc"),"backings":[{"kind":"git_file","repository":"repo","commit":"a".repeat(40),"object_format":"sha1","path":"fixture.txt","entry_mode":"100644"}]}),
    )
}
fn verification(hash: &str) -> EvalRecordInput {
    record(
        "verification",
        json!({"kind":"verification","artifact_id":"artifact","backing":{"kind":"stored_blob","digest":hash},"state":"exact","checked_at":"2026-10-22T12:01:00Z","producer":"client","reason":"Verified exact backing"}),
    )
}
fn upload(store: &mut Store, bytes: &[u8]) {
    let hash = digest(bytes);
    store
        .begin_eval_blob("p", &hash, bytes.len() as u64)
        .unwrap();
    store.put_eval_blob_chunk("p", &hash, 0, bytes).unwrap();
    store.seal_eval_blob("p", &hash).unwrap();
}

#[test]
fn stored_artifact_verification_rejects_missing_unverified_or_mismatched_bytes() {
    for scenario in ["missing", "pending", "digest", "size"] {
        let temp = tempfile::tempdir().unwrap();
        let mut store = Store::open(&temp.path().join("history.db")).unwrap();
        let expected_size = if scenario == "size" { 4 } else { 3 };
        store
            .record_eval_bundle("p", &[artifact(expected_size)], &actor())
            .unwrap();
        let hash = match scenario {
            "pending" => {
                store.begin_eval_blob("p", &digest(b"abc"), 3).unwrap();
                digest(b"abc")
            }
            "digest" => {
                upload(&mut store, b"xyz");
                digest(b"xyz")
            }
            "size" => {
                upload(&mut store, b"abc");
                digest(b"abc")
            }
            _ => digest(b"abc"),
        };
        assert!(
            store
                .record_eval_bundle("p", &[verification(&hash)], &actor())
                .is_err(),
            "accepted invalid {scenario} backing"
        );
    }
}

#[test]
fn matching_verified_backing_is_append_only_and_visible_from_artifact_history() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("history.db")).unwrap();
    store
        .record_eval_bundle("p", &[artifact(3)], &actor())
        .unwrap();
    upload(&mut store, b"abc");
    store
        .record_eval_bundle("p", &[verification(&digest(b"abc"))], &actor())
        .unwrap();
    let why = store.why_eval_record("p", "artifact", 64).unwrap();
    assert!(why.records.iter().any(|r| r.input.id == "verification"));
    assert_eq!(
        why.records
            .iter()
            .find(|r| r.input.id == "artifact")
            .unwrap()
            .input,
        artifact(3)
    );
}
