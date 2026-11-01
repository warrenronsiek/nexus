// @feature runtime
// @feature observability-ui
// @spec docs/features/runtime.md
// @spec docs/features/observability-ui.md
// @boundary browser-wire: authenticate before bounded JSON collection and fixed asset lookup.
use super::{auth::SESSION_SECONDS, read::ReadEnvelope, BrowserState, BODY_TIMEOUT};
use axum::body::to_bytes;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};

mod embedded {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/web/dist/embedded.rs"));
}
const COOKIE: &str = "nexus_browser_session";
const MAX_READ_BYTES: usize = 64 * 1024;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub(super) async fn protect(
    State(state): State<Arc<BrowserState>>,
    request: Request,
    next: Next,
) -> Response {
    let permitted = if request.method() == Method::POST {
        state.origin.permits_post(request.headers())
    } else {
        state.origin.permits_host(request.headers())
    };
    let mut response = if permitted {
        next.run(request).await
    } else {
        error(StatusCode::FORBIDDEN, "browser origin is not allowed")
    };
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_SECURITY_POLICY, "default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; font-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'self'".parse().unwrap());
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    headers.insert(header::X_FRAME_OPTIONS, "DENY".parse().unwrap());
    headers.insert(header::REFERRER_POLICY, "no-referrer".parse().unwrap());
    if !headers.contains_key(header::CACHE_CONTROL) {
        headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    }
    response
}

pub(super) async fn asset(request: Request) -> Response {
    let path = request.uri().path();
    let Some((mime, bytes)) = embedded::asset(path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let cache = if path.starts_with("/assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    (
        [(header::CONTENT_TYPE, mime), (header::CACHE_CONTROL, cache)],
        bytes,
    )
        .into_response()
}
pub(super) async fn session(
    State(state): State<Arc<BrowserState>>,
    headers: HeaderMap,
) -> Response {
    if state.auth.authenticated(token(&headers), Instant::now()) {
        authenticated(&state)
    } else {
        Json(json!({"authenticated":false})).into_response()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Login {
    username: String,
    password: String,
}
pub(super) async fn login(State(state): State<Arc<BrowserState>>, request: Request) -> Response {
    let Some(credentials) = state.auth.credentials.clone() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "browser credentials are not configured; run nexus setup with web credentials",
        );
    };
    let Some(permit) = state.auth.admit_login(Instant::now()) else {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "browser login capacity exceeded; retry later",
        );
    };
    let login: Login = match body(request, 4096).await {
        Ok(login) => login,
        Err(response) => return *response,
    };
    if login.username.len() > 64 || login.password.len() > 1024 {
        return error(StatusCode::BAD_REQUEST, "browser credentials exceed limits");
    }
    if !verify_login(credentials, login, permit).await {
        return error(StatusCode::UNAUTHORIZED, "invalid browser credentials");
    }
    issue_session(&state)
}

fn issue_session(state: &BrowserState) -> Response {
    let Some(token) = state.auth.create_session(Instant::now()) else {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "browser session capacity exceeded",
        );
    };
    let mut response = authenticated(state);
    response.headers_mut().insert(
        header::SET_COOKIE,
        cookie(state, &token, SESSION_SECONDS).parse().unwrap(),
    );
    response
}
async fn verify_login(
    credentials: super::credentials::Credentials,
    login: Login,
    permit: tokio::sync::OwnedSemaphorePermit,
) -> bool {
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        credentials.verifies(&login.username, login.password.as_bytes())
    })
    .await
    .unwrap_or(false)
}

pub(super) async fn logout(State(state): State<Arc<BrowserState>>, headers: HeaderMap) -> Response {
    state.auth.revoke(token(&headers));
    let mut response = Json(json!({"authenticated":false})).into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, cookie(&state, "", 0).parse().unwrap());
    response
}
pub(super) async fn read(State(state): State<Arc<BrowserState>>, request: Request) -> Response {
    if !state
        .auth
        .authenticated(token(request.headers()), Instant::now())
    {
        return error(StatusCode::UNAUTHORIZED, "browser authentication required");
    }
    let Ok(_permit) = state.requests.clone().try_acquire_owned() else {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "browser read capacity exceeded",
        );
    };
    let envelope: ReadEnvelope = match body(request, MAX_READ_BYTES).await {
        Ok(envelope) => envelope,
        Err(response) => return *response,
    };
    let (query, project) = match envelope.prepare() {
        Ok(prepared) => prepared,
        Err(_) => {
            return error(
                StatusCode::BAD_REQUEST,
                "invalid browser read or project scope",
            )
        }
    };
    if !state.project_scope.permits(&query, project.as_deref()) {
        return error(
            StatusCode::BAD_REQUEST,
            "select a project for this read; native remote browser transport cannot read all projects",
        );
    }
    match tokio::time::timeout(
        Duration::from_secs(15),
        state.dispatcher.read(query, project),
    )
    .await
    {
        Ok(Ok(bytes)) if bytes.len() <= MAX_RESPONSE_BYTES => {
            ([(header::CONTENT_TYPE, "application/json")], bytes).into_response()
        }
        _ => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "browser read is temporarily unavailable or exceeds its bounded response",
        ),
    }
}
async fn body<T: DeserializeOwned>(request: Request, limit: usize) -> Result<T, Box<Response>> {
    if !request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("application/json"))
        })
    {
        return Err(Box::new(error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "application/json is required",
        )));
    }
    let bytes = match tokio::time::timeout(BODY_TIMEOUT, to_bytes(request.into_body(), limit)).await
    {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => {
            return Err(Box::new(error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "browser request exceeds its limit",
            )))
        }
        Err(_) => {
            return Err(Box::new(error(
                StatusCode::REQUEST_TIMEOUT,
                "browser request body timed out",
            )))
        }
    };
    serde_json::from_slice(&bytes)
        .map_err(|_| Box::new(error(StatusCode::BAD_REQUEST, "invalid browser request")))
}
fn token(headers: &HeaderMap) -> Option<&str> {
    let mut values = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|part| part.trim().split_once('='))
        .filter(|(name, _)| *name == COOKIE)
        .map(|(_, value)| value);
    let token = values.next()?;
    (values.next().is_none() && token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()))
        .then_some(token)
}
fn cookie(state: &BrowserState, token: &str, max_age: u64) -> String {
    let secure = if state.origin.secure() {
        "; Secure"
    } else {
        ""
    };
    format!("{COOKIE}={token}; Path=/ui/api; HttpOnly; SameSite=Strict; Max-Age={max_age}{secure}")
}
fn authenticated(state: &BrowserState) -> Response {
    let username = &state
        .auth
        .credentials
        .as_ref()
        .expect("session has configured credentials")
        .username;
    Json(json!({
        "authenticated":true,
        "username":username,
        "default_project_id":state.project_scope.default_project_id(),
        "global_project_reads":state.project_scope.global_project_reads()
    }))
    .into_response()
}
fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error":message}))).into_response()
}
