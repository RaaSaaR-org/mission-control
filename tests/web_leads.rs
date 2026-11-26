//! Integration tests for the dashboard's CRM pages and their JSON endpoints:
//! the lead board and table, quick capture, the lead detail page, events,
//! and the guarded writes under `/api/leads` and `/api/events`. The router
//! runs in-process on a temporary repo.

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

fn day(n: i64) -> String {
    (chrono::Local::now().date_naive() + chrono::Duration::days(n))
        .format("%Y-%m-%d")
        .to_string()
}

fn repo() -> TempDir {
    let tmp = TempDir::new().unwrap();
    init::run(tmp.path(), false, false, Some("WebTest"), false, true).unwrap();
    write(
        tmp.path(),
        "customers/CUST-001-zema/CUST-001.md",
        "---\nid: CUST-001\nname: Zema\nstatus: active\n---\n\n# Zema\n",
    );
    write(
        tmp.path(),
        "events/EVT-001-fair/EVT-001.md",
        &format!(
            "---\nid: EVT-001\ntitle: Automatica\nslug: fair\nstatus: attended\ntype: trade-fair\ndate: {}\n---\n\n# Automatica\n\n## Goals\n\n## Debrief\n",
            day(-3)
        ),
    );
    write(
        tmp.path(),
        "leads/LEAD-001-jane-doe/LEAD-001.md",
        &format!(
            "---\nid: LEAD-001\nname: \"Jane <b>Doe</b>\"\nslug: jane-doe\ncompany: Acme\nrole: CTO\nemail: jane@acme.example\nphone: \"+49 1\"\nstatus: new\nsource: event\nevent: \"[[EVT-001|Automatica]]\"\nintroduce_to: [\"[[CUST-001|Zema]]\"]\nwarmth: hot\nnext_follow_up: {}\nlast_contact: \"\"\nconverted_to: \"\"\ntags: []\n---\n\n# Jane Doe\n\n## Summary\n\nRuns two robot cells.\n\n## Interactions\n\n- {} · event · Met at the booth\n\n## Notes\n",
            day(-1),
            day(-3)
        ),
    );
    write(
        tmp.path(),
        "leads/LEAD-001-jane-doe/research.md",
        "# Research\n\n## Company\n\nAcme makes widgets.\n",
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

async fn text(resp: Response) -> String {
    let bytes = to_bytes(resp.into_body(), 1 << 24).await.unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn body_json(resp: Response) -> Value {
    serde_json::from_str(&text(resp).await).unwrap()
}

fn lead_file(tmp: &TempDir) -> String {
    std::fs::read_to_string(tmp.path().join("leads/LEAD-001-jane-doe/LEAD-001.md")).unwrap()
}

#[tokio::test]
async fn crm_pages_render_and_escape() {
    let tmp = repo();
    let router = local(&tmp);
    for uri in [
        "/",
        "/leads",
        "/leads?due=1&warmth=hot",
        "/leads/list",
        "/leads/list?sort=next_follow_up&dir=desc",
        "/leads/new",
        "/leads/new?event=EVT-001",
        "/events",
        "/events/new",
        "/entity/LEAD-001",
        "/entity/EVT-001",
        "/entity/CUST-001",
    ] {
        let resp = get(&router, uri).await;
        assert_eq!(resp.status(), StatusCode::OK, "{uri}");
        let html = text(resp).await;
        assert!(html.starts_with("<!DOCTYPE html>"), "{uri}");
        assert!(!html.contains("<b>Doe</b>"), "{uri}: names are escaped");
        // No inline scripts besides the theme boot script in the head.
        assert_eq!(html.matches("<script>").count(), 1, "{uri}");
    }
}

#[tokio::test]
async fn board_detail_and_overview_show_the_pipeline() {
    let tmp = repo();
    let router = local(&tmp);
    let board = text(get(&router, "/leads").await).await;
    assert!(board.contains(r#"id="board" data-api="/leads""#));
    assert!(board.contains(r#"data-id="LEAD-001" data-status="new""#));
    assert!(board.contains("1d overdue"));
    assert!(board.contains(r#"<template id="move-menu-tpl">"#));

    let detail = text(get(&router, "/entity/LEAD-001").await).await;
    assert!(detail.contains("stage-route"));
    assert!(detail.contains("Met at the booth"));
    assert!(detail.contains("Runs two robot cells."));
    assert!(detail.contains("Acme makes widgets."));
    assert!(detail.contains(r#"href="mailto:jane@acme.example""#));
    assert!(detail.contains(r#"href="tel:+491""#));
    assert!(detail.contains(r#"data-lead-form="convert""#));

    let event = text(get(&router, "/entity/EVT-001").await).await;
    assert!(event.contains("People met"));
    assert!(event.contains("/leads?event=EVT-001"));

    let home = text(get(&router, "/").await).await;
    assert!(home.contains("Follow-ups"));
    assert!(home.contains("1 overdue, 0 today"));
    assert!(home.contains("funnel-row"));

    let new = text(get(&router, "/leads/new").await).await;
    assert!(new.contains(r#"<option value="EVT-001" selected>"#));
}

#[tokio::test]
async fn moving_a_lead_updates_the_file_and_returns_its_card() {
    let tmp = repo();
    let router = local(&tmp);
    let resp = send(
        &router,
        write_req(
            Method::POST,
            "/api/leads/LEAD-001/move",
            json!({"status": "qualified"}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["old_status"], "new");
    assert_eq!(v["lead"]["status"], "qualified");
    assert!(v["html"]["card"]
        .as_str()
        .unwrap()
        .contains(r#"data-status="qualified""#));
    assert!(v["version"].is_string() && v["prev_version"].is_string());
    assert!(lead_file(&tmp).contains("status: qualified"));

    let bad = send(
        &router,
        write_req(
            Method::POST,
            "/api/leads/LEAD-001/move",
            json!({"status": "nope"}),
        ),
    )
    .await;
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(bad).await["field"], "status");
    let bad_id = send(
        &router,
        write_req(
            Method::POST,
            "/api/leads/..%2Fx/move",
            json!({"status": "new"}),
        ),
    )
    .await;
    assert_eq!(bad_id.status(), StatusCode::BAD_REQUEST);
    let missing = send(
        &router,
        write_req(
            Method::POST,
            "/api/leads/LEAD-099/move",
            json!({"status": "new"}),
        ),
    )
    .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn logging_an_interaction_moves_new_leads_on() {
    let tmp = repo();
    let router = local(&tmp);
    let resp = send(
        &router,
        write_req(
            Method::POST,
            "/api/leads/LEAD-001/interactions",
            json!({"type": "call", "note": "Wants a demo", "next_follow_up": day(7)}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let v = body_json(resp).await;
    assert_eq!(v["lead"]["status"], "contacted");
    let file = lead_file(&tmp);
    assert!(file.contains(&format!("- {} · call · Wants a demo", day(0))));
    assert!(file.contains(&format!("next_follow_up: {}", day(7))));
    assert!(file.contains(&format!("last_contact: {}", day(0))));

    let empty = send(
        &router,
        write_req(
            Method::POST,
            "/api/leads/LEAD-001/interactions",
            json!({"type": "call", "note": "  "}),
        ),
    )
    .await;
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(empty).await["field"], "note");
    let bad_type = send(
        &router,
        write_req(
            Method::POST,
            "/api/leads/LEAD-001/interactions",
            json!({"type": "fax", "note": "x"}),
        ),
    )
    .await;
    assert_eq!(body_json(bad_type).await["field"], "type");
}

#[tokio::test]
async fn create_patch_and_convert_use_the_shared_commands() {
    let tmp = repo();
    let router = local(&tmp);
    let resp = send(
        &router,
        write_req(
            Method::POST,
            "/api/leads",
            json!({"name": "Max Mustermann", "company": "Zema", "event": "EVT-001", "warmth": "warm", "notes": "Talked robots"}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let v = body_json(resp).await;
    assert_eq!(v["lead"]["id"], "LEAD-002");
    assert_eq!(v["href"], "/entity/LEAD-002");
    let file =
        std::fs::read_to_string(tmp.path().join("leads/LEAD-002-max-mustermann/LEAD-002.md"))
            .unwrap();
    assert!(file.contains(r#"event: "[[EVT-001|Automatica]]""#));
    assert!(file.contains("source: event"));
    assert!(file.contains("Talked robots"));

    let unnamed = send(
        &router,
        write_req(Method::POST, "/api/leads", json!({"name": " "})),
    )
    .await;
    assert_eq!(body_json(unnamed).await["field"], "name");
    let unknown = send(
        &router,
        write_req(
            Method::POST,
            "/api/leads",
            json!({"name": "X", "colour": "red"}),
        ),
    )
    .await;
    assert_eq!(unknown.status(), StatusCode::BAD_REQUEST);

    let patched = send(
        &router,
        write_req(
            Method::PATCH,
            "/api/leads/LEAD-002",
            json!({"role": "COO", "introduce_to": ["CUST-001"], "tags": ["pilot"], "warmth": ""}),
        ),
    )
    .await;
    assert_eq!(patched.status(), StatusCode::OK);
    let file =
        std::fs::read_to_string(tmp.path().join("leads/LEAD-002-max-mustermann/LEAD-002.md"))
            .unwrap();
    assert!(file.contains("role: COO"));
    assert!(file.contains("[[CUST-001|Zema]]"));
    assert!(file.contains("- pilot"));
    assert!(file.contains("warmth: ''"));
    let nothing = send(
        &router,
        write_req(Method::PATCH, "/api/leads/LEAD-002", json!({})),
    )
    .await;
    assert_eq!(nothing.status(), StatusCode::BAD_REQUEST);

    // The company matches customer Zema, so no customer needs naming.
    let converted = send(
        &router,
        write_req(Method::POST, "/api/leads/LEAD-002/convert", json!({})),
    )
    .await;
    assert_eq!(converted.status(), StatusCode::OK);
    let v = body_json(converted).await;
    assert_eq!(v["customer"], "CUST-001");
    assert_eq!(v["lead"]["status"], "converted");
    let contact = v["contact"].as_str().unwrap().to_string();
    assert_eq!(v["contact_href"], format!("/entity/{contact}"));
    let again = send(
        &router,
        write_req(Method::POST, "/api/leads/LEAD-002/convert", json!({})),
    )
    .await;
    assert_eq!(again.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn events_can_be_created() {
    let tmp = repo();
    let router = local(&tmp);
    let resp = send(
        &router,
        write_req(
            Method::POST,
            "/api/events",
            json!({"title": "Hannover Messe", "date": "2027-04-12", "end_date": "2027-04-16", "type": "trade-fair", "location": "Hannover"}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let v = body_json(resp).await;
    assert_eq!(v["id"], "EVT-002");
    assert_eq!(v["href"], "/entity/EVT-002");
    let page = text(get(&router, "/events").await).await;
    assert!(page.contains("Hannover Messe"));
    assert!(page.contains("Upcoming"));
    let bad = send(
        &router,
        write_req(
            Method::POST,
            "/api/events",
            json!({"title": "X", "date": "soon"}),
        ),
    )
    .await;
    assert_eq!(body_json(bad).await["field"], "date");
}

#[tokio::test]
async fn crm_writes_are_guarded_and_read_only_hides_the_edit_ui() {
    let tmp = repo();
    let router = local(&tmp);
    let no_header = Request::post("/api/leads")
        .header(header::HOST, "localhost:5000")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"name": "X"}).to_string()))
        .unwrap();
    assert_eq!(
        send(&router, no_header).await.status(),
        StatusCode::FORBIDDEN
    );
    let mut cross = write_req(Method::POST, "/api/leads", json!({"name": "X"}));
    cross
        .headers_mut()
        .insert(header::ORIGIN, "http://evil.example".parse().unwrap());
    assert_eq!(send(&router, cross).await.status(), StatusCode::FORBIDDEN);

    let ro = app(
        &tmp,
        ServeOptions {
            read_only: true,
            ..Default::default()
        },
    );
    let resp = send(
        &ro,
        write_req(
            Method::POST,
            "/api/leads/LEAD-001/move",
            json!({"status": "lost"}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert!(lead_file(&tmp).contains("status: new"));
    for uri in ["/leads", "/entity/LEAD-001", "/leads/new", "/events"] {
        let html = text(get(&ro, uri).await).await;
        assert!(!html.contains("data-lead-form"), "{uri}");
        assert!(!html.contains("move-menu-tpl"), "{uri}");
        assert!(!html.contains("data-lead-stage"), "{uri}");
    }
    assert!(text(get(&ro, "/leads/new").await)
        .await
        .contains("This dashboard is read-only"));
}

#[tokio::test]
async fn disabled_crm_kinds_say_how_to_enable_them() {
    let tmp = repo();
    let cfg_path = tmp.path().join("config/config.yml");
    let cfg = std::fs::read_to_string(&cfg_path).unwrap();
    let cfg = if cfg.contains("\npaths:") {
        cfg.replace("  leads: leads/\n", "")
            .replace("  events: events/\n", "")
    } else {
        format!("{cfg}\npaths:\n  tasks: tasks/\n")
    };
    std::fs::write(&cfg_path, cfg).unwrap();
    let router = local(&tmp);
    for uri in ["/leads", "/leads/list", "/leads/new", "/events"] {
        let resp = get(&router, uri).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{uri}");
    }
    let resp = send(
        &router,
        write_req(
            Method::POST,
            "/api/leads/LEAD-001/move",
            json!({"status": "lost"}),
        ),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn conversion_guards_reach_the_dashboard() {
    let tmp = repo();
    let router = local(&tmp);
    // The Converted lane can't be reached by a move: convert makes the contact.
    let moved = send(
        &router,
        write_req(
            Method::POST,
            "/api/leads/LEAD-001/move",
            json!({"status": "converted"}),
        ),
    )
    .await;
    assert_eq!(moved.status(), StatusCode::BAD_REQUEST);
    assert!(lead_file(&tmp).contains("status: new"));

    // A future interaction date is a field error and changes nothing.
    let future = send(
        &router,
        write_req(
            Method::POST,
            "/api/leads/LEAD-001/interactions",
            json!({"type": "call", "note": "Typo", "date": day(400)}),
        ),
    )
    .await;
    assert_eq!(future.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(future).await["field"], "date");
    assert!(!lead_file(&tmp).contains("Typo"));

    // The page caps the composer date and offers linking to an existing contact.
    let html = text(get(&router, "/entity/LEAD-001").await).await;
    assert!(html.contains(&format!(
        r#"name="date" value="{0}" required max="{0}""#,
        day(0)
    )));
    assert!(html.contains(r#"name="link_existing""#));
    assert!(html.contains(r#"name="last_contact""#));

    // A contact for the same person already exists: 409, then link.
    write(
        tmp.path(),
        "customers/CUST-001-zema/contacts/CONT-001-jane.md",
        "---\nid: CONT-001\nname: Jane\ncustomer: \"[[CUST-001|Zema]]\"\nemail: JANE@acme.example\nstatus: active\n---\n\n# Jane\n\n## Notes\n",
    );
    let dup = send(
        &router,
        write_req(
            Method::POST,
            "/api/leads/LEAD-001/convert",
            json!({"customer": "CUST-001"}),
        ),
    )
    .await;
    assert_eq!(dup.status(), StatusCode::CONFLICT);
    assert!(body_json(dup).await["message"]
        .as_str()
        .unwrap()
        .contains("CONT-001"));
    let linked = send(
        &router,
        write_req(
            Method::POST,
            "/api/leads/LEAD-001/convert",
            json!({"customer": "CUST-001", "link_existing": true}),
        ),
    )
    .await;
    assert_eq!(linked.status(), StatusCode::OK);
    let v = body_json(linked).await;
    assert_eq!(v["contact"], "CONT-001");
    assert_eq!(v["contact_created"], false);
    assert_eq!(v["lead"]["status"], "converted");

    // Converted: the stage route is read-only and moves out are refused.
    let html = text(get(&router, "/entity/LEAD-001").await).await;
    assert!(!html.contains(r#"data-lead-stage="qualified""#));
    assert!(!html.contains("Mark lost"));
    let out = send(
        &router,
        write_req(
            Method::POST,
            "/api/leads/LEAD-001/move",
            json!({"status": "qualified"}),
        ),
    )
    .await;
    assert_eq!(out.status(), StatusCode::CONFLICT);
}
