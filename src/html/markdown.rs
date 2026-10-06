//! Markdown rendering with wikilinks, entity auto-links and sanitised raw HTML.

use super::catalog::Catalog;
use super::format::{entity_href, escape_html, url_encode};
use crate::checklist::{markdown_options, CheckItem};
use pulldown_cmark::{CowStr, Event, Parser, Tag, TagEnd};
use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

pub(crate) static WIKILINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[([^\]|]+)(?:\|([^\]]+))?\]\]").expect("static regex"));

pub(crate) static OBSIDIAN_COMMENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)%%.*?%%").expect("static regex"));

/// Where a Markdown document lives, so relative links (`notes/a.md`,
/// `../RES-001-x/RES-001.md`, `assets/photo.jpg`) can be resolved.
pub(crate) struct DocContext<'a> {
    /// Repo root; links may not leave it.
    pub root: &'a Path,
    /// Directory of the document being rendered.
    pub dir: &'a Path,
}

/// Render markdown to HTML, resolving `[[wikilinks]]` and auto-linking entity IDs.
///
/// Raw HTML in the source is escaped except for a small set of attribute-free
/// formatting tags, and links with scripting schemes are neutralised, so notes
/// pasted from elsewhere can't run code in the dashboard.
pub fn render_markdown(md: &str, catalog: &Catalog) -> String {
    render_markdown_in(md, catalog, None)
}

/// [`render_markdown`] for a document at a known location: relative links to
/// entity files go to their detail page, other repo files to `/files/...`.
pub(crate) fn render_markdown_in(md: &str, catalog: &Catalog, doc: Option<&DocContext>) -> String {
    render_markdown_checks(md, catalog, doc, None)
}

/// [`render_markdown_in`] with tickable task lists: `checks` are the source
/// positions of the document's task-list items (see [`crate::checklist`]),
/// in order. Each checkbox carries its item's line and text so the page can
/// toggle it; the boxes stay disabled until the dashboard script enables
/// them. If the rendered items don't line up with `checks` (e.g. an item
/// hidden in a `%% comment %%`), every box stays read-only.
pub(crate) fn render_markdown_checks(
    md: &str,
    catalog: &Catalog,
    doc: Option<&DocContext>,
    checks: Option<&[CheckItem]>,
) -> String {
    // Drop Obsidian `%% comments %%`, then turn [[ID|alias]] into regular
    // markdown links so they render inline.
    let md = OBSIDIAN_COMMENT_RE.replace_all(md, "");
    let md = WIKILINK_RE.replace_all(&md, |caps: &regex::Captures| {
        let target = caps[1].trim();
        let alias = caps.get(2).map(|m| m.as_str().trim());
        match catalog
            .canonical_id(target)
            .filter(|id| catalog.name(id).is_some())
        {
            Some(id) => {
                let text = alias.or_else(|| catalog.name(id)).unwrap_or(id);
                format!("[{}]({})", text.replace(['[', ']'], ""), entity_href(id))
            }
            None => alias.unwrap_or(target).to_string(),
        }
    });

    let options = markdown_options();

    let link = |url: CowStr<'static>| -> CowStr<'static> {
        let url = safe_url(url);
        match doc {
            Some(doc) => resolve_relative(url, doc, catalog),
            None => url,
        }
    };

    // Merge consecutive raw-HTML events so multi-line comments and blocks are
    // sanitised as a whole.
    let mut events: Vec<Event> = Vec::new();
    for event in Parser::new_ext(&md, options) {
        match event {
            Event::Html(h) | Event::InlineHtml(h) => match events.last_mut() {
                Some(Event::Html(prev)) => {
                    *prev = CowStr::from(format!("{prev}{h}"));
                }
                _ => events.push(Event::Html(h)),
            },
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            }) => events.push(Event::Start(Tag::Link {
                link_type,
                dest_url: link(CowStr::from(dest_url.into_string())),
                title,
                id,
            })),
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            }) => events.push(Event::Start(Tag::Image {
                link_type,
                dest_url: link(CowStr::from(dest_url.into_string())),
                title,
                id,
            })),
            other => events.push(other),
        }
    }

    // Sanitise raw HTML and keep the allowed tags balanced: a tag opened in a
    // paragraph closes with it, and a stray closing tag is dropped, so notes
    // can't leak bold text into the rest of the page or close its containers.
    let markers = events
        .iter()
        .filter(|e| matches!(e, Event::TaskListMarker(_)))
        .count();
    let mut checks = checks.filter(|c| c.len() == markers).map(|c| c.iter());
    let mut open = OpenTags::default();
    let mut out: Vec<Event> = Vec::with_capacity(events.len());
    for event in events {
        match event {
            Event::Html(h) => out.push(Event::Html(CowStr::from(open.sanitize(&h)))),
            Event::TaskListMarker(checked) => match checks.as_mut().and_then(Iterator::next) {
                Some(item) => out.push(Event::Html(CowStr::from(format!(
                    r#"<input type="checkbox" class="task-check" disabled data-line="{}" data-text="{}"{}>
"#,
                    item.line,
                    escape_html(&item.text),
                    if checked { " checked" } else { "" }
                )))),
                None => out.push(event),
            },
            Event::Start(Tag::HtmlBlock) | Event::End(TagEnd::HtmlBlock) => out.push(event),
            Event::Start(_) => {
                open.depth += 1;
                out.push(event);
            }
            Event::End(_) => {
                let closers = open.close_to(open.depth);
                if !closers.is_empty() {
                    out.push(Event::Html(CowStr::from(closers)));
                }
                open.depth = open.depth.saturating_sub(1);
                out.push(event);
            }
            other => out.push(other),
        }
    }
    let rest = open.close_to(0);
    if !rest.is_empty() {
        out.push(Event::Html(CowStr::from(rest)));
    }

    let mut html_output = String::new();
    pulldown_cmark::html::push_html(&mut html_output, out.into_iter());
    auto_link_entity_ids(&html_output, catalog)
}

/// Replace URLs with a scripting scheme (`javascript:`, `data:`, ...) by `#`.
fn safe_url(url: CowStr) -> CowStr {
    let lower = url.trim().to_ascii_lowercase();
    match url_scheme(&lower) {
        Some(scheme) if !matches!(scheme, "http" | "https" | "mailto" | "tel") => {
            CowStr::Borrowed("#")
        }
        _ => url,
    }
}

/// The scheme of a URL (`https` in `https://x`), if it has one.
fn url_scheme(url: &str) -> Option<&str> {
    let scheme_end = url.find(':')?;
    let path_start = url.find(['/', '?', '#']).unwrap_or(usize::MAX);
    (scheme_end < path_start).then(|| &url[..scheme_end])
}

/// Point a relative link at what it names inside the repo: an entity's
/// detail page, or `/files/<path>` for any other file. Absolute URLs,
/// anchors and links leaving the repo are returned unchanged.
fn resolve_relative<'a>(url: CowStr<'a>, doc: &DocContext, catalog: &Catalog) -> CowStr<'a> {
    let raw = url.trim();
    if raw.is_empty() || raw.starts_with(['#', '/', '?']) || url_scheme(raw).is_some() {
        return url;
    }
    let (path_part, fragment) = match raw.find('#') {
        Some(i) => (&raw[..i], &raw[i..]),
        None => (raw, ""),
    };
    let path_part = path_part.split('?').next().unwrap_or_default();
    let decoded = percent_decode(path_part);
    let mut target = doc.dir.to_path_buf();
    for part in decoded.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if !target.pop() {
                    return url;
                }
            }
            p => target.push(p),
        }
    }
    let Ok(rel) = target.strip_prefix(doc.root) else {
        return url;
    };
    let href = match catalog.id_for_path(&target) {
        Some(id) => entity_href(id),
        None => {
            let segments: Vec<String> = rel
                .components()
                .map(|c| url_encode(&c.as_os_str().to_string_lossy()))
                .collect();
            if segments.is_empty() {
                return url;
            }
            format!("/files/{}", segments.join("/"))
        }
    };
    CowStr::from(format!("{href}{fragment}"))
}

/// Decode `%XX` escapes; invalid sequences are kept as written.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(b) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// Raw-HTML tags that are kept (without attributes); everything else is escaped.
static ALLOWED_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)&lt;(/?)(br|hr|b|i|u|s|em|strong|small|sub|sup|mark|kbd|del|ins|details|summary|p|div|span|ul|ol|li|blockquote|code|pre|table|thead|tbody|tr|th|td|h[1-6])\s*(/?)&gt;",
    )
    .expect("static regex")
});

/// Allowed raw tags that are still open, with the Markdown block depth they
/// were opened at.
#[derive(Default)]
struct OpenTags {
    stack: Vec<(String, usize)>,
    depth: usize,
}

impl OpenTags {
    /// Escape raw HTML, then restore allowed tags (balanced) and entities.
    fn sanitize(&mut self, raw: &str) -> String {
        static COMMENT_RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->").expect("static regex"));
        static ENTITY_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"&amp;(#[0-9]{1,7}|#x[0-9a-fA-F]{1,6}|[a-zA-Z][a-zA-Z0-9]{1,31});")
                .expect("static regex")
        });
        let without_comments = COMMENT_RE.replace_all(raw, "");
        let escaped = escape_html(&without_comments);
        let tags = ALLOWED_RE.replace_all(&escaped, |caps: &regex::Captures| {
            let tag = caps[2].to_ascii_lowercase();
            let closing = !caps[1].is_empty();
            let self_closing = !caps[3].is_empty();
            if matches!(tag.as_str(), "br" | "hr") {
                return format!("<{tag}>");
            }
            if closing {
                return match self.stack.iter().rposition(|(t, _)| *t == tag) {
                    Some(i) => self.close_from(i),
                    None => String::new(),
                };
            }
            if self_closing {
                return format!("<{tag}></{tag}>");
            }
            self.stack.push((tag.clone(), self.depth));
            format!("<{tag}>")
        });
        ENTITY_RE.replace_all(&tags, "&$1;").into_owned()
    }

    /// Close every tag from stack index `i` up, innermost first.
    fn close_from(&mut self, i: usize) -> String {
        self.stack
            .drain(i..)
            .rev()
            .map(|(t, _)| format!("</{t}>"))
            .collect()
    }

    /// Close tags opened at Markdown depth `depth` or deeper.
    fn close_to(&mut self, depth: usize) -> String {
        let i = self
            .stack
            .iter()
            .position(|(_, d)| *d >= depth)
            .unwrap_or(self.stack.len());
        self.close_from(i)
    }
}

/// Replace known entity IDs in HTML text with links (skipping existing links,
/// code, and tag attributes).
fn auto_link_entity_ids(html: &str, catalog: &Catalog) -> String {
    static SKIP_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?s)<a[\s>].*?</a>|<code>.*?</code>|<pre>.*?</pre>|<[^>]*>")
            .expect("static regex")
    });
    let link = |text: &str| -> String {
        catalog
            .id_regex()
            .replace_all(text, |caps: &regex::Captures| {
                let id = &caps[0];
                match catalog.name(id) {
                    Some(name) => format!(
                        r#"<a href="{}" class="entity-link" title="{}">{id}</a>"#,
                        entity_href(id),
                        escape_html(name)
                    ),
                    None => id.to_string(),
                }
            })
            .into_owned()
    };

    let mut result = String::with_capacity(html.len());
    let mut last = 0;
    for m in SKIP_RE.find_iter(html) {
        result.push_str(&link(&html[last..m.start()]));
        result.push_str(m.as_str());
        last = m.end();
    }
    result.push_str(&link(&html[last..]));
    result
}

/// Strip a leading H1 heading from markdown that duplicates the page title.
pub(crate) fn strip_leading_h1(md: &str) -> &str {
    match md.strip_prefix("# ") {
        Some(rest) => match rest.find('\n') {
            Some(pos) => rest[pos..].trim_start_matches('\n'),
            None => "",
        },
        None => md,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityKind;
    use crate::html::catalog::tests::{catalog, rec, test_config};

    fn render(md: &str) -> String {
        let (_d, cfg) = test_config();
        let cat = catalog(
            vec![rec(EntityKind::Task, "TASK-001", "title: First")],
            &cfg,
        );
        render_markdown(md, &cat)
    }

    #[test]
    fn strip_leading_h1_removes_title() {
        assert_eq!(strip_leading_h1("# Title\n\nBody"), "Body");
        assert_eq!(strip_leading_h1("# Title"), "");
        assert_eq!(strip_leading_h1("Body"), "Body");
    }

    #[test]
    fn obsidian_comments_are_removed() {
        assert_eq!(
            OBSIDIAN_COMMENT_RE.replace_all("a %% hidden\nstuff %% b", ""),
            "a  b"
        );
    }

    #[test]
    fn wikilinks_and_ids_become_links() {
        let html = render("See [[TASK-001]] and TASK-001, not TASK-999.");
        assert!(html.contains(r#"<a href="/entity/TASK-001">First</a>"#));
        assert!(html.contains(r#"class="entity-link" title="First">TASK-001</a>"#));
        assert!(html.contains("TASK-999"));
        assert!(!html.contains("/entity/TASK-999"));
    }

    #[test]
    fn raw_html_is_sanitised() {
        let html = render("Hi <script>alert(1)</script> <b>bold</b><br/>\n\n<img src=x onerror=alert(1)>\n\n<!-- secret -->&nbsp;");
        assert!(!html.contains("<script"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(html.contains("<b>bold</b><br>"));
        assert!(!html.contains("<img"));
        assert!(!html.contains("secret"));
    }

    #[test]
    fn raw_tags_stay_balanced() {
        // An unclosed tag ends with its paragraph; a stray closer is dropped.
        let html = render("Bold <b>start\n\nNext para </div> end");
        assert!(html.contains("<p>Bold <b>start</b></p>"), "{html}");
        assert!(html.contains("<p>Next para  end</p>"), "{html}");
        // Block tags may wrap Markdown content.
        let html =
            render("<details>\n<summary>More</summary>\n\nHidden *text*\n\n</details>\n\nAfter");
        let close = html.find("</details>").unwrap();
        assert!(html.find("<em>text</em>").unwrap() < close);
        assert!(close < html.find("After").unwrap());
        // Unclosed block tags close at the end of the document.
        let html = render("<div>\n\nInside");
        assert!(html.trim_end().ends_with("</div>"), "{html}");
    }

    #[test]
    fn relative_links_resolve_inside_the_repo() {
        let (_d, cfg) = test_config();
        let mut task = rec(EntityKind::Task, "TASK-001", "title: First");
        task.source_path = cfg.tasks_dir.join("todo").join("TASK-001-first.md");
        let cat = catalog(vec![task], &cfg);
        let dir = cfg.root.join("research").join("RES-001-x");
        let doc = DocContext {
            root: &cfg.root,
            dir: &dir,
        };
        let html = render_markdown_in(
            "[a](../../tasks/todo/TASK-001-first.md#x) [b](notes/c%20d.md) [c](https://e.com/x.md) [d](../../../etc/passwd) [e](#top)",
            &cat,
            Some(&doc),
        );
        assert!(html.contains(r#"href="/entity/TASK-001#x""#), "{html}");
        assert!(html.contains(r#"href="/files/research/RES-001-x/notes/c%20d.md""#));
        assert!(html.contains(r#"href="https://e.com/x.md""#));
        assert!(html.contains(r#"href="../../../etc/passwd""#));
        assert!(html.contains(r##"href="#top""##));
    }

    #[test]
    fn scripting_links_are_neutralised() {
        let html = render(
            "[x](javascript:alert(1)) [y](https://e.com) [z](/entity/TASK-001) [w](JavaScript:x)",
        );
        assert!(!html.to_lowercase().contains("javascript:"));
        assert!(html.contains(r#"href="https://e.com""#));
        assert!(html.contains(r#"href="/entity/TASK-001""#));
    }
}
