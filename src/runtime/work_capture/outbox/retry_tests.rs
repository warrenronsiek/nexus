// @feature work-history
// @spec docs/features/work-history.md
use super::*;

#[test]
fn retry_invalid_respects_the_selected_limit() {
    let root = tempfile::tempdir().unwrap();
    let queue = Outbox::writer(root.path()).unwrap();
    for id in ["first", "second"] {
        queue.enqueue(id, b"{}").unwrap();
        let entry = queue
            .pending()
            .unwrap()
            .into_iter()
            .find(|entry| entry.path.file_stem().unwrap() == id)
            .unwrap();
        queue.reject(&entry, "invalid").unwrap();
    }
    assert_eq!(
        queue.retry_invalid(InvalidRetry::Preview).unwrap().eligible,
        2
    );
    assert!(queue.pending().unwrap().is_empty());
    assert!(queue
        .retry_invalid(InvalidRetry::Requeue { limit: 0 })
        .is_err());
    assert!(queue
        .retry_invalid(InvalidRetry::Requeue { limit: 257 })
        .is_err());
    assert_eq!(
        queue
            .retry_invalid(InvalidRetry::Requeue { limit: 1 })
            .unwrap()
            .requeued,
        1
    );
    assert_eq!(queue.pending().unwrap().len(), 1);
    assert_eq!(
        queue
            .retry_invalid(InvalidRetry::Requeue { limit: 1 })
            .unwrap()
            .requeued,
        1
    );
    assert_eq!(queue.pending().unwrap().len(), 2);
}

#[test]
fn retry_invalid_rejects_symlinks_before_moving_any_payload() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let root = tempfile::tempdir().unwrap();
    let outside = root.path().join("original");
    std::fs::write(&outside, b"private fixture").unwrap();
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o600)).unwrap();
    let directory = root.path().join("queue");
    let queue = Outbox::writer(&directory).unwrap();
    let link = directory.join("unsafe.rejected-invalid");
    symlink(&outside, &link).unwrap();
    assert!(queue
        .retry_invalid(InvalidRetry::Requeue { limit: 128 })
        .is_err());
    assert!(std::fs::symlink_metadata(&link)
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(queue.pending().unwrap().is_empty());
    assert_eq!(std::fs::read(outside).unwrap(), b"private fixture");
}

#[test]
fn retry_invalid_preserves_payload_scope_and_other_rejection_classes() {
    let root = tempfile::tempdir().unwrap();
    let current = root.path().join("current");
    let other = root.path().join("other");
    let original = br#"{"original":"capture"}"#;
    {
        let queue = Outbox::writer(&other).unwrap();
        queue.enqueue("foreign", original).unwrap();
        queue
            .reject(&queue.pending().unwrap().remove(0), "invalid")
            .unwrap();
    }
    let queue = Outbox::writer(&current).unwrap();
    for (id, reason) in [
        ("recover", "invalid"),
        ("conflict", "conflict"),
        ("corrupt", "corrupt"),
    ] {
        queue.enqueue(id, original).unwrap();
        let pending = queue
            .pending()
            .unwrap()
            .into_iter()
            .find(|entry| entry.path.file_stem().unwrap() == id)
            .unwrap();
        queue.reject(&pending, reason).unwrap();
    }
    let preview = queue.retry_invalid(InvalidRetry::Preview).unwrap();
    assert_eq!(preview.eligible, 1);
    assert_eq!(preview.requeued, 0);
    assert!(queue.pending().unwrap().is_empty());
    let result = queue
        .retry_invalid(InvalidRetry::Requeue { limit: 128 })
        .unwrap();
    assert_eq!(result.requeued, 1);
    let pending = queue.pending().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].bytes, original);
    assert!(current.join("conflict.rejected-conflict").exists());
    assert!(current.join("corrupt.rejected-corrupt").exists());
    assert!(other.join("foreign.rejected-invalid").exists());
    assert_eq!(
        queue
            .retry_invalid(InvalidRetry::Requeue { limit: 128 })
            .unwrap()
            .requeued,
        0
    );
}
