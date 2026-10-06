//! Integration tests for entity preview cards (`GET /api/preview/{id}`), the
//! fragments behind the dashboard's hover and focus previews.

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use mc::commands::init;
use mc::commands::serve::{router, ServeOptions};
use mc::config::{self, RepoMode};
use std::path::Path;
use tempfile::TempDir;
use tower::ServiceExt;

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn repo() -> TempDir {
    let tmp = TempDir::new().unwrap();
    init::run(tmp.path(), false, false, Some("WebTest"), false, true).unwrap();
    let today = chrono::Local::now().date_naive();
    let soon = (today + chrono::Duration::days(2)).format("%Y-%m-%d");
    write(
        tmp.path(),
        "tasks/todo/TASK-001-fix.md",
        &format!(
            "---\nid: TASK-001\ntitle: Fix the robot\nstatus: todo\npriority: 2\nowner: Jane Doe\ndue_date: {soon}\nsprint: \"[[SPR-001|Alpha]]\"\n---\n\n# Fix the robot\n\nThe gripper slips. See [[TASK-002]].\n\n- [x] Order parts\n- [ ] Fit parts\n"
        ),
    );
    write(
        tmp.path(),
        "tasks/todo/TASK-002-evil.md",
        "---\nid: TASK-002\ntitle: \"<script>alert(1)</script>\"\nstatus: \"<img src=x onerror=alert(1)>\"\nowner: \"<b>Mallory</b>\"\n---\n\n<iframe src=javascript:alert(1)></iframe> Body & more\n",
    );
    write(
        tmp.path(),
        "sprints/SPR-001-alpha.md",
        "---\nid: SPR-001\ntitle: Alpha\nstatus: active\n---\n",
    );
    tmp
}

fn app(tmp: &TempDir, opts: ServeOptions) -> Router {
    let cfg = config::load_config(tmp.path(), RepoMode::Standalone).unwrap();
    router(&cfg, &opts)
}

async fn send(router: &Router, method: Method, uri: &str) -> Response {
    router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn text(resp: Response) -> String {
    let bytes = to_bytes(resp.into_body(), 1 << 24).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn known_id_returns_a_card_fragment() {
    let tmp = repo();
    let router = app(&tmp, ServeOptions::default());
    let resp = send(&router, Method::GET, "/api/preview/TASK-001").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(resp.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    assert_eq!(resp.headers()[header::CACHE_CONTROL], "no-store");
    let html = text(resp).await;
    // A fragment, not a page.
    assert!(
        html.starts_with(r#"<div class="pv-card is-soon">"#),
        "{html}"
    );
    assert!(!html.contains("<!DOCTYPE"));
    assert!(html.contains(r#"<span class="pv-kind">Task</span>"#));
    assert!(html.contains(r#"<a href="/entity/TASK-001">Fix the robot</a>"#));
    assert!(html.contains("badge-todo"));
    assert!(html.contains("pri-high"));
    assert!(html.contains("Jane Doe"));
    assert!(html.contains(r#"class="due due-soon""#));
    assert!(html.contains(">Alpha</a>"));
    // Plain-text excerpt: the H1 is dropped and wikilinks show names.
    assert!(
        html.contains("The gripper slips. See &lt;script&gt;"),
        "{html}"
    );
    assert!(html.contains("1 of 2 done"));
}

#[tokio::test]
async fn unknown_id_is_a_404_card() {
    let tmp = repo();
    let router = app(&tmp, ServeOptions::default());
    for uri in ["/api/preview/TASK-999", "/api/preview/nonsense"] {
        let resp = send(&router, Method::GET, uri).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{uri}");
        let html = text(resp).await;
        assert!(html.contains("pv-card is-missing"), "{uri}");
        assert!(html.contains("Not found"), "{uri}");
    }
}

#[tokio::test]
async fn hostile_values_are_escaped() {
    let tmp = repo();
    let router = app(&tmp, ServeOptions::default());
    let html = text(send(&router, Method::GET, "/api/preview/TASK-002").await).await;
    assert!(!html.contains("<script"), "{html}");
    assert!(!html.contains("<img"), "{html}");
    assert!(!html.contains("<b>"), "{html}");
    assert!(!html.contains("<iframe"), "{html}");
    assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(html.contains("&lt;b&gt;Mallory&lt;/b&gt;"));
    let missing = text(
        send(
            &router,
            Method::GET,
            "/api/preview/%3Cimg%20src%3Dx%20onerror%3Dalert(1)%3E",
        )
        .await,
    )
    .await;
    assert!(!missing.contains("<img"), "{missing}");
    assert!(missing.contains("&lt;img"), "{missing}");
}

#[tokio::test]
async fn previews_work_read_only_and_behind_a_base_path() {
    let tmp = repo();
    let router = app(
        &tmp,
        ServeOptions {
            base_path: "/hq".into(),
            read_only: true,
            ..Default::default()
        },
    );
    let resp = send(&router, Method::GET, "/hq/api/preview/TASK-001").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let html = text(resp).await;
    assert!(html.contains(r#"href="/hq/entity/TASK-001""#));
    assert!(!html.contains(r#"href="/entity/"#));
    // Only GET is routed.
    let resp = send(&router, Method::POST, "/hq/api/preview/TASK-001").await;
    assert!(resp.status().is_client_error());
}

#[tokio::test]
async fn pages_ship_the_preview_script() {
    let tmp = repo();
    let router = app(&tmp, ServeOptions::default());
    let html = text(send(&router, Method::GET, "/entity/TASK-001").await).await;
    assert!(html.contains(r#""/api/preview/""#));
    assert!(html.contains(".preview-pop"));
}
