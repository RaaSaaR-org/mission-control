//! Integration tests for the dashboard's interactive layer: the JSON API under
//! `/api/`, write protection and the edit UI. The router runs in-process on a
//! temporary repo, so no socket is bound and no real data is touched.

use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use mc::commands::init;
use mc::commands::serve::{router, ServeOptions};
use mc::config::{self, RepoMode};
use serde_json::{json, Value};
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
    write(
        tmp.path(),
        "tasks/todo/TASK-001-fix.md",
        "---\nid: TASK-001\ntitle: Fix the robot\nstatus: todo\npriority: 2\nowner: Jane Doe\ntags: [ops, vla]\n---\n\n# Fix the robot\n\nNotes stay put.\n",
    );
    write(
        tmp.path(),
        "tasks/todo/TASK-002-plan.md",
        "---\nid: TASK-002\ntitle: Plan\nstatus: in-progress\npriority: 3\n---\n",
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

fn local(tmp: &TempDir) -> Router {
    app(tmp, ServeOptions::default())
}

async fn send(router: &Router, req: Request<Body>) -> Response {
    router.clone().oneshot(req).await.unwrap()
}

async fn get(router: &Router, uri: &str) -> Response {
    send(router, Request::get(uri).body(Body::empty()).unwrap()).await
}

/// A write as the dashboard sends it: same-origin, with the request header.
fn write_req(method: Method, uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "localhost:5000")
        .header(header::ORIGIN, "http://localhost:5000")
        .header("x-mc-request", "1")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn body_text(resp: Response) -> String {
    let bytes = to_bytes(resp.into_body(), 1 << 24).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn body_json(resp: Response) -> Value {
    serde_json::from_str(&body_text(resp).await).unwrap()
}

fn read(tmp: &TempDir, rel: &str) -> String {
    std::fs::read_to_string(tmp.path().join(rel)).unwrap()
}

#[tokio::test]
async fn pages_render_and_unknown_paths_404() {
    let tmp = repo();
    let r = local(&tmp);
    for uri in [
        "/",
        "/tasks",
        "/tasks/list",
        "/sprints",
        "/entity/TASK-001",
        "/search?q=robot",
    ] {
        assert_eq!(get(&r, uri).await.status(), StatusCode::OK, "{uri}");
    }
    for uri in ["/entity/TASK-404", "/nope", "/api/nope"] {
        assert_eq!(get(&r, uri).await.status(), StatusCode::NOT_FOUND, "{uri}");
    }
    let resp = get(&r, "/api/nope").await;
    assert_eq!(body_json(resp).await["error"], "not_found");
}

#[tokio::test]
async fn edit_ui_appears_only_when_editable() {
    let tmp = repo();
    let board = body_text(get(&local(&tmp), "/tasks").await).await;
    assert!(board.contains(r#"<html lang="en" data-edit>"#));
    assert!(board.contains(r#"class="card-move""#));
    assert!(board.contains(r#"id="move-menu-tpl""#));
    assert!(board.contains(r#"<dialog class="sheet new-task""#));
    let detail = body_text(get(&local(&tmp), "/entity/TASK-001").await).await;
    assert!(detail.contains(r#"data-edit-task="TASK-001""#));
    assert!(detail.contains(r#"<option value="SPR-001">Alpha</option>"#));

    let ro = app(
        &tmp,
        ServeOptions {
            read_only: true,
            ..Default::default()
        },
    );
    for uri in ["/tasks", "/entity/TASK-001"] {
        let html = body_text(get(&ro, uri).await).await;
        assert!(html.contains(r#"<html lang="en">"#), "{uri}");
        assert!(!html.contains(r#"class="card-move""#), "{uri}");
        assert!(!html.contains(r#"<dialog class="sheet new-task""#), "{uri}");
        assert!(!html.contains(r#"<form class="edit-panel""#), "{uri}");
        assert!(html.contains("read-only"), "{uri}");
    }
}

#[tokio::test]
async fn move_updates_the_file_and_returns_fragments() {
    let tmp = repo();
    let r = local(&tmp);
    let v0 = body_json(get(&r, "/api/version").await).await["version"].clone();

    let resp = send(
        &r,
        write_req(
            Method::POST,
            "/api/tasks/TASK-001/move",
            json!({"status": "done"}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()[header::CACHE_CONTROL], "no-store");
    let v = body_json(resp).await;
    assert_eq!(v["old_status"], "todo");
    assert_eq!(v["task"]["status"], "done");
    assert_eq!(v["task"]["path"], "tasks/done/TASK-001-fix.md");
    // A done task renders as a compact card for the done lane.
    let card = v["html"]["card"].as_str().unwrap();
    assert!(card.contains(r#"data-id="TASK-001" data-status="done""#));
    assert!(card.contains("is-compact"));
    assert!(v["html"]["title_block"]
        .as_str()
        .unwrap()
        .contains("title-block"));
    assert_ne!(v["version"], v0, "a write changes the repo version");
    assert_eq!(
        body_json(get(&r, "/api/version").await).await["version"],
        v["version"]
    );
    assert!(read(&tmp, "tasks/done/TASK-001-fix.md").contains("status: done"));
    // Moving back and forth leaves the body as it was.
    for status in ["todo", "done", "todo"] {
        let resp = send(
            &r,
            write_req(
                Method::POST,
                "/api/tasks/TASK-001/move",
                json!({"status": status}),
            ),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
    let settled = read(&tmp, "tasks/todo/TASK-001-fix.md");
    assert!(settled.contains("---\n\n# Fix the robot\n"));
    // Further round trips are byte-stable.
    for status in ["done", "todo"] {
        let resp = send(
            &r,
            write_req(
                Method::POST,
                "/api/tasks/TASK-001/move",
                json!({"status": status}),
            ),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
    assert_eq!(read(&tmp, "tasks/todo/TASK-001-fix.md"), settled);
}

#[tokio::test]
async fn patch_changes_fields_and_keeps_the_body() {
    let tmp = repo();
    let r = local(&tmp);
    let patch = json!({"priority": 1, "owner": "Max Muster", "sprint": "SPR-001", "due_date": "2026-11-30"});
    for _ in 0..3 {
        let resp = send(
            &r,
            write_req(Method::PATCH, "/api/tasks/TASK-001", patch.clone()),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }
    let file = read(&tmp, "tasks/todo/TASK-001-fix.md");
    assert!(file.contains("priority: 1"));
    assert!(file.contains("owner: Max Muster"));
    assert!(file.contains(r#"sprint: "[[SPR-001]]""#));
    assert!(file.contains("due_date: 2026-11-30"));
    assert!(file.contains("tags:\n- ops\n- vla"));
    // Repeated edits don't add blank lines between frontmatter and body.
    assert!(file.contains("---\n\n# Fix the robot\n\nNotes stay put."));

    // Clearing fields and changing status in one request.
    let resp = send(
        &r,
        write_req(
            Method::PATCH,
            "/api/tasks/TASK-001",
            json!({"owner": "", "sprint": "", "due_date": "", "status": "review"}),
        ),
    )
    .await;
    let v = body_json(resp).await;
    assert_eq!(v["task"]["status"], "review");
    assert_eq!(v["task"]["owner"], "");
    assert_eq!(v["task"]["sprint"], "");
    assert!(v["html"]["title_block"]
        .as_str()
        .unwrap()
        .contains("No owner"));
}

#[tokio::test]
async fn create_task_uses_the_new_task_logic() {
    let tmp = repo();
    let r = local(&tmp);
    let resp = send(
        &r,
        write_req(
            Method::POST,
            "/api/tasks",
            json!({"title": "Ship the dashboard", "status": "todo", "priority": 2, "owner": "Jane Doe", "sprint": "SPR-001", "due_date": "2026-10-20"}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let v = body_json(resp).await;
    assert_eq!(v["task"]["id"], "TASK-003");
    assert_eq!(v["href"], "/entity/TASK-003");
    assert_eq!(v["task"]["sprint"], "SPR-001");
    assert!(v["html"]["card"]
        .as_str()
        .unwrap()
        .contains("Ship the dashboard"));
    let file = read(&tmp, "tasks/todo/TASK-003-ship-the-dashboard.md");
    assert!(file.contains("status: todo"));
    assert!(file.contains("priority: 2"));
    assert!(file.contains("due_date: 2026-10-20"));

    // A task created as done lands where `mc task move` would put it.
    let resp = send(
        &r,
        write_req(
            Method::POST,
            "/api/tasks",
            json!({"title": "Already shipped", "status": "done"}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let v = body_json(resp).await;
    assert_eq!(v["task"]["path"], "tasks/done/TASK-004-already-shipped.md");
}

#[tokio::test]
async fn invalid_input_is_rejected_with_the_field() {
    let tmp = repo();
    let r = local(&tmp);
    let cases = [
        (
            Method::POST,
            "/api/tasks/TASK-001/move",
            json!({"status": "nope"}),
            "status",
        ),
        (
            Method::PATCH,
            "/api/tasks/TASK-001",
            json!({"priority": 9}),
            "priority",
        ),
        (
            Method::PATCH,
            "/api/tasks/TASK-001",
            json!({"due_date": "31.10.2026"}),
            "due_date",
        ),
        (
            Method::PATCH,
            "/api/tasks/TASK-001",
            json!({"due_date": "2026-1-5"}),
            "due_date",
        ),
        (
            Method::PATCH,
            "/api/tasks/TASK-001",
            json!({"sprint": "SPR-999"}),
            "sprint",
        ),
        (
            Method::PATCH,
            "/api/tasks/TASK-001",
            json!({"owner": "a\nb"}),
            "owner",
        ),
        (Method::POST, "/api/tasks", json!({"title": "  "}), "title"),
        (
            Method::PATCH,
            "/api/tasks/TASK-001",
            json!({"title": " "}),
            "title",
        ),
        (
            Method::PATCH,
            "/api/tasks/TASK-001",
            json!({"project": "PROJ-404"}),
            "project",
        ),
        (
            Method::POST,
            "/api/tasks",
            json!({"title": "x", "project": "PROJ-404"}),
            "project",
        ),
    ];
    for (method, uri, body, field) in cases {
        let resp = send(&r, write_req(method, uri, body.clone())).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{body}");
        let v = body_json(resp).await;
        assert_eq!(v["field"], field, "{body}");
        assert!(v["message"].as_str().is_some_and(|m| !m.is_empty()));
    }
    // Unknown fields, bad JSON, path traversal and missing tasks.
    let resp = send(
        &r,
        write_req(Method::PATCH, "/api/tasks/TASK-001", json!({"tags": ["x"]})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let resp = send(
        &r,
        write_req(Method::PATCH, "/api/tasks/TASK-001", json!({})),
    )
    .await;
    assert_eq!(body_json(resp).await["error"], "empty");
    for uri in [
        "/api/tasks/..%2F..%2Fconfig/move",
        "/api/tasks/PROJ-001/move",
        "/api/tasks/TASK-1a/move",
    ] {
        let resp = send(&r, write_req(Method::POST, uri, json!({"status": "done"}))).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{uri}");
    }
    let resp = send(
        &r,
        write_req(
            Method::POST,
            "/api/tasks/TASK-404/move",
            json!({"status": "done"}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    // Nothing was written.
    assert!(read(&tmp, "tasks/todo/TASK-001-fix.md").contains("status: todo\npriority: 2"));
}

#[tokio::test]
async fn writes_need_the_header_and_same_origin() {
    let tmp = repo();
    let r = local(&tmp);
    let move_req = |f: &dyn Fn(axum::http::request::Builder) -> axum::http::request::Builder| {
        f(Request::post("/api/tasks/TASK-001/move")
            .header(header::CONTENT_TYPE, "application/json"))
        .body(Body::from(r#"{"status":"done"}"#))
        .unwrap()
    };
    let cases: [(&str, &dyn Fn(_) -> _); 5] = [
        ("missing_header", &|b: axum::http::request::Builder| {
            b.header(header::HOST, "localhost:5000")
        }),
        ("cross_origin", &|b: axum::http::request::Builder| {
            b.header("x-mc-request", "1")
                .header(header::HOST, "localhost:5000")
                .header(header::ORIGIN, "http://evil.example")
        }),
        ("cross_origin", &|b: axum::http::request::Builder| {
            b.header("x-mc-request", "1")
                .header(header::HOST, "localhost:5000")
                .header(header::ORIGIN, "null")
        }),
        ("cross_origin", &|b: axum::http::request::Builder| {
            b.header("x-mc-request", "1")
                .header("sec-fetch-site", "cross-site")
        }),
        // DNS rebinding: the page and the Host header agree, but aren't local.
        ("bad_host", &|b: axum::http::request::Builder| {
            b.header("x-mc-request", "1")
                .header(header::HOST, "evil.example:5000")
                .header(header::ORIGIN, "http://evil.example:5000")
        }),
    ];
    for (code, build) in cases {
        let resp = send(&r, move_req(build)).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{code}");
        assert_eq!(body_json(resp).await["error"], code);
    }
    assert!(read(&tmp, "tasks/todo/TASK-001-fix.md").contains("status: todo"));
    // Reads need neither.
    assert_eq!(get(&r, "/api/palette").await.status(), StatusCode::OK);
}

#[tokio::test]
async fn read_only_modes_refuse_writes() {
    let tmp = repo();
    let read_only = ServeOptions {
        read_only: true,
        ..Default::default()
    };
    let proxied = ServeOptions {
        base_path: "/hq".into(),
        ..Default::default()
    };
    for (opts, uri) in [
        (read_only, "/api/tasks/TASK-001/move"),
        (proxied, "/hq/api/tasks/TASK-001/move"),
    ] {
        let r = app(&tmp, opts);
        let resp = send(&r, write_req(Method::POST, uri, json!({"status": "done"}))).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{uri}");
        assert_eq!(body_json(resp).await["error"], "read_only");
    }
    assert!(read(&tmp, "tasks/todo/TASK-001-fix.md").contains("status: todo"));

    // --allow-edits turns editing back on behind a proxy, checked against
    // the forwarded host.
    let r = app(
        &tmp,
        ServeOptions {
            base_path: "/hq".into(),
            allow_edits: true,
            ..Default::default()
        },
    );
    let req = Request::post("/hq/api/tasks/TASK-001/move")
        .header(header::HOST, "127.0.0.1:5000")
        .header("x-forwarded-host", "hq.example.com")
        .header(header::ORIGIN, "https://hq.example.com")
        .header("x-mc-request", "1")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"status":"review"}"#))
        .unwrap();
    let resp = send(&r, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let card = body_json(resp).await["html"]["card"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(card.contains(r#"href="/hq/entity/TASK-001""#));
}

#[tokio::test]
async fn palette_lists_pages_and_entities() {
    let tmp = repo();
    let resp = get(&local(&tmp), "/api/palette").await;
    assert_eq!(resp.headers()[header::CONTENT_TYPE], "application/json");
    let v = body_json(resp).await;
    assert_eq!(v["editable"], true);
    let pages: Vec<&str> = v["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["href"].as_str().unwrap())
        .collect();
    assert!(pages.contains(&"/") && pages.contains(&"/tasks/list") && pages.contains(&"/sprints"));
    assert!(pages.contains(&"/meetings/calendar"));
    let task = v["entities"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "TASK-001")
        .unwrap();
    assert_eq!(task["t"], "Fix the robot");
    assert_eq!(task["tone"], "pending");
    assert_eq!(task["tags"], json!(["ops", "vla"]));

    // Behind a proxy the palette is served under the base path and read-only.
    let proxied = app(
        &tmp,
        ServeOptions {
            base_path: "/hq/".into(),
            ..Default::default()
        },
    );
    let v = body_json(get(&proxied, "/hq/api/palette").await).await;
    assert_eq!(v["editable"], false);
    assert_eq!(
        get(&proxied, "/hq/api/version").await.status(),
        StatusCode::OK
    );
    let html = body_text(get(&proxied, "/hq/tasks").await).await;
    assert!(html.contains(r#"action="/hq/search""#));
    assert!(html.contains(r#"<html lang="en">"#));
}

#[tokio::test]
async fn version_tracks_outside_edits() {
    let tmp = repo();
    let r = local(&tmp);
    let version = |r: Router| async move {
        body_json(get(&r, "/api/version").await).await["version"].clone()
    };
    let a = version(r.clone()).await;
    assert_eq!(a, version(r.clone()).await, "stable while nothing changes");
    write(
        tmp.path(),
        "tasks/todo/TASK-009-new.md",
        "---\nid: TASK-009\ntitle: From the CLI\nstatus: todo\n---\n",
    );
    assert_ne!(a, version(r.clone()).await);
}

#[tokio::test]
async fn patch_renames_and_relinks_a_task() {
    let tmp = repo();
    write(
        tmp.path(),
        "projects/PROJ-001-apollo/PROJ-001.md",
        "---\nid: PROJ-001\nname: Apollo\nstatus: active\n---\n",
    );
    let r = local(&tmp);
    let detail = body_text(get(&r, "/entity/TASK-001").await).await;
    let form = edit_form(&detail);
    assert!(form.contains(r#"name="title" value="Fix the robot" required"#));
    assert!(form.contains(r#"<select name="project">"#));

    let resp = send(
        &r,
        write_req(
            Method::PATCH,
            "/api/tasks/TASK-001",
            json!({"title": "Fix the arm", "project": "PROJ-001"}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["task"]["title"], "Fix the arm");
    let file = read(&tmp, "tasks/todo/TASK-001-fix.md");
    assert!(file.contains("title: Fix the arm"), "{file}");
    assert!(
        file.contains("\n# Fix the arm\n\nNotes stay put.\n"),
        "{file}"
    );
    assert!(!file.contains("Fix the robot"));
    assert!(
        file.contains("- '[[PROJ-001]]'") || file.contains("- \"[[PROJ-001]]\""),
        "{file}"
    );

    // Clearing the project; a task linking several projects isn't rewritten.
    let resp = send(
        &r,
        write_req(Method::PATCH, "/api/tasks/TASK-001", json!({"project": ""})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(read(&tmp, "tasks/todo/TASK-001-fix.md").contains("projects: []"));
    write(
        tmp.path(),
        "tasks/todo/TASK-003-two.md",
        "---\nid: TASK-003\ntitle: Two\nstatus: todo\nprojects: ['[[PROJ-001]]', '[[PROJ-002]]']\n---\n",
    );
    let resp = send(
        &r,
        write_req(Method::PATCH, "/api/tasks/TASK-003", json!({"project": ""})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["field"], "project");
    let detail = body_text(get(&r, "/entity/TASK-003").await).await;
    assert!(!edit_form(&detail).contains(r#"<select name="project">"#));
}

/// The task edit form in a detail page.
fn edit_form(html: &str) -> &str {
    let start = html.find(r#"<form class="edit-panel""#).expect("edit form");
    let end = start + html[start..].find("</form>").unwrap();
    &html[start..end]
}

#[tokio::test]
async fn writes_report_the_version_they_started_from() {
    let tmp = repo();
    let r = local(&tmp);
    let before = body_json(get(&r, "/api/version").await).await["version"].clone();
    // Something else edits a file just before the page writes.
    write(
        tmp.path(),
        "tasks/todo/TASK-002-plan.md",
        "---\nid: TASK-002\ntitle: Plan B\nstatus: in-progress\npriority: 3\n---\n",
    );
    let outside = body_json(get(&r, "/api/version").await).await["version"].clone();
    assert_ne!(before, outside);
    let resp = send(
        &r,
        write_req(
            Method::POST,
            "/api/tasks/TASK-001/move",
            json!({"status": "review"}),
        ),
    )
    .await;
    let v = body_json(resp).await;
    // The page compares prev_version with the version it knew, so the
    // outside edit isn't mistaken for part of its own write.
    assert_eq!(v["prev_version"], outside);
    assert_ne!(v["version"], outside);
    for (method, uri, body) in [
        (Method::PATCH, "/api/tasks/TASK-001", json!({"priority": 1})),
        (Method::POST, "/api/tasks", json!({"title": "New"})),
        (
            Method::POST,
            "/api/entities/TASK-001/comments",
            json!({"text": "Hi"}),
        ),
    ] {
        let v = body_json(send(&r, write_req(method, uri, body)).await).await;
        assert!(v["prev_version"].is_string(), "{uri}");
        assert_ne!(v["prev_version"], v["version"], "{uri}");
    }
}
