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
