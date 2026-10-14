// @feature runtime
// @spec docs/features/runtime.md
// @boundary unauthenticated-http-connection
use anyhow::Result;
use axum::body::Body;
use axum::Router;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tower::ServiceExt;

const MAX_HTTP_CONNECTIONS: usize = 128;
const MAX_HTTP_HEADER_BYTES: usize = 32 * 1024;
#[cfg(not(test))]
const HTTP_HEADER_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const HTTP_HEADER_TIMEOUT: Duration = Duration::from_millis(75);

#[derive(Clone)]
struct ConnectionAdmission {
    slots: Arc<Semaphore>,
    active: Arc<AtomicUsize>,
}

impl ConnectionAdmission {
    fn new(capacity: usize) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(capacity)),
            active: Arc::new(AtomicUsize::new(0)),
        }
    }

    async fn reserve(&self) -> ConnectionReservation {
        let permit = self
            .slots
            .clone()
            .acquire_owned()
            .await
            .expect("HTTP connection admission is never closed");
        ConnectionReservation {
            permit,
            active: self.active.clone(),
        }
    }

    #[cfg(test)]
    fn active(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }
}

struct ConnectionReservation {
    permit: OwnedSemaphorePermit,
    active: Arc<AtomicUsize>,
}

impl ConnectionReservation {
    fn activate(self) -> ActiveConnection {
        self.active.fetch_add(1, Ordering::AcqRel);
        ActiveConnection {
            _permit: self.permit,
            active: self.active,
        }
    }
}

struct ActiveConnection {
    _permit: OwnedSemaphorePermit,
    active: Arc<AtomicUsize>,
}

impl Drop for ActiveConnection {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) async fn serve(listener: TcpListener, router: Router) -> Result<()> {
    serve_with_admission(
        listener,
        router,
        ConnectionAdmission::new(MAX_HTTP_CONNECTIONS),
        HTTP_HEADER_TIMEOUT,
    )
    .await
}

async fn serve_with_admission(
    listener: TcpListener,
    router: Router,
    admission: ConnectionAdmission,
    header_timeout: Duration,
) -> Result<()> {
    loop {
        let reservation = admission.reserve().await;
        let stream = accept(&listener).await;
        let connection = reservation.activate();
        let router = router.clone();
        tokio::spawn(async move {
            let _connection = connection;
            serve_connection(stream, router, header_timeout).await;
        });
    }
}

async fn accept(listener: &TcpListener) -> TcpStream {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => return stream,
            Err(_) => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    }
}

async fn serve_connection(stream: TcpStream, router: Router, header_timeout: Duration) {
    let service = service_fn(move |request| {
        let router = router.clone();
        async move { router.oneshot(request.map(Body::new)).await }
    });
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(header_timeout)
        .max_header_size(MAX_HTTP_HEADER_BYTES)
        .keep_alive(false);
    let _ = builder
        .serve_connection(TokioIo::new(stream), service)
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::get;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn stalled_headers_release_bounded_connection_capacity() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let admission = ConnectionAdmission::new(1);
        let observed = admission.clone();
        let server = tokio::spawn(serve_with_admission(
            listener,
            Router::new().route("/", get(|| async { "ready" })),
            admission,
            HTTP_HEADER_TIMEOUT,
        ));

        let mut stalled = TcpStream::connect(address).await.unwrap();
        stalled.write_all(b"GET / HTTP/1.1\r\nHost:").await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while observed.active() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the first connection must consume the only slot");

        let mut waiting = TcpStream::connect(address).await.unwrap();
        waiting
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = Vec::new();
        assert!(
            tokio::time::timeout(
                Duration::from_millis(25),
                waiting.read_to_end(&mut response)
            )
            .await
            .is_err(),
            "a second connection bypassed admission while the slot was occupied"
        );

        let mut discarded = Vec::new();
        tokio::time::timeout(Duration::from_secs(1), stalled.read_to_end(&mut discarded))
            .await
            .expect("stalled headers must reach EOF after the deadline")
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), waiting.read_to_end(&mut response))
            .await
            .expect("capacity must become available after the stalled connection closes")
            .unwrap();
        assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200 OK"));

        server.abort();
    }
}
