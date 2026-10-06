//! Integration tests for the `mc serve` dashboard.
//!
//! The router is driven in-process with `tower::ServiceExt::oneshot`, so no
//! socket is bound.

use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use mc::commands::init;
use mc::commands::serve::build_router;
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
    let late = (today - chrono::Duration::days(3)).format("%Y-%m-%d");
    write(
        tmp.path(),
        "tasks/todo/TASK-001-fix.md",
        &format!(
            "---\nid: TASK-001\ntitle: \"Fix <script>alert(1)</script>\"\nstatus: todo\npriority: 1\nowner: Jane Doe\ndue_date: {late}\ntags: [ops]\n---\n\nSee TASK-002 and <img src=x onerror=alert(1)>.\n\n[bad](javascript:alert(1))\n"
        ),
    );
    write(
        tmp.path(),
        "tasks/todo/TASK-002-plan.md",
        "---\nid: TASK-002\ntitle: Plan\nstatus: in-progress\npriority: 3\n---\n",
    );
    tmp
}

fn router_for(tmp: &TempDir, base: &str) -> Router {
    let cfg = config::load_config(tmp.path(), RepoMode::Standalone).unwrap();
    build_router(&cfg, base)
}

async fn get(router: &Router, uri: &str) -> Response {
    router
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn text(resp: Response) -> String {
    let bytes = to_bytes(resp.into_body(), 1 << 24).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn every_page_renders_in_the_shell() {
    let tmp = repo();
    let router = router_for(&tmp, "");
    for uri in [
        "/",
        "/tasks",
        "/tasks/list",
        "/tasks/list?sort=due_date&dir=desc",
        "/customers",
        "/projects",
        "/meetings",
        "/meetings/calendar",
        "/meetings/calendar?month=2026-02&overlays=1",
        "/meetings/calendar?month=nope",
        "/research",
        "/sprints",
        "/proposals",
        "/contacts",
        "/entity/TASK-001",
        "/search",
        "/search?q=fix",
    ] {
        let resp = get(&router, uri).await;
        assert_eq!(resp.status(), StatusCode::OK, "{uri}");
        let html = text(resp).await;
        assert!(html.starts_with("<!DOCTYPE html>"), "{uri}");
        assert!(html.contains(r#"class="toast-region""#), "{uri}");
        assert!(html.contains(r#"<dialog class="palette""#), "{uri}");
        assert!(html.contains(r#"class="theme-switch""#), "{uri}");
        assert!(html.contains("@layer mc-tokens"), "{uri}");
        // Names and titles are always escaped.
        assert!(!html.contains("<script>alert"), "{uri}");
    }
}

#[tokio::test]
async fn board_exposes_interaction_hooks() {
    let tmp = repo();
    let html = text(get(&router_for(&tmp, ""), "/tasks").await).await;
    assert!(html.contains(r#"data-id="TASK-001" data-status="todo""#));
    assert!(html.contains(r#"data-status="in-progress""#));
    assert!(html.contains("kanban-card pri-critical is-overdue"));
    assert!(html.contains(r#"<span class="kanban-late">1 late</span>"#));
    assert!(html.contains(r#"id="board""#));
}

#[tokio::test]
async fn task_list_honours_sort() {
    let tmp = repo();
    let router = router_for(&tmp, "");
    let html = text(get(&router, "/tasks/list?sort=id&dir=desc").await).await;
    let a = html.find(r#"data-id="TASK-002""#).unwrap();
    let b = html.find(r#"data-id="TASK-001""#).unwrap();
    assert!(a < b, "descending ID order");
    assert!(html.contains(r#"aria-sort="descending""#));
}

#[tokio::test]
async fn detail_page_sanitises_markdown() {
    let tmp = repo();
    let html = text(get(&router_for(&tmp, ""), "/entity/TASK-001").await).await;
    assert!(html.contains(r#"<dl class="title-block">"#));
    assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;"));
    assert!(!html.contains("javascript:alert"));
    assert!(html.contains(r#"class="entity-link""#));
}

#[tokio::test]
async fn unknown_paths_are_404_pages() {
    let tmp = repo();
    let router = router_for(&tmp, "");
    for uri in ["/entity/TASK-999", "/nope"] {
        let resp = get(&router, uri).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert!(text(resp).await.contains("Not found"));
    }
}

#[tokio::test]
async fn font_and_index_are_served() {
    let tmp = repo();
    let router = router_for(&tmp, "");
    let resp = get(&router, "/assets/archivo.woff2").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()[header::CONTENT_TYPE], "font/woff2");
    assert!(resp.headers()[header::CACHE_CONTROL]
        .to_str()
        .unwrap()
        .contains("immutable"));
    let bytes = to_bytes(resp.into_body(), 1 << 24).await.unwrap();
    assert_eq!(&bytes[..4], b"wOF2");

    let resp = get(&router, "/index.json").await;
    assert_eq!(resp.headers()[header::CONTENT_TYPE], "application/json");
    let v: serde_json::Value = serde_json::from_str(&text(resp).await).unwrap();
    let ids: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"TASK-001") && ids.contains(&"TASK-002"));
}

#[tokio::test]
async fn base_path_prefixes_every_url() {
    let tmp = repo();
    let router = router_for(&tmp, "/hq/");
    for uri in [
        "/hq",
        "/hq/tasks",
        "/hq/tasks/list",
        "/hq/entity/TASK-001",
        "/hq/meetings",
        "/hq/meetings/calendar?month=2026-03&overlays=1",
    ] {
        let resp = get(&router, uri).await;
        assert_eq!(resp.status(), StatusCode::OK, "{uri}");
        let html = text(resp).await;
        for attr in ["href=\"/", "src=\"/", "action=\"/", "url(\"/"] {
            for (i, _) in html.match_indices(attr) {
                assert!(
                    html[i + attr.len()..].starts_with("hq/"),
                    "{uri}: unprefixed {attr}…"
                );
            }
        }
    }
    let resp = get(&router, "/hq/").await;
    assert_eq!(resp.status(), StatusCode::PERMANENT_REDIRECT);
    let resp = get(&router, "/hq/assets/archivo.woff2").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn meeting_calendar_places_meetings_by_day() {
    let tmp = repo();
    write(
        tmp.path(),
        "meetings/2026-03-12-kickoff.md",
        "---\nid: MTG-001\ntitle: Kickoff\ndate: 2026-03-12\ntime: \"9:30\"\nstatus: scheduled\n---\n",
    );
    write(
        tmp.path(),
        "meetings/2026-03-12-review.md",
        "---\nid: MTG-002\ntitle: Review\ndate: 2026-03-12\ntime: '14:00'\nstatus: completed\n---\n",
    );
    write(
        tmp.path(),
        "meetings/offsite.md",
        "---\nid: MTG-003\ntitle: Offsite\ndate: someday\n---\n",
    );
    let router = router_for(&tmp, "");
    let html = text(get(&router, "/meetings/calendar?month=2026-03").await).await;
    assert!(html.contains(r#"<h2 class="cal-month" id="cal-month">March 2026</h2>"#));
    // March 2026 starts on a Sunday, so the grid opens on Monday 23 February.
    let first_day = html.find(r#"class="cal-day"#).unwrap();
    assert!(html[first_day..].contains(r#"datetime="2026-02-23""#));
    assert!(
        html[first_day..].find(r#"datetime="2026-02-23""#)
            < html[first_day..].find(r#"datetime="2026-03-01""#)
    );
    let cell = html.split(r#"datetime="2026-03-12""#).nth(1).unwrap();
    let cell = &cell[..cell.find("</td>").unwrap()];
    assert!(cell.find("/entity/MTG-001").unwrap() < cell.find("/entity/MTG-002").unwrap());
    assert!(cell.contains(r#"<time class="cal-time">09:30</time>"#));
    // The phone agenda and the undated panel.
    assert!(html.contains(r#"class="cal-agenda""#));
    assert!(html.contains("Undated (1)"));
    assert!(html.contains("someday"));
    // Both views link to each other.
    assert!(html.contains(r#"<a href="/meetings/calendar" class="active" aria-current="page">"#));
    let list = text(get(&router, "/meetings").await).await;
    assert!(list.contains(r#"<a href="/meetings/calendar">"#));

    let bad = get(&router, "/meetings/calendar?month=2026-13").await;
    assert_eq!(bad.status(), StatusCode::OK);
    assert!(text(bad)
        .await
        .contains("Couldn’t read the month “2026-13”"));
}

#[tokio::test]
async fn meeting_calendar_keeps_the_base_path() {
    let tmp = repo();
    write(
        tmp.path(),
        "meetings/2026-03-12-kickoff.md",
        "---\nid: MTG-001\ntitle: Kickoff\ndate: 2026-03-12\n---\n",
    );
    let router = router_for(&tmp, "/hq");
    let html = text(get(&router, "/hq/meetings/calendar?month=2026-03&overlays=1").await).await;
    assert!(
        html.contains(r#"href="/hq/meetings/calendar?month=2026-02&amp;overlays=1" rel="prev""#)
    );
    assert!(
        html.contains(r#"href="/hq/meetings/calendar?month=2026-04&amp;overlays=1" rel="next""#)
    );
    assert!(html.contains(r#"href="/hq/meetings/calendar?month=2026-03" title="Hide sprints"#));
    assert!(html.contains(r#"href="/hq/meetings" data-view-key="l""#));
}
