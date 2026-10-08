use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use mc::{
    commands::{
        init,
        new::{self, MilestoneInput, TaskInput},
        serve::{router, ServeOptions},
        task::{self, TaskUpdate},
    },
    config::{self, RepoMode},
    data::{self, TaskFilter},
    entity::EntityKind,
    frontmatter,
};
use tower::ServiceExt;

fn repo(embedded: bool) -> (tempfile::TempDir, config::ResolvedConfig) {
    let tmp = tempfile::tempdir().unwrap();
    init::run(tmp.path(), false, embedded, Some("Milestones"), false, true).unwrap();
    let cfg = config::load_config(
        tmp.path(),
        if embedded {
            RepoMode::Embedded
        } else {
            RepoMode::Standalone
        },
    )
    .unwrap();
    (tmp, cfg)
}
fn milestone(cfg: &config::ResolvedConfig) -> new::Created {
    new::create_milestone(
        cfg,
        &MilestoneInput {
            title: "AP3: Integration".into(),
            description: Some("Train <robot> & validate".into()),
            start_date: Some("2026-09-01".into()),
            due_date: Some("2026-11-30".into()),
            ..Default::default()
        },
    )
    .unwrap()
}
#[test]
fn milestones_work_in_both_modes_and_old_repos() {
    for embedded in [false, true] {
        let (_tmp, cfg) = repo(embedded);
        // Upgrading an existing repo doesn't require a new template.
        std::fs::remove_file(cfg.templates_dir.join("milestone.md")).unwrap();
        let m = milestone(&cfg);
        assert_eq!(m.id, "MS-001");
        for given in ["ms-1", "1", "AP3: integration", "[[MS-001]]"] {
            assert_eq!(new::resolve_milestone(&cfg, given).unwrap(), m.id);
        }
        let created = new::create_task(
            &cfg,
            &TaskInput {
                title: "Train".into(),
                milestone: Some("ms-1".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let (fm, _) = frontmatter::parse_file(&created.path).unwrap();
        assert_eq!(frontmatter::get_str(&fm, "milestone"), Some("[[MS-001]]"));
        assert_eq!(
            data::collect_tasks_filtered(
                &cfg,
                &TaskFilter {
                    milestone: Some("AP3: Integration"),
                    ..TaskFilter::all()
                }
            )
            .unwrap()
            .len(),
            1
        );
        task::update_task(
            &cfg,
            &created.id,
            &TaskUpdate {
                milestone: Some("".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(data::collect_tasks_filtered(
            &cfg,
            &TaskFilter {
                milestone: Some("MS-001"),
                ..TaskFilter::all()
            }
        )
        .unwrap()
        .is_empty());
    }
}
#[test]
fn invalid_dates_and_missing_references_do_not_write() {
    let (_tmp, cfg) = repo(false);
    for (start, due) in [("2026-10-10", "2026-10-01"), ("2026-02-30", "2026-03-01")] {
        assert!(new::create_milestone(
            &cfg,
            &MilestoneInput {
                title: "Invalid".into(),
                start_date: Some(start.into()),
                due_date: Some(due.into()),
                ..Default::default()
            }
        )
        .is_err());
    }
    assert!(data::collect_entities(EntityKind::Milestone, &cfg)
        .unwrap()
        .is_empty());
    let t = new::create_task(&cfg, &TaskInput::new("Original")).unwrap();
    let before = std::fs::read(&t.path).unwrap();
    assert!(task::update_task(
        &cfg,
        &t.id,
        &TaskUpdate {
            title: Some("Changed".into()),
            milestone: Some("MS-404".into()),
            ..Default::default()
        }
    )
    .is_err());
    assert_eq!(std::fs::read(&t.path).unwrap(), before);
}
#[test]
fn cli_can_assign_and_filter_by_milestone() {
    let (_tmp, cfg) = repo(false);
    milestone(&cfg);
    let t = new::create_task(&cfg, &TaskInput::new("Train")).unwrap();
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_mc"))
            .arg("--root")
            .arg(&cfg.root)
            .args(args)
            .output()
            .unwrap()
    };
    assert!(run(&["task", "set", &t.id, "--milestone", "ms-1"])
        .status
        .success());
    let result = run(&["--json", "list", "tasks", "--milestone", "AP3: Integration"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let rows: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(rows[0]["milestone"], "MS-001");
    assert_eq!(rows.as_array().unwrap().len(), 1);
}
async fn html(app: &axum::Router, uri: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("host", "localhost:5000")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    String::from_utf8(
        to_bytes(response.into_body(), 1 << 24)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap()
}
#[tokio::test]
async fn timeline_filters_links_and_web_assignment_round_trip() {
    let (_tmp, cfg) = repo(false);
    milestone(&cfg);
    for (title, status) in [
        ("Finished", "done"),
        ("Open", "todo"),
        ("Dropped", "cancelled"),
    ] {
        new::create_task(
            &cfg,
            &TaskInput {
                title: title.into(),
                status: Some(status.into()),
                milestone: Some("MS-001".into()),
                due_date: Some("2026-10-01".into()),
                ..Default::default()
            },
        )
        .unwrap();
    }
    new::create_task(&cfg, &TaskInput::new("Unassigned")).unwrap();
    let app = router(&cfg, &ServeOptions::default());
    let gantt = html(&app, "/milestones").await;
    assert!(gantt.contains("1/2 done"));
    assert!(gantt.contains("Train &lt;robot&gt; &amp; validate"));
    assert!(gantt.contains("/tasks/list?milestone=MS-001"));
    assert!(gantt.contains("gantt-task"));
    assert!(html(&app, "/").await.contains("Open timeline"));
    let filtered = html(&app, "/tasks/list?milestone=MS-001").await;
    assert!(filtered.contains("Finished"));
    assert!(!filtered.contains(">Unassigned<"));
    let request = Request::builder()
        .method("PATCH")
        .uri("/api/tasks/TASK-004")
        .header("host", "localhost:5000")
        .header("origin", "http://localhost:5000")
        .header("x-mc-request", "1")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"milestone":"MS-001"}"#))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(request).await.unwrap().status(),
        StatusCode::OK
    );
    let detail = html(&app, "/entity/TASK-004").await;
    assert!(detail.contains(r#"value="MS-001" selected"#));
    assert!(html(&app, "/entity/MS-001").await.contains("Unassigned"));
}

#[test]
fn configs_from_before_milestones_enable_them_with_defaults() {
    let (tmp, _) = repo(false);
    // A real pre-milestone config: `paths:` lists tasks but nothing about
    // milestones, and there is no template or folder yet.
    let config_path = tmp.path().join("config/config.yml");
    let config = std::fs::read_to_string(&config_path).unwrap();
    let mut old = String::new();
    let mut skip_block = false;
    for line in config.lines() {
        if skip_block && line.starts_with("    - ") {
            continue;
        }
        skip_block = line == "  milestone:";
        if line.contains("milestone") {
            continue;
        }
        old.push_str(line);
        old.push('\n');
    }
    assert!(!old.contains("milestone"), "{old}");
    std::fs::write(&config_path, old).unwrap();
    std::fs::remove_file(tmp.path().join("templates/milestone.md")).unwrap();
    std::fs::remove_dir_all(tmp.path().join("milestones")).unwrap();

    let cfg = config::load_config(tmp.path(), RepoMode::Standalone).unwrap();
    assert!(cfg.entity_available(&EntityKind::Milestone));
    assert_eq!(cfg.id_prefixes.milestone, "MS");
    assert_eq!(
        cfg.statuses.milestone,
        ["planned", "active", "completed", "cancelled"]
    );
    let m = milestone(&cfg);
    assert_eq!(m.id, "MS-001");
    assert!(m.path.starts_with(tmp.path().join("milestones")));
    let doc = std::fs::read_to_string(m.path.join("MS-001.md")).unwrap();
    let (fm, body) = frontmatter::split_frontmatter(&doc).unwrap();
    let fm: serde_yaml::Value = serde_yaml::from_str(&fm).unwrap();
    assert_eq!(frontmatter::get_str(&fm, "status"), Some("planned"));
    // Like every other entity: a blank line after the frontmatter, and the
    // description only once (in the frontmatter).
    assert!(body.starts_with("\n# AP3: Integration\n"), "{body:?}");
    assert_eq!(doc.matches("Train <robot> & validate").count(), 1, "{doc}");

    let t = new::create_task(
        &cfg,
        &TaskInput {
            title: "Assigned".into(),
            milestone: Some("MS-001".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let found = data::collect_tasks_filtered(
        &cfg,
        &TaskFilter {
            milestone: Some("ms-1"),
            ..TaskFilter::all()
        },
    )
    .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, t.id);
}

#[test]
fn empty_milestone_filter_lists_unassigned_tasks() {
    let (_tmp, cfg) = repo(false);
    milestone(&cfg);
    let assigned = new::create_task(
        &cfg,
        &TaskInput {
            title: "Assigned".into(),
            milestone: Some("MS-001".into()),
            ..Default::default()
        },
    )
    .unwrap();
    // Written by `mc new task` since milestones exist: `milestone: ''`.
    let empty = new::create_task(&cfg, &TaskInput::new("Empty")).unwrap();
    // Every task from before milestones: no `milestone` key at all.
    let todo = cfg.tasks_dir.join("todo");
    std::fs::write(
        todo.join("TASK-099-old.md"),
        "---\nid: TASK-099\ntitle: Old\nstatus: todo\n---\n\n# Old\n",
    )
    .unwrap();
    let ids = |milestone: &str| -> Vec<String> {
        data::collect_tasks_filtered(
            &cfg,
            &TaskFilter {
                milestone: Some(milestone),
                ..TaskFilter::all()
            },
        )
        .unwrap()
        .into_iter()
        .map(|t| t.id)
        .collect()
    };
    assert_eq!(ids(""), [empty.id.clone(), "TASK-099".to_string()]);
    assert_eq!(ids("MS-001"), [assigned.id]);
    // An unknown milestone is an error, not an empty list.
    let Err(err) = data::collect_tasks_filtered(
        &cfg,
        &TaskFilter {
            milestone: Some("MS-404"),
            ..TaskFilter::all()
        },
    ) else {
        panic!("an unknown milestone must not filter to an empty list");
    };
    assert!(err.to_string().contains("MS-404"), "{err}");
}

#[test]
fn ambiguous_milestone_titles_need_the_id() {
    let (_tmp, cfg) = repo(false);
    let first = milestone(&cfg);
    let second = milestone(&cfg);
    assert_eq!(second.id, "MS-002");
    let err = new::resolve_milestone(&cfg, "AP3: Integration").unwrap_err();
    assert_eq!(err.exit_code(), 2, "{err}");
    assert_eq!(new::resolve_milestone(&cfg, "ms-2").unwrap(), second.id);
    assert_eq!(new::resolve_milestone(&cfg, &first.id).unwrap(), first.id);
}

#[test]
fn validate_reports_broken_milestone_links_and_statuses() {
    let (_tmp, cfg) = repo(false);
    let m = milestone(&cfg);
    let file = m.path.join("MS-001.md");
    let doc = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, doc.replace("status: planned", "status: bogus")).unwrap();
    std::fs::write(
        cfg.tasks_dir.join("todo/TASK-001-dangling.md"),
        "---\nid: TASK-001\ntitle: Dangling\nstatus: todo\nmilestone: \"[[MS-099]]\"\n---\n\n# Dangling\n",
    )
    .unwrap();
    let issues: Vec<String> = mc::commands::validate::validate_programmatic(&cfg)
        .unwrap()
        .into_iter()
        .map(|i| format!("{} {}: {}", i.check, i.path, i.message))
        .collect();
    assert!(
        issues
            .iter()
            .any(|i| i.contains("TASK-001") && i.contains("MS-099")),
        "{issues:#?}"
    );
    assert!(
        issues
            .iter()
            .any(|i| i.contains("MS-001") && i.contains("bogus")),
        "{issues:#?}"
    );
}

#[test]
fn cli_checks_milestone_flags_before_the_preview() {
    let (_tmp, cfg) = repo(false);
    let run = |args: &[&str]| {
        std::process::Command::new(env!("CARGO_BIN_EXE_mc"))
            .arg("--root")
            .arg(&cfg.root)
            .arg("-y")
            .args(args)
            .output()
            .unwrap()
    };
    for bad in [
        &[
            "new",
            "milestone",
            "Bad",
            "--start-date",
            "2026-03-01",
            "--due-date",
            "2026-02-01",
        ][..],
        &["new", "milestone", "Bad", "--status", "nope"],
        &["new", "milestone", "Bad", "--projects", "PROJ-404"],
        &["new", "milestone", " "],
    ] {
        let out = run(bad);
        assert!(!out.status.success(), "{bad:?}");
        assert!(
            out.stdout.is_empty(),
            "{bad:?}: {}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
    assert!(data::collect_entities(EntityKind::Milestone, &cfg)
        .unwrap()
        .is_empty());
    let out = run(&[
        "new",
        "milestone",
        "Good",
        "--status",
        "Active",
        "--owner",
        "al",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let ms = data::collect_entities(EntityKind::Milestone, &cfg).unwrap();
    assert_eq!(
        frontmatter::get_str(&ms[0].frontmatter, "status"),
        Some("active")
    );
    assert_eq!(
        frontmatter::get_str(&ms[0].frontmatter, "owner"),
        Some("al")
    );
}

#[tokio::test]
async fn task_views_keep_every_filter_and_show_empty_milestones() {
    let (_tmp, cfg) = repo(false);
    milestone(&cfg);
    milestone(&cfg); // MS-002: no tasks
    new::create_task(
        &cfg,
        &TaskInput {
            title: "Assigned".into(),
            status: Some("todo".into()),
            milestone: Some("MS-001".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let app = router(&cfg, &ServeOptions::default());
    let list = html(&app, "/tasks/list?status=todo&milestone=MS-001").await;
    assert!(
        list.contains(r#"href="/tasks?status=todo&amp;milestone=MS-001""#),
        "{list}"
    );
    assert!(!list.contains("&amp;amp;"));
    let board = html(&app, "/tasks?owner=al&milestone=MS-001").await;
    assert!(board.contains(r#"href="/tasks/list?owner=al&amp;milestone=MS-001""#));
    // A milestone without tasks is still a selectable, visibly set filter.
    let empty = html(&app, "/tasks/list?milestone=MS-002").await;
    assert!(
        empty.contains(r#"<option value="MS-002" selected>"#),
        "{empty}"
    );
}

#[tokio::test]
async fn timeline_handles_open_ended_finished_and_filtered_milestones() {
    let (_tmp, cfg) = repo(false);
    new::create_milestone(
        &cfg,
        &MilestoneInput {
            title: "Only start".into(),
            start_date: Some("2026-09-01".into()),
            ..Default::default()
        },
    )
    .unwrap();
    new::create_milestone(
        &cfg,
        &MilestoneInput {
            title: "Shipped long ago".into(),
            status: Some("completed".into()),
            start_date: Some("2020-01-01".into()),
            due_date: Some("2020-02-01".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let app = router(&cfg, &ServeOptions::default());
    let gantt = html(&app, "/milestones").await;
    assert!(gantt.contains("2 work packages"));
    // A start without a deadline draws an open-ended bar, not "No deadline".
    assert!(gantt.contains("gantt-bar is-open"), "{gantt}");
    assert!(gantt.contains("from 2026-09-01, no deadline"));
    // No tasks: no "0%" that reads as "nothing done".
    assert!(gantt.contains("no tasks"));
    assert!(!gantt.contains("<b>0%</b>"));
    // The overview leaves finished milestones out (and so their dates).
    let overview = html(&app, "/").await;
    let chart = overview
        .split("<h2>Milestones</h2>")
        .nth(1)
        .and_then(|rest| rest.split("</section>").next())
        .expect("overview has a milestone chart");
    assert!(chart.contains("Only start"));
    assert!(!chart.contains("Shipped long ago"));
    assert!(
        !chart.contains("2020"),
        "finished work must not stretch the axis"
    );
    let filtered = html(&app, "/milestones?project=PROJ-404").await;
    assert!(filtered.contains("No milestones for this project"));
    assert!(filtered.contains("0 work packages"));
}
