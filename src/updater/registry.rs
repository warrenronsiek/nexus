// @feature installation
// @spec docs/features/installation.md
// @boundary release-registry-http: bounded fixed-origin GitHub metadata and release assets, with an isolated loopback-only test transport.
use super::release::{hex, Artifact, ReleaseManifest};
use anyhow::{ensure, Context, Result};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum RevisionRelation {
    Current,
    Behind,
    Ahead,
    Diverged,
    Unknown,
}
#[derive(Deserialize)]
struct GitReference {
    object: GitObject,
}
#[derive(Deserialize)]
struct GitObject {
    sha: String,
}
#[derive(Deserialize)]
struct Comparison {
    status: ComparisonStatus,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum ComparisonStatus {
    Identical,
    Ahead,
    Behind,
    Diverged,
}

pub(super) struct Registry {
    client: Client,
    api: String,
    downloads: String,
}
impl Registry {
    pub(super) fn github() -> Result<Self> {
        Ok(Self {
            client: Client::builder()
                .https_only(true)
                .connect_timeout(Duration::from_secs(5))
                .user_agent(concat!("nexus-update/", env!("CARGO_PKG_VERSION")))
                .build()?,
            api: "https://api.github.com".into(),
            downloads: "https://github.com".into(),
        })
    }
    #[cfg(test)]
    pub(in crate::updater) fn local(origin: &str) -> Self {
        Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(1))
                .build()
                .unwrap(),
            api: origin.into(),
            downloads: origin.into(),
        }
    }
    pub(super) async fn main_revision(&self) -> Result<String> {
        let bytes = self
            .metadata(&format!(
                "{}/repos/warrenronsiek/nexus/git/ref/heads/main",
                self.api
            ))
            .await?
            .context("Nexus main reference is unavailable")?;
        let reference: GitReference = serde_json::from_slice(&bytes)?;
        ensure!(
            hex(&reference.object.sha, 40),
            "GitHub returned an invalid main revision"
        );
        Ok(reference.object.sha)
    }
    pub(super) async fn compare(&self, current: &str, main: &str) -> Result<RevisionRelation> {
        if current == main {
            return Ok(RevisionRelation::Current);
        }
        if !hex(current, 40) {
            return Ok(RevisionRelation::Unknown);
        }
        ensure!(hex(main, 40), "invalid main revision");
        // GitHub includes file patches only on the first page; page two retains
        // ancestry status without downloading an unrelated potentially large diff.
        let url = format!(
            "{}/repos/warrenronsiek/nexus/compare/{current}...{main}?per_page=1&page=2",
            self.api
        );
        let Some(bytes) = self.metadata(&url).await? else {
            return Ok(RevisionRelation::Unknown);
        };
        let comparison: Comparison = serde_json::from_slice(&bytes)?;
        Ok(match comparison.status {
            ComparisonStatus::Identical => RevisionRelation::Current,
            ComparisonStatus::Ahead => RevisionRelation::Behind,
            ComparisonStatus::Behind => RevisionRelation::Ahead,
            ComparisonStatus::Diverged => RevisionRelation::Diverged,
        })
    }
    pub(super) async fn manifest(&self, revision: &str) -> Result<Option<ReleaseManifest>> {
        ensure!(hex(revision, 40), "invalid release revision");
        self.metadata(&self.asset_url(revision, "manifest.json"))
            .await?
            .map(|bytes| ReleaseManifest::decode(&bytes, revision))
            .transpose()
    }
    fn asset_url(&self, revision: &str, name: &str) -> String {
        format!(
            "{}/warrenronsiek/nexus/releases/download/main-{revision}/{name}",
            self.downloads
        )
    }
    async fn metadata(&self, url: &str) -> Result<Option<Vec<u8>>> {
        let response = self
            .client
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .timeout(Duration::from_secs(15))
            .send()
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        };
        let mut response = response.error_for_status()?;
        ensure!(
            response
                .content_length()
                .is_none_or(|length| length <= 64 * 1024),
            "release metadata exceeds 64 KiB"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= 64 * 1024,
                "release metadata exceeds 64 KiB"
            );
            bytes.extend_from_slice(&chunk);
        }
        Ok(Some(bytes))
    }
    pub(super) async fn download(
        &self,
        revision: &str,
        artifact: &Artifact,
        destination: &Path,
    ) -> Result<()> {
        let response = self.download_response(revision, artifact).await?;
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(destination)
            .await?;
        write_download(response, &mut file, artifact.size).await
    }
    async fn download_response(
        &self,
        revision: &str,
        artifact: &Artifact,
    ) -> Result<reqwest::Response> {
        let response = self
            .client
            .get(self.asset_url(revision, &artifact.name))
            .timeout(Duration::from_secs(600))
            .send()
            .await?
            .error_for_status()?;
        ensure!(
            response
                .content_length()
                .is_none_or(|length| length == artifact.size),
            "release download size differs from the manifest"
        );
        Ok(response)
    }
}

async fn write_download(
    mut response: reqwest::Response,
    file: &mut tokio::fs::File,
    expected: u64,
) -> Result<()> {
    let mut size = 0_u64;
    while let Some(chunk) = response.chunk().await? {
        size += chunk.len() as u64;
        ensure!(
            size <= expected,
            "release download exceeds its declared size"
        );
        file.write_all(&chunk).await?;
    }
    ensure!(
        size == expected,
        "release download ended before its declared size"
    );
    file.sync_all().await?;
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    fn accept(listener: &std::net::TcpListener) -> std::net::TcpStream {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            match listener.accept() {
                Ok((stream, _)) => return stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "registry request did not arrive"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(error) => panic!("{error}"),
            }
        }
    }
    fn respond(mut stream: std::net::TcpStream, path: &str, status: u16, body: &[u8]) {
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut request = String::new();
        BufReader::new(&stream).read_line(&mut request).unwrap();
        assert!(request.starts_with(&format!("GET {path} ")));
        write!(
            stream,
            "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(body).unwrap();
    }
    pub(in crate::updater) fn serve(
        responses: Vec<(&str, u16, Vec<u8>)>,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let routes: Vec<_> = responses
            .into_iter()
            .map(|(path, status, body)| (path.to_owned(), status, body))
            .collect();
        let task = std::thread::spawn(move || {
            for (path, status, body) in routes {
                respond(accept(&listener), &path, status, &body);
            }
        });
        (address, task)
    }

    #[tokio::test]
    async fn main_comparison_and_verified_download_work_without_git_or_live_network() {
        let revision = "a".repeat(40);
        let comparison = format!(
            "/repos/warrenronsiek/nexus/compare/{}...{revision}?per_page=1&page=2",
            "b".repeat(40)
        );
        let manifest_path =
            format!("/warrenronsiek/nexus/releases/download/main-{revision}/manifest.json");
        let asset_path = format!(
            "/warrenronsiek/nexus/releases/download/main-{revision}/nexus-aarch64-apple-darwin"
        );
        let (origin, server) = serve(vec![
            (
                "/repos/warrenronsiek/nexus/git/ref/heads/main",
                200,
                serde_json::to_vec(&serde_json::json!({"object":{"sha":revision}})).unwrap(),
            ),
            (&comparison, 200, br#"{"status":"ahead"}"#.to_vec()),
            (
                &manifest_path,
                200,
                serde_json::to_vec(&super::super::release::tests::fixture()).unwrap(),
            ),
            (&asset_path, 200, b"verified binary".to_vec()),
        ]);
        let registry = Registry::local(&origin);
        assert_eq!(registry.main_revision().await.unwrap(), revision);
        assert_eq!(
            registry.compare(&"b".repeat(40), &revision).await.unwrap(),
            RevisionRelation::Behind
        );
        let release = registry.manifest(&revision).await.unwrap().unwrap();
        let artifact = release.binary("aarch64-apple-darwin").unwrap();
        let file = tempfile::NamedTempFile::new().unwrap();
        registry
            .download(&revision, artifact, file.path())
            .await
            .unwrap();
        artifact.verify(file.path()).unwrap();
        server.join().unwrap();
    }

    #[tokio::test]
    async fn unpublished_main_and_oversize_downloads_do_not_look_like_verified_releases() {
        let revision = "a".repeat(40);
        let path = format!("/warrenronsiek/nexus/releases/download/main-{revision}/manifest.json");
        let (origin, server) = serve(vec![(&path, 404, Vec::new())]);
        assert!(Registry::local(&origin)
            .manifest(&revision)
            .await
            .unwrap()
            .is_none());
        server.join().unwrap();
        let path = format!(
            "/warrenronsiek/nexus/releases/download/main-{revision}/nexus-aarch64-apple-darwin"
        );
        let (origin, server) = serve(vec![(&path, 200, vec![0; 100])]);
        let release = super::super::release::ReleaseManifest::decode(
            &serde_json::to_vec(&super::super::release::tests::fixture()).unwrap(),
            &revision,
        )
        .unwrap();
        let destination = tempfile::NamedTempFile::new().unwrap();
        assert!(Registry::local(&origin)
            .download(
                &revision,
                release.binary("aarch64-apple-darwin").unwrap(),
                destination.path()
            )
            .await
            .is_err());
        server.join().unwrap();
    }
}
