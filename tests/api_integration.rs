//! Integration tests for `mc api serve`.
//!
//! These tests boot the router in-process and drive it via `tower::ServiceExt`
//! `oneshot` calls. No TCP socket is bound, so the suite is fast and
//! deterministic.

use std::sync::Arc;

use argon2::password_hash::{rand_core::OsRng, SaltString};
use argon2::{Argon2, PasswordHasher};
use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use axum::response::Response;
use axum::Router;
use mc::api::auth::TokenStore;
use mc::api::{build_router, ApiServerConfig};
use mc::commands::init;
use mc::config::{self, RepoMode};
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt;

const BEARER: &str = "test-correct-horse";

fn hash_token(secret: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(secret.as_bytes(), &salt)
        .unwrap()
        .to_string()
}

fn router(read_only: bool) -> (Router, TempDir) {
    let tmp = TempDir::new().unwrap();
    init::run(tmp.path(), false, false, Some("TestRepo"), false, true).unwrap();
    let cfg = config::load_config(tmp.path(), RepoMode::Standalone).unwrap();

    let yaml = format!(
        "tokens:\n  - name: test\n    hash: \"{}\"\n    capabilities: [read, write]\n",
        hash_token(BEARER)
    );
    let store = TokenStore::from_yaml(&yaml).unwrap();

    let router = build_router(
        cfg,
        &ApiServerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            tokens: store,
            read_only,
        },
    );
    (router, tmp)
}

async fn send(router: &Router, req: Request<Body>) -> Response {
    router.clone().oneshot(req).await.unwrap()
}

async fn body_json(resp: Response) -> Value {
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    serde_json::from_slice(&bytes).expect("response body must be JSON")
}

async fn body_text(resp: Response) -> String {
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn authed(method: Method, path: &str, body: Option<Value>) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("Authorization", format!("Bearer {BEARER}"));
    if body.is_some() {
        builder = builder.header("Content-Type", "application/json");
    }
    let body = match body {
        Some(v) => Body::from(serde_json::to_vec(&v).unwrap()),
        None => Body::empty(),
    };
    builder.body(body).unwrap()
}

// ───────────────────────── health & spec ─────────────────────────

#[tokio::test]
async fn healthz_is_ok() {
    let (r, _t) = router(false);
    let resp = send(
        &r,
        Request::builder()
            .uri("/healthz")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_text(resp).await, "ok");
}

#[tokio::test]
async fn readyz_is_ready_when_repo_exists() {
    let (r, _t) = router(false);
    let resp = send(
        &r,
        Request::builder()
            .uri("/readyz")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn openapi_spec_is_served_unauth_and_lists_paths() {
    let (r, _t) = router(false);
    let resp = send(
        &r,
        Request::builder()
            .uri("/v1/openapi.json")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    let paths = v.pointer("/paths").expect("paths in spec");
    let obj = paths.as_object().expect("paths is object");
    // Spot-check a representative subset across all tags.
    for expected in [
        "/healthz",
        "/readyz",
        "/v1/config",
        "/v1/status",
        "/v1/entities/{kind}",
        "/v1/entities/{kind}/{id}",
        "/v1/tasks",
        "/v1/tasks/{id}/move",
        "/v1/customers",
        "/v1/index",
        "/v1/validate",
    ] {
        assert!(
            obj.contains_key(expected),
            "missing OpenAPI path {expected}"
        );
    }
    // Bearer security scheme is registered.
    let schemes = v
        .pointer("/components/securitySchemes/bearer")
        .expect("bearer scheme");
    assert_eq!(schemes.pointer("/scheme").unwrap().as_str(), Some("bearer"));
}

// ───────────────────────── auth ─────────────────────────

#[tokio::test]
async fn missing_bearer_is_401_problem_json() {
    let (r, _t) = router(false);
    let resp = send(
        &r,
        Request::builder()
            .uri("/v1/config")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let ct = resp.headers().get("content-type").cloned();
    let v = body_json(resp).await;
    assert_eq!(ct.unwrap().to_str().unwrap(), "application/problem+json");
    assert_eq!(v["status"], 401);
    assert_eq!(v["type"], "https://docs.mc.dev/errors/unauthenticated");
}

#[tokio::test]
async fn invalid_bearer_is_401_problem_json() {
    let (r, _t) = router(false);
    let resp = send(
        &r,
        Request::builder()
            .uri("/v1/config")
            .header("Authorization", "Bearer wrong")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let v = body_json(resp).await;
    assert_eq!(v["detail"], "invalid bearer token");
}

#[tokio::test]
async fn read_only_rejects_writes() {
    let (r, _t) = router(true);
    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/customers",
            Some(serde_json::json!({"name": "X"})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let v = body_json(resp).await;
    assert_eq!(v["detail"], "server is read-only");
}

// ───────────────────────── reads ─────────────────────────

#[tokio::test]
async fn config_round_trip() {
    let (r, _t) = router(false);
    let resp = send(&r, authed(Method::GET, "/v1/config", None)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["mode"], "standalone");
    assert!(v["prefixes"]["customer"].is_string());
    assert!(v["statuses"]["task"].as_array().unwrap().len() >= 4);
}

#[tokio::test]
async fn list_customers_includes_created() {
    let (r, _t) = router(false);
    // Create one.
    let create = send(
        &r,
        authed(
            Method::POST,
            "/v1/customers",
            Some(serde_json::json!({"name": "Acme", "status": "active"})),
        ),
    )
    .await;
    assert_eq!(create.status(), StatusCode::CREATED);
    let body = body_json(create).await;
    assert_eq!(body["id"], "CUST-001");
    assert_eq!(body["name"], "Acme");
    // List.
    let list = send(&r, authed(Method::GET, "/v1/entities/customer", None)).await;
    assert_eq!(list.status(), StatusCode::OK);
    let arr = body_json(list).await;
    let arr = arr.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "CUST-001");
    assert_eq!(arr[0]["_kind"], "customer");
}

#[tokio::test]
async fn unknown_kind_is_400() {
    let (r, _t) = router(false);
    let resp = send(&r, authed(Method::GET, "/v1/entities/orange", None)).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn missing_entity_is_404() {
    let (r, _t) = router(false);
    let resp = send(
        &r,
        authed(Method::GET, "/v1/entities/customer/CUST-999", None),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ───────────────────────── writes ─────────────────────────

#[tokio::test]
async fn create_task_and_move_round_trip() {
    let (r, _t) = router(false);
    // Create a customer to scope the task to.
    let _ = send(
        &r,
        authed(
            Method::POST,
            "/v1/customers",
            Some(serde_json::json!({"name": "Acme", "status": "active"})),
        ),
    )
    .await;

    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/tasks",
            Some(serde_json::json!({"title": "Smoke", "customer": "CUST-001", "priority": 2})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let v = body_json(resp).await;
    assert_eq!(v["id"], "TASK-001");
    assert_eq!(v["name"], "Smoke");
    assert!(v["path"].as_str().unwrap().contains("/todo/"));

    // Move to done.
    let mv = send(
        &r,
        authed(
            Method::POST,
            "/v1/tasks/TASK-001/move",
            Some(serde_json::json!({"status": "done"})),
        ),
    )
    .await;
    assert_eq!(mv.status(), StatusCode::OK);
    let v = body_json(mv).await;
    assert_eq!(v["old_status"], "backlog");
    assert_eq!(v["new_status"], "done");
    assert!(v["path"].as_str().unwrap().contains("/done/"));

    // Tasks list reflects the new status.
    let tasks = send(&r, authed(Method::GET, "/v1/tasks", None)).await;
    let arr = body_json(tasks).await;
    let arr = arr.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["status"], "done");
}

#[tokio::test]
async fn invalid_task_status_is_400() {
    let (r, _t) = router(false);
    let _ = send(
        &r,
        authed(
            Method::POST,
            "/v1/customers",
            Some(serde_json::json!({"name": "Acme"})),
        ),
    )
    .await;
    let _ = send(
        &r,
        authed(
            Method::POST,
            "/v1/tasks",
            Some(serde_json::json!({"title": "T", "customer": "CUST-001"})),
        ),
    )
    .await;

    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/tasks/TASK-001/move",
            Some(serde_json::json!({"status": "frob"})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let v = body_json(resp).await;
    assert_eq!(v["type"], "https://docs.mc.dev/errors/bad-request");
}

#[tokio::test]
async fn validate_returns_ok_on_clean_repo() {
    let (r, _t) = router(false);
    let resp = send(&r, authed(Method::POST, "/v1/validate", None)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["ok"], true);
    assert_eq!(v["issues"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn index_rebuild_returns_counts() {
    let (r, _t) = router(false);
    let _ = send(
        &r,
        authed(
            Method::POST,
            "/v1/customers",
            Some(serde_json::json!({"name": "Acme"})),
        ),
    )
    .await;
    let resp = send(&r, authed(Method::POST, "/v1/index", None)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["customers"], 1);
    assert_eq!(v["tasks"], 0);
}

// ───────────────────────── concurrency ─────────────────────────

#[tokio::test]
async fn concurrent_task_creates_get_distinct_ids() {
    let (r, _t) = router(false);
    let _ = send(
        &r,
        authed(
            Method::POST,
            "/v1/customers",
            Some(serde_json::json!({"name": "Acme"})),
        ),
    )
    .await;

    let r = Arc::new(r);
    let mut handles = Vec::new();
    for i in 0..10 {
        let r = r.clone();
        handles.push(tokio::spawn(async move {
            let req = authed(
                Method::POST,
                "/v1/tasks",
                Some(serde_json::json!({
                    "title": format!("T{i}"),
                    "customer": "CUST-001"
                })),
            );
            let resp = r.as_ref().clone().oneshot(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::CREATED);
            body_json(resp).await["id"].as_str().unwrap().to_string()
        }));
    }
    let mut ids = Vec::new();
    for h in handles {
        ids.push(h.await.unwrap());
    }
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 10, "ids must be distinct: {ids:?}");
}

// ───────────────────────── docs page + body limit + repo lock ─────────────────────────

#[tokio::test]
async fn docs_page_is_served_unauth() {
    let (r, _t) = router(false);
    let resp = send(
        &r,
        Request::builder()
            .uri("/v1/docs")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(ct.starts_with("text/html"));
    let html = body_text(resp).await;
    assert!(
        html.contains("rapi-doc") && html.contains("/v1/openapi.json"),
        "rapidoc page must reference the spec"
    );
    // The third-party viewer is pinned by hash (tokens are typed on this page).
    assert!(html.contains(r#"integrity="sha384-"#) && html.contains("crossorigin"));
}

#[tokio::test]
async fn oversized_body_is_rejected() {
    let (r, _t) = router(false);
    // 128 KiB JSON blob — well over the 64 KiB limit.
    let huge = format!(r#"{{"name":"X","tags":"{}"}}"#, "a".repeat(128 * 1024));
    let req = Request::builder()
        .method(Method::POST)
        .uri("/v1/customers")
        .header("Authorization", format!("Bearer {BEARER}"))
        .header("Content-Type", "application/json")
        .body(Body::from(huge))
        .unwrap();
    let resp = send(&r, req).await;
    assert!(
        resp.status() == StatusCode::PAYLOAD_TOO_LARGE || resp.status() == StatusCode::BAD_REQUEST,
        "expected 413 or 400 for oversized body, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn cross_process_repo_lock_blocks_second_acquire() {
    use mc::api::RepoLock;
    let tmp = TempDir::new().unwrap();
    init::run(tmp.path(), false, false, Some("LockTest"), false, true).unwrap();

    let _first = RepoLock::acquire(tmp.path()).expect("first lock");
    let second = RepoLock::acquire(tmp.path());
    assert!(
        second.is_err(),
        "second RepoLock::acquire must fail while the first is held"
    );
    let msg = second.err().unwrap().to_string();
    assert!(
        msg.contains("already running"),
        "error must mention the cause, got: {msg}"
    );
}

// ─────────────────── MCP / REST surface drift snapshot ───────────────────

/// Hard-coded list of MCP write tools that must each have a corresponding
/// REST POST endpoint. If MCP grows a new write tool, add the REST endpoint
/// in the same PR and update this list — the test will fail loudly otherwise.
#[tokio::test]
async fn rest_covers_every_mcp_write_tool() {
    let (r, _t) = router(false);
    let resp = send(
        &r,
        Request::builder()
            .uri("/v1/openapi.json")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let spec = body_json(resp).await;
    let paths = spec["paths"].as_object().unwrap();

    let expected_post_paths = [
        "/v1/customers",
        "/v1/projects",
        "/v1/meetings",
        "/v1/research",
        "/v1/tasks",
        "/v1/sprints",
        "/v1/proposals",
        "/v1/contacts",
        "/v1/tasks/{id}/move",
        "/v1/entities/{kind}/{id}/checklist/{item}",
        "/v1/entities/{kind}/{id}/comments",
        "/v1/index",
        "/v1/validate",
    ];
    for p in expected_post_paths {
        let entry = paths
            .get(p)
            .unwrap_or_else(|| panic!("OpenAPI missing path {p}"));
        assert!(
            entry.get("post").is_some(),
            "OpenAPI path {p} missing POST operation"
        );
    }
}

// ───────────────────────── input handling ─────────────────────────

#[tokio::test]
async fn config_exposes_site_name_and_available_kinds() {
    let (r, _t) = router(false);
    let v = body_json(send(&r, authed(Method::GET, "/v1/config", None)).await).await;
    // `mc init --name TestRepo` writes `site.name`; with no `brand:` section
    // that is the display name.
    assert_eq!(v["name"], "TestRepo");
    let kinds: Vec<&str> = v["available_kinds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"customers") && kinds.contains(&"tasks"));
    let configured = v["configured_entities"].as_array().unwrap();
    let mut sorted = configured.clone();
    sorted.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    assert_eq!(configured, &sorted);
}

#[tokio::test]
async fn list_fields_accept_json_arrays() {
    let (r, _t) = router(false);
    // Dependencies must exist; loose IDs are stored canonically.
    for title in ["Dep"; 9] {
        let resp = send(
            &r,
            authed(
                Method::POST,
                "/v1/tasks",
                Some(serde_json::json!({ "title": title })),
            ),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::CREATED);
    }
    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/tasks",
            Some(serde_json::json!({
                "title": "Array tags",
                "tags": ["alpha", "beta"],
                "depends_on": ["task-9"],
            })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);

    let tasks = body_json(send(&r, authed(Method::GET, "/v1/tasks?tag=beta", None)).await).await;
    let tasks = tasks.as_array().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["tags"], serde_json::json!(["alpha", "beta"]));
    // Wiki-link brackets are stripped, as in MCP and `mc list --json`.
    assert_eq!(tasks[0]["depends_on"], serde_json::json!(["TASK-009"]));
}

#[tokio::test]
async fn invalid_dates_and_priority_are_400() {
    let (r, _t) = router(false);
    for (path, body) in [
        (
            "/v1/meetings",
            serde_json::json!({"title": "M", "date": "../../escape"}),
        ),
        (
            "/v1/tasks",
            serde_json::json!({"title": "T", "priority": 9}),
        ),
        (
            "/v1/sprints",
            serde_json::json!({"title": "S", "start_date": "2026-02-10", "end_date": "2026-02-01"}),
        ),
    ] {
        let resp = send(&r, authed(Method::POST, path, Some(body))).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{path}");
        let v = body_json(resp).await;
        assert!(
            v["detail"].as_str().unwrap().starts_with("Invalid"),
            "{path}: {v}"
        );
    }
}

#[tokio::test]
async fn task_scoped_to_unknown_project_is_404() {
    let (r, _t) = router(false);
    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/tasks",
            Some(serde_json::json!({"title": "T", "project": "PROJ-404"})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn moving_a_non_task_is_400_and_changes_nothing() {
    let (r, tmp) = router(false);
    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/meetings",
            Some(serde_json::json!({"title": "Kickoff", "date": "2026-01-27"})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/tasks/MTG-001/move",
            Some(serde_json::json!({"status": "todo"})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(tmp.path().join("meetings/2026-01-27-kickoff.md").is_file());
    assert!(!tmp.path().join("todo").exists());
}

#[tokio::test]
async fn get_entity_with_mismatched_kind_is_404() {
    let (r, _t) = router(false);
    send(
        &r,
        authed(
            Method::POST,
            "/v1/customers",
            Some(serde_json::json!({"name": "Acme"})),
        ),
    )
    .await;
    let ok = send(
        &r,
        authed(Method::GET, "/v1/entities/customer/CUST-001", None),
    )
    .await;
    assert_eq!(ok.status(), StatusCode::OK);
    let wrong = send(&r, authed(Method::GET, "/v1/entities/task/CUST-001", None)).await;
    assert_eq!(wrong.status(), StatusCode::NOT_FOUND);
}

// ───────────────────────── checklists & comments ─────────────────────────

#[tokio::test]
async fn checklist_and_comments_round_trip() {
    let (r, t) = router(false);
    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/tasks",
            Some(serde_json::json!({"title": "Boxes"})),
        ),
    )
    .await;
    let path = t
        .path()
        .join(body_json(resp).await["path"].as_str().unwrap());
    let original = std::fs::read_to_string(&path).unwrap();
    let (fm, _) = mc::frontmatter::split_frontmatter(&original).unwrap();
    let doc = format!("---\n{fm}\n---\n- [ ] One\n\n```\n- [ ] decoy\n```\n\n- [ ] Two\n");
    std::fs::write(&path, &doc).unwrap();

    let list = body_json(
        send(
            &r,
            authed(Method::GET, "/v1/entities/task/TASK-001/checklist", None),
        )
        .await,
    )
    .await;
    assert_eq!(list["total"], 2);
    assert_eq!(list["items"][1]["text"], "Two");

    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/entities/task/TASK-001/checklist/2",
            Some(serde_json::json!({"expect_text": "Two"})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(
        (v["changed"].as_bool(), v["done"].as_u64()),
        (Some(true), Some(1))
    );
    let ticked = std::fs::read_to_string(&path).unwrap();
    assert_eq!(ticked, doc.replace("- [ ] Two", "- [x] Two"));

    // Stale text: 409 problem-json, file untouched.
    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/entities/task/TASK-001/checklist/1",
            Some(serde_json::json!({"expect_text": "Uno"})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    assert!(body_json(resp).await["type"]
        .as_str()
        .unwrap()
        .ends_with("/conflict"));
    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/entities/task/TASK-001/checklist/7",
            Some(serde_json::json!({})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), ticked);

    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/entities/task/TASK-001/comments",
            Some(serde_json::json!({"text": "Done **soon**", "author": "Bot"})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let v = body_json(resp).await;
    assert_eq!(v["count"], 1);
    assert_eq!(v["comment"]["author"], "Bot");
    assert_eq!(v["comment"]["body"], "Done **soon**");
    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.starts_with(&ticked));
    assert!(content.contains("\n## Comments\n\n### "));

    // Only tasks and meetings take comments; empty text is rejected.
    send(
        &r,
        authed(
            Method::POST,
            "/v1/sprints",
            Some(serde_json::json!({"title": "S1"})),
        ),
    )
    .await;
    for (uri, body) in [
        (
            "/v1/entities/sprint/SPR-001/comments",
            serde_json::json!({"text": "hi"}),
        ),
        (
            "/v1/entities/task/TASK-001/comments",
            serde_json::json!({"text": "  "}),
        ),
    ] {
        let resp = send(&r, authed(Method::POST, uri, Some(body))).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{uri}");
    }
}

#[tokio::test]
async fn read_only_rejects_checks_and_comments() {
    let (r, _t) = router(true);
    for uri in [
        "/v1/entities/task/TASK-001/checklist/1",
        "/v1/entities/task/TASK-001/comments",
    ] {
        let resp = send(
            &r,
            authed(Method::POST, uri, Some(serde_json::json!({"text": "x"}))),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{uri}");
    }
}

// ───────────────────────── problem+json everywhere ─────────────────────────

async fn assert_problem(resp: Response, status: StatusCode, kind: &str) -> Value {
    assert_eq!(resp.status(), status);
    let ct = resp.headers()["content-type"].to_str().unwrap().to_string();
    assert_eq!(ct, "application/problem+json", "{status}");
    let v = body_json(resp).await;
    assert_eq!(v["status"], status.as_u16());
    assert_eq!(v["type"], format!("https://docs.mc.dev/errors/{kind}"));
    assert!(!v["detail"].as_str().unwrap().is_empty(), "{v}");
    v
}

fn raw(method: Method, path: &str, content_type: Option<&str>, body: &str) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("Authorization", format!("Bearer {BEARER}"));
    if let Some(ct) = content_type {
        b = b.header("Content-Type", ct);
    }
    b.body(Body::from(body.to_string())).unwrap()
}

#[tokio::test]
async fn rejections_and_unknown_routes_are_problem_json() {
    let (r, _t) = router(false);
    let json = Some("application/json");
    // Unparseable JSON, a missing field, and no JSON content type.
    for (ct, body) in [(json, "{not json"), (json, r#"{"statu":"todo"}"#)] {
        let resp = send(&r, raw(Method::POST, "/v1/tasks/TASK-001/move", ct, body)).await;
        assert_problem(resp, StatusCode::BAD_REQUEST, "bad-request").await;
    }
    let resp = send(&r, raw(Method::POST, "/v1/tasks", None, r#"{"title":"x"}"#)).await;
    assert_problem(
        resp,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "unsupported-media-type",
    )
    .await;
    // A query parameter of the wrong type, a path segment of the wrong type.
    let resp = send(&r, authed(Method::GET, "/v1/tasks?priority=abc", None)).await;
    assert_problem(resp, StatusCode::BAD_REQUEST, "bad-request").await;
    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/entities/task/TASK-001/checklist/first",
            Some(serde_json::json!({})),
        ),
    )
    .await;
    assert_problem(resp, StatusCode::BAD_REQUEST, "bad-request").await;
    // Unknown routes, with or without a token, and a wrong method.
    let resp = send(&r, authed(Method::GET, "/v1/nothing", None)).await;
    assert_problem(resp, StatusCode::NOT_FOUND, "not-found").await;
    let resp = send(
        &r,
        Request::builder()
            .uri("/nothing")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_problem(resp, StatusCode::NOT_FOUND, "not-found").await;
    let resp = send(&r, authed(Method::DELETE, "/v1/config", None)).await;
    assert!(resp.headers().contains_key("allow"));
    assert_problem(resp, StatusCode::METHOD_NOT_ALLOWED, "method-not-allowed").await;
    // Oversized body.
    let huge = format!(r#"{{"name":"X","tags":"{}"}}"#, "a".repeat(128 * 1024));
    let resp = send(&r, raw(Method::POST, "/v1/customers", json, &huge)).await;
    assert_problem(resp, StatusCode::PAYLOAD_TOO_LARGE, "payload-too-large").await;
}

// ───────────────────────── validate is a read ─────────────────────────

#[tokio::test]
async fn validate_works_for_read_only_servers_and_tokens() {
    // Read-only server, read-write token.
    let (r, _t) = router(true);
    for method in [Method::GET, Method::POST] {
        let resp = send(&r, authed(method.clone(), "/v1/validate", None)).await;
        assert_eq!(resp.status(), StatusCode::OK, "{method}");
        assert_eq!(body_json(resp).await["ok"], true);
    }

    // Read-only token on a writable server.
    let tmp = TempDir::new().unwrap();
    init::run(tmp.path(), false, false, Some("RO"), false, true).unwrap();
    let cfg = config::load_config(tmp.path(), RepoMode::Standalone).unwrap();
    let yaml = format!(
        "tokens:\n  - name: ro\n    hash: \"{}\"\n    capabilities: [read]\n",
        hash_token(BEARER)
    );
    let r = build_router(
        cfg,
        &ApiServerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            tokens: TokenStore::from_yaml(&yaml).unwrap(),
            read_only: false,
        },
    );
    let resp = send(&r, authed(Method::POST, "/v1/validate", None)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let resp = send(&r, authed(Method::POST, "/v1/index", None)).await;
    assert_problem(resp, StatusCode::FORBIDDEN, "forbidden").await;
}

// ───────────────────────── repo-relative paths ─────────────────────────

#[tokio::test]
async fn responses_never_show_server_paths() {
    let (r, t) = router(false);
    let root = t.path().display().to_string();
    let canonical = t.path().canonicalize().unwrap().display().to_string();
    let leaks = |s: &str| s.contains(&root) || s.contains(&canonical);

    // A dependency must exist, so create it first.
    let dep = authed(
        Method::POST,
        "/v1/tasks",
        Some(serde_json::json!({"title": "Dep"})),
    );
    assert_eq!(send(&r, dep).await.status(), StatusCode::CREATED);
    let created = body_json(
        send(
            &r,
            authed(
                Method::POST,
                "/v1/tasks",
                Some(serde_json::json!({"title": "Paths", "depends_on": "task-1"})),
            ),
        )
        .await,
    )
    .await;
    assert!(created["path"]
        .as_str()
        .unwrap()
        .starts_with("tasks/todo/TASK-002"));

    for uri in [
        "/v1/entities/task",
        "/v1/tasks",
        "/v1/entities/task/TASK-002",
    ] {
        let text = body_text(send(&r, authed(Method::GET, uri, None)).await).await;
        assert!(!leaks(&text), "{uri}: {text}");
    }
    let one =
        body_json(send(&r, authed(Method::GET, "/v1/entities/task/TASK-002", None)).await).await;
    assert!(one["source_path"]
        .as_str()
        .unwrap()
        .starts_with("tasks/todo/"));
    assert_eq!(
        one["frontmatter"]["depends_on"],
        serde_json::json!(["TASK-001"])
    );
    let list = body_json(send(&r, authed(Method::GET, "/v1/tasks", None)).await).await;
    assert_eq!(list[0]["_kind"], "task");
    assert!(list[0]["_source"]
        .as_str()
        .unwrap()
        .starts_with("tasks/todo/"));

    let moved = body_json(
        send(
            &r,
            authed(
                Method::POST,
                "/v1/tasks/TASK-002/move",
                Some(serde_json::json!({"status": "done"})),
            ),
        )
        .await,
    )
    .await;
    assert!(
        moved["path"].as_str().unwrap().starts_with("tasks/done/"),
        "{moved}"
    );

    // An error detail that names the file names it relative to the repo.
    let file = t.path().join(moved["path"].as_str().unwrap());
    let content = std::fs::read_to_string(&file).unwrap();
    std::fs::write(
        &file,
        format!("{content}\n## Comments\n\n### 2026-01-01 10:00 · A\n\n```\nopen fence\n"),
    )
    .unwrap();
    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/entities/task/TASK-002/comments",
            Some(serde_json::json!({"text": "hi"})),
        ),
    )
    .await;
    let v = assert_problem(resp, StatusCode::BAD_REQUEST, "bad-request").await;
    let detail = v["detail"].as_str().unwrap();
    assert!(
        !leaks(detail) && detail.contains("tasks/done/TASK-002"),
        "{detail}"
    );
}

// ───────────────────────── forgiving IDs and statuses ─────────────────────────

#[tokio::test]
async fn ids_and_statuses_are_forgiving_like_the_cli() {
    let (r, _t) = router(false);
    send(
        &r,
        authed(
            Method::POST,
            "/v1/tasks",
            Some(serde_json::json!({"title": "Loose"})),
        ),
    )
    .await;
    for uri in [
        "/v1/entities/task/task-1",
        "/v1/entities/task/TASK-0001",
        "/v1/entities/task/1",
    ] {
        let resp = send(&r, authed(Method::GET, uri, None)).await;
        assert_eq!(resp.status(), StatusCode::OK, "{uri}");
        assert_eq!(body_json(resp).await["id"], "TASK-001");
    }
    let resp = send(&r, authed(Method::GET, "/v1/entities/task/TSK-1", None)).await;
    let v = assert_problem(resp, StatusCode::BAD_REQUEST, "invalid-id").await;
    assert!(v["detail"].as_str().unwrap().contains("TASK-001"), "{v}");

    // `doing` is in-progress (not "did you mean done"), case is ignored.
    for (given, stored) in [("doing", "in-progress"), ("Review", "review")] {
        let resp = send(
            &r,
            authed(
                Method::POST,
                "/v1/tasks/task-1/move",
                Some(serde_json::json!({"status": given})),
            ),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK, "{given}");
        assert_eq!(body_json(resp).await["new_status"], stored);
    }
    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/tasks/TASK-001/move",
            Some(serde_json::json!({"status": "revew"})),
        ),
    )
    .await;
    let v = assert_problem(resp, StatusCode::BAD_REQUEST, "bad-request").await;
    assert!(
        v["detail"]
            .as_str()
            .unwrap()
            .contains("did you mean 'review'"),
        "{v}"
    );

    // Only tasks move.
    send(
        &r,
        authed(
            Method::POST,
            "/v1/meetings",
            Some(serde_json::json!({"title": "Sync"})),
        ),
    )
    .await;
    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/tasks/MTG-001/move",
            Some(serde_json::json!({"status": "done"})),
        ),
    )
    .await;
    assert_problem(resp, StatusCode::BAD_REQUEST, "bad-request").await;

    // Status filters take the same spellings.
    let tasks =
        body_json(send(&r, authed(Method::GET, "/v1/tasks?status=REVIEW", None)).await).await;
    assert_eq!(tasks.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn patch_task_fields_and_next_tasks() {
    let (r, tmp) = router(false);
    for (path, body) in [
        ("/v1/customers", serde_json::json!({"name": "Acme"})),
        (
            "/v1/tasks",
            serde_json::json!({"title": "Blocker", "status": "todo"}),
        ),
        ("/v1/tasks", serde_json::json!({"title": "Follow-up"})),
    ] {
        let resp = send(&r, authed(Method::POST, path, Some(body))).await;
        assert_eq!(resp.status(), StatusCode::CREATED);
    }

    let resp = send(
        &r,
        authed(
            Method::PATCH,
            "/v1/tasks/task-2",
            Some(serde_json::json!({
                "priority": 1,
                "owner": "alice",
                "customer": "cust-1",
                "tags": ["ops", "q4"],
                "depends_on": "TASK-001",
                "due_date": "2026-10-31",
            })),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["id"], "TASK-002");
    assert_eq!(
        v["changed"],
        serde_json::json!([
            "priority",
            "owner",
            "due_date",
            "customers",
            "tags",
            "depends_on"
        ])
    );
    assert_eq!(v["path"], "tasks/todo/TASK-002-follow-up.md");
    let file =
        std::fs::read_to_string(tmp.path().join("tasks/todo/TASK-002-follow-up.md")).unwrap();
    assert!(file.contains("\"[[CUST-001]]\""), "{file}");
    assert!(file.contains("\"[[TASK-001]]\""), "{file}");

    // TASK-002 waits on TASK-001.
    let resp = send(&r, authed(Method::GET, "/v1/tasks/next", None)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["tasks"][0]["id"], "TASK-001");
    assert_eq!(
        (v["actionable"].as_u64(), v["blocked"].as_u64()),
        (Some(1), Some(1))
    );

    let resp = send(
        &r,
        authed(
            Method::PATCH,
            "/v1/tasks/TASK-001",
            Some(serde_json::json!({"status": "completed"})),
        ),
    )
    .await;
    let v = body_json(resp).await;
    assert_eq!(v["new_status"], "done");
    assert_eq!(v["path"], "tasks/done/TASK-001-blocker.md");
    let resp = send(
        &r,
        authed(Method::GET, "/v1/tasks/next?owner=ALICE&limit=1", None),
    )
    .await;
    let v = body_json(resp).await;
    assert_eq!(v["tasks"][0]["id"], "TASK-002");
    assert_eq!(v["blocked"], 0);

    // Mistakes are problem+json and change nothing.
    let before =
        std::fs::read_to_string(tmp.path().join("tasks/todo/TASK-002-follow-up.md")).unwrap();
    for (body, status, kind) in [
        (
            serde_json::json!({}),
            StatusCode::BAD_REQUEST,
            "bad-request",
        ),
        (
            serde_json::json!({"priority": 7}),
            StatusCode::BAD_REQUEST,
            "bad-request",
        ),
        (
            serde_json::json!({"due_date": "31.10.2026"}),
            StatusCode::BAD_REQUEST,
            "bad-request",
        ),
        (
            serde_json::json!({"projects": "PROJ-404"}),
            StatusCode::NOT_FOUND,
            "not-found",
        ),
    ] {
        let resp = send(
            &r,
            authed(Method::PATCH, "/v1/tasks/TASK-002", Some(body.clone())),
        )
        .await;
        assert_problem(resp, status, kind).await;
    }
    let resp = send(
        &r,
        authed(
            Method::PATCH,
            "/v1/tasks/TASK-002",
            Some(serde_json::json!({"colour": "red"})),
        ),
    )
    .await;
    // Unknown fields are refused rather than silently ignored.
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let resp = send(
        &r,
        authed(
            Method::PATCH,
            "/v1/tasks/TASK-404",
            Some(serde_json::json!({"owner": "x"})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("tasks/todo/TASK-002-follow-up.md")).unwrap(),
        before
    );
}

#[tokio::test]
async fn patch_task_needs_write_and_spec_lists_it() {
    let (r, _t) = router(true);
    let resp = send(
        &r,
        authed(
            Method::PATCH,
            "/v1/tasks/TASK-001",
            Some(serde_json::json!({"owner": "x"})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    // Reads still work on a read-only server.
    let resp = send(&r, authed(Method::GET, "/v1/tasks/next", None)).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = send(
        &r,
        Request::builder()
            .uri("/v1/openapi.json")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let spec = body_json(resp).await;
    assert!(spec.pointer("/paths/~1v1~1tasks~1{id}/patch").is_some());
    assert!(spec.pointer("/paths/~1v1~1tasks~1next/get").is_some());
    // The spec's version follows the crate instead of a hard-coded string.
    assert_eq!(spec["info"]["version"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn milestones_can_be_created_assigned_filtered_and_cleared() {
    let (r, _t) = router(false);
    let response = send(&r,authed(Method::POST,"/v1/milestones",Some(serde_json::json!({"title":"Delivery", "description":"Ship it", "start_date":"2026-10-01", "due_date":"2026-10-31"})))).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let created = body_json(response).await;
    assert_eq!(created["id"], "MS-001");
    let response = send(
        &r,
        authed(
            Method::POST,
            "/v1/tasks",
            Some(serde_json::json!({"title":"Ship", "milestone":"ms-1"})),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let response = send(&r, authed(Method::GET, "/v1/tasks?milestone=MS-001", None)).await;
    assert_eq!(response.status(), StatusCode::OK);
    let tasks = body_json(response).await;
    assert_eq!(tasks.as_array().unwrap().len(), 1);
    assert_eq!(tasks[0]["milestone"], "MS-001");
    let response = send(
        &r,
        authed(
            Method::PATCH,
            "/v1/tasks/TASK-001",
            Some(serde_json::json!({"milestone":""})),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let tasks =
        body_json(send(&r, authed(Method::GET, "/v1/tasks?milestone=MS-001", None)).await).await;
    assert!(tasks.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn milestone_errors_are_typed_and_write_nothing() {
    let (r, t) = router(false);
    // A deadline before the start is a 400 and creates nothing.
    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/milestones",
            Some(serde_json::json!({"title":"Late", "start_date":"2026-10-10", "due_date":"2026-10-01"})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert!(no_milestones(t.path()));
    // `projects` takes a comma-separated string like every other list field.
    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/projects",
            Some(serde_json::json!({"name":"Robot"})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/milestones",
            Some(serde_json::json!({"title":"Beta", "projects":"proj-1"})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let resp = send(
        &r,
        authed(Method::GET, "/v1/entities/milestones/MS-001", None),
    )
    .await;
    let ms = body_json(resp).await;
    assert_eq!(ms["frontmatter"]["projects"][0], "PROJ-001");

    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/tasks",
            Some(serde_json::json!({"title":"T"})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let path = t
        .path()
        .join(body_json(resp).await["path"].as_str().unwrap());
    let before = std::fs::read(&path).unwrap();
    let resp = send(
        &r,
        authed(
            Method::PATCH,
            "/v1/tasks/TASK-001",
            Some(serde_json::json!({"title":"Renamed", "milestone":"MS-404"})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let resp = send(&r, authed(Method::GET, "/v1/tasks?milestone=MS-404", None)).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    // By title; an empty filter lists the tasks without a milestone.
    let resp = send(&r, authed(Method::GET, "/v1/tasks?milestone=Beta", None)).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(body_json(resp).await.as_array().unwrap().is_empty());
    let resp = send(&r, authed(Method::GET, "/v1/tasks?milestone=", None)).await;
    let tasks = body_json(resp).await;
    assert_eq!(tasks.as_array().unwrap().len(), 1);
    assert_eq!(tasks[0]["id"], "TASK-001");
}

#[tokio::test]
async fn milestones_cannot_be_created_read_only() {
    let (r, t) = router(true);
    let resp = send(
        &r,
        authed(
            Method::POST,
            "/v1/milestones",
            Some(serde_json::json!({"title":"Nope"})),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert!(no_milestones(t.path()));
}

/// Whether `milestones/` holds no milestone folder (init leaves a `.gitkeep`).
fn no_milestones(root: &std::path::Path) -> bool {
    std::fs::read_dir(root.join("milestones"))
        .unwrap()
        .all(|e| !e.unwrap().file_name().to_string_lossy().starts_with("MS-"))
}
