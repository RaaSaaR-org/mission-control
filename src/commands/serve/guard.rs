//! Protection for every request that can change data.
//!
//! The dashboard has no login, so a write must prove it comes from the
//! dashboard itself:
//! - edits must be enabled (`--read-only` and proxy mode turn them off);
//! - the `X-MC-Request: 1` header must be present. Browsers only let a page
//!   set it on cross-origin requests after a CORS preflight, which this
//!   server never approves, so forms and scripts on other sites can't send it;
//! - `Origin`, when sent, must match the host the request was made to;
//! - when served locally (no base path), the `Host` must be a loopback name,
//!   which blocks DNS-rebinding pages that pose as same-origin.

use super::api::ApiError;
use super::AppState;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;

/// Header every write request must carry.
pub(super) const REQUEST_HEADER: &str = "x-mc-request";

pub(super) async fn protect_writes(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        return next.run(req).await;
    }
    match check(&state, req.headers()) {
        Ok(()) => next.run(req).await,
        Err(e) => e.into_response(),
    }
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
}

fn check(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    if !state.editable {
        let hint = if state.base_path.is_empty() {
            "Restart mc serve without --read-only to edit."
        } else {
            "Restart mc serve with --allow-edits to edit behind a proxy."
        };
        return Err(ApiError::forbidden(
            "read_only",
            &format!("This dashboard is read-only. {hint}"),
        ));
    }
    if header(headers, REQUEST_HEADER) != Some("1") {
        return Err(ApiError::forbidden(
            "missing_header",
            "Write requests need the X-MC-Request: 1 header.",
        ));
    }
    if matches!(
        header(headers, "sec-fetch-site"),
        Some("cross-site" | "same-site")
    ) {
        return Err(cross_origin());
    }
    let host = header(headers, "host");
    let forwarded = header(headers, "x-forwarded-host")
        .and_then(|h| h.split(',').next())
        .map(str::trim);
    if state.base_path.is_empty() {
        if let Some(h) = host {
            if !is_loopback_host(h) {
                return Err(ApiError::forbidden(
                    "bad_host",
                    "Edits are only accepted on localhost.",
                ));
            }
        }
    }
    if let Some(origin) = header(headers, "origin") {
        let authority = origin_authority(origin).ok_or_else(cross_origin)?;
        let matches = |h: Option<&str>| h.is_some_and(|h| h.eq_ignore_ascii_case(authority));
        let same = matches(host) || (!state.base_path.is_empty() && matches(forwarded));
        if !same {
            return Err(cross_origin());
        }
    }
    Ok(())
}

fn cross_origin() -> ApiError {
    ApiError::forbidden(
        "cross_origin",
        "Write requests must come from the dashboard itself.",
    )
}

/// `host[:port]` of an origin like `http://localhost:5000`; `None` for `null`
/// or anything that isn't an http(s) origin.
fn origin_authority(origin: &str) -> Option<&str> {
    let rest = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))?;
    (!rest.is_empty() && !rest.contains('/')).then_some(rest)
}

/// Whether a `Host` header names this machine.
fn is_loopback_host(host: &str) -> bool {
    let name = if let Some(rest) = host.strip_prefix('[') {
        // [::1]:5000
        rest.split(']').next().unwrap_or("")
    } else {
        host.rsplit_once(':').map_or(host, |(h, _)| h)
    };
    let name = name.to_ascii_lowercase();
    name == "localhost"
        || name.ends_with(".localhost")
        || name == "::1"
        || name
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_hosts() {
        for h in [
            "localhost",
            "localhost:5000",
            "127.0.0.1:5000",
            "127.1.2.3",
            "[::1]:80",
            "app.localhost:1",
        ] {
            assert!(is_loopback_host(h), "{h}");
        }
        for h in [
            "evil.com",
            "evil.com:5000",
            "10.0.0.1:5000",
            "localhost.evil.com",
        ] {
            assert!(!is_loopback_host(h), "{h}");
        }
    }

    #[test]
    fn origin_parsing() {
        assert_eq!(
            origin_authority("http://localhost:5000"),
            Some("localhost:5000")
        );
        assert_eq!(
            origin_authority("https://hq.example.com"),
            Some("hq.example.com")
        );
        assert_eq!(origin_authority("null"), None);
        assert_eq!(origin_authority("file://"), None);
    }
}
