// @feature delivery-coordination
// @feature runtime
// @spec docs/features/delivery-coordination.md
// @spec docs/features/runtime.md
// @entrypoint DeliveryWatcher::start
// @boundary github-rest: decode bounded provider snapshots into typed revision-qualified delivery evidence.
pub use crate::delivery::GithubSource;
use crate::delivery::{CheckState, DeliveryCheck, PullRequestSnapshot, PullRequestState};
use crate::work_history::GitRevision;
use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use reqwest::{header, Client, StatusCode, Url};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};

const PAGE_SIZE: usize = 100;
const MAX_PAGES: usize = 2;
const MAX_BODY_BYTES: usize = 1024 * 1024;

pub struct GithubWatcher {
    client: Client,
    base: Url,
}

#[derive(Debug)]
pub struct GithubPoll {
    pub snapshot: PullRequestSnapshot,
    pub provider_updated_at: DateTime<Utc>,
    pub digest: String,
}

impl GithubWatcher {
    pub fn new(api_base: &str, token: Option<String>) -> Result<Self> {
        let base = Url::parse(&format!("{}/", api_base.trim_end_matches('/')))?;
        anyhow::ensure!(
            base.username().is_empty()
                && base.password().is_none()
                && base.query().is_none()
                && base.fragment().is_none(),
            "GitHub API URL cannot contain credentials, query or fragment"
        );
        anyhow::ensure!(
            base.scheme() == "https"
                || (base.scheme() == "http"
                    && matches!(base.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))),
            "GitHub API URL must use HTTPS"
        );
        let mut headers = header::HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            header::HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            header::USER_AGENT,
            header::HeaderValue::from_static("nexus-delivery-watcher"),
        );
        headers.insert(
            "x-github-api-version",
            header::HeaderValue::from_static("2022-11-28"),
        );
        if let Some(token) = token {
            anyhow::ensure!(
                !token.is_empty()
                    && token.len() <= 4096
                    && token.bytes().all(|b| b.is_ascii_graphic()),
                "invalid GitHub credential"
            );
            let mut authorization = header::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| anyhow!("invalid GitHub credential"))?;
            authorization.set_sensitive(true);
            headers.insert(header::AUTHORIZATION, authorization);
        }
        let client = Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5))
            .build()?;
        Ok(Self { client, base })
    }

    fn from_environment() -> Result<Self> {
        let base = std::env::var("NEXUS_DELIVERY_GITHUB_API_URL")
            .unwrap_or_else(|_| "https://api.github.com".into());
        let token = std::env::var_os("NEXUS_DELIVERY_GITHUB_TOKEN_FILE")
            .map(|path| {
                crate::secret_file::OpenedSecretFile::open(Path::new(&path))
                    .and_then(|file| file.read_utf8())
                    .map(|token| token.trim().to_owned())
                    .map_err(|_| anyhow!("GitHub credential file unavailable or not private"))
            })
            .transpose()?;
        Self::new(&base, token)
    }

    pub async fn poll(&self, source: &GithubSource, repository: &str) -> Result<GithubPoll> {
        source.validate()?;
        tokio::time::timeout(
            Duration::from_secs(30),
            self.poll_snapshot(source, repository),
        )
        .await
        .context("GitHub poll deadline exceeded")?
    }

    async fn poll_snapshot(&self, source: &GithubSource, repository: &str) -> Result<GithubPoll> {
        let path = format!(
            "repos/{}/{}/pulls/{}",
            source.owner, source.repository, source.number
        );
        let initial: ProviderPull = self.get(&path).await?;
        let (base, head) = initial.revisions(repository)?;
        let prefix = format!(
            "repos/{}/{}/commits/{}",
            source.owner, source.repository, head.commit
        );
        let checks = self.provider_checks(&prefix, &head).await?;
        let final_pull: ProviderPull = self.get(&path).await?;
        finish_poll(final_pull, base, head, checks)
    }

    async fn provider_checks(
        &self,
        prefix: &str,
        head: &GitRevision,
    ) -> Result<Vec<DeliveryCheck>> {
        let mut checks = BTreeMap::new();
        for check in self
            .collect_checks::<ProviderChecks>(&format!("{prefix}/check-runs"), head)
            .await?
            .into_iter()
            .chain(
                self.collect_checks::<ProviderStatuses>(&format!("{prefix}/status"), head)
                    .await?,
            )
        {
            checks.insert(check.id.clone(), check);
        }
        anyhow::ensure!(
            checks.len() <= 128,
            "GitHub checks and statuses exceed 128 observations"
        );
        Ok(checks.into_values().collect())
    }

    async fn collect_checks<P: ProviderPage>(
        &self,
        path: &str,
        head: &GitRevision,
    ) -> Result<Vec<DeliveryCheck>> {
        let mut checks = Vec::new();
        for page in 1..=MAX_PAGES {
            let response: P = self
                .get(&format!(
                    "{path}?per_page={PAGE_SIZE}&page={page}{}",
                    P::QUERY
                ))
                .await?;
            let total = response.total();
            anyhow::ensure!(total <= 128, "GitHub checks exceed 128 observations");
            let values = response.into_checks(head)?;
            let count = values.len();
            anyhow::ensure!(
                count <= PAGE_SIZE,
                "GitHub check page exceeds observation limit"
            );
            checks.extend(values);
            if count < PAGE_SIZE || page * PAGE_SIZE >= total {
                return Ok(checks);
            }
        }
        bail!("GitHub checks exceed pagination limit")
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = self.base.join(path)?;
        for attempt in 0..3 {
            let response = self
                .client
                .get(url.clone())
                .send()
                .await
                .map_err(|_| anyhow!("GitHub request unavailable"))?;
            let status = response.status();
            if (status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()) && attempt < 2
            {
                let retry = response
                    .headers()
                    .get(header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(1)
                    .min(2);
                tokio::time::sleep(Duration::from_secs(retry)).await;
                continue;
            }
            if !status.is_success() {
                bail!("GitHub HTTP {}", status.as_u16());
            }
            return decode_response(response).await;
        }
        bail!("GitHub retry limit exceeded")
    }
}

async fn decode_response<T: DeserializeOwned>(mut response: reqwest::Response) -> Result<T> {
    anyhow::ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= MAX_BODY_BYTES as u64),
        "GitHub response exceeds observation limit"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow!("GitHub response unavailable"))?
    {
        anyhow::ensure!(
            bytes.len().saturating_add(chunk.len()) <= MAX_BODY_BYTES,
            "GitHub response exceeds observation limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| anyhow!("GitHub response shape invalid"))
}

fn finish_poll(
    final_pull: ProviderPull,
    base: GitRevision,
    head: GitRevision,
    checks: Vec<DeliveryCheck>,
) -> Result<GithubPoll> {
    anyhow::ensure!(
        final_pull.head.sha == head.commit && final_pull.base.sha == base.commit,
        "GitHub pull-request head changed during poll"
    );
    let state = if final_pull.merged {
        PullRequestState::Merged
    } else if final_pull.state == "open" {
        PullRequestState::Open
    } else {
        PullRequestState::Closed
    };
    let mut snapshot = PullRequestSnapshot {
        observed_at: DateTime::UNIX_EPOCH,
        state,
        base,
        head,
        base_ref: final_pull.base.branch,
        head_ref: final_pull.head.branch,
        checks,
    };
    let canonical = serde_json::to_vec(&snapshot)?;
    anyhow::ensure!(
        canonical.len() <= 60 * 1024,
        "GitHub snapshot exceeds delivery observation budget"
    );
    let digest = blake3::hash(&canonical).to_hex().to_string();
    snapshot.observed_at = Utc::now();
    Ok(GithubPoll {
        snapshot,
        provider_updated_at: final_pull.updated_at,
        digest,
    })
}

fn bounded_text(value: &str, maximum: usize) -> Result<()> {
    anyhow::ensure!(
        !value.is_empty() && value.len() <= maximum && !value.chars().any(char::is_control),
        "GitHub metadata exceeds observation contract"
    );
    Ok(())
}
fn bounded_url(value: Option<String>) -> Result<Option<String>> {
    if let Some(value) = &value {
        bounded_text(value, 2048)?;
    }
    Ok(value)
}
fn revision(repository: &str, commit: &str) -> Result<GitRevision> {
    anyhow::ensure!(
        matches!(commit.len(), 40 | 64) && commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "GitHub commit identity invalid"
    );
    Ok(GitRevision {
        repository: repository.into(),
        object_format: if commit.len() == 40 { "sha1" } else { "sha256" }.into(),
        commit: commit.to_ascii_lowercase(),
    })
}
fn check_state(status: &str, conclusion: Option<&str>) -> CheckState {
    match (status, conclusion) {
        ("queued" | "waiting" | "pending" | "requested", _) => CheckState::Queued,
        ("in_progress", _) => CheckState::Running,
        ("completed", Some("success")) => CheckState::Succeeded,
        ("completed", Some("failure" | "timed_out" | "action_required" | "startup_failure")) => {
            CheckState::Failed
        }
        ("completed", Some("cancelled" | "stale")) => CheckState::Canceled,
        ("completed", Some("skipped" | "neutral")) => CheckState::Skipped,
        _ => CheckState::Unknown,
    }
}
#[derive(Deserialize)]
struct ProviderRevision {
    sha: String,
    #[serde(rename = "ref", default)]
    branch: Option<String>,
}
#[derive(Deserialize)]
struct ProviderPull {
    state: String,
    #[serde(default)]
    merged: bool,
    updated_at: DateTime<Utc>,
    base: ProviderRevision,
    head: ProviderRevision,
}
impl ProviderPull {
    fn revisions(&self, repository: &str) -> Result<(GitRevision, GitRevision)> {
        Ok((
            revision(repository, &self.base.sha)?,
            revision(repository, &self.head.sha)?,
        ))
    }
}
#[derive(Deserialize)]
struct ProviderChecks {
    total_count: usize,
    check_runs: Vec<ProviderCheck>,
}
#[derive(Deserialize)]
struct ProviderCheck {
    id: u64,
    name: String,
    head_sha: String,
    status: String,
    conclusion: Option<String>,
    html_url: Option<String>,
}
#[derive(Deserialize)]
struct ProviderStatuses {
    total_count: usize,
    statuses: Vec<ProviderStatus>,
}
#[derive(Deserialize)]
struct ProviderStatus {
    id: u64,
    context: String,
    state: String,
    target_url: Option<String>,
}

trait ProviderPage: DeserializeOwned {
    const QUERY: &'static str;
    fn total(&self) -> usize;
    fn into_checks(self, head: &GitRevision) -> Result<Vec<DeliveryCheck>>;
}
impl ProviderPage for ProviderChecks {
    const QUERY: &'static str = "&filter=latest";
    fn total(&self) -> usize {
        self.total_count
    }
    fn into_checks(self, head: &GitRevision) -> Result<Vec<DeliveryCheck>> {
        self.check_runs
            .into_iter()
            .map(|check| {
                anyhow::ensure!(
                    check.head_sha == head.commit,
                    "GitHub check revision differs from pinned head"
                );
                bounded_text(&check.name, 512)?;
                Ok(DeliveryCheck {
                    id: format!("github:check:{}", check.id),
                    name: check.name,
                    state: check_state(&check.status, check.conclusion.as_deref()),
                    revision: head.clone(),
                    url: bounded_url(check.html_url)?,
                })
            })
            .collect()
    }
}
impl ProviderPage for ProviderStatuses {
    const QUERY: &'static str = "";
    fn total(&self) -> usize {
        self.total_count
    }
    fn into_checks(self, head: &GitRevision) -> Result<Vec<DeliveryCheck>> {
        self.statuses
            .into_iter()
            .map(|status| {
                bounded_text(&status.context, 512)?;
                let state = match status.state.as_str() {
                    "success" => CheckState::Succeeded,
                    "failure" | "error" => CheckState::Failed,
                    "pending" => CheckState::Running,
                    _ => CheckState::Unknown,
                };
                Ok(DeliveryCheck {
                    id: format!("github:status:{}", status.id),
                    name: status.context,
                    state,
                    revision: head.clone(),
                    url: bounded_url(status.target_url)?,
                })
            })
            .collect()
    }
}

pub struct DeliveryWatcher {
    task: tokio::task::JoinHandle<()>,
}
impl Drop for DeliveryWatcher {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl DeliveryWatcher {
    pub fn start(service: Arc<crate::NexusService>) -> Self {
        Self {
            task: tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(5));
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tick.tick().await;
                    let target = service.clone();
                    let watches =
                        tokio::task::spawn_blocking(move || target.claim_delivery_watches()).await;
                    let Ok(Ok(watches)) = watches else {
                        continue;
                    };
                    let mut polls = tokio::task::JoinSet::new();
                    for watch in watches {
                        let target = service.clone();
                        polls.spawn(async move {
                            let poll = match GithubWatcher::from_environment() {
                                Ok(client) => {
                                    client
                                        .poll(
                                            &watch.registration.source,
                                            &watch.registration.repository_id,
                                        )
                                        .await
                                }
                                Err(error) => Err(error),
                            };
                            let _ = tokio::task::spawn_blocking(move || {
                                target.apply_delivery_watch_poll(&watch, poll)
                            })
                            .await;
                        });
                    }
                    while polls.join_next().await.is_some() {}
                }
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_snapshot_must_fit_the_delivery_observation_budget() {
        let base = revision(&"r".repeat(500), &"a".repeat(40)).unwrap();
        let head = revision(&"r".repeat(500), &"b".repeat(40)).unwrap();
        let provider = ProviderPull {
            state: "open".into(),
            merged: false,
            updated_at: Utc::now(),
            base: ProviderRevision {
                sha: base.commit.clone(),
                branch: None,
            },
            head: ProviderRevision {
                sha: head.commit.clone(),
                branch: None,
            },
        };
        let checks = (0..128)
            .map(|id| DeliveryCheck {
                id: id.to_string(),
                name: "n".repeat(512),
                state: CheckState::Succeeded,
                revision: head.clone(),
                url: None,
            })
            .collect();
        let error = finish_poll(provider, base, head, checks)
            .expect_err("oversized provider snapshot was accepted");
        assert!(error.to_string().contains("snapshot exceeds"));
    }
}
