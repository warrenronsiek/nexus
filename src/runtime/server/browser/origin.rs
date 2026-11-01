// @feature runtime
// @feature observability-ui
// @spec docs/features/runtime.md
// @spec docs/features/observability-ui.md
// @boundary browser-origin: authority and Origin are checked without trusting forwarded headers.
use anyhow::{ensure, Result};
use axum::http::{header, HeaderMap, Uri};
use std::net::{IpAddr, SocketAddr};

#[derive(Clone)]
pub(super) enum BrowserOrigin {
    Loopback,
    Https { origin: String, authority: String },
}
impl BrowserOrigin {
    pub(super) fn parse(origin: Option<&str>) -> Result<Self> {
        let Some(origin) = origin else {
            return Ok(Self::Loopback);
        };
        let url = reqwest::Url::parse(origin)?;
        ensure!(
            url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && url.path() == "/",
            "NEXUS_WEB_ORIGIN must be an HTTPS origin without credentials, path, query or fragment"
        );
        let canonical = url.origin().ascii_serialization();
        let authority = canonical
            .strip_prefix("https://")
            .expect("validated HTTPS")
            .to_owned();
        Ok(Self::Https {
            origin: canonical,
            authority,
        })
    }
    pub(super) fn secure(&self) -> bool {
        matches!(self, Self::Https { .. })
    }
    pub(super) fn permits_host(&self, headers: &HeaderMap) -> bool {
        let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
            return false;
        };
        if headers.get_all(header::HOST).iter().count() != 1 {
            return false;
        }
        match self {
            Self::Loopback => loopback_authority(host),
            Self::Https { authority, .. } => host == authority,
        }
    }
    pub(super) fn permits_post(&self, headers: &HeaderMap) -> bool {
        if !self.permits_host(headers) {
            return false;
        }
        let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
            return false;
        };
        if headers.get_all(header::ORIGIN).iter().count() != 1 {
            return false;
        }
        match self {
            Self::Loopback => headers
                .get(header::HOST)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|host| origin == format!("http://{host}")),
            Self::Https {
                origin: expected, ..
            } => origin == expected,
        }
    }
}
fn loopback_authority(value: &str) -> bool {
    let Ok(uri) = format!("http://{value}").parse::<Uri>() else {
        return false;
    };
    let Some(authority) = uri.authority() else {
        return false;
    };
    if authority.as_str() != value || value.contains('@') {
        return false;
    }
    let host = authority
        .host()
        .trim_start_matches('[')
        .trim_end_matches(']');
    host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

pub(super) fn validate_listener(address: SocketAddr, origin: &BrowserOrigin) -> Result<()> {
    ensure!(
        address.ip().is_loopback() || origin.secure(),
        "browser listener requires loopback or an explicit HTTPS public origin"
    );
    Ok(())
}
