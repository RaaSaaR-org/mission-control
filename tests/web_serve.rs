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
        assert!(
            html.contains(r#"<link rel="stylesheet" href="/assets/app.css?v="#),
            "{uri}"
        );
        assert!(
            !html.contains("@layer mc-tokens"),
            "{uri}: CSS is linked, not inlined"
        );
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
async fn css_and_js_are_separate_cacheable_assets() {
    let tmp = repo();
    let router = router_for(&tmp, "");
    let html = text(get(&router, "/tasks").await).await;
    let version = |name: &str| {
        let start = html.find(&format!("/assets/{name}?v=")).expect(name) + name.len() + 11;
        html[start..start + 12].to_string()
    };
    assert!(version("app.css").bytes().all(|b| b.is_ascii_hexdigit()));
    assert_ne!(version("app.css"), version("app.js"));
    assert!(html.contains(r#"<script src="/assets/app.js?v="#));
    assert!(html.contains(" defer></script>"));
    // Only the tiny theme boot script stays inline.
    assert!(html.len() < 60_000, "page is {} bytes", html.len());

    for (uri, mime, needle) in [
        (
            "/assets/app.css",
            "text/css; charset=utf-8",
            "@layer mc-tokens",
        ),
        (
            "/assets/app.js",
            "text/javascript; charset=utf-8",
            "softRefresh",
        ),
    ] {
        let resp = get(&router, &format!("{uri}?v=abc")).await;
        assert_eq!(resp.status(), StatusCode::OK, "{uri}");
        assert_eq!(resp.headers()[header::CONTENT_TYPE], mime);
        assert!(resp.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("immutable"));
        assert!(text(resp).await.contains(needle), "{uri}");
    }
    // The font is found next to the stylesheet under any base path.
    let css = text(get(&router, "/assets/app.css").await).await;
    assert!(css.contains(r#"url("archivo.woff2")"#));
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
    let html = text(get(&router, "/hq/tasks").await).await;
    assert!(html.contains(r#"href="/hq/assets/app.css?v="#));
    assert!(html.contains(r#"src="/hq/assets/app.js?v="#));
    for uri in ["/hq/assets/app.css", "/hq/assets/app.js"] {
        assert_eq!(get(&router, uri).await.status(), StatusCode::OK, "{uri}");
    }
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

#[tokio::test]
async fn loose_entity_ids_redirect_and_misses_offer_search() {
    let tmp = repo();
    for (base, uri, to) in [
        ("", "/entity/task-1", "/entity/TASK-001"),
        ("", "/entity/TASK-0001", "/entity/TASK-001"),
        ("/hq", "/hq/entity/task1", "/hq/entity/TASK-001"),
    ] {
        let resp = get(&router_for(&tmp, base), uri).await;
        assert_eq!(resp.status(), StatusCode::PERMANENT_REDIRECT, "{uri}");
        assert_eq!(resp.headers()[header::LOCATION], to, "{uri}");
    }
    let resp = get(&router_for(&tmp, ""), "/entity/task-99").await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert!(text(resp).await.contains(r#"href="/search?q=task-99""#));
}

#[tokio::test]
async fn disabled_kinds_say_how_to_enable_them() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "config/config.yml",
        "site:\n  name: Small\npaths:\n  tasks: tasks/\n  meetings: meetings/\n",
    );
    let cfg = config::load_config(tmp.path(), RepoMode::Standalone).unwrap();
    assert!(!cfg.entity_available(&mc::entity::EntityKind::Proposal));
    let router = build_router(&cfg, "");
    let resp = get(&router, "/proposals").await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let html = text(resp).await;
    assert!(html.contains("Not available"), "{html}");
    assert!(html.contains("<code>proposals: proposals/</code>"));
}

/// The browser-side fixes live in the shipped script and stylesheet; these
/// checks keep them from being dropped silently.
#[tokio::test]
async fn shipped_script_and_styles_keep_their_fixes() {
    let tmp = repo();
    let router = router_for(&tmp, "");
    let js = text(get(&router, "/assets/app.js").await).await;
    // Tab passes through the sidebar search; the palette returns focus.
    assert!(!js.contains(r#"siteInput.addEventListener("focus""#));
    assert!(js.contains("palOpener"));
    // Own writes don't swallow an outside change made just before them.
    assert!(js.contains("data.prev_version !== known"));
    assert!(js.contains(r#"hideNotice("offline")"#));
    // Undo acts on the live card after a refresh; refresh keeps the place.
    assert!(js.contains("live !== current"));
    assert!(js.contains("scrollLeft = scrolls[i]"));
    // "Create another" refreshes when the dialog closes; Back resets filters.
    assert!(js.contains("refreshOnClose"));
    assert!(js.contains(r#""pageshow""#));
    // Loose IDs, date ties and month names in the palette; Mac Ctrl+K.
    assert!(js.contains("byScoreThenDate"));
    assert!(js.contains(r#""Sep""#) && !js.contains("toLocaleDateString"));
    assert!(js.contains("isMac && typing(e.target)"));

    let css = text(get(&router, "/assets/app.css").await).await;
    assert!(css.contains(
        ".nav-toggle { position: absolute; opacity: 0; pointer-events: none; visibility: hidden; }"
    ));
    assert!(css.contains(".filter-cell:has(select:focus-visible)"));
    assert!(css.contains("width: fit-content;"));
    assert!(css.contains(".fp-scroll.at-end"));
    assert!(css.contains(".tb-cell:last-child { border-right: 0; }"));
}
