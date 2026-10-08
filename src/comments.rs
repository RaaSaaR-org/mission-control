//! Comments on tasks and meetings, stored in the entity's own Markdown file.
//!
//! Comments live in a trailing `## Comments` section, one level-3 heading
//! per comment, so they diff cleanly in git and read naturally in any editor:
//!
//! ```markdown
//! ## Comments
//!
//! ### 2026-10-06 14:32 · Jane Doe
//!
//! Shipped the fix, see TASK-012.
//! ```
//!
//! Hand edits are tolerated: a heading that doesn't parse as
//! `date time · author` is shown as written, and text before the first
//! comment heading becomes an untitled comment. The CLI (`mc comment`), the
//! MCP tool, the REST API and the dashboard all add comments through [`add`].

use crate::checklist::{body_start, markdown_options};
use crate::config::ResolvedConfig;
use crate::data::EntityRecord;
use crate::entity::EntityKind;
use crate::error::{McError, McResult};
use crate::util;
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};
use regex::Regex;
use serde::Serialize;
use std::ops::Range;
use std::path::Path;
use std::sync::LazyLock;

/// Kinds that take comments.
pub const COMMENTABLE: &[EntityKind] = &[EntityKind::Task, EntityKind::Meeting];

/// Heading of the comments section.
pub const SECTION_TITLE: &str = "Comments";

/// Longest accepted author name, in characters.
pub const MAX_AUTHOR: usize = 80;

static HEADING_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\d{4}-\d{2}-\d{2})(?:[ T](\d{1,2}:\d{2}))?\s*(?:[·•|—–-]\s*(.*))?$")
        .expect("static regex")
});

/// The Obsidian link footer `serialize_document` keeps at the end of a file.
static LINKS_FOOTER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^%% mc-links:.*%%[ \t]*(?:\r?\n)?\z").expect("static regex"));

/// One parsed comment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Comment {
    /// `YYYY-MM-DD`, when the heading starts with a date.
    pub date: Option<String>,
    /// `HH:MM`, when the heading has a time.
    pub time: Option<String>,
    /// Author from the heading, if any.
    pub author: Option<String>,
    /// The heading as written (empty for text before the first heading).
    pub heading: String,
    /// The comment's Markdown.
    pub body: String,
}

/// Whether `kind` takes comments.
pub fn is_commentable(kind: EntityKind) -> bool {
    COMMENTABLE.contains(&kind)
}

/// Byte range of the `## Comments` section in `body`: from its heading to
/// the next heading of level 1 or 2, or the end. The last such section wins.
pub fn section_range(body: &str) -> Option<Range<usize>> {
    let mut start = None;
    let mut found = None;
    let mut in_h2: Option<(usize, String)> = None;
    for (event, span) in Parser::new_ext(body, markdown_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                if level <= HeadingLevel::H2 {
                    if let Some(s) = start.take() {
                        found = Some(s..span.start);
                    }
                }
                if level == HeadingLevel::H2 {
                    in_h2 = Some((span.start, String::new()));
                }
            }
            Event::Text(t) | Event::Code(t) => {
                if let Some((_, text)) = in_h2.as_mut() {
                    text.push_str(&t);
                }
            }
            Event::End(TagEnd::Heading(HeadingLevel::H2)) => {
                if let Some((at, text)) = in_h2.take() {
                    if text.trim().eq_ignore_ascii_case(SECTION_TITLE) {
                        start = Some(at);
                    }
                }
            }
            _ => {}
        }
    }
    match start {
        Some(s) => Some(s..body.len()),
        None => found,
    }
}

/// Split a body into the notes without the comments section, and the
/// comments.
pub fn split(body: &str) -> (String, Vec<Comment>) {
    match section_range(body) {
        Some(range) => {
            let main = format!("{}{}", &body[..range.start], &body[range.end..]);
            (main, parse_section(&body[range]))
        }
        None => (body.to_string(), Vec::new()),
    }
}

/// Comments in a body (empty when it has no comments section).
pub fn comments(body: &str) -> Vec<Comment> {
    split(body).1
}

/// Parse the comments section (starting at its `## Comments` heading).
fn parse_section(section: &str) -> Vec<Comment> {
    // Skip the section heading itself.
    let mut events = Parser::new_ext(section, markdown_options()).into_offset_iter();
    let mut content_start = section.len();
    for (event, span) in events.by_ref() {
        if let Event::End(TagEnd::Heading(_)) = event {
            content_start = span.end;
            break;
        }
    }
    let content = &section[content_start..];

    // Each H3 starts a comment: (heading text, body start, heading start).
    let mut heads: Vec<(String, usize, usize)> = Vec::new();
    let mut current: Option<(String, usize)> = None;
    for (event, span) in Parser::new_ext(content, markdown_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading {
                level: HeadingLevel::H3,
                ..
            }) => current = Some((String::new(), span.start)),
            Event::Text(t) | Event::Code(t) => {
                if let Some((text, _)) = current.as_mut() {
                    text.push_str(&t);
                }
            }
            Event::End(TagEnd::Heading(HeadingLevel::H3)) => {
                if let Some((text, start)) = current.take() {
                    heads.push((text, span.end, start));
                }
            }
            _ => {}
        }
    }

    let mut out = Vec::new();
    let lead_end = heads.first().map_or(content.len(), |h| h.2);
    let lead = content[..lead_end].trim();
    if !lead.is_empty() {
        out.push(Comment {
            date: None,
            time: None,
            author: None,
            heading: String::new(),
            body: lead.to_string(),
        });
    }
    for (i, (heading, body_start, _)) in heads.iter().enumerate() {
        let body_end = heads.get(i + 1).map_or(content.len(), |h| h.2);
        let heading = heading.trim().to_string();
        let (date, time, author) = match HEADING_RE.captures(&heading) {
            Some(c) => (
                c.get(1).map(|m| m.as_str().to_string()),
                c.get(2).map(|m| m.as_str().to_string()),
                c.get(3)
                    .map(|m| m.as_str().trim().to_string())
                    .filter(|a| !a.is_empty()),
            ),
            None => (None, None, None),
        };
        out.push(Comment {
            date,
            time,
            author,
            heading,
            body: LINKS_FOOTER_RE
                .replace(content[*body_start..body_end].trim(), "")
                .trim()
                .to_string(),
        });
    }
    out
}

/// The default comment author: the repo's `git config user.name`, else the
/// OS user, else "unknown".
pub fn default_author(root: &Path) -> String {
    let git = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "user.name"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    git.or_else(|| std::env::var("USER").ok())
        .or_else(|| std::env::var("USERNAME").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Validate an author name: one line, at most [`MAX_AUTHOR`] characters.
pub fn check_author(author: &str) -> McResult<String> {
    let author = author.split_whitespace().collect::<Vec<_>>().join(" ");
    if author.chars().any(char::is_control) || author.chars().count() > MAX_AUTHOR {
        return Err(McError::usage(
            format!("Author names are one line of at most {MAX_AUTHOR} characters."),
            None,
        ));
    }
    Ok(author)
}

/// `author` with the characters Markdown would treat as markup (code,
/// emphasis, links, HTML, entities, closing `#`s, strikethrough) escaped, so
/// the heading reads back as the literal name (`Jane <jane@x.org>`).
fn escape_author(author: &str) -> String {
    let mut out = String::with_capacity(author.len());
    for c in author.chars() {
        if matches!(
            c,
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '&' | '#' | '~'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Demote headings of level 1-3 in a comment to level 4, so a comment can't
/// end its own comment or the comments section.
fn demote_headings(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for (event, span) in Parser::new_ext(text, markdown_options()).into_offset_iter() {
        let Event::Start(Tag::Heading { level, .. }) = event else {
            continue;
        };
        if level > HeadingLevel::H3 || span.start < last {
            continue;
        }
        let src = text[span.clone()].trim_end_matches(['\n', '\r']);
        let title = if src.trim_start().starts_with('#') {
            // ATX: drop the opening and any closing hashes.
            let t = src.trim_start().trim_start_matches('#').trim();
            t.trim_end_matches('#').trim_end()
        } else {
            // Setext: everything but the underline.
            src.rsplit_once('\n').map_or(src, |(t, _)| t).trim()
        };
        out.push_str(&text[last..span.start]);
        out.push_str("#### ");
        out.push_str(&title.replace('\n', " "));
        last = span.start + src.len();
    }
    out.push_str(&text[last..]);
    out
}

/// Normalise comment text: LF line endings, no control characters or
/// trailing whitespace, headings demoted, and no code fence or HTML block
/// left open. Empty text is an error.
fn clean_text(text: &str) -> McResult<String> {
    let text: String = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|&c| !c.is_control() || c == '\n' || c == '\t')
        .collect();
    let text = demote_headings(text.trim());
    if text.trim().is_empty() {
        return Err(McError::usage(
            "A comment needs some text.",
            Some("Pass the text as an argument, or - to read it from stdin.".into()),
        ));
    }
    self_contained(text)
}

/// Whether a heading written after `text` (and a blank line) is still a
/// heading, i.e. `text` doesn't leave a code fence or HTML block open that
/// would swallow the comments and sections after it.
fn closes_itself(text: &str) -> bool {
    let probe = format!("{text}\n\n### probe\n");
    let at = text.len() + 2;
    Parser::new_ext(&probe, markdown_options())
        .into_offset_iter()
        .any(|(event, span)| matches!(event, Event::Start(Tag::Heading { .. })) && span.start == at)
}

/// `text`, with a closing line added when it ends inside an unclosed code
/// fence or HTML block; an error when that doesn't help.
fn self_contained(text: String) -> McResult<String> {
    if closes_itself(&text) {
        return Ok(text);
    }
    // Fences opened in the text (longest first, so a closer is long enough),
    // then the end markers of the HTML blocks that run past blank lines.
    let mut fences: Vec<&str> = text
        .lines()
        .filter_map(|l| {
            let l = l.trim_start_matches([' ', '>']);
            let run = l.len() - l.trim_start_matches(['`', '~']).len();
            let fence = &l[..run];
            (run >= 3 && fence.chars().all(|c| c == fence.as_bytes()[0] as char)).then_some(fence)
        })
        .collect();
    fences.sort_by_key(|f| std::cmp::Reverse(f.len()));
    let html = [
        "-->",
        "</pre>",
        "</script>",
        "</style>",
        "</textarea>",
        "?>",
        "]]>",
        ">",
    ];
    for closer in fences.into_iter().chain(html) {
        let closed = format!("{text}\n{closer}");
        if closes_itself(&closed) {
            return Ok(closed);
        }
    }
    Err(McError::usage(
        "A comment can't leave a code block or HTML block open.",
        Some("Close the ``` fence or HTML tag at the end of the comment.".into()),
    ))
}

/// `content` (a whole file) with a comment appended to its comments section,
/// creating the section at the end of the file if needed. Everything
/// already in the file stays as it was.
pub fn append(content: &str, stamp: &str, author: &str, text: &str) -> String {
    let crlf = content.contains("\r\n");
    let nl = if crlf { "\r\n" } else { "\n" };
    let start = body_start(content);
    let section = section_range(&content[start..]).map(|r| (start + r.start)..(start + r.end));
    let mut at = section.as_ref().map_or(content.len(), |r| r.end);
    // Stay above mc's link footer, which belongs at the very end.
    let footer = LINKS_FOOTER_RE.find(content).map(|m| m.start());
    if let Some(f) = footer.filter(|f| *f >= start && *f < at) {
        at = f;
    }
    let (before, after) = content.split_at(at);

    let mut block = String::new();
    // Separate from what comes before with exactly one blank line.
    let mut trailing = 0;
    let mut rest = before;
    while let Some(r) = rest.strip_suffix(nl).or_else(|| rest.strip_suffix('\n')) {
        trailing += 1;
        rest = r;
    }
    if !before.is_empty() {
        block.push_str(&nl.repeat(2usize.saturating_sub(trailing)));
    }
    if section.is_none() {
        block.push_str(&format!("## {SECTION_TITLE}{nl}{nl}"));
    }
    let heading = if author.is_empty() {
        format!("### {stamp}")
    } else {
        format!("### {stamp} · {}", escape_author(author))
    };
    block.push_str(&heading);
    block.push_str(nl);
    block.push_str(nl);
    block.push_str(&text.replace('\n', nl));
    block.push_str(nl);
    if !after.is_empty() && footer != Some(at) {
        block.push_str(nl);
    }
    format!("{before}{block}{after}")
}

/// A comment just added, with its position.
#[derive(Debug, Clone, Serialize)]
pub struct Added {
    pub id: String,
    pub comment: Comment,
    /// Comments on the entity after adding this one.
    pub count: usize,
    pub path: String,
}

/// Add a comment to a task or meeting. `author` defaults to
/// [`default_author`]; the timestamp is the local time to the minute.
pub fn add(
    cfg: &ResolvedConfig,
    entity: &EntityRecord,
    text: &str,
    author: Option<&str>,
) -> McResult<Added> {
    if !is_commentable(entity.kind) {
        return Err(McError::usage(
            format!(
                "{} is a {}; only tasks and meetings take comments.",
                entity.id,
                entity.kind.label()
            ),
            None,
        ));
    }
    let text = clean_text(text)?;
    let author = match author.map(check_author).transpose()? {
        Some(a) if !a.is_empty() => a,
        _ => default_author(&cfg.root),
    };
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let path = entity
        .source_path
        .strip_prefix(&cfg.root)
        .unwrap_or(&entity.source_path)
        .display()
        .to_string();
    // Read, check and write under the repo's write lock, so a tick or another
    // comment written at the same time is not lost.
    let _lock = crate::lock::acquire(cfg)?;
    let content = std::fs::read_to_string(&entity.source_path)?;
    let before = comments(&content[body_start(&content)..]).len();
    let updated = append(&content, &stamp, &author, &text);

    // Check the new comment reads back as its own comment before writing: a
    // hand-edited file can leave a block open that would swallow it.
    let all = comments(&updated[body_start(&updated)..]);
    let comment = all
        .last()
        .filter(|c| all.len() == before + 1 && c.heading == format!("{stamp} · {author}"))
        .cloned()
        .ok_or_else(|| {
            // Blame the file only when a plain comment would not fit either.
            let probe = append(&content, &stamp, "probe", "x");
            let plain = comments(&probe[body_start(&probe)..]);
            let file_is_open = plain.len() != before + 1
                || plain.last().is_none_or(|c| c.heading != format!("{stamp} · probe"));
            McError::usage(
                format!("The comment would not read back as a separate comment in {path}."),
                file_is_open.then(|| {
                    "An earlier comment leaves a code block or HTML block open; close it in the file.".into()
                }),
            )
        })?;
    util::atomic_write(&entity.source_path, updated.as_bytes())?;
    Ok(Added {
        id: entity.id.clone(),
        comment,
        count: all.len(),
        path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAMP: &str = "2026-10-06 14:32";

    #[test]
    fn append_creates_the_section_once() {
        let doc = "---\nid: TASK-001\n---\n\n# Title\n\nNotes.\n";
        let one = append(doc, STAMP, "Jane Doe", "First!");
        assert_eq!(
            one,
            format!("{doc}\n## Comments\n\n### {STAMP} · Jane Doe\n\nFirst!\n")
        );
        let two = append(&one, "2026-10-06 15:00", "Max", "Second\n\n- a\n- b");
        assert!(two.starts_with(&one));
        assert!(two.ends_with("\n\n### 2026-10-06 15:00 · Max\n\nSecond\n\n- a\n- b\n"));
        let parsed = comments(&two);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].author.as_deref(), Some("Jane Doe"));
        assert_eq!(parsed[0].date.as_deref(), Some("2026-10-06"));
        assert_eq!(parsed[0].time.as_deref(), Some("14:32"));
        assert_eq!(parsed[0].body, "First!");
        assert_eq!(parsed[1].body, "Second\n\n- a\n- b");
    }

    #[test]
    fn append_handles_missing_newlines_and_empty_bodies() {
        assert_eq!(
            append("---\nid: X\n---\nNo newline", STAMP, "A", "Hi"),
            format!("---\nid: X\n---\nNo newline\n\n## Comments\n\n### {STAMP} · A\n\nHi\n")
        );
        assert_eq!(
            append("---\nid: X\n---\n", STAMP, "A", "Hi"),
            format!("---\nid: X\n---\n\n## Comments\n\n### {STAMP} · A\n\nHi\n")
        );
        // Extra blank lines at the end are kept, not doubled.
        let doc = "---\nid: X\n---\nBody\n\n\n";
        assert!(append(doc, STAMP, "A", "Hi").starts_with(&format!("{doc}## Comments")));
    }

    #[test]
    fn append_goes_into_an_existing_section_before_later_headings() {
        let doc = "---\nid: X\n---\nBody\n\n## Comments\n\n### 2026-01-01 · Old\n\nOld one\n\n## Appendix\n\nMore\n";
        let out = append(doc, STAMP, "New", "New one");
        assert_eq!(
            out,
            "---\nid: X\n---\nBody\n\n## Comments\n\n### 2026-01-01 · Old\n\nOld one\n\n### 2026-10-06 14:32 · New\n\nNew one\n\n## Appendix\n\nMore\n"
        );
        let (main, parsed) = split(&out[out.find("Body").unwrap()..]);
        assert_eq!(main, "Body\n\n## Appendix\n\nMore\n");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].author.as_deref(), Some("Old"));
        assert_eq!(parsed[0].time, None);
    }

    #[test]
    fn comments_stay_above_the_link_footer() {
        let doc =
            "---\nid: X\nprojects: [\"[[PROJ-001]]\"]\n---\nBody\n%% mc-links: [[PROJ-001]] %%\n";
        let one = append(doc, STAMP, "A", "Hi");
        assert_eq!(
            one,
            format!("---\nid: X\nprojects: [\"[[PROJ-001]]\"]\n---\nBody\n\n## Comments\n\n### {STAMP} · A\n\nHi\n%% mc-links: [[PROJ-001]] %%\n")
        );
        let two = append(&one, STAMP, "B", "Again");
        assert!(two
            .ends_with("Hi\n\n### 2026-10-06 14:32 · B\n\nAgain\n%% mc-links: [[PROJ-001]] %%\n"));
        let parsed = comments(&two);
        assert_eq!(parsed[1].body, "Again");
        // A frontmatter rewrite moves the footer below the comments; it is
        // still not part of the last comment.
        let (fm, body) = crate::frontmatter::split_frontmatter(&two).unwrap();
        let fm = crate::frontmatter::parse_raw(&fm, Path::new("x")).unwrap();
        let rewritten = crate::frontmatter::serialize_document(&fm, &body);
        assert_eq!(comments(&rewritten)[1].body, "Again");
    }

    #[test]
    fn crlf_files_get_crlf_comments() {
        let doc = "---\r\nid: X\r\n---\r\nBody\r\n";
        let out = append(doc, STAMP, "A", "Two\nlines");
        assert_eq!(
            out,
            format!("{doc}\r\n## Comments\r\n\r\n### {STAMP} · A\r\n\r\nTwo\r\nlines\r\n")
        );
    }

    #[test]
    fn hand_edited_sections_still_parse() {
        let body = "Notes\n\n## comments\n\nLoose remark before any heading.\n\n### Some thoughts\n\nText\n\n```\n### not a heading\n```\n";
        let (main, parsed) = split(body);
        assert_eq!(main, "Notes\n\n");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].heading, "");
        assert_eq!(parsed[0].body, "Loose remark before any heading.");
        assert_eq!(parsed[1].heading, "Some thoughts");
        assert_eq!(parsed[1].date, None);
        assert!(parsed[1].body.contains("### not a heading"));
        // A "## Comments" inside a code block is not a section.
        assert!(section_range("```\n## Comments\n```\n").is_none());
    }

    #[test]
    fn comment_headings_are_demoted() {
        let text =
            clean_text("# Big\n\n## Comments\n\nSetext\n---\n\n#### Small\n\n```\n# code\n```")
                .unwrap();
        assert_eq!(
            text,
            "#### Big\n\n#### Comments\n\n#### Setext\n\n#### Small\n\n```\n# code\n```"
        );
        let doc = append("---\nid: X\n---\nBody\n", STAMP, "A", &text);
        assert_eq!(comments(&doc).len(), 1);
        assert!(clean_text("  \n ").is_err());
    }

    #[test]
    fn unclosed_blocks_are_closed_so_later_comments_survive() {
        for (text, closer) in [
            ("```\nlog output", "```"),
            ("~~~~ sh\nls\n~~~", "~~~~"),
            ("> ```\n> quoted", ""),
            ("<!-- open", "-->"),
            ("<pre>\nopen", "</pre>"),
            ("<script>\nx", "</script>"),
            ("<?php", "?>"),
        ] {
            let cleaned = clean_text(text).unwrap();
            if closer.is_empty() {
                assert_eq!(cleaned, text);
            } else {
                assert_eq!(cleaned, format!("{text}\n{closer}"));
            }
            let doc = "---\nid: X\n---\nBody\n\n## Comments\n\n### 2026-01-01 · X\n\nold\n\n## Appendix\n\n- [ ] after\n";
            let one = append(doc, STAMP, "A", &cleaned);
            let two = append(&one, STAMP, "B", "Second");
            let (main, parsed) = split(&two[two.find("Body").unwrap()..]);
            assert_eq!(parsed.len(), 3, "{text:?}\n{two}");
            assert_eq!(parsed[2].author.as_deref(), Some("B"));
            assert!(main.contains("- [ ] after"), "{text:?}");
        }
    }

    #[test]
    fn control_characters_are_stripped() {
        assert_eq!(
            clean_text("hi \x1b]52;c;eA==\x07 \x1b[2J\0 there\r\nnext\tline").unwrap(),
            "hi ]52;c;eA== [2J there\nnext\tline"
        );
        assert!(clean_text("\x1b\x07").is_err());
    }

    #[test]
    fn authors_are_single_lines() {
        assert_eq!(check_author("  Jane   Doe ").unwrap(), "Jane Doe");
        assert_eq!(check_author("a\nb").unwrap(), "a b");
        assert!(check_author(&"x".repeat(81)).is_err());
    }

    #[test]
    fn authors_with_markup_characters_read_back_literally() {
        let doc = "---\nid: X\n---\nBody\n";
        for author in [
            "Jane Doe <jane@example.com>",
            "a `b`",
            "**bold**",
            "x #",
            "a &amp; b",
            "~~x~~",
            "Ann_ [x] \\ y",
            "O'Brien",
        ] {
            let out = append(doc, STAMP, author, "hi");
            let parsed = comments(&out);
            assert_eq!(parsed.len(), 1, "{author}");
            assert_eq!(parsed[0].heading, format!("{STAMP} · {author}"), "{out}");
            assert_eq!(parsed[0].author.as_deref(), Some(author));
        }
    }

    #[test]
    fn add_accepts_markup_authors_and_reports_relative_paths() {
        let tmp = tempfile::TempDir::new().unwrap();
        crate::commands::init::run(tmp.path(), false, false, Some("T"), false, true).unwrap();
        let cfg =
            crate::config::load_config(tmp.path(), crate::config::RepoMode::Standalone).unwrap();
        let created = crate::commands::new::create_task(
            &cfg,
            &crate::commands::new::TaskInput::new("Commented"),
        )
        .unwrap();
        let rec = crate::data::find_entity_by_id(&created.id, &cfg).unwrap();
        let added = add(&cfg, &rec, "hi", Some("Jane <j@x.org>")).unwrap();
        assert_eq!(added.comment.author.as_deref(), Some("Jane <j@x.org>"));
        assert!(!added.path.starts_with('/'), "{}", added.path);
    }
}
