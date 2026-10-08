//! Markdown task lists (`- [ ] item` / `- [x] item`) in entity files.
//!
//! Items are located with pulldown-cmark's source offsets, so look-alikes in
//! code blocks, indented code and Obsidian `%% comments %%` are never
//! touched. Ticking an item rewrites exactly one byte (the space or `x`
//! between the brackets); the frontmatter and everything else in the file
//! stay byte-for-byte as they were. The CLI (`mc check`), the MCP tools, the
//! REST API and the dashboard all go through [`set_checked`].

use crate::comments;
use crate::data::EntityRecord;
use crate::entity::EntityKind;
use crate::error::{McError, McResult};
use crate::frontmatter;
use crate::util;
use pulldown_cmark::{Event, Options, Parser, Tag};
use regex::Regex;
use serde::Serialize;
use std::ops::Range;
use std::path::Path;
use std::sync::LazyLock;

static OBSIDIAN_COMMENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)%%.*?%%").expect("static regex"));

/// One task-list item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckItem {
    /// 1-based position among the file's task-list items.
    pub index: usize,
    /// 1-based line in the file.
    pub line: usize,
    pub checked: bool,
    /// The item's text on its first line, trimmed (Markdown as written).
    pub text: String,
    /// Byte offset of the state character (` ` or `x`) in the file.
    #[serde(skip)]
    pub offset: usize,
}

/// Which item to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// 1-based position, as listed by `mc check <ID>`.
    Index(usize),
    /// 1-based line in the file.
    Line(usize),
}

/// The result of [`set_checked`].
#[derive(Debug, Clone, Serialize)]
pub struct CheckChange {
    /// The item after the change.
    pub item: CheckItem,
    /// False when the item was already in the requested state.
    pub changed: bool,
    /// Checked items in the file after the change.
    pub done: usize,
    /// All items in the file.
    pub total: usize,
}

/// Parser options shared with the dashboard's Markdown renderer, so both
/// see the same document structure.
pub(crate) fn markdown_options() -> Options {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options
}

/// Byte offset where the body starts in a file's content.
pub(crate) fn body_start(content: &str) -> usize {
    match frontmatter::split_frontmatter(content) {
        Some((_, body)) => content.len() - body.len(),
        None => 0,
    }
}

/// Task-list items in a whole file (frontmatter is skipped).
pub fn items(content: &str) -> Vec<CheckItem> {
    let start = body_start(content);
    items_in(content, start..content.len())
}

/// The checklist of an entity of `kind`: [`items`] without those inside the
/// comments section of tasks and meetings, numbered from 1.
pub fn entity_items(kind: EntityKind, content: &str) -> Vec<CheckItem> {
    checklist_from(kind, content, body_start(content))
}

/// [`entity_items`] for a body without its frontmatter (as in
/// `EntityRecord::body`); lines and offsets are relative to `body`.
pub(crate) fn body_items(kind: EntityKind, body: &str) -> Vec<CheckItem> {
    checklist_from(kind, body, 0)
}

fn checklist_from(kind: EntityKind, content: &str, start: usize) -> Vec<CheckItem> {
    let section = comments::is_commentable(kind)
        .then(|| comments::section_range(&content[start..]))
        .flatten()
        .map(|r| (start + r.start)..(start + r.end));
    let mut items = items_in(content, start..content.len());
    if let Some(section) = section {
        items.retain(|i| !section.contains(&i.offset));
        for (n, item) in items.iter_mut().enumerate() {
            item.index = n + 1;
        }
    }
    items
}

/// Byte ranges of inline code spans and code blocks in `text`.
pub(crate) fn code_ranges(text: &str) -> Vec<Range<usize>> {
    Parser::new_ext(text, markdown_options())
        .into_offset_iter()
        .filter(|(event, _)| matches!(event, Event::Code(_) | Event::Start(Tag::CodeBlock(_))))
        .map(|(_, span)| span)
        .collect()
}

/// Byte ranges of Obsidian `%% comments %%` in `text`. A `%%` inside code
/// (`` `%%` ``) neither opens nor closes a comment.
fn obsidian_comments(text: &str) -> Vec<Range<usize>> {
    let code = code_ranges(text);
    if code.is_empty() {
        return OBSIDIAN_COMMENT_RE
            .find_iter(text)
            .map(|m| m.range())
            .collect();
    }
    // Blank out code (same length, so offsets stay valid) before matching.
    let mut masked = text.as_bytes().to_vec();
    for r in &code {
        masked[r.clone()].fill(b' ');
    }
    let masked = String::from_utf8_lossy(&masked);
    OBSIDIAN_COMMENT_RE
        .find_iter(&masked)
        .map(|m| m.range())
        .collect()
}

/// Task-list items whose marker lies in `range` of `content`, in document
/// order. Line numbers and offsets are relative to all of `content`.
pub(crate) fn items_in(content: &str, range: Range<usize>) -> Vec<CheckItem> {
    let base = range.start;
    let text = &content[range];
    let hidden = obsidian_comments(text);
    let mut items = Vec::new();
    for (event, span) in Parser::new_ext(text, markdown_options()).into_offset_iter() {
        let Event::TaskListMarker(checked) = event else {
            continue;
        };
        if hidden.iter().any(|h| h.contains(&span.start)) {
            continue;
        }
        // The span may include the whitespace after the list marker.
        let Some(open) = text[span.clone()].find('[') else {
            continue;
        };
        let state = span.start + open + 1;
        let offset = base + state;
        let line_end = text[state..].find('\n').map_or(text.len(), |i| state + i);
        let after = text.get(state + 2..line_end).unwrap_or("");
        items.push(CheckItem {
            index: items.len() + 1,
            line: content[..offset].matches('\n').count() + 1,
            checked,
            text: after.trim().to_string(),
            offset,
        });
    }
    items
}

/// Tick (`checked = true`) or untick one item of `entity`'s checklist (see
/// [`entity_items`]).
///
/// - `expect_checked`: the state the caller saw. If the file disagrees, it
///   changed since the caller read it and the call fails with a conflict
///   instead of flipping the item back. `None` makes the call idempotent.
/// - `expect_text`: the item text the caller saw; a mismatch is a conflict.
///
/// Only the state byte is written; nothing else in the file changes. The
/// read-modify-write holds the repo's write lock.
pub fn set_checked(
    entity: &EntityRecord,
    target: Target,
    checked: bool,
    expect_checked: Option<bool>,
    expect_text: Option<&str>,
) -> McResult<CheckChange> {
    set_checked_in(
        &entity.source_path,
        entity.kind,
        target,
        checked,
        expect_checked,
        expect_text,
    )
}

fn set_checked_in(
    path: &Path,
    kind: EntityKind,
    target: Target,
    checked: bool,
    expect_checked: Option<bool>,
    expect_text: Option<&str>,
) -> McResult<CheckChange> {
    let _lock = crate::lock::acquire_for_file(path)?;
    let content = std::fs::read_to_string(path)?;
    let all = entity_items(kind, &content);
    let found = all.iter().find(|i| match target {
        Target::Index(n) => i.index == n,
        Target::Line(n) => i.line == n,
    });
    let item = match (found, target) {
        (Some(item), _) => item,
        (None, Target::Line(n)) => {
            return Err(McError::conflict(
                format!("There's no checklist item on line {n} any more."),
                Some("The file changed on disk; reload to see the current list.".into()),
            ))
        }
        (None, Target::Index(n)) => {
            let hint = match all.len() {
                0 => "This entity has no checklist items (lines like `- [ ] Do it`).".to_string(),
                1 => "It has one checklist item: use 1.".to_string(),
                len => format!("Use a number from 1 to {len}."),
            };
            return Err(McError::not_found(
                format!("There is no checklist item {n}."),
                Some(hint),
            ));
        }
    };
    if let Some(text) = expect_text {
        if text.trim() != item.text {
            return Err(McError::conflict(
                format!("Checklist item {} changed on disk.", item.index),
                Some(format!(
                    "It now reads “{}”. Reload and try again.",
                    item.text
                )),
            ));
        }
    }
    if expect_checked.is_some_and(|e| e != item.checked) {
        let state = if item.checked { "checked" } else { "unchecked" };
        return Err(McError::conflict(
            format!("Checklist item {} is already {state}.", item.index),
            Some("It was changed elsewhere; reload to see the current list.".into()),
        ));
    }

    let mut item = item.clone();
    let changed = item.checked != checked;
    if changed {
        let mut bytes = content.into_bytes();
        bytes[item.offset] = if checked { b'x' } else { b' ' };
        util::atomic_write(path, &bytes)?;
        item.checked = checked;
    }
    let done = all
        .iter()
        .filter(|i| {
            if i.index == item.index {
                checked
            } else {
                i.checked
            }
        })
        .count();
    Ok(CheckChange {
        item,
        changed,
        done,
        total: all.len(),
    })
}

/// `(done, total)` for a list of items.
pub fn progress(items: &[CheckItem]) -> (usize, usize) {
    (items.iter().filter(|i| i.checked).count(), items.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "---\nid: TASK-001\ntitle: \"Boxes\"   # odd spacing stays\n---\n\n# Boxes\n\n- [ ] First\n- [x] Second **bold**\n\n```\n- [ ] decoy in code\n```\n\n    - [ ] indented decoy\n\n%%\n- [ ] hidden\n%%\n\n1. [X] Numbered\n   - [ ] Nested\n\n* [ ]not a task\n";

    #[test]
    fn finds_items_and_skips_decoys() {
        let items = items(DOC);
        let texts: Vec<&str> = items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, ["First", "Second **bold**", "Numbered", "Nested"]);
        assert_eq!(
            items.iter().map(|i| i.line).collect::<Vec<_>>(),
            [8, 9, 21, 22]
        );
        assert_eq!(
            items.iter().map(|i| i.checked).collect::<Vec<_>>(),
            [false, true, true, false]
        );
        for item in &items {
            assert!(matches!(DOC.as_bytes()[item.offset], b' ' | b'x' | b'X'));
        }
        assert_eq!(progress(&items), (2, 4));
    }

    #[test]
    fn frontmatter_lookalikes_are_ignored() {
        let doc = "---\nnote: \"- [ ] not me\"\n---\n- [ ] me\n";
        let items = items(doc);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].line, 4);
    }

    fn file(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("TASK-001.md");
        std::fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn toggling_changes_one_byte() {
        let (_d, path) = file(DOC);
        let change = set_checked_in(
            &path,
            EntityKind::Task,
            Target::Index(4),
            true,
            Some(false),
            None,
        )
        .unwrap();
        assert!(change.changed);
        assert_eq!((change.done, change.total), (3, 4));
        assert_eq!(change.item.text, "Nested");
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, DOC.replace("   - [ ] Nested", "   - [x] Nested"));
        // The code-block decoy on the same pattern is untouched.
        assert!(after.contains("- [ ] decoy in code"));

        // Unticking an uppercase X writes a space; by line works too.
        set_checked_in(
            &path,
            EntityKind::Task,
            Target::Line(21),
            false,
            None,
            Some("Numbered"),
        )
        .unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("1. [ ] Numbered"));
        assert_eq!(after.len(), DOC.len());
    }

    #[test]
    fn already_in_state_is_a_no_op_unless_a_state_was_expected() {
        let (_d, path) = file(DOC);
        let change =
            set_checked_in(&path, EntityKind::Task, Target::Index(2), true, None, None).unwrap();
        assert!(!change.changed);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DOC);
        let err = set_checked_in(
            &path,
            EntityKind::Task,
            Target::Index(2),
            true,
            Some(false),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, McError::Conflict { .. }), "{err}");
    }

    #[test]
    fn stale_lines_and_texts_are_conflicts() {
        let (_d, path) = file(DOC);
        // Line 12 is the code-block decoy: not an item.
        let err = set_checked_in(
            &path,
            EntityKind::Task,
            Target::Line(12),
            true,
            Some(false),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, McError::Conflict { .. }));
        let err = set_checked_in(
            &path,
            EntityKind::Task,
            Target::Line(8),
            true,
            Some(false),
            Some("Other"),
        )
        .unwrap_err();
        assert!(matches!(err, McError::Conflict { .. }));
        let err = set_checked_in(&path, EntityKind::Task, Target::Index(9), true, None, None)
            .unwrap_err();
        assert!(matches!(err, McError::NotFound { .. }));
        assert!(err.hint().unwrap().contains("1 to 4"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DOC);
    }

    #[test]
    fn comment_items_are_not_part_of_a_task_checklist() {
        let doc = "---\nid: TASK-001\n---\n- [ ] Body\n\n## Comments\n\n### 2026-10-06 10:00 · A\n\n- [ ] In a comment\n";
        assert_eq!(entity_items(EntityKind::Task, doc).len(), 1);
        assert_eq!(entity_items(EntityKind::Research, doc).len(), 2);
        let (_d, path) = file(doc);
        let err = set_checked_in(&path, EntityKind::Task, Target::Index(2), true, None, None)
            .unwrap_err();
        assert!(matches!(err, McError::NotFound { .. }));
        let err = set_checked_in(
            &path,
            EntityKind::Task,
            Target::Line(10),
            true,
            Some(false),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, McError::Conflict { .. }));
    }

    #[test]
    fn crlf_files_keep_their_line_endings() {
        let doc = "---\r\nid: TASK-001\r\n---\r\n- [ ] One\r\n- [ ] Two\r\n";
        let (_d, path) = file(doc);
        let change = set_checked_in(
            &path,
            EntityKind::Task,
            Target::Index(2),
            true,
            Some(false),
            Some("Two"),
        )
        .unwrap();
        assert_eq!(change.item.line, 5);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            doc.replace("- [ ] Two", "- [x] Two")
        );
    }

    #[test]
    fn percent_signs_in_code_do_not_hide_items() {
        let doc = "---\nid: TASK-003\n---\nObsidian comments use `%%` markers.\n\n- [ ] Buy milk\n- [ ] Call Bob\n\n```\n%%\n```\n\n- [ ] After fence\n\n%%\n- [ ] really hidden\n%%\n\nProgress: 50%% done\n%% mc-links: [[SPR-001]] %%\n";
        let texts: Vec<String> = items(doc).into_iter().map(|i| i.text).collect();
        assert_eq!(texts, ["Buy milk", "Call Bob", "After fence"]);
    }
}
