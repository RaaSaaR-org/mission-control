//! Integration tests for checklists and comments on the dashboard: rendering,
//! the `/api/entities/{id}/checks` and `/comments` endpoints, and their write
//! protection. The router runs in-process on a temporary repo.

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

const TASK: &str = "tasks/todo/TASK-001-boxes.md";
const TASK_DOC: &str = "---\nid: TASK-001\ntitle: Boxes\nstatus: todo\n---\n\n# Boxes\n\n- [ ] Write spec\n- [x] Book room\n\n```\n- [ ] decoy\n```\n";

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn repo() -> TempDir {
    let tmp = TempDir::new().unwrap();
    init::run(tmp.path(), false, false, Some("NotesTest"), false, true).unwrap();
    write(tmp.path(), TASK, TASK_DOC);
    write(
        tmp.path(),
        "sprints/SPR-001-alpha.md",
        "---\nid: SPR-001\ntitle: Alpha\nstatus: active\n---\n\n- [ ] Plan\n",
    );
    tmp
}

fn app(tmp: &TempDir, opts: ServeOptions) -> Router {
    let cfg = config::load_config(tmp.path(), RepoMode::Standalone).unwrap();
    router(&cfg, &opts)
}

async fn send(router: &Router, req: Request<Body>) -> Response {
    router.clone().oneshot(req).await.unwrap()
}

async fn page(router: &Router, uri: &str) -> String {
    let resp = send(router, Request::get(uri).body(Body::empty()).unwrap()).await;
    assert_eq!(resp.status(), StatusCode::OK, "{uri}");
    body_text(resp).await
}

/// A write as the dashboard sends it: same-origin, with the request header.
fn write_req(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
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
async fn checkboxes_and_composer_render_only_when_editable() {
    let tmp = repo();
    let html = page(&app(&tmp, ServeOptions::default()), "/entity/TASK-001").await;
    assert!(html.contains(r#"class="task-check" disabled data-line="9" data-text="Write spec">"#));
    assert!(html.contains(r#"data-line="10" data-text="Book room" checked>"#));
    assert!(html.contains("<span data-check-done>1</span> of <span data-check-total>2</span>"));
    assert!(html.contains("data-comment-form hidden"));
    // No comments yet: the section waits for the script to reveal it.
    assert!(html.contains(r#"data-comments hidden"#));

    let read_only = ServeOptions {
        read_only: true,
        ..Default::default()
    };
    let html = page(&app(&tmp, read_only), "/entity/TASK-001").await;
    assert!(!html.contains("data-line="));
    assert!(html.contains(r#"<input disabled="" type="checkbox" checked=""/>"#));
    assert!(html.contains("data-check-progress"));
    assert!(!html.contains(r#"<form class="comment-composer""#));
    assert!(!html.contains(r#"id="comments""#));

    // Sprints get checklists but no comments.
    let html = page(&app(&tmp, ServeOptions::default()), "/entity/SPR-001").await;
    assert!(html.contains(r#"data-text="Plan""#));
    assert!(!html.contains(r#"<form class="comment-composer""#));
}

#[tokio::test]
async fn ticking_writes_one_byte_and_stale_requests_conflict() {
    let tmp = repo();
    let r = app(&tmp, ServeOptions::default());
    let resp = send(
        &r,
        write_req(
            "/api/entities/TASK-001/checks",
            json!({"line": 9, "checked": true, "text": "Write spec"}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["item"]["checked"], true);
    assert_eq!(
        (v["done"].as_u64(), v["total"].as_u64()),
        (Some(2), Some(2))
    );
    assert!(v["version"].is_string());
    let ticked = TASK_DOC.replace("- [ ] Write spec", "- [x] Write spec");
    assert_eq!(read(&tmp, TASK), ticked);

    // The same request again expects an unticked box: conflict, no change.
    // So do a changed text, and a line that isn't an item (the decoy).
    for body in [
        json!({"line": 9, "checked": true, "text": "Write spec"}),
        json!({"line": 10, "checked": false, "text": "Book a room"}),
        json!({"line": 13, "checked": true, "text": "decoy"}),
    ] {
        let resp = send(&r, write_req("/api/entities/TASK-001/checks", body.clone())).await;
        assert_eq!(resp.status(), StatusCode::CONFLICT, "{body}");
        assert_eq!(body_json(resp).await["error"], "conflict");
    }
    assert_eq!(read(&tmp, TASK), ticked);

    // Unticking goes back to the original bytes.
    let resp = send(
        &r,
        write_req(
            "/api/entities/TASK-001/checks",
            json!({"line": 9, "checked": false, "text": "Write spec"}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(read(&tmp, TASK), TASK_DOC);

    for (uri, status) in [
        ("/api/entities/TASK-999/checks", StatusCode::NOT_FOUND),
        ("/api/entities/..%2Fetc/checks", StatusCode::BAD_REQUEST),
    ] {
        let resp = send(&r, write_req(uri, json!({"line": 9, "checked": true}))).await;
        assert_eq!(resp.status(), status, "{uri}");
    }
}

#[tokio::test]
async fn comments_append_and_render() {
    let tmp = repo();
    let r = app(&tmp, ServeOptions::default());
    let resp = send(
        &r,
        write_req(
            "/api/entities/TASK-001/comments",
            json!({"text": "Ready for **review**, see TASK-001.\n\n<script>x</script>", "author": "Jane Doe"}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let v = body_json(resp).await;
    assert_eq!(v["count"], 1);
    assert_eq!(v["comment"]["author"], "Jane Doe");
    let item = v["html"].as_str().unwrap();
    assert!(item.starts_with(r#"<li class="comment" id="comment-1">"#));
    assert!(item.contains("<strong>review</strong>"));
    assert!(item.contains("&lt;script&gt;"));

    let content = read(&tmp, TASK);
    assert!(content.starts_with(TASK_DOC), "existing text is kept");
    assert!(content.contains("\n## Comments\n\n### "));
    assert!(content.contains(" · Jane Doe\n\nReady for **review**"));

    // The page shows the comment once, in the comments section only.
    let html = page(&r, "/entity/TASK-001").await;
    assert_eq!(html.matches("Ready for <strong>review</strong>").count(), 1);
    assert!(html.contains("data-comment-count>1<"));
    assert!(!html.contains(r#"<h2>Comments</h2>"#));
    // Checklist positions still line up after the file grew.
    assert!(html.contains(r#"data-line="9" data-text="Write spec""#));

    for (uri, body) in [
        ("/api/entities/TASK-001/comments", json!({"text": "   "})),
        (
            "/api/entities/TASK-001/comments",
            json!({"text": "x", "author": "a\nb"}),
        ),
        ("/api/entities/SPR-001/comments", json!({"text": "hi"})),
    ] {
        let resp = send(&r, write_req(uri, body.clone())).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{body}");
    }
}

#[tokio::test]
async fn comments_are_self_contained_and_free_of_control_characters() {
    let tmp = repo();
    let r = app(&tmp, ServeOptions::default());
    for (text, author) in [
        ("```\nlog output", "A"),
        ("hi \u{1b}]52;c;eA==\u{7} \u{1b}[2J \u{0} there", "B"),
        ("Second, separate comment", "C"),
    ] {
        let body = json!({"text": text, "author": author});
        let resp = send(&r, write_req("/api/entities/TASK-001/comments", body)).await;
        assert_eq!(resp.status(), StatusCode::CREATED);
        assert_eq!(body_json(resp).await["comment"]["author"], author);
    }
    let content = read(&tmp, TASK);
    assert!(content.contains("```\nlog output\n```\n"), "{content}");
    assert!(content.contains("hi ]52;c;eA== [2J  there"), "{content}");
    assert!(!content.chars().any(|c| c.is_control() && c != '\n'));
    let html = page(&r, "/entity/TASK-001").await;
    assert!(html.contains("data-comment-count>3<"));
}

#[tokio::test]
async fn writes_are_guarded() {
    let tmp = repo();
    let r = app(&tmp, ServeOptions::default());
    for uri in [
        "/api/entities/TASK-001/checks",
        "/api/entities/TASK-001/comments",
    ] {
        // No X-MC-Request header: a plain cross-site form could send this.
        let req = Request::post(uri)
            .header(header::HOST, "localhost:5000")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({"line": 9, "checked": true, "text": "x"}).to_string(),
            ))
            .unwrap();
        let resp = send(&r, req).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{uri}");
        assert_eq!(body_json(resp).await["error"], "missing_header");
    }

    let read_only = app(
        &tmp,
        ServeOptions {
            read_only: true,
            ..Default::default()
        },
    );
    let resp = send(
        &read_only,
        write_req(
            "/api/entities/TASK-001/checks",
            json!({"line": 9, "checked": true}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(resp).await["error"], "read_only");
    let resp = send(
        &read_only,
        write_req("/api/entities/TASK-001/comments", json!({"text": "hi"})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(read(&tmp, TASK), TASK_DOC);
}

#[tokio::test]
async fn comment_fragments_carry_the_base_path() {
    let tmp = repo();
    let r = app(
        &tmp,
        ServeOptions {
            base_path: "/hq".into(),
            allow_edits: true,
            ..Default::default()
        },
    );
    let req = Request::post("/hq/api/entities/TASK-001/comments")
        .header(header::HOST, "localhost:5000")
        .header("x-mc-request", "1")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"text": "See TASK-001"}).to_string()))
        .unwrap();
    let resp = send(&r, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let v = body_json(resp).await;
    assert!(v["html"]
        .as_str()
        .unwrap()
        .contains(r#"href="/hq/entity/TASK-001""#));
}

#[tokio::test]
async fn long_comments_fit_and_oversized_bodies_get_a_clear_error() {
    let tmp = repo();
    let r = app(&tmp, ServeOptions::default());
    let detail = page(&r, "/entity/TASK-001").await;
    assert!(detail.contains(r#"maxlength="20000""#));
    // Under the character limit, but 38 KB of UTF-8: must fit.
    let umlauts = "ä".repeat(19_000);
    let resp = send(
        &r,
        write_req("/api/entities/TASK-001/comments", json!({"text": umlauts})),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    assert!(read(&tmp, TASK).contains(&umlauts));
    // Far over it: a clear message on the field, not a raw buffer error.
    let resp = send(
        &r,
        write_req(
            "/api/entities/TASK-001/comments",
            json!({"text": "ä".repeat(70_000)}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let v = body_json(resp).await;
    assert_eq!(v["error"], "too_large");
    assert_eq!(v["field"], "text");
    assert_eq!(v["message"], "Keep comments under 20000 characters.");
}
