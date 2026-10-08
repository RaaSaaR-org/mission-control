//! Repo-wide write lock shared by every surface.
//!
//! The CLI, the MCP server, `mc api serve` and the dashboard can all write to
//! the same repo at once (separate processes, and concurrent tool calls inside
//! one MCP server). ID allocation is scan-then-write and checklist ticks,
//! comments and moves are read-modify-write, so the shared write functions
//! (`commands::new::create_*`, `commands::task::move_task_programmatic`,
//! `comments::add`, `checklist::set_checked`, `frontmatter::update_file`)
//! hold this advisory `flock` while they work.
//!
//! The lock file lives in the git dir of the git repo that contains the mc
//! root (found by walking up, following a worktree's or submodule's `.git`
//! file), so it never shows up in `git status` and one git repo has one lock
//! even when it holds nested mc configs (e.g. a customer folder with its own
//! `config/config.yml`). Without git it is `.mc-write.lock` next to the
//! config (ignored by the `.gitignore` that `mc init` writes). It is a
//! different file from `.mc-api.lock`, which a running `mc api serve` holds
//! for its whole lifetime.
//!
//! The lock is re-entrant per thread: a write function that calls another
//! one does not deadlock on itself.

use crate::config::{self, RepoMode, ResolvedConfig};
use crate::error::McResult;
use fs2::FileExt;
use std::cell::RefCell;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

thread_local! {
    /// Lock files this thread currently holds.
    static HELD: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}

/// Name of the lock file outside `.git/` (see the module docs).
pub const LOCK_FILE: &str = ".mc-write.lock";

/// Held write lock; released on drop.
#[must_use = "the lock is released when the guard is dropped"]
pub struct WriteLock {
    held: Option<(File, PathBuf)>,
}

impl Drop for WriteLock {
    fn drop(&mut self) {
        if let Some((file, key)) = self.held.take() {
            let _ = FileExt::unlock(&file);
            HELD.with(|h| {
                let mut h = h.borrow_mut();
                if let Some(i) = h.iter().rposition(|k| *k == key) {
                    h.remove(i);
                }
            });
        }
    }
}

/// Where the write lock of the repo at `root` lives.
pub fn lock_path(root: &Path, mode: RepoMode) -> PathBuf {
    let abs = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
    if let Some(git_dir) = abs.ancestors().find_map(git_dir_at) {
        return git_dir.join("mc-write.lock");
    }
    match mode {
        RepoMode::Standalone => root.join(LOCK_FILE),
        RepoMode::Embedded => root.join(".mc").join(LOCK_FILE),
    }
}

/// The git dir of a repo whose work tree is `dir`: `dir/.git` itself, or the
/// directory a `.git` file (worktree, submodule) points to with `gitdir:`.
fn git_dir_at(dir: &Path) -> Option<PathBuf> {
    let git = dir.join(".git");
    if git.is_dir() {
        return Some(git);
    }
    let text = std::fs::read_to_string(&git).ok()?;
    let target = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim();
    let target = dir.join(target);
    target.is_dir().then_some(target)
}

/// Take the write lock of `cfg`'s repo, waiting for other writers.
pub fn acquire(cfg: &ResolvedConfig) -> McResult<WriteLock> {
    acquire_at(&lock_path(&cfg.root, cfg.mode))
}

/// Take the write lock of the repo that contains `file`. Files outside any
/// repo (e.g. scratch files in tests) need no lock.
pub fn acquire_for_file(file: &Path) -> McResult<WriteLock> {
    let abs = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
    match abs.parent().map(config::find_repo_root) {
        Some(Ok((root, mode))) => acquire_at(&lock_path(&root, mode)),
        _ => Ok(WriteLock { held: None }),
    }
}

fn acquire_at(path: &Path) -> McResult<WriteLock> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    // The same file can be reached through different spellings of the root.
    let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if HELD.with(|h| h.borrow().contains(&key)) {
        return Ok(WriteLock { held: None });
    }
    file.lock_exclusive()?;
    HELD.with(|h| h.borrow_mut().push(key.clone()));
    Ok(WriteLock {
        held: Some((file, key)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn lock_file_prefers_git_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert_eq!(
            lock_path(tmp.path(), RepoMode::Standalone),
            tmp.path().join(LOCK_FILE)
        );
        assert_eq!(
            lock_path(tmp.path(), RepoMode::Embedded),
            tmp.path().join(".mc").join(LOCK_FILE)
        );
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        assert_eq!(
            lock_path(tmp.path(), RepoMode::Embedded),
            tmp.path().join(".git").join("mc-write.lock")
        );
    }

    #[test]
    fn lock_file_lives_in_the_enclosing_git_dir() {
        // An embedded repo in a subfolder of a git repo.
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        let pkg = tmp.path().join("pkg");
        std::fs::create_dir_all(pkg.join(".mc")).unwrap();
        assert_eq!(
            lock_path(&pkg, RepoMode::Embedded),
            tmp.path().join(".git").join("mc-write.lock")
        );

        // A worktree or submodule: `.git` is a file naming the git dir,
        // relative or absolute.
        let tmp = tempfile::TempDir::new().unwrap();
        let git_dir = tmp.path().join("main.git").join("worktrees").join("wt");
        std::fs::create_dir_all(&git_dir).unwrap();
        let wt = tmp.path().join("wt");
        std::fs::create_dir(&wt).unwrap();
        std::fs::write(wt.join(".git"), "gitdir: ../main.git/worktrees/wt\n").unwrap();
        assert_eq!(
            lock_path(&wt, RepoMode::Standalone),
            wt.join("../main.git/worktrees/wt").join("mc-write.lock")
        );
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", git_dir.display())).unwrap();
        assert_eq!(
            lock_path(&wt, RepoMode::Standalone),
            git_dir.join("mc-write.lock")
        );
        // A `.git` file pointing nowhere falls back to the plain lock file.
        std::fs::write(wt.join(".git"), "gitdir: missing\n").unwrap();
        assert_eq!(lock_path(&wt, RepoMode::Standalone), wt.join(LOCK_FILE));
    }

    #[test]
    fn nested_mc_config_shares_the_git_repos_lock() {
        // Like a real repo where each customer folder has its own config.
        let tmp = tempfile::TempDir::new().unwrap();
        crate::commands::init::run(tmp.path(), false, false, Some("T"), false, true).unwrap();
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
        let cfg = config::load_config(tmp.path(), RepoMode::Standalone).unwrap();
        let cust = cfg.customers_dir.join("CUST-001-acme");
        std::fs::create_dir_all(cust.join("config")).unwrap();
        std::fs::write(cust.join("config").join("config.yml"), "{}\n").unwrap();
        let file = cust.join("tasks").join("todo").join("TASK-001-x.md");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "---\nid: TASK-001\nstatus: todo\n---\n- [ ] a\n").unwrap();

        let outer = acquire(&cfg).unwrap();
        assert!(outer.held.is_some());
        // The file lookup lands on the same lock, so it is a re-entrant no-op.
        let inner = acquire_for_file(&file).unwrap();
        assert!(inner.held.is_none());
        drop((inner, outer));

        crate::frontmatter::update_file(&file, |fm| {
            crate::frontmatter::set_str(fm, "owner", "Bob")
        })
        .unwrap();
        assert!(!cust.join(LOCK_FILE).exists());
        assert!(!tmp.path().join(LOCK_FILE).exists());
        assert!(tmp.path().join(".git").join("mc-write.lock").exists());
    }

    #[test]
    fn reentrant_on_one_thread_exclusive_across_threads() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join(LOCK_FILE);
        let outer = acquire_at(&path).unwrap();
        // Nested acquisition on the same thread does not deadlock.
        let inner = acquire_at(&path).unwrap();
        drop(inner);

        let entered = Arc::new(AtomicBool::new(false));
        let t = {
            let (path, entered) = (path.clone(), entered.clone());
            std::thread::spawn(move || {
                let _l = acquire_at(&path).unwrap();
                entered.store(true, Ordering::SeqCst);
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(
            !entered.load(Ordering::SeqCst),
            "second thread got the lock"
        );
        drop(outer);
        t.join().unwrap();
        assert!(entered.load(Ordering::SeqCst));
    }

    #[test]
    fn files_outside_a_repo_need_no_lock() {
        let tmp = tempfile::TempDir::new().unwrap();
        let lock = acquire_for_file(&tmp.path().join("x.md")).unwrap();
        assert!(lock.held.is_none());
    }

    #[test]
    fn config_and_file_lookups_share_one_lock() {
        for embedded in [false, true] {
            let tmp = tempfile::TempDir::new().unwrap();
            crate::commands::init::run(tmp.path(), false, embedded, Some("T"), false, true)
                .unwrap();
            let cfg = config::load_config(tmp.path(), config::detect_mode(tmp.path())).unwrap();
            let task = crate::commands::new::create_task(
                &cfg,
                &crate::commands::new::TaskInput::new("Locked"),
            )
            .unwrap();
            let outer = acquire(&cfg).unwrap();
            assert!(outer.held.is_some());
            // Same lock file, so the nested lookup is a no-op instead of a deadlock.
            let inner = acquire_for_file(&task.path).unwrap();
            assert!(inner.held.is_none(), "embedded: {embedded}");
        }
    }
}
