// @feature runtime
// @feature configuration
// @feature sos
// @spec docs/features/runtime.md
// @spec docs/features/configuration.md
// @spec docs/features/sos.md
use super::*;
use crate::config::{ClientConfig, Config};
use crate::coordination::api::{PapercutCommand, PapercutDumpScope, SosCommand, SosDumpScope};
use crate::coordination::domain::HookContext;
use std::os::unix::fs::PermissionsExt;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TEST_SECRET: &str = "test-secret-that-is-at-least-32-bytes";

fn write_private_secret(path: &Path) {
    std::fs::write(path, TEST_SECRET).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

fn remote_client(
    address: std::net::SocketAddr,
    request_timeout: Duration,
) -> (tempfile::TempDir, NexusClient) {
    let temporary = tempfile::tempdir().unwrap();
    let secret_file = temporary.path().join("secret");
    write_private_secret(&secret_file);
    let config = Config {
        client: ClientConfig::Remote(RemoteClientConfig {
            endpoint: format!("http://{address}"),
            project_id: "response-bound-project".into(),
            secret_file: Some(secret_file),
        }),
        ..Config::default()
    };
    let mut client = NexusClient::new(
        &LoadedConfig {
            config,
            sources: Vec::new(),
            hash: "response-bound-test".into(),
        },
        None,
    );
    client.request_timeout = request_timeout;
    (temporary, client)
}

async fn read_request_headers(stream: &mut tokio::net::TcpStream) {
    let mut request = Vec::new();
    let mut byte = [0_u8; 1];
    while !request.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).await.unwrap();
        request.push(byte[0]);
    }
    let headers = String::from_utf8(request).unwrap();
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or_default();
    let mut body = vec![0_u8; content_length];
    stream.read_exact(&mut body).await.unwrap();
}

fn issue_context() -> HookContext {
    HookContext {
        session_id: "response-policy-session".into(),
        project_root: Some("/tmp/project".into()),
        agent: "codex".into(),
        turn_id: None,
        model: None,
    }
}

#[tokio::test]
async fn silent_remote_peer_cannot_hold_an_ordinary_request_forever() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let silent_peer = tokio::spawn(async move {
        let (_connection, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let temporary = tempfile::tempdir().unwrap();
    let secret_file = temporary.path().join("secret");
    write_private_secret(&secret_file);
    let config = Config {
        client: ClientConfig::Remote(RemoteClientConfig {
            endpoint: format!("http://{address}"),
            project_id: "timeout-project".into(),
            secret_file: Some(secret_file),
        }),
        ..Config::default()
    };
    let mut client = NexusClient::new(
        &LoadedConfig {
            config,
            sources: Vec::new(),
            hash: "timeout-test".into(),
        },
        None,
    );
    client.request_timeout = Duration::from_millis(50);
    let started = Instant::now();

    let result = client.request(&ServiceRequest::Status).await;

    assert!(result.is_err());
    assert!(started.elapsed() < Duration::from_secs(1));
    silent_peer.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn optional_client_evidence_cannot_delay_the_primary_request() {
    let started = Instant::now();
    let gate = BlockingCaptureGate::default();

    let captured = gate
        .capture(Duration::from_millis(25), || {
            std::thread::sleep(Duration::from_millis(250));
            "late evidence"
        })
        .await;

    assert_eq!(captured, None);
    assert!(started.elapsed() < Duration::from_millis(200));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timed_out_optional_capture_retains_single_flight_capacity_until_worker_exit() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;

    let gate = BlockingCaptureGate::default();
    let (started_sender, started_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let first = gate
        .capture(Duration::from_millis(25), move || {
            started_sender.send(()).unwrap();
            release_receiver.recv().unwrap();
        })
        .await;
    assert_eq!(first, None);
    started_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("the first capture worker must start");

    let second_started = Arc::new(AtomicBool::new(false));
    let second_started_in_worker = second_started.clone();
    assert_eq!(
        gate.capture(Duration::from_millis(25), move || {
            second_started_in_worker.store(true, Ordering::SeqCst)
        })
        .await,
        None
    );
    assert!(!second_started.load(Ordering::SeqCst));

    release_sender.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        while gate
            .capture(Duration::from_millis(25), || ())
            .await
            .is_none()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("capture capacity must return when the worker exits");
}

#[tokio::test]
async fn chunked_remote_response_is_rejected_after_one_mebibyte() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_request_headers(&mut stream).await;
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n",
            )
            .await
            .unwrap();
        let body = format!(
            "{{\"padding\":\"{}\"}}",
            "x".repeat(MAX_REMOTE_RESPONSE_BYTES + 1)
        );
        for chunk in body.as_bytes().chunks(16 * 1024) {
            stream
                .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                .await
                .unwrap();
            stream.write_all(chunk).await.unwrap();
            stream.write_all(b"\r\n").await.unwrap();
        }
        let _ = stream.write_all(b"0\r\n\r\n").await;
    });
    let (_temporary, client) = remote_client(address, Duration::from_secs(2));

    let error = client.request(&ServiceRequest::Status).await.unwrap_err();

    assert!(
        error.to_string().contains("response exceeds"),
        "unexpected error: {error:#}"
    );
    peer.await.unwrap();
}

#[tokio::test]
async fn oversized_content_length_is_rejected_before_body_arrives() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_request_headers(&mut stream).await;
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    MAX_REMOTE_RESPONSE_BYTES + 1
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
    });
    let (_temporary, client) = remote_client(address, Duration::from_secs(1));
    let started = Instant::now();

    let error = client.request(&ServiceRequest::Status).await.unwrap_err();

    assert!(
        error.to_string().contains("response exceeds"),
        "unexpected error: {error:#}"
    );
    assert!(started.elapsed() < Duration::from_millis(250));
    peer.abort();
}

#[test]
fn only_explicit_registry_dumps_select_the_complete_response_policy() {
    let dump = ServiceRequest::Papercuts(PapercutCommand::Dump {
        context: issue_context(),
        scope: PapercutDumpScope::Project,
    });
    let list = ServiceRequest::Papercuts(PapercutCommand::List {
        context: issue_context(),
    });
    let sos_dump = ServiceRequest::Sos(SosCommand::Dump {
        context: issue_context(),
        scope: SosDumpScope::Project,
    });
    let sos_list = ServiceRequest::Sos(SosCommand::List {
        context: issue_context(),
    });

    assert_eq!(
        RemoteResponsePolicy::for_request(&dump),
        RemoteResponsePolicy::CompleteRegistry
    );
    assert_eq!(
        RemoteResponsePolicy::for_request(&sos_dump),
        RemoteResponsePolicy::CompleteRegistry
    );
    assert_eq!(
        RemoteResponsePolicy::for_request(&list),
        RemoteResponsePolicy::Bounded
    );
    assert_eq!(
        RemoteResponsePolicy::for_request(&sos_list),
        RemoteResponsePolicy::Bounded
    );
    assert_eq!(
        RemoteResponsePolicy::for_request(&ServiceRequest::Status),
        RemoteResponsePolicy::Bounded
    );
}
