//! Protection for every request, and above all for those that change data.
//!
//! When served locally (no base path), every request -- reads too -- must
//! name this machine in its `Host` header. A DNS-rebinding page (a foreign
//! name that resolves to 127.0.0.1) would otherwise read the whole repo.
//!
//! Every response says it may not be framed by another site, so a hidden
//! iframe can't trick a click onto a checkbox or a move menu. Responses that
//! aren't HTML pages (images, SVGs, fonts, JSON) also get a sandbox CSP, so
//! an SVG opened directly can never run scripts.
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
use axum::http::{header as h, HeaderMap, HeaderValue, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;

/// Header every write request must carry.
pub(super) const REQUEST_HEADER: &str = "x-mc-request";

/// CSP for everything that isn't an HTML page: shown, never run.
const INERT_CSP: &str = "default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; sandbox";

pub(super) async fn protect(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    if state.base_path.is_empty() {
        if let Some(host) = header(req.headers(), "host") {
            if !is_loopback_host(host) {
                let mut res = ApiError::forbidden(
                    "bad_host",
                    "mc serve only answers on localhost. Use --base-path behind a reverse proxy.",
                )
                .into_response();
                harden(res.headers_mut());
                return res;
            }
        }
    }
    let mut res = if matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        next.run(req).await
    } else {
        match check(&state, req.headers()) {
            Ok(()) => next.run(req).await,
            Err(e) => e.into_response(),
        }
    };
    harden(res.headers_mut());
    res
}

/// Security headers for every response: no framing by other sites, no
/// content sniffing, no referrer to other origins, and no scripts in
/// anything that isn't an HTML page.
fn harden(headers: &mut HeaderMap) {
    headers.insert(h::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(h::REFERRER_POLICY, HeaderValue::from_static("same-origin"));
    headers
        .entry(h::X_CONTENT_TYPE_OPTIONS)
        .or_insert(HeaderValue::from_static("nosniff"));
    let html = headers
        .get(h::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("text/html"));
    let csp = match headers.get(h::CONTENT_SECURITY_POLICY) {
        Some(v) => match v.to_str() {
            Ok(csp) if !csp.contains("frame-ancestors") => {
                format!("{csp}; frame-ancestors 'none'")
            }
            _ => return,
        },
        None if html => "frame-ancestors 'none'".to_string(),
        None => format!("{INERT_CSP}; frame-ancestors 'none'"),
    };
    if let Ok(v) = HeaderValue::from_str(&csp) {
        headers.insert(h::CONTENT_SECURITY_POLICY, v);
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
    use crate::commands::init;
    use crate::config::{self, ResolvedConfig};
    use axum::body::Body;
    use axum::http::StatusCode;
    use tower::ServiceExt;

    fn repo() -> (tempfile::TempDir, ResolvedConfig) {
        let tmp = tempfile::TempDir::new().unwrap();
        init::run(tmp.path(), false, false, Some("Guard"), false, true).unwrap();
        let cfg = config::load_config(tmp.path(), config::RepoMode::Standalone).unwrap();
        (tmp, cfg)
    }

    async fn get(cfg: &ResolvedConfig, base: &str, uri: &str, host: Option<&str>) -> Response {
        let router = super::super::build_router(cfg, base);
        let mut req = Request::builder().uri(uri);
        if let Some(host) = host {
            req = req.header("host", host);
        }
        router
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    fn csp(res: &Response) -> &str {
        res.headers()[h::CONTENT_SECURITY_POLICY].to_str().unwrap()
    }

    #[tokio::test]
    async fn reads_need_a_loopback_host_when_local() {
        let (_tmp, cfg) = repo();
        for uri in ["/", "/index.json", "/api/version"] {
            let res = get(&cfg, "", uri, Some("attacker.example:5000")).await;
            assert_eq!(res.status(), StatusCode::FORBIDDEN, "{uri}");
            let res = get(&cfg, "", uri, Some("localhost:5000")).await;
            assert_eq!(res.status(), StatusCode::OK, "{uri}");
        }
        // Behind a proxy the public name is expected.
        let res = get(&cfg, "/hq", "/hq/index.json", Some("hq.example.com")).await;
        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn pages_cannot_be_framed_and_files_stay_inert() {
        let (tmp, mut cfg) = repo();
        let res = get(&cfg, "", "/", None).await;
        assert_eq!(res.headers()[h::X_FRAME_OPTIONS], "DENY");
        assert_eq!(csp(&res), "frame-ancestors 'none'");

        std::fs::write(tmp.path().join("research/x.svg"), "<svg/>").unwrap();
        let res = get(&cfg, "", "/files/research/x.svg", None).await;
        assert_eq!(res.status(), StatusCode::OK);
        assert!(csp(&res).contains("sandbox") && csp(&res).ends_with("frame-ancestors 'none'"));

        // Brand files referenced by the custom stylesheet: an SVG there must
        // not run scripts either.
        let brand = tmp.path().join("assets/brand");
        std::fs::create_dir_all(&brand).unwrap();
        std::fs::write(brand.join("brand.css"), "").unwrap();
        std::fs::write(brand.join("evil.svg"), "<svg><script>1</script></svg>").unwrap();
        cfg.brand.custom_css = Some(brand.join("brand.css"));
        cfg.brand.logo = Some(brand.join("evil.svg"));
        for uri in ["/brand/asset/evil.svg", "/brand/logo"] {
            let res = get(&cfg, "", uri, None).await;
            assert_eq!(res.status(), StatusCode::OK, "{uri}");
            assert_eq!(res.headers()[h::CONTENT_TYPE], "image/svg+xml");
            assert!(csp(&res).contains("sandbox"), "{uri}");
            assert_eq!(res.headers()[h::X_CONTENT_TYPE_OPTIONS], "nosniff");
        }
    }

    #[tokio::test]
    async fn app_assets_are_guarded_like_pages() {
        let (_tmp, cfg) = repo();
        for uri in ["/assets/app.css", "/assets/app.js", "/assets/archivo.woff2"] {
            let res = get(&cfg, "", uri, Some("attacker.example:5000")).await;
            assert_eq!(res.status(), StatusCode::FORBIDDEN, "{uri}");
            let res = get(&cfg, "", uri, Some("127.0.0.1:5000")).await;
            assert_eq!(res.status(), StatusCode::OK, "{uri}");
            assert_eq!(res.headers()[h::X_FRAME_OPTIONS], "DENY", "{uri}");
            assert_eq!(res.headers()[h::X_CONTENT_TYPE_OPTIONS], "nosniff");
            // Long-lived caching from the asset routes survives the guard.
            let cache = res.headers()[h::CACHE_CONTROL].to_str().unwrap();
            assert!(cache.contains("immutable"), "{uri}: {cache}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn symlinks_do_not_expose_hidden_paths() {
        let (tmp, cfg) = repo();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".git/config.md"), "[remote]\nurl = secret\n").unwrap();
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::write(root.join(".claude/agent.png"), "png").unwrap();
        std::os::unix::fs::symlink("../.git/config.md", root.join("research/gitcfg.md")).unwrap();
        std::os::unix::fs::symlink("../.claude", root.join("research/claude")).unwrap();
        for uri in [
            "/files/research/gitcfg.md",
            "/files/research/claude/agent.png",
        ] {
            let res = get(&cfg, "", uri, None).await;
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{uri}");
        }
        // A symlink to a visible file still works.
        std::fs::write(root.join("research/real.png"), "png").unwrap();
        std::os::unix::fs::symlink("real.png", root.join("research/alias.png")).unwrap();
        let res = get(&cfg, "", "/files/research/alias.png", None).await;
        assert_eq!(res.status(), StatusCode::OK);
    }

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
