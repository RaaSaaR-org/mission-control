//! Repo files linked from notes (`/files/...`) in the `mc serve` dashboard.

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
    init::run(tmp.path(), false, false, Some("FilesTest"), false, true).unwrap();
    let root = tmp.path();
    write(
        root,
        "research/RES-001-robots/RES-001.md",
        "---\nid: RES-001\ntitle: Robots\nstatus: draft\n---\n\nSee [the deep dive](notes/deep%20dive.md#top), [plan](../../tasks/todo/TASK-001-plan.md) and ![chart](notes/chart.svg).\n",
    );
    write(
        root,
        "research/RES-001-robots/notes/deep dive.md",
        "# Deep <dive>\n\nBack to [main](../RES-001.md). <b>Unclosed\n\nNext\n",
    );
    write(
        root,
        "research/RES-001-robots/notes/chart.svg",
        r#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#,
    );
    write(
        root,
        "tasks/todo/TASK-001-plan.md",
        "---\nid: TASK-001\ntitle: Plan\nstatus: todo\n---\n",
    );
    write(root, ".env", "SECRET=1\n");
    write(root, "research/RES-001-robots/.hidden.md", "# Hidden\n");
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
async fn relative_links_in_notes_resolve() {
    let tmp = repo();
    let router = router_for(&tmp, "");
    let html = text(get(&router, "/entity/RES-001").await).await;
    assert!(html.contains(r#"href="/files/research/RES-001-robots/notes/deep%20dive.md#top""#));
    assert!(html.contains(r#"href="/entity/TASK-001">plan</a>"#));
    assert!(html.contains(r#"src="/files/research/RES-001-robots/notes/chart.svg""#));

    let resp = get(
        &router,
        "/files/research/RES-001-robots/notes/deep%20dive.md",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let html = text(resp).await;
    assert!(html.contains("Deep &lt;dive&gt;</h1>"));
    assert!(html.contains(r#"href="/entity/RES-001">main</a>"#));
    // The unclosed <b> ends with its paragraph.
    assert!(html.contains("<b>Unclosed</b></p>"));
    // Breadcrumb leads back to the research entity.
    assert!(html.contains(r#"href="/entity/RES-001" title="Robots""#));
}

#[tokio::test]
async fn entity_files_redirect_to_their_page() {
    let tmp = repo();
    let router = router_for(&tmp, "");
    let resp = get(&router, "/files/research/RES-001-robots/RES-001.md").await;
    assert!(resp.status().is_redirection());
    assert_eq!(resp.headers()[header::LOCATION], "/entity/RES-001");
}

#[tokio::test]
async fn files_are_served_inert_and_only_from_the_repo() {
    let tmp = repo();
    let router = router_for(&tmp, "");
    let resp = get(&router, "/files/research/RES-001-robots/notes/chart.svg").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()[header::CONTENT_TYPE], "image/svg+xml");
    let csp = resp.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap();
    assert!(csp.contains("sandbox") && csp.contains("default-src 'none'"));

    for uri in [
        "/files/.env",
        "/files/research/RES-001-robots/.hidden.md",
        "/files/config/config.yml",
        "/files/research/RES-001-robots/notes/..%2F..%2F..%2F.env",
        "/files/..%2F..%2Fetc%2Fpasswd",
        "/files/research/RES-001-robots/notes",
        "/files/research/missing.md",
    ] {
        let resp = get(&router, uri).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{uri}");
        assert!(!text(resp).await.contains("SECRET"), "{uri}");
    }
}

#[tokio::test]
async fn file_links_carry_the_base_path() {
    let tmp = repo();
    let router = router_for(&tmp, "/hq");
    let html = text(get(&router, "/hq/entity/RES-001").await).await;
    assert!(html.contains(r#"href="/hq/files/research/RES-001-robots/notes/deep%20dive.md#top""#));
    let resp = get(&router, "/hq/files/research/RES-001-robots/RES-001.md").await;
    assert_eq!(resp.headers()[header::LOCATION], "/hq/entity/RES-001");
}

#[tokio::test]
async fn api_rejects_unknown_methods_with_json() {
    let tmp = repo();
    let router = router_for(&tmp, "");
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/tasks/TASK-001")
                .header("x-mc-request", "1")
                .header(header::HOST, "localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert!(text(resp).await.contains(r#""error":"method_not_allowed""#));
}
