// @feature evals
// @spec docs/features/evals.md
use nexus::persistence::Store;
use sha2::{Digest, Sha256};

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
fn corrupt_pending_content_can_be_aborted_and_uploaded_again() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("history.db")).unwrap();
    let hash = digest(b"abc");
    let original = store.begin_eval_blob("p", &hash, 3).unwrap();
    store.put_eval_blob_chunk("p", &hash, 0, b"bad").unwrap();
    assert!(store.seal_eval_blob("p", &hash).is_err());
    assert!(store.abort_eval_blob("p", &hash).unwrap());
    let replacement = store.begin_eval_blob("p", &hash, 3).unwrap();
    assert_ne!(original.upload_id, replacement.upload_id);
    store.put_eval_blob_chunk("p", &hash, 0, b"abc").unwrap();
    assert!(store.seal_eval_blob("p", &hash).unwrap().verified);
    assert!(store.abort_eval_blob("p", &hash).is_err());
    assert_eq!(
        store.get_eval_blob_chunk("p", &hash, 0).unwrap().unwrap(),
        b"abc"
    );
}

#[test]
fn verification_cannot_publish_a_replacement_upload_generation() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = Store::open(&temp.path().join("history.db")).unwrap();
    let hash = digest(b"abc");
    let original = store.begin_eval_blob("p", &hash, 3).unwrap();
    store.put_eval_blob_chunk("p", &hash, 0, b"abc").unwrap();
    store.abort_eval_blob("p", &hash).unwrap();
    let replacement = store.begin_eval_blob("p", &hash, 3).unwrap();
    store.put_eval_blob_chunk("p", &hash, 0, b"bad").unwrap();
    assert!(store
        .finish_eval_blob_verification("p", &hash, &original.upload_id, &hash)
        .is_err());
    assert!(!store.eval_blob("p", &hash).unwrap().unwrap().verified);
    assert_ne!(original.upload_id, replacement.upload_id);
}
