//! End-to-end tests for terminal UX: colors, glyph fallback, did-you-mean
//! suggestions, JSON output and exit codes. Runs the real `mc` binary.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

fn mc(root: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mc"));
    cmd.arg("--root").arg(root);
    for var in [
        "NO_COLOR",
        "CLICOLOR",
        "CLICOLOR_FORCE",
        "MC_ASCII",
        "MC_WIDTH",
    ] {
        cmd.env_remove(var);
    }
    cmd.env("LANG", "en_US.UTF-8")
        .env_remove("LC_ALL")
        .env_remove("LC_CTYPE");
    cmd
}

fn run(root: &Path, args: &[&str]) -> Output {
    mc(root).args(args).output().expect("failed to run mc")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// Standalone repo with one project and three tasks.
fn repo() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let init = Command::new(env!("CARGO_BIN_EXE_mc"))
        .args(["-y", "init", "--name", "UxTest"])
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(init.status.success(), "init failed: {}", stderr(&init));
    let root = tmp.path();
    for args in [
        vec!["-y", "new", "project", "Data Pipeline"],
        vec![
            "-y",
            "new",
            "task",
            "Fix login bug",
            "--project",
            "PROJ-001",
            "--priority",
            "1",
        ],
        vec!["-y", "new", "task", "Write docs", "--status", "todo"],
        vec![
            "-y",
            "new",
            "task",
            "A rather long task title that goes on and on for a while",
        ],
    ] {
        let o = run(root, &args);
        assert!(o.status.success(), "{args:?} failed: {}", stderr(&o));
    }
    tmp
}

#[test]
fn no_color_env_disables_ansi() {
    let tmp = repo();
    let o = mc(tmp.path())
        .env("NO_COLOR", "1")
        .args(["list", "tasks"])
        .output()
        .unwrap();
    assert!(o.status.success());
    assert!(
        !stdout(&o).contains('\x1b'),
        "unexpected ANSI: {}",
        stdout(&o)
    );
}

#[test]
fn color_always_forces_ansi_even_when_piped() {
    let tmp = repo();
    let o = run(tmp.path(), &["--color", "always", "list", "tasks"]);
    assert!(o.status.success());
    assert!(stdout(&o).contains("\x1b["));
    let o = run(tmp.path(), &["--color", "never", "list", "tasks"]);
    assert!(!stdout(&o).contains('\x1b'));
}

#[test]
fn piped_output_is_plain_and_untruncated() {
    let tmp = repo();
    let out = stdout(&run(tmp.path(), &["list", "tasks"]));
    assert!(!out.contains('\x1b'));
    // No decorative glyphs or rules when stdout is not a terminal.
    assert!(!out.contains('◌') && !out.contains('─'), "{out}");
    assert!(out.contains("A rather long task title that goes on and on for a while"));
    // Header plus one row per task, nothing else (stable for awk / wc -l).
    assert_eq!(out.lines().count(), 4, "{out}");
    assert!(out.lines().next().unwrap().contains("ID"));
}

#[test]
fn every_piped_command_is_plain_ascii() {
    let tmp = repo();
    // Give `show` a checklist and a quote so its markdown rendering is exercised.
    let task = tmp.path().join("tasks/todo/TASK-002-write-docs.md");
    let content = std::fs::read_to_string(&task).unwrap();
    std::fs::write(
        &task,
        format!("{content}\n- [ ] open\n- [x] closed\n> quoted\n"),
    )
    .unwrap();
    assert!(
        run(tmp.path(), &["task", "move", "TASK-002", "in-progress"])
            .status
            .success()
    );
    for args in [
        vec!["status"],
        vec!["list", "tasks"],
        vec!["task", "board"],
        vec!["task", "next"],
        vec!["show", "TASK-002"],
        vec!["show", "PROJ-001"],
        vec!["validate"],
        vec!["task", "move", "TASK-003", "done"],
        vec!["-y", "new", "task", "Piped"],
    ] {
        let o = run(tmp.path(), &args);
        assert!(o.status.success(), "{args:?}: {}", stderr(&o));
        let out = stdout(&o);
        assert!(
            out.is_ascii(),
            "{args:?} printed non-ASCII when piped:\n{out}"
        );
        assert!(!out.contains('\x1b'), "{args:?} printed colors when piped");
    }
}

#[test]
fn width_override_truncates_to_fit() {
    let tmp = repo();
    let o = mc(tmp.path())
        .env("MC_WIDTH", "50")
        .args(["list", "tasks"])
        .output()
        .unwrap();
    let out = stdout(&o);
    for line in out.lines() {
        assert!(line.chars().count() <= 50, "too wide: {line:?}");
    }
    assert!(out.contains("TASK-003"));
}

#[test]
fn empty_result_keeps_stdout_clean_when_piped() {
    let tmp = repo();
    let o = run(tmp.path(), &["list", "tasks", "--status", "review"]);
    assert!(o.status.success());
    assert!(stdout(&o).is_empty(), "{}", stdout(&o));
    assert!(stderr(&o).contains("No tasks"));
}

#[test]
fn task_move_suggests_closest_status() {
    let tmp = repo();
    let o = run(tmp.path(), &["task", "move", "TASK-001", "doen"]);
    assert_eq!(o.status.code(), Some(2));
    let err = stderr(&o);
    assert!(err.contains("not a valid task status"), "{err}");
    assert!(err.contains("did you mean 'done'"), "{err}");
}

#[test]
fn task_move_accepts_loose_id_and_status() {
    let tmp = repo();
    let o = run(tmp.path(), &["task", "move", "1", "WIP"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let o = run(tmp.path(), &["--json", "show", "task-1"]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["status"], "in-progress");
    assert_eq!(v["id"], "TASK-001");
    assert_eq!(v["_kind"], "task");
}

#[test]
fn task_move_to_same_status_is_a_noop() {
    let tmp = repo();
    let o = run(tmp.path(), &["task", "move", "TASK-002", "todo"]);
    assert!(o.status.success());
    assert!(stdout(&o).contains("already"), "{}", stdout(&o));
}

#[test]
fn show_unknown_id_suggests_nearby() {
    let tmp = repo();
    let o = run(tmp.path(), &["show", "TASK-010"]);
    assert_eq!(o.status.code(), Some(1));
    let err = stderr(&o);
    assert!(err.contains("task TASK-010 not found"), "{err}");
    assert!(err.contains("did you mean TASK-001"), "{err}");
}

#[test]
fn show_unknown_prefix_suggests_prefix() {
    let tmp = repo();
    let o = run(tmp.path(), &["show", "TSK-2"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        stderr(&o).contains("did you mean TASK-002"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn list_status_typo_is_usage_error() {
    let tmp = repo();
    let o = run(tmp.path(), &["list", "tasks", "--status", "in-prog"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        stderr(&o).contains("did you mean 'in-progress'"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn list_singular_alias_works() {
    let tmp = repo();
    let o = run(tmp.path(), &["ls", "task"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("TASK-001"));
}

#[test]
fn json_list_is_an_array_of_entities() {
    let tmp = repo();
    let o = run(tmp.path(), &["list", "tasks", "--json"]);
    assert!(o.status.success());
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0]["id"], "TASK-001");
    // Wiki-links are stripped like in data/*.json.
    assert_eq!(arr[0]["projects"][0], "PROJ-001");
    assert!(arr[0]["_source"].as_str().unwrap().ends_with(".md"));
}

#[test]
fn json_status_board_next_and_validate() {
    let tmp = repo();
    let status: serde_json::Value =
        serde_json::from_slice(&run(tmp.path(), &["status", "--json"]).stdout).unwrap();
    assert_eq!(status["counts"]["tasks"]["total"], 3);
    assert_eq!(status["repo"]["mode"], "standalone");

    let board: serde_json::Value =
        serde_json::from_slice(&run(tmp.path(), &["task", "board", "--json"]).stdout).unwrap();
    assert!(board["columns"].as_array().unwrap().len() >= 4);

    let next: serde_json::Value =
        serde_json::from_slice(&run(tmp.path(), &["task", "next", "--json"]).stdout).unwrap();
    // todo beats backlog even at lower priority.
    assert_eq!(next[0]["id"], "TASK-002");

    let o = run(tmp.path(), &["validate", "--json"]);
    assert!(o.status.success());
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["ok"], true);
}

#[test]
fn validate_failure_exits_1_with_json_issues() {
    let tmp = repo();
    let task = tmp.path().join("tasks/todo/TASK-002-write-docs.md");
    let content = std::fs::read_to_string(&task).unwrap();
    std::fs::write(&task, content.replace("status: todo", "status: bogus")).unwrap();

    let o = run(tmp.path(), &["validate", "--json"]);
    assert_eq!(o.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert!(v["count"].as_u64().unwrap() >= 1);

    let o = run(tmp.path(), &["validate"]);
    assert_eq!(o.status.code(), Some(1));
    // Paths are shown relative to the repo root.
    assert!(stdout(&o).contains("tasks/todo/TASK-002-write-docs.md"));
    assert!(!stdout(&o).contains(&tmp.path().display().to_string()));
}

#[test]
fn json_errors_are_json_on_stderr() {
    let tmp = repo();
    let o = run(tmp.path(), &["--json", "show", "TASK-999"]);
    assert_eq!(o.status.code(), Some(1));
    let v: serde_json::Value = serde_json::from_slice(&o.stderr).unwrap();
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not found"));

    let o = run(tmp.path(), &["--json", "mcp"]);
    assert_eq!(o.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_slice(&o.stderr).unwrap();
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not supported"));
}

#[test]
fn ascii_mode_avoids_unicode_glyphs() {
    let tmp = repo();
    let o = mc(tmp.path())
        .env("MC_ASCII", "1")
        .args(["task", "move", "TASK-003", "done"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.is_ascii(), "non-ASCII output: {out}");
    assert!(out.contains("backlog -> done"), "{out}");
}

#[test]
fn export_requires_an_unambiguous_match() {
    let tmp = repo();
    for name in ["Acme Inc", "Acme Labs"] {
        assert!(run(tmp.path(), &["-y", "new", "customer", name])
            .status
            .success());
    }
    // A single letter used to match the first customer containing it.
    let o = run(tmp.path(), &["export", "customer", "a"]);
    assert_eq!(o.status.code(), Some(1));

    let o = run(tmp.path(), &["export", "customer", "acme"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("matches 2 customers"));

    let o = run(tmp.path(), &["export", "customer", "cust-2", "--json"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["customer"], "CUST-002-acme-labs");
}

#[test]
fn embedded_mode_rejects_customers_with_usage_code() {
    let tmp = TempDir::new().unwrap();
    let init = Command::new(env!("CARGO_BIN_EXE_mc"))
        .args(["-y", "init", "--embedded"])
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(init.status.success());
    let o = run(tmp.path(), &["list", "customers"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("embedded mode"));
}

#[test]
fn standalone_kind_missing_from_paths_is_not_blamed_on_embedded_mode() {
    let tmp = repo();
    let cfg_path = tmp.path().join("config").join("config.yml");
    let cfg = std::fs::read_to_string(&cfg_path).unwrap();
    std::fs::write(&cfg_path, cfg.replace("  customers: customers/\n", "")).unwrap();
    let o = run(tmp.path(), &["list", "customers"]);
    assert_eq!(o.status.code(), Some(2));
    let err = stderr(&o);
    assert!(err.contains("not enabled in this repo"), "{err}");
    assert!(err.contains("customers: customers/"), "{err}");
    assert!(!err.contains("embedded"), "{err}");
}

#[cfg(unix)]
#[test]
fn closed_pipe_does_not_panic() {
    let tmp = repo();
    // Enough rows to overflow the pipe buffer.
    let todo = tmp.path().join("tasks/todo");
    for i in 100..1600 {
        let body = format!(
            "---\nid: TASK-{i}\ntitle: Generated task number {i} with some padding text\nstatus: backlog\npriority: 3\n---\n"
        );
        std::fs::write(todo.join(format!("TASK-{i}-generated.md")), body).unwrap();
    }
    let mut child = mc(tmp.path())
        .args(["list", "tasks"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut buf = [0u8; 16];
    child.stdout.as_mut().unwrap().read_exact(&mut buf).unwrap();
    drop(child.stdout.take());
    let mut err = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut err)
        .unwrap();
    let status = child.wait().unwrap();
    assert!(!err.contains("panicked"), "stderr: {err}");
    assert_ne!(status.code(), Some(101), "mc panicked");
}

#[test]
fn check_lists_and_ticks_checklist_items() {
    let tmp = repo();
    let task = tmp.path().join("tasks/todo/TASK-002-write-docs.md");
    let content = std::fs::read_to_string(&task).unwrap();
    let (fm, _) = mc::frontmatter::split_frontmatter(&content).unwrap();
    let doc = format!("---\n{fm}\n---\n- [ ] Outline\n- [x] Draft\n\n```\n- [ ] decoy\n```\n");
    std::fs::write(&task, &doc).unwrap();

    let o = run(tmp.path(), &["check", "task-2"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("1 of 2 done"), "{out}");
    assert!(
        out.contains("1  [ ] Outline") && out.contains("2  [x] Draft"),
        "{out}"
    );
    assert!(!out.contains("decoy") && !out.contains('\x1b'), "{out}");

    let o = run(tmp.path(), &["check", "TASK-002", "1"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("Checked TASK-002 #1 Outline"));
    assert_eq!(
        std::fs::read_to_string(&task).unwrap(),
        doc.replace("- [ ] Outline", "- [x] Outline")
    );
    // Ticking again changes nothing; --uncheck restores the original bytes.
    let o = run(tmp.path(), &["check", "TASK-002", "1"]);
    assert!(stdout(&o).contains("already checked"));
    let o = run(
        tmp.path(),
        &["--json", "check", "TASK-002", "1", "--uncheck"],
    );
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["changed"], true);
    assert_eq!(v["item"]["checked"], false);
    assert_eq!(std::fs::read_to_string(&task).unwrap(), doc);

    let o = run(tmp.path(), &["check", "TASK-002", "5"]);
    assert_eq!(o.status.code(), Some(1));
    // Shared sentence-style messages follow the CLI's error style.
    assert!(
        stderr(&o).contains("error: there is no checklist item 5\n"),
        "{}",
        stderr(&o)
    );
    assert!(stderr(&o).contains("1 to 2"), "{}", stderr(&o));
}

#[test]
fn check_never_prints_control_characters_from_the_file() {
    let tmp = repo();
    let task = tmp.path().join("tasks/todo/TASK-002-write-docs.md");
    let content = std::fs::read_to_string(&task).unwrap();
    let (fm, _) = mc::frontmatter::split_frontmatter(&content).unwrap();
    let doc = format!("---\n{fm}\n---\n- [ ] top \x1b]52;c;ZXZpbA==\x07check \x1b[2J end\n");
    std::fs::write(&task, &doc).unwrap();
    for args in [
        vec!["check", "TASK-002"],
        vec!["check", "TASK-002", "1"],
        vec!["--color", "always", "check", "TASK-002", "1"],
        vec!["--color", "always", "check", "TASK-002"],
    ] {
        let o = run(tmp.path(), &args);
        assert!(o.status.success(), "{args:?}: {}", stderr(&o));
        let out = stdout(&o);
        assert!(
            out.contains("top ]52;c;ZXZpbA==check [2J end"),
            "{args:?}: {out:?}"
        );
        assert!(!out.contains('\x07') && !out.contains("\x1b]52") && !out.contains("\x1b[2J"));
    }
}

#[test]
fn comment_appends_from_args_and_stdin() {
    let tmp = repo();
    let task = tmp.path().join("tasks/todo/TASK-002-write-docs.md");
    let before = std::fs::read_to_string(&task).unwrap();

    let o = run(
        tmp.path(),
        &["comment", "task-2", "First take", "--author", "Jane Doe"],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("Commented on TASK-002 as Jane Doe"));

    let mut child = mc(tmp.path())
        .args(["--json", "comment", "TASK-002", "-", "--author", "Bot"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        let mut stdin = child.stdin.take().unwrap();
        stdin
            .write_all(b"From stdin\n\n## Not a section\n")
            .unwrap();
    }
    let o = child.wait_with_output().unwrap();
    assert!(o.status.success());
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["count"], 2);
    assert_eq!(v["comment"]["body"], "From stdin\n\n#### Not a section");

    let after = std::fs::read_to_string(&task).unwrap();
    assert!(after.starts_with(before.trim_end()));
    assert_eq!(after.matches("## Comments").count(), 1);
    assert!(after.contains(" · Jane Doe\n\nFirst take\n\n### "));

    // An unclosed code fence is closed, so the next comment stays separate.
    let o = run(
        tmp.path(),
        &["--json", "comment", "TASK-002", "```\nlog output"],
    );
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["comment"]["body"], "```\nlog output\n```");
    let o = run(
        tmp.path(),
        &["--json", "comment", "TASK-002", "Separate", "--author", "B"],
    );
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["count"], 4);
    assert_eq!(v["comment"]["author"], "B");

    // A hand-edited file whose last comment leaves a fence open would swallow
    // a new comment: it is refused and the file left alone.
    let broken = format!("{}\n\n### 2026-01-01 · X\n\n```\nopen\n", after.trim_end());
    std::fs::write(&task, &broken).unwrap();
    let o = run(tmp.path(), &["comment", "TASK-002", "lost?"]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert_eq!(std::fs::read_to_string(&task).unwrap(), broken);
    std::fs::write(&task, &after).unwrap();

    // Only tasks and meetings take comments; empty text is a usage error.
    let o = run(tmp.path(), &["comment", "PROJ-001", "hi"]);
    assert_eq!(o.status.code(), Some(2));
    let o = run(tmp.path(), &["comment", "TASK-002", "  "]);
    assert_eq!(o.status.code(), Some(2));
}

/// Give TASK-002 a body with a table, a checklist and references.
fn rich_task(root: &Path) -> String {
    let task = root.join("tasks/todo/TASK-002-write-docs.md");
    let content = std::fs::read_to_string(&task).unwrap();
    let body = "\n## Plan\n\n| Step | Owner |\n|------|------:|\n| Draft | [[TASK-001]] |\n\n- [ ] open\n- [x] closed\n\nSee PROJ-001 and [[TASK-404|gone]].\n";
    std::fs::write(&task, format!("{content}{body}")).unwrap();
    body.to_string()
}

#[test]
fn piped_show_prints_markdown_source() {
    let tmp = repo();
    rich_task(tmp.path());
    let piped = run(tmp.path(), &["show", "TASK-002"]);
    assert!(piped.status.success(), "{}", stderr(&piped));
    let out = stdout(&piped);
    // The body is printed as written (indented), with no rendering or footer.
    for line in [
        "  ## Plan",
        "  | Step | Owner |",
        "  | Draft | [[TASK-001]] |",
        "  - [ ] open",
    ] {
        assert!(
            out.lines().any(|l| l == line),
            "missing {line:?} in:\n{out}"
        );
    }
    assert!(!out.contains("LINKS") && !out.contains('\x1b'), "{out}");
    // --raw prints the same on a terminal; here it must not change anything.
    let raw = run(tmp.path(), &["show", "TASK-002", "--raw"]);
    assert_eq!(stdout(&raw), out);
    // --json keeps the source body.
    let json = run(tmp.path(), &["--json", "show", "TASK-002"]);
    let v: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert!(v["_body"].as_str().unwrap().contains("| Step | Owner |"));
}

#[test]
fn rendered_show_lays_out_markdown_and_lists_links() {
    let tmp = repo();
    rich_task(tmp.path());
    let cfg = mc::config::load_config(tmp.path(), mc::config::RepoMode::Standalone).unwrap();
    let entity = mc::cli::suggest::find_entity("TASK-002", &cfg, None).unwrap();
    let task1 = mc::cli::suggest::find_entity("TASK-001", &cfg, None).unwrap();
    let catalog = mc::commands::show::load_catalog(&cfg);

    // Tests don't run on a terminal, so the view uses ASCII glyphs.
    let lines = mc::commands::show::rendered_view(&entity, &cfg, &catalog, false);
    let text = lines.join("\n");
    assert!(!text.contains("\x1b]8;;"), "{text}");
    for line in [
        "  Plan",
        "  +-------+------------------------+",
        "  | Step  |                  Owner |",
        "  +=======+========================+",
        "  | Draft | TASK-001 Fix login bug |",
        "  [ ] open",
        "  [x] closed",
        "  See PROJ-001 and gone.",
        "  LINKS",
    ] {
        assert!(
            lines.iter().any(|l| console_plain(l) == line),
            "missing {line:?} in:\n{text}"
        );
    }
    // The footer gives each linked entity's path, so it works without OSC 8.
    let rel = task1
        .source_path
        .strip_prefix(tmp.path())
        .unwrap()
        .display()
        .to_string();
    assert!(text.contains(&rel), "{text}");
    assert!(!text.contains("TASK-404  "), "unknown IDs are not listed");

    // With hyperlinks, references point at the files on disk.
    let linked = mc::commands::show::rendered_view(&entity, &cfg, &catalog, true).join("\n");
    assert!(linked.contains("\x1b]8;;file:///"), "{linked}");
    assert!(
        linked.contains("TASK-001-fix-login-bug.md\x1b\\"),
        "{linked}"
    );
}

#[test]
fn rendered_show_has_checklist_progress_and_a_comments_section() {
    let tmp = repo();
    rich_task(tmp.path());
    let o = run(
        tmp.path(),
        &[
            "comment",
            "TASK-002",
            "Blocked on [[TASK-001]]\n\n- [ ] in a comment",
            "--author",
            "Jane Doe",
        ],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let cfg = mc::config::load_config(tmp.path(), mc::config::RepoMode::Standalone).unwrap();
    let entity = mc::cli::suggest::find_entity("TASK-002", &cfg, None).unwrap();
    let catalog = mc::commands::show::load_catalog(&cfg);
    let lines: Vec<String> = mc::commands::show::rendered_view(&entity, &cfg, &catalog, false)
        .iter()
        .map(|l| console_plain(l))
        .collect();
    let text = lines.join("\n");
    // The template criterion, open and closed; not the comment's box.
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("  checklist  ") && l.ends_with("1 of 3 done")),
        "{text}"
    );
    // Comments come after the notes as their own section, not as `## Comments`.
    assert!(!text.contains("Comments\n  --"), "{text}");
    let at = lines
        .iter()
        .position(|l| l == "  COMMENTS  1")
        .expect(&text);
    assert!(lines[at + 1].starts_with("  - Jane Doe  "), "{text}");
    assert_eq!(lines[at + 2], "    Blocked on TASK-001 Fix login bug");
    assert!(
        lines.contains(&"    [ ] in a comment".to_string()),
        "{text}"
    );
    assert!(at < lines.iter().position(|l| l == "  LINKS").unwrap());
}

/// Drop ANSI colours (the environment may force them).
fn console_plain(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(unix)]
#[test]
fn show_open_runs_the_editor() {
    let tmp = repo();
    let o = mc(tmp.path())
        .env("VISUAL", "echo opened")
        .args(["show", "TASK-002", "--open"])
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.starts_with("opened "), "{out}");
    assert!(out.trim_end().ends_with("TASK-002-write-docs.md"), "{out}");
}

#[test]
fn init_rejects_embedded_with_project_and_skips_prompts_when_piped() {
    let tmp = TempDir::new().unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_mc"))
        .args(["-y", "init", "--embedded", "--project"])
        .arg(tmp.path())
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    assert!(!tmp.path().join(".mc").exists());

    // No -y, stdin not a terminal: defaults are taken without fake prompts.
    let dir = tmp.path().join("acme");
    std::fs::create_dir(&dir).unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_mc"))
        .arg("init")
        .arg(&dir)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    assert!(
        !out.contains("[Y/n]") && !out.contains("Repository name"),
        "{out}"
    );
    let config = std::fs::read_to_string(dir.join("config/config.yml")).unwrap();
    assert!(config.contains("name: acme"), "{config}");
}

/// Frontmatter values with terminal escapes (window title, clear screen,
/// OSC 8 link, bell) come out without control characters everywhere.
#[test]
fn file_values_never_reach_the_terminal_as_escape_sequences() {
    let tmp = repo();
    let evil = "Evil \x1b]0;PWNED\x07 \x1b[2J cleared \x1b]8;;https://evil.example\x1b\\click";
    std::fs::write(
        tmp.path().join("tasks/todo/TASK-009-evil.md"),
        format!(
            "---\nid: TASK-009\ntitle: \"{}\"\nstatus: todo\npriority: 1\nowner: \"Own\\e[31mer\"\n---\n",
            evil.replace('\x1b', "\\e").replace('\x07', "\\a").replace("\\e\\", "\\e\\\\")
        ),
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("tasks/todo/TASK-010-bad-status.md"),
        "---\nid: TASK-010\ntitle: Bad\nstatus: \"\\e]0;VPWN\\a\"\n---\n",
    )
    .unwrap();
    // A meeting today for the "Coming up" section of `mc status`.
    std::fs::write(
        tmp.path().join("meetings/MTG-009-evil.md"),
        format!(
            "---\nid: MTG-009\ntitle: \"Mtg \\e]0;PWNED\\a\"\ndate: {}\ntime: \"10:00\\e[2J\"\nstatus: scheduled\n---\n",
            chrono::Local::now().format("%Y-%m-%d")
        ),
    )
    .unwrap();
    let commands: [&[&str]; 6] = [
        &["list", "tasks"],
        &["task", "board"],
        &["task", "next", "-n", "5"],
        &["status"],
        &["validate"],
        &["-y", "task", "move", "9", "in-progress"],
    ];
    for args in commands {
        let o = run(tmp.path(), &[&["--color", "never"], args].concat());
        let out = stdout(&o);
        assert!(
            !out.contains('\x1b') && !out.contains('\x07'),
            "{args:?}: {out:?}"
        );
        if args[0] != "validate" {
            assert!(out.contains("Evil ]0;PWNED"), "{args:?}: {out}");
        }
        if args[0] == "status" {
            assert!(out.contains("Mtg ]0;PWNED"), "{out}");
        }
        // With colours on, only the CLI's own SGR styling remains.
        let o = run(tmp.path(), &[&["--color", "always"], args].concat());
        let out = stdout(&o);
        assert!(
            !out.contains("\x1b]") && !out.contains("\x1b[2J") && !out.contains('\x07'),
            "{args:?}: {out:?}"
        );
    }
}

#[test]
fn id_filters_accept_loose_ids_and_reject_unknown_ones() {
    let tmp = repo();
    let o = run(tmp.path(), &["list", "tasks", "--project", "proj-1"]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("Fix login bug"), "{}", stdout(&o));

    let o = run(tmp.path(), &["task", "board", "--project", "1"]);
    assert!(stdout(&o).contains("TASK-001"), "{}", stdout(&o));
    let o = run(
        tmp.path(),
        &["--json", "task", "next", "--project", "Proj1"],
    );
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v[0]["id"], "TASK-001");

    // An ID that doesn't exist fails with a suggestion instead of an empty list.
    let o = run(tmp.path(), &["list", "tasks", "--project", "PROJ-002"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stderr(&o).contains("did you mean PROJ-001"),
        "{}",
        stderr(&o)
    );
    // So does an ID of the wrong kind.
    let o = run(tmp.path(), &["list", "tasks", "--project", "TASK-001"]);
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
    let o = run(tmp.path(), &["list", "contacts", "--customer", "cust-1"]);
    assert_eq!(o.status.code(), Some(1), "{}", stderr(&o));
}

#[test]
fn list_tasks_open_overdue_and_sort() {
    let tmp = repo();
    for args in [
        vec!["-y", "new", "task", "Late one", "--due-date", "2020-01-02"],
        vec![
            "-y",
            "new",
            "task",
            "Later one",
            "--due-date",
            "2020-01-01",
            "--priority",
            "4",
        ],
    ] {
        assert!(run(tmp.path(), &args).status.success());
    }
    assert!(run(tmp.path(), &["-y", "task", "move", "3", "done"])
        .status
        .success());
    let ids = |args: &[&str]| -> Vec<String> {
        let o = run(tmp.path(), &[&["--json", "list", "tasks"], args].concat());
        assert!(o.status.success(), "{args:?}: {}", stderr(&o));
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
        v.as_array()
            .unwrap()
            .iter()
            .map(|t| t["id"].as_str().unwrap().to_string())
            .collect()
    };
    // The default listing is unchanged: every task, by ID.
    assert_eq!(
        ids(&[]),
        ["TASK-001", "TASK-002", "TASK-003", "TASK-004", "TASK-005"]
    );
    assert_eq!(
        ids(&["--open"]),
        ["TASK-001", "TASK-002", "TASK-004", "TASK-005"]
    );
    assert_eq!(ids(&["--overdue"]), ["TASK-004", "TASK-005"]);
    assert_eq!(
        ids(&["--overdue", "--sort", "due"]),
        ["TASK-005", "TASK-004"]
    );
    assert_eq!(
        ids(&["--open", "--sort", "priority"]),
        ["TASK-001", "TASK-004", "TASK-002", "TASK-005"]
    );
    let o = run(tmp.path(), &["list", "tasks", "--sort", "size"]);
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn meetings_are_listed_by_date() {
    let tmp = repo();
    for (title, date) in [("Later", "2026-03-01"), ("Earlier", "2026-01-15")] {
        let o = run(tmp.path(), &["-y", "new", "meeting", title, "--date", date]);
        assert!(o.status.success(), "{}", stderr(&o));
    }
    let out = stdout(&run(tmp.path(), &["list", "meetings"]));
    let earlier = out.find("Earlier").unwrap();
    assert!(earlier < out.find("Later").unwrap(), "{out}");
}

#[test]
fn out_of_range_priority_is_a_usage_error() {
    let tmp = repo();
    for args in [
        &["list", "tasks", "--priority", "7"][..],
        &["list", "tasks", "--priority", "0"],
        &["-y", "new", "task", "x", "--priority", "9"],
    ] {
        let o = run(tmp.path(), args);
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", stderr(&o));
    }
}

#[test]
fn json_mode_reports_argument_errors_as_json() {
    let tmp = repo();
    let o = run(tmp.path(), &["--json", "list", "tasks", "--bogus"]);
    assert_eq!(o.status.code(), Some(2));
    let v: serde_json::Value = serde_json::from_slice(&o.stderr).unwrap();
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unexpected argument '--bogus'"));
    assert_eq!(v["error"]["exit_code"], 2);
    // Help is still help.
    let o = run(tmp.path(), &["--json", "list", "--help"]);
    assert!(o.status.success());
    assert!(stdout(&o).contains("Usage:"));
}

#[test]
fn color_flag_applies_to_help_and_argument_errors() {
    let tmp = repo();
    let o = run(tmp.path(), &["--color", "always", "show", "--help"]);
    assert!(stdout(&o).contains('\x1b'));
    let o = mc(tmp.path())
        .env("CLICOLOR_FORCE", "1")
        .args(["--color=never", "show", "--help"])
        .output()
        .unwrap();
    assert!(!stdout(&o).contains('\x1b'), "{}", stdout(&o));
    let o = mc(tmp.path())
        .env("CLICOLOR_FORCE", "1")
        .args(["--color", "never", "show", "x", "extra"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(!stderr(&o).contains('\x1b'), "{}", stderr(&o));
}

#[test]
fn help_lists_command_options_before_global_ones() {
    let tmp = repo();
    let out = stdout(&run(tmp.path(), &["new", "task", "--help"]));
    let own = out.find("--project").unwrap();
    let global = out.find("Global options:").unwrap();
    assert!(own < global, "{out}");
    assert!(out[global..].contains("--root") && out[global..].contains("--json"));
    assert!(!out[..global].contains("--root"), "{out}");
}

#[test]
fn completions_need_no_repo() {
    let tmp = TempDir::new().unwrap();
    for shell in ["bash", "zsh", "fish"] {
        let o = Command::new(env!("CARGO_BIN_EXE_mc"))
            .args(["completions", shell])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        assert!(o.status.success(), "{shell}: {}", stderr(&o));
        assert!(stdout(&o).contains("mc"), "{shell}");
    }
    let o = Command::new(env!("CARGO_BIN_EXE_mc"))
        .args(["completions", "tcsh"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn api_serve_usage_mistakes_exit_2() {
    let tmp = repo();
    for args in [
        &["api", "serve", "--log-format", "xml"][..],
        &["api", "serve", "--bind", "nope"],
        &["api", "serve", "--port", "5321"],
    ] {
        let o = run(tmp.path(), args);
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", stderr(&o));
    }
    let o = mc(tmp.path())
        .args(["api", "hash-token"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2), "{}", stderr(&o));
}

#[test]
fn comment_without_text_needs_a_terminal_for_the_editor() {
    let tmp = repo();
    let o = mc(tmp.path())
        .args(["comment", "TASK-002"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("no comment text"), "{}", stderr(&o));
}

#[test]
fn json_paths_are_repo_relative() {
    let tmp = repo();
    let root = tmp.path().display().to_string();
    let o = run(
        tmp.path(),
        &["--json", "-y", "task", "move", "2", "in-progress"],
    );
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["path"], "tasks/todo/TASK-002-write-docs.md");
    let o = run(
        tmp.path(),
        &["--json", "-y", "task", "move", "2", "in-progress"],
    );
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["path"], "tasks/todo/TASK-002-write-docs.md");

    let o = mc(tmp.path())
        .env("VISUAL", "true")
        .args(["--json", "show", "TASK-002", "--open"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["path"], "tasks/todo/TASK-002-write-docs.md");

    assert!(run(tmp.path(), &["-y", "new", "customer", "Acme"])
        .status
        .success());
    let o = run(tmp.path(), &["--json", "export", "customer", "1"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert!(!v["path"].as_str().unwrap().starts_with(&root), "{v}");
}

#[test]
fn task_set_changes_fields_like_the_dashboard() {
    let tmp = repo();
    let root = tmp.path();
    let o = run(
        root,
        &[
            "--json",
            "task",
            "set",
            "task-2",
            "--priority",
            "2",
            "--owner",
            "alice",
            "--due-date",
            "2026-10-31",
            "--tags",
            "docs, web",
            "--project",
            "proj-1",
            "--status",
            "done",
        ],
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["id"], "TASK-002");
    assert_eq!(
        v["changed"],
        serde_json::json!(["priority", "owner", "due_date", "projects", "tags", "status"])
    );
    assert_eq!(v["new_status"], "done");
    assert_eq!(v["path"], "tasks/done/TASK-002-write-docs.md");

    let o = run(root, &["--json", "show", "TASK-002"]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["owner"], "alice");
    assert_eq!(v["priority"], 2);
    assert_eq!(v["projects"], serde_json::json!(["PROJ-001"]));
    assert_eq!(v["tags"], serde_json::json!(["docs", "web"]));

    // Clearing, then nothing left to change.
    let o = run(root, &["task", "set", "2", "--owner", "", "--due-date", ""]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("(cleared)"), "{}", stdout(&o));
    let o = run(root, &["task", "set", "2", "--owner", ""]);
    assert!(stdout(&o).contains("nothing changed"), "{}", stdout(&o));

    // Usage mistakes exit 2 and leave the file alone.
    let file = root.join("tasks/done/TASK-002-write-docs.md");
    let before = std::fs::read_to_string(&file).unwrap();
    for args in [
        vec!["task", "set", "2"],
        vec!["task", "set", "2", "--priority", "7"],
        vec!["task", "set", "2", "--due-date", "2026-1-5"],
        vec!["task", "set", "2", "--status", "doen"],
        vec!["task", "set", "2", "--sprint", "SPR-009"],
        vec!["task", "set", "PROJ-001", "--owner", "x"],
    ] {
        let o = run(root, &args);
        assert_ne!(o.status.code(), Some(0), "{args:?}");
        assert_ne!(o.status.code(), Some(101), "{args:?} panicked");
    }
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
}
