//! The notes of a detail page: the Markdown body with tickable checklists,
//! its checklist progress, and the comments section of tasks and meetings.

use super::components::{avatar, progress_bar};
use super::format::{escape_html, fmt_day_relative, parse_date};
use super::markdown::{render_markdown_checks, render_markdown_in, strip_leading_h1, DocContext};
use super::Page;
use crate::checklist::{self, CheckItem};
use crate::comments::{self, Comment};
use crate::data::EntityRecord;

/// Longest comment, in characters, the dashboard accepts.
pub const MAX_COMMENT: usize = 20_000;

/// An entity's body split for display.
pub(crate) struct Notes {
    /// The body without its comments section.
    pub md: String,
    /// Checklist items in `md`, with their positions in the file.
    pub checks: Vec<CheckItem>,
    pub comments: Vec<Comment>,
}

impl Notes {
    /// Split `entity`'s body. Comments are only split out for kinds that
    /// take them; checklist positions come from the file on disk.
    pub(crate) fn of(entity: &EntityRecord) -> Self {
        let commentable = comments::is_commentable(entity.kind);
        let (md, comment_list) = if commentable {
            comments::split(&entity.body)
        } else {
            (entity.body.clone(), Vec::new())
        };
        let checks = std::fs::read_to_string(&entity.source_path)
            .map(|content| checklist::entity_items(entity.kind, &content))
            .unwrap_or_default();
        Self {
            md,
            checks,
            comments: comment_list,
        }
    }
}

fn doc_context<'a>(page: &'a Page, entity: &'a EntityRecord) -> Option<DocContext<'a>> {
    entity.source_path.parent().map(|dir| DocContext {
        root: &page.cfg.root,
        dir,
    })
}

/// The rendered body (checklist progress plus prose), or `None` when the
/// notes are empty.
pub(crate) fn body_html(page: &Page, entity: &EntityRecord, notes: &Notes) -> Option<String> {
    let md = strip_leading_h1(notes.md.trim());
    if md.trim().is_empty() {
        return None;
    }
    let doc = doc_context(page, entity);
    let checks = page.editable.then_some(notes.checks.as_slice());
    let prose = render_markdown_checks(md, page.catalog, doc.as_ref(), checks);
    let (done, total) = checklist::progress(&notes.checks);
    let progress = if total > 0 {
        checklist_progress(done, total)
    } else {
        String::new()
    };
    Some(format!(
        r#"{progress}<div class="detail-body prose">{prose}</div>"#
    ))
}

/// "Checklist ▬▬▬── 3 of 5 done".
fn checklist_progress(done: usize, total: usize) -> String {
    format!(
        r#"<div class="checklist-progress" data-check-progress><span class="checklist-label">Checklist</span>{}<span class="progress-caption"><span data-check-done>{done}</span> of <span data-check-total>{total}</span> done</span></div>"#,
        progress_bar(done, total)
    )
}

/// The comments section of a task or meeting. Empty sections are rendered
/// `hidden` and revealed by the dashboard script together with the
/// composer, which needs it to post.
pub(crate) fn comments_section(page: &Page, entity: &EntityRecord, notes: &Notes) -> String {
    if !comments::is_commentable(entity.kind) || (notes.comments.is_empty() && !page.editable) {
        return String::new();
    }
    let count = notes.comments.len();
    let items: String = notes
        .comments
        .iter()
        .enumerate()
        .map(|(i, c)| comment_html(page, entity, c, i + 1))
        .collect();
    let composer = if page.editable {
        composer(&comments::default_author(&page.cfg.root))
    } else {
        String::new()
    };
    format!(
        r#"<section class="comments" id="comments" aria-labelledby="comments-title" data-comments{hidden}><h2 class="section-title" id="comments-title">Comments <span class="count readout" data-comment-count>{count}</span></h2><ol class="comment-list">{items}</ol>{composer}</section>"#,
        hidden = if count == 0 { " hidden" } else { "" },
    )
}

/// One comment as a list item (also sent to the page after posting).
pub fn comment_html(page: &Page, entity: &EntityRecord, c: &Comment, n: usize) -> String {
    let who = c
        .author
        .clone()
        .or_else(|| (c.date.is_none() && !c.heading.is_empty()).then(|| c.heading.clone()));
    let name = match &who {
        Some(w) => format!(r#"<span class="comment-author">{}</span>"#, escape_html(w)),
        None => r#"<span class="comment-author muted">Note</span>"#.to_string(),
    };
    let when = c
        .date
        .as_deref()
        .and_then(parse_date)
        .map(|d| {
            let day = fmt_day_relative(d, page.today);
            let (iso, shown) = match &c.time {
                Some(t) => (
                    format!("{}T{t}", d.format("%Y-%m-%d")),
                    format!("{day}, {t}"),
                ),
                None => (d.format("%Y-%m-%d").to_string(), day),
            };
            format!(r#"<time class="comment-time" datetime="{iso}">{shown}</time>"#)
        })
        .unwrap_or_default();
    let doc = doc_context(page, entity);
    let body = render_markdown_in(&c.body, page.catalog, doc.as_ref());
    format!(
        r#"<li class="comment" id="comment-{n}"><header class="comment-head">{}{name}{when}</header><div class="comment-body prose">{body}</div></li>"#,
        avatar(who.as_deref().unwrap_or(""))
    )
}

/// The comment form. It starts hidden: posting needs the dashboard script.
fn composer(default_author: &str) -> String {
    format!(
        r#"<form class="comment-composer" data-comment-form hidden><label class="form-label" for="comment-text">Add a comment</label><textarea id="comment-text" name="text" rows="3" maxlength="{max_text}" placeholder="Write a comment. Markdown works, and IDs like TASK-001 link up." required></textarea><p class="form-error" role="alert" hidden></p><div class="comment-actions"><label class="comment-as"><span>As</span><input type="text" name="author" placeholder="{author}" maxlength="{max}" autocomplete="name" aria-label="Your name"></label><span class="comment-hint"><kbd data-mod-enter>⌘↵</kbd> to post</span><button type="submit" class="btn btn-primary btn-sm">Comment</button></div></form>"#,
        author = escape_html(default_author),
        max = comments::MAX_AUTHOR,
        max_text = MAX_COMMENT,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityKind;
    use crate::html::catalog::tests::{catalog, rec, test_config};

    fn task_file(cfg: &crate::config::ResolvedConfig, body: &str) -> EntityRecord {
        let path = cfg.tasks_dir.join("todo").join("TASK-001-x.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let content = format!("---\nid: TASK-001\ntitle: X\n---\n{body}");
        std::fs::write(&path, &content).unwrap();
        let mut r = rec(EntityKind::Task, "TASK-001", "title: X");
        r.body = body.to_string();
        r.source_path = path;
        r
    }

    #[test]
    fn checkboxes_carry_their_line_only_when_editable() {
        let (_d, cfg) = test_config();
        let task = task_file(
            &cfg,
            "- [ ] One \"q\"\n- [x] Two\n\n## Comments\n\n### 2026-10-06 09:05 · Jane\n\n- [ ] in a comment\n",
        );
        let cat = catalog(vec![task.clone()], &cfg);
        let notes = Notes::of(&task);
        assert_eq!(notes.checks.len(), 2, "comment items aren't body items");
        assert_eq!(notes.comments.len(), 1);

        let page = Page::new(&cfg, &cat, "").with_editable(true);
        let html = body_html(&page, &task, &notes).unwrap();
        assert!(
            html.contains(r#"data-line="5" data-text="One &quot;q&quot;""#),
            "{html}"
        );
        assert!(html.contains(r#"data-line="6" data-text="Two" checked>"#));
        assert!(html.contains("<span data-check-done>1</span> of <span data-check-total>2</span>"));
        assert!(!html.contains("in a comment"));

        let read_only = Page::new(&cfg, &cat, "");
        let html = body_html(&read_only, &task, &notes).unwrap();
        assert!(!html.contains("data-line"));
        assert!(html.contains(r#"<input disabled="" type="checkbox"/>"#));
        assert!(html.contains("data-check-progress"));
    }

    #[test]
    fn misaligned_items_stay_read_only() {
        let (_d, cfg) = test_config();
        // The renderer drops %% comments that span lines; the checklist
        // module skips the item inside, so counts still line up...
        let task = task_file(&cfg, "%%\n- [ ] hidden\n%%\n- [ ] shown\n");
        let cat = catalog(vec![task.clone()], &cfg);
        let page = Page::new(&cfg, &cat, "").with_editable(true);
        let notes = Notes::of(&task);
        let html = body_html(&page, &task, &notes).unwrap();
        assert!(
            html.contains(r#"data-line="8" data-text="shown""#),
            "{html}"
        );
        // ...but if they don't, nothing is tickable.
        let mut stale = Notes::of(&task);
        stale.checks.clear();
        let html = body_html(&page, &task, &stale).unwrap();
        assert!(!html.contains("data-line"));
    }

    #[test]
    fn comments_render_sanitised_with_composer_when_editable() {
        let (_d, cfg) = test_config();
        let task = task_file(
            &cfg,
            "Body\n\n## Comments\n\n### 2026-10-06 09:05 · Jane <b>Doe</b>\n\nHi <script>x</script> TASK-001\n\n### Loose heading\n\nText\n",
        );
        let cat = catalog(vec![task.clone()], &cfg);
        let notes = Notes::of(&task);
        let page = Page::new(&cfg, &cat, "").with_editable(true);
        let html = comments_section(&page, &task, &notes);
        assert!(html.contains("data-comment-count>2<"));
        assert!(html.contains(r#"<time class="comment-time" datetime="2026-10-06T09:05">"#));
        // Raw HTML in a heading is dropped, not rendered.
        assert!(html.contains(r#"<span class="comment-author">Jane Doe</span>"#));
        assert!(!html.contains("<b>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains(r#"class="entity-link""#));
        assert!(html.contains(r#"<span class="comment-author">Loose heading</span>"#));
        assert!(html.contains("data-comment-form hidden"));
        assert!(!html.contains("<section class=\"comments\" id=\"comments\" aria-labelledby=\"comments-title\" data-comments hidden"));

        // Read-only pages have no composer, and no section without comments.
        let read_only = Page::new(&cfg, &cat, "");
        assert!(!comments_section(&read_only, &task, &notes).contains("<form"));
        let empty = task_file(&cfg, "Body\n");
        assert_eq!(comments_section(&read_only, &empty, &Notes::of(&empty)), "");
        let html = comments_section(&page, &empty, &Notes::of(&empty));
        assert!(html.contains("data-comments hidden"));
    }
}
