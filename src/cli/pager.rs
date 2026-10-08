//! Paging long output and opening files in an editor.

use crate::error::{McError, McResult};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// The pager to use: `MC_PAGER`, then `PAGER`, then `less`. An empty value
/// or `cat` turns paging off. The value is a shell command line (like git,
/// it may quote paths with spaces and carry options).
pub fn command() -> Option<String> {
    let raw = std::env::var("MC_PAGER")
        .or_else(|_| std::env::var("PAGER"))
        .unwrap_or_else(|_| "less".into());
    match raw.split_whitespace().next() {
        None | Some("cat") => None,
        Some(_) => Some(raw),
    }
}

/// Whether the pager is `less` (which passes colours and OSC 8 links with `-R`).
pub fn is_less(cmd: &str) -> bool {
    cmd.split_whitespace()
        .next()
        .and_then(|p| Path::new(p).file_name())
        .is_some_and(|n| n == "less")
}

/// `cmd` (a shell command line) as a process, with `args` appended as
/// separate arguments. On Unix it runs through `sh -c`, so quoting works and
/// paths may contain spaces; elsewhere it is split on whitespace.
fn shell(cmd: &str, args: &[&std::ffi::OsStr]) -> Command {
    #[cfg(unix)]
    {
        let mut c = Command::new("sh");
        c.arg("-c")
            .arg(format!("{cmd} \"$@\""))
            .arg("sh")
            .args(args);
        c
    }
    #[cfg(not(unix))]
    {
        let mut parts = cmd.split_whitespace();
        let mut c = Command::new(parts.next().unwrap_or_default());
        c.args(parts).args(args);
        c
    }
}

/// Pipe `text` through the pager and wait for it to exit. Returns `false`
/// when the pager could not be started (the caller then prints directly).
pub fn page(cmd: &str, text: &str) -> bool {
    let mut c = shell(cmd, &[]);
    c.stdin(Stdio::piped());
    // Like git: quit if it fits, keep colours, don't clear the screen.
    if std::env::var_os("LESS").is_none() {
        c.env("LESS", "FRX");
    }
    if std::env::var_os("LV").is_none() {
        c.env("LV", "-c");
    }
    let Ok(mut child) = c.spawn() else {
        return false;
    };
    // The pager owns the terminal now: Ctrl-C is for it, and quitting early
    // must not kill us with SIGPIPE halfway through the write.
    let _guard = signals::Ignore::new();
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    let _ = child.wait();
    true
}

/// `$VISUAL`, else `$EDITOR`, when set.
fn editor() -> Option<String> {
    ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.trim().is_empty())
}

/// Open `path` in `$VISUAL` / `$EDITOR`, or the system opener when neither is set.
pub fn open(path: &Path) -> McResult<()> {
    if let Some(e) = editor() {
        return run_editor(&e, path);
    }
    let parts: Vec<String> = if cfg!(target_os = "macos") {
        vec!["open".into()]
    } else if cfg!(windows) {
        vec!["cmd".into(), "/C".into(), "start".into(), String::new()]
    } else {
        vec!["xdg-open".into()]
    };
    let (prog, args) = parts.split_first().expect("opener command is never empty");
    let status = Command::new(prog)
        .args(args)
        .arg(path)
        .status()
        .map_err(|e| McError::Other(format!("could not run '{prog}': {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(McError::Other(format!("'{prog}' exited with {status}")))
    }
}

/// Edit `path` in `$VISUAL` / `$EDITOR` (else `vi`) and wait until the
/// editor exits. Unlike [`open`], never hands off to a system opener, which
/// returns before the file is saved.
pub fn edit(path: &Path) -> McResult<()> {
    let fallback = if cfg!(windows) { "notepad" } else { "vi" };
    run_editor(&editor().unwrap_or_else(|| fallback.into()), path)
}

fn run_editor(editor: &str, path: &Path) -> McResult<()> {
    let status = shell(editor, &[path.as_os_str()])
        .status()
        .map_err(|e| McError::Other(format!("could not run '{editor}': {e}")))?;
    match status.code() {
        _ if status.success() => Ok(()),
        // The shell's "command not found".
        Some(127) if cfg!(unix) => Err(McError::Other(format!("could not run '{editor}'"))),
        _ => Err(McError::Other(format!("'{editor}' exited with {status}"))),
    }
}

#[cfg(unix)]
mod signals {
    const SIGINT: i32 = 2;
    const SIGPIPE: i32 = 13;
    const SIG_IGN: usize = 1;

    extern "C" {
        fn signal(signum: i32, handler: usize) -> usize;
    }

    /// Ignores SIGINT and SIGPIPE until dropped, then restores the previous handlers.
    pub struct Ignore {
        int: usize,
        pipe: usize,
    }

    impl Ignore {
        pub fn new() -> Self {
            // SAFETY: swapping signal dispositions is async-signal-safe; the
            // previous handlers are restored on drop.
            unsafe {
                Ignore {
                    int: signal(SIGINT, SIG_IGN),
                    pipe: signal(SIGPIPE, SIG_IGN),
                }
            }
        }
    }

    impl Drop for Ignore {
        fn drop(&mut self) {
            // SAFETY: see `new`.
            const SIG_ERR: usize = usize::MAX;
            unsafe {
                if self.int != SIG_ERR {
                    signal(SIGINT, self.int);
                }
                if self.pipe != SIG_ERR {
                    signal(SIGPIPE, self.pipe);
                }
            }
        }
    }
}

#[cfg(not(unix))]
mod signals {
    pub struct Ignore;

    impl Ignore {
        pub fn new() -> Self {
            Ignore
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn less_is_detected_by_file_name() {
        assert!(is_less("less -R"));
        assert!(is_less("/usr/bin/less"));
        assert!(!is_less("more"));
        assert!(!is_less(""));
    }

    #[cfg(unix)]
    #[test]
    fn editor_paths_may_contain_spaces_and_quotes() {
        let dir = tempfile::TempDir::new().unwrap();
        let bin = dir.path().join("my editor");
        std::fs::create_dir(&bin).unwrap();
        let script = bin.join("ed.sh");
        std::fs::write(&script, "#!/bin/sh\necho \"edited $1\" > \"$1.log\"\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let file = dir.path().join("a b.md");
        std::fs::write(&file, "x").unwrap();

        let quoted = format!("'{}'", script.display());
        run_editor(&quoted, &file).unwrap();
        let log = std::fs::read_to_string(dir.path().join("a b.md.log")).unwrap();
        assert_eq!(log.trim(), format!("edited {}", file.display()));

        let err = run_editor("/nonexistent/editor -w", &file).unwrap_err();
        assert!(err.to_string().contains("could not run"), "{err}");
    }
}
