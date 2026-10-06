//! Paging long output and opening files in an editor.

use crate::error::{McError, McResult};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// The pager to use: `MC_PAGER`, then `PAGER`, then `less`. An empty value
/// or `cat` turns paging off.
pub fn command() -> Option<Vec<String>> {
    let raw = std::env::var("MC_PAGER")
        .or_else(|_| std::env::var("PAGER"))
        .unwrap_or_else(|_| "less".into());
    let parts: Vec<String> = raw.split_whitespace().map(String::from).collect();
    match parts.first().map(String::as_str) {
        None | Some("cat") => None,
        Some(_) => Some(parts),
    }
}

/// Whether the pager is `less` (which passes colours and OSC 8 links with `-R`).
pub fn is_less(cmd: &[String]) -> bool {
    cmd.first()
        .and_then(|p| Path::new(p).file_name())
        .is_some_and(|n| n == "less")
}

/// Pipe `text` through the pager and wait for it to exit. Returns `false`
/// when the pager could not be started (the caller then prints directly).
pub fn page(cmd: &[String], text: &str) -> bool {
    let Some((prog, args)) = cmd.split_first() else {
        return false;
    };
    let mut c = Command::new(prog);
    c.args(args).stdin(Stdio::piped());
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

/// Open `path` in `$VISUAL` / `$EDITOR`, or the system opener when neither is set.
pub fn open(path: &Path) -> McResult<()> {
    let editor = ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.trim().is_empty());
    let parts: Vec<String> = match editor {
        Some(e) => e.split_whitespace().map(String::from).collect(),
        None if cfg!(target_os = "macos") => vec!["open".into()],
        None if cfg!(windows) => vec!["cmd".into(), "/C".into(), "start".into(), String::new()],
        None => vec!["xdg-open".into()],
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
        assert!(is_less(&["less".into(), "-R".into()]));
        assert!(is_less(&["/usr/bin/less".into()]));
        assert!(!is_less(&["more".into()]));
        assert!(!is_less(&[]));
    }
}
