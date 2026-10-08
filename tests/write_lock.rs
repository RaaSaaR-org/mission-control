//! Cross-process write safety and scriptable creation: runs the real `mc`
//! binary several times at once against one repo.

use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn mc(root: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_mc"));
    cmd.arg("--root").arg(root);
    cmd
}

fn repo() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let init = Command::new(env!("CARGO_BIN_EXE_mc"))
        .args(["-y", "init", "--name", "LockTest"])
        .arg(tmp.path())
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    tmp
}

fn json(o: &Output) -> serde_json::Value {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).expect("stdout is JSON")
}

#[test]
fn parallel_processes_get_unique_ids() {
    let tmp = repo();
    let root = tmp.path();
    let children: Vec<_> = (0..8)
        .map(|i| {
            mc(root)
                .args(["--json", "-y", "new", "task", &format!("Race {i}")])
                .spawn()
                .unwrap()
        })
        .collect();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    let list = json(&mc(root).args(["--json", "list", "tasks"]).output().unwrap());
    let mut ids: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 8, "{ids:?}");
    // The lock file lives in .git/, so the work tree stays clean.
    assert!(root.join(".git").join("mc-write.lock").is_file());
    assert!(!root.join(".mc-write.lock").exists());
}

#[test]
fn new_with_json_prints_id_and_relative_path() {
    let tmp = repo();
    let root = tmp.path();
    let out = json(
        &mc(root)
            .args(["--json", "new", "task", "Scripted", "--status", "TODO"])
            .output()
            .unwrap(),
    );
    assert_eq!(out["id"], "TASK-001");
    assert_eq!(out["kind"], "task");
    assert_eq!(out["title"], "Scripted");
    assert_eq!(out["path"], "tasks/todo/TASK-001-scripted.md");
    assert!(root.join("tasks/todo/TASK-001-scripted.md").is_file());

    // Errors are JSON on stderr, and nothing is printed before them.
    let bad = mc(root)
        .args([
            "--json",
            "new",
            "sprint",
            "W41",
            "--start-date",
            "2026-10-05",
        ])
        .args(["--end-date", "2026-10-01"])
        .output()
        .unwrap();
    assert_eq!(bad.status.code(), Some(2));
    assert!(bad.stdout.is_empty());
    let err: serde_json::Value = serde_json::from_slice(&bad.stderr).unwrap();
    assert!(err["error"]["message"]
        .as_str()
        .unwrap()
        .starts_with("Invalid end date"));
}

#[test]
fn new_checks_flags_before_printing_a_summary() {
    let tmp = repo();
    let out = mc(tmp.path())
        .args(["-y", "new", "task", "Bad", "--due-date", "2026-13-01"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        out.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}
