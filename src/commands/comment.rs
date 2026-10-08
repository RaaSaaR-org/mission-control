//! `mc comment`: add a comment to a task or meeting.

use crate::cli::ui;
use crate::cli::{pager, suggest};
use crate::comments;
use crate::config::ResolvedConfig;
use crate::data::EntityRecord;
use crate::error::{McError, McResult};
use crate::html::display_name;
use colored::*;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};

/// Lines of the editor template that start with this are dropped.
const TEMPLATE_MARK: &str = "# mc:";

/// Add `text` as a comment on `id`: `-` reads it from stdin, and without
/// text it is written in `$VISUAL` / `$EDITOR`.
pub fn run(
    id: &str,
    text: Option<&str>,
    author: Option<&str>,
    cfg: &ResolvedConfig,
) -> McResult<()> {
    let entity = suggest::find_entity(id, cfg, None)?;
    let stdin_tty = std::io::stdin().is_terminal();
    let mut draft = None;
    let text = match text {
        Some("-") => {
            if stdin_tty {
                eprintln!("Reading the comment from stdin; finish with Ctrl-D.");
            }
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        }
        Some(t) => t.to_string(),
        None if stdin_tty => {
            if !comments::is_commentable(entity.kind) {
                return Err(McError::usage(
                    format!(
                        "{} is a {}; only tasks and meetings take comments.",
                        entity.id,
                        entity.kind.label()
                    ),
                    None,
                ));
            }
            let (path, text) = compose(&entity, pager::edit)?;
            draft = Some(path);
            text
        }
        None => {
            return Err(McError::usage(
                "no comment text",
                Some(format!(
                    "pass it as an argument, or `-` to read stdin: mc comment {} -",
                    entity.id
                )),
            ))
        }
    };
    let added = match comments::add(cfg, &entity, &text, author) {
        Ok(added) => added,
        Err(e) => {
            if let Some(path) = &draft {
                eprintln!("Your comment is saved in {}", path.display());
            }
            return Err(e);
        }
    };
    if let Some(path) = draft {
        let _ = std::fs::remove_file(path);
    }
    if ui::get().json {
        println!("{}", serde_json::to_string_pretty(&added)?);
        return Ok(());
    }
    let who = added.comment.author.as_deref().unwrap_or("unknown");
    ui::success(format!(
        "Commented on {} as {}",
        added.id.cyan().bold(),
        who.bold()
    ));
    let c = &added.comment;
    let when = [c.date.as_deref(), c.time.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    println!(
        "  {}",
        format!(
            "{when} {} {} {} {}",
            ui::glyphs().sep,
            ui::count(added.count, "comment", "comments"),
            ui::glyphs().sep,
            added.path
        )
        .dimmed()
    );
    Ok(())
}

/// Write a comment in the editor (`edit`, normally [`pager::edit`]). Returns
/// the draft file and its text without the template lines; an empty comment
/// cancels.
fn compose(
    entity: &EntityRecord,
    edit: impl FnOnce(&Path) -> McResult<()>,
) -> McResult<(PathBuf, String)> {
    let id: String = entity
        .id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or_default();
    let path =
        std::env::temp_dir().join(format!("mc-comment-{id}-{}-{nonce}.md", std::process::id()));
    let name = ui::clean(display_name(entity));
    std::fs::write(
        &path,
        format!(
            "\n{TEMPLATE_MARK} Comment on {} ({name})\n\
             {TEMPLATE_MARK} Write Markdown above. Lines starting with \"{TEMPLATE_MARK}\" are dropped;\n\
             {TEMPLATE_MARK} an empty comment cancels.\n",
            ui::clean(&entity.id)
        ),
    )?;
    edit(&path)?;
    let text = strip_template(&std::fs::read_to_string(&path)?);
    if text.trim().is_empty() {
        let _ = std::fs::remove_file(&path);
        return Err(McError::Other(
            "comment cancelled: the comment is empty".into(),
        ));
    }
    Ok((path, text))
}

/// The editor text without the template's marker lines.
fn strip_template(text: &str) -> String {
    text.lines()
        .filter(|l| !l.starts_with(TEMPLATE_MARK))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> EntityRecord {
        EntityRecord {
            kind: crate::entity::EntityKind::Task,
            id: "TASK-001".into(),
            frontmatter: serde_yaml::from_str("id: TASK-001\ntitle: Fix it").unwrap(),
            body: String::new(),
            source_path: PathBuf::from("tasks/todo/TASK-001-fix-it.md"),
        }
    }

    #[test]
    fn editor_text_becomes_the_comment() {
        let (path, text) = compose(&task(), |p| {
            let template = std::fs::read_to_string(p).unwrap();
            assert!(
                template.contains("# mc: Comment on TASK-001 (Fix it)"),
                "{template}"
            );
            std::fs::write(p, format!("## Done\n\nShipped.\n{template}")).unwrap();
            Ok(())
        })
        .unwrap();
        assert_eq!(text, "## Done\n\nShipped.");
        std::fs::remove_file(path).unwrap();

        // Saving the template untouched cancels and cleans up.
        let mut draft = PathBuf::new();
        let err = compose(&task(), |p| {
            draft = p.to_path_buf();
            Ok(())
        })
        .unwrap_err();
        assert!(err.to_string().contains("cancelled"), "{err}");
        assert!(!draft.exists());
    }

    #[test]
    fn template_lines_are_dropped_but_headings_stay() {
        let text = "## Result\n\nShipped.\n\n# mc: Comment on TASK-001 (x)\n# mc: an empty comment cancels.\n";
        assert_eq!(strip_template(text), "## Result\n\nShipped.");
        assert_eq!(strip_template("\n# mc: only the template\n"), "");
    }
}
