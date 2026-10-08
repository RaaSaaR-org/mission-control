//! Data-layer regressions seen through the public API: what mc writes,
//! `mc validate` accepts, and what lookups report.

use mc::cli::suggest;
use mc::commands::{init, new, task, validate};
use mc::config::{self, RepoMode, ResolvedConfig};
use mc::data;
use mc::entity::EntityKind;
use mc::error::McError;
use tempfile::TempDir;

fn repo() -> (TempDir, ResolvedConfig) {
    let tmp = TempDir::new().unwrap();
    init::run(tmp.path(), false, false, Some("T"), false, true).unwrap();
    let cfg = config::load_config(tmp.path(), RepoMode::Standalone).unwrap();
    (tmp, cfg)
}

fn issues(cfg: &ResolvedConfig) -> Vec<String> {
    validate::validate_programmatic(cfg)
        .unwrap()
        .into_iter()
        .map(|i| format!("{} {}: {}", i.check, i.path, i.message))
        .collect()
}

#[test]
fn four_digit_ids_validate_and_sort_numerically() {
    let (_tmp, cfg) = repo();
    let todo = cfg.tasks_dir.join("todo");
    std::fs::write(
        todo.join("TASK-999-last.md"),
        "---\nid: TASK-999\ntitle: Last\nstatus: todo\n---\n",
    )
    .unwrap();
    let created = new::create_task(&cfg, &new::TaskInput::new("Thousandth")).unwrap();
    assert_eq!(created.id, "TASK-1000");
    assert_eq!(issues(&cfg), Vec::<String>::new());
    let ids: Vec<String> = data::collect_tasks(&cfg)
        .unwrap()
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(ids, ["TASK-999", "TASK-1000"]);
}

#[test]
fn custom_active_status_is_filed_in_todo_and_validates() {
    let (tmp, _) = repo();
    let path = tmp.path().join("config/config.yml");
    let yml = std::fs::read_to_string(&path).unwrap();
    let yml = yml.replacen(
        "  task:\n    - backlog",
        "  task:\n    - backlog\n    - blocked",
        1,
    );
    std::fs::write(&path, yml).unwrap();
    let cfg = config::load_config(tmp.path(), RepoMode::Standalone).unwrap();
    assert!(cfg.statuses.task.iter().any(|s| s == "blocked"));

    let mut input = new::TaskInput::new("Waiting on vendor");
    input.status = Some("blocked".into());
    let created = new::create_task(&cfg, &input).unwrap();
    assert!(created.path.starts_with(cfg.tasks_dir.join("todo")));
    assert_eq!(issues(&cfg), Vec::<String>::new());

    task::move_task_programmatic(&cfg, &created.id, "done", None).unwrap();
    task::move_task_programmatic(&cfg, &created.id, "blocked", None).unwrap();
    assert!(created.path.is_file());
    assert_eq!(issues(&cfg), Vec::<String>::new());
}

#[test]
fn broken_frontmatter_is_named_not_reported_missing() {
    let (_tmp, cfg) = repo();
    new::create_task(&cfg, &new::TaskInput::new("Ship it")).unwrap();
    std::fs::write(
        cfg.tasks_dir.join("todo/TASK-010-broken.md"),
        "---\nid: TASK-010\ntitle: \"unterminated\nstatus: todo\n---\n",
    )
    .unwrap();
    let Err(err) = suggest::find_entity("TASK-10", &cfg, Some(EntityKind::Task)) else {
        panic!("broken file was found");
    };
    assert!(matches!(err, McError::Usage { .. }), "{err}");
    let msg = err.to_string();
    assert!(
        msg.starts_with("tasks/todo/TASK-010-broken.md has invalid frontmatter"),
        "{msg}"
    );

    // validate passes the parser's position on.
    let yaml: Vec<String> = issues(&cfg)
        .into_iter()
        .filter(|i| i.starts_with("yaml-validity"))
        .collect();
    assert_eq!(yaml.len(), 1, "{yaml:?}");
    // Both name the same file line (the title is on line 3).
    let at = "quoted scalar at line 3 column 8";
    assert!(yaml[0].contains(at), "{yaml:?}");
    assert!(msg.contains(at), "{msg}");
}
