// @feature runtime
// @spec docs/features/runtime.md
use super::*;

#[tokio::test]
async fn oversized_stdio_frame_recovery_preserves_the_following_frame() {
    for capacity in [8192, bounded_io::MAX_JSON_FRAME_BYTES + 512] {
        let mut input = vec![b'x'; bounded_io::MAX_JSON_FRAME_BYTES + 100];
        input.extend_from_slice(b"\n{\"id\":9,\"method\":\"ping\"}\n");
        let mut reader = tokio::io::BufReader::with_capacity(capacity, std::io::Cursor::new(input));

        let frame = next_stdio_frame(&mut reader).await.unwrap().unwrap();

        assert_eq!(frame, br#"{"id":9,"method":"ping"}"#);
        assert!(next_stdio_frame(&mut reader).await.unwrap().is_none());
    }
}

#[tokio::test]
async fn oversized_stdio_frame_recovery_times_out_without_a_newline() {
    use tokio::io::AsyncWriteExt;
    let (mut input, output) = tokio::io::duplex(2 * 1024 * 1024);
    input
        .write_all(&vec![
            b'x';
            super::super::bounded_io::MAX_JSON_FRAME_BYTES + 1
        ])
        .await
        .unwrap();
    let mut reader = tokio::io::BufReader::new(output);

    let error = tokio::time::timeout(
        std::time::Duration::from_secs(4),
        next_stdio_frame(&mut reader),
    )
    .await
    .expect("oversized frame recovery must have a deadline")
    .unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    drop(input);
}
