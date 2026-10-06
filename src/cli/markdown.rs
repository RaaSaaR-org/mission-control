//! Markdown rendering for the terminal (`mc show`).
//!
//! Walks pulldown-cmark's event stream and lays out headings, emphasis,
//! word-wrapped paragraphs, nested and task lists, quotes, boxed code blocks,
//! rules and width-aware tables with box-drawing borders. Entity references
//! (`TASK-069`, `[[TASK-069|alias]]`) and relative file links become OSC 8
//! hyperlinks to the file on disk when enabled, so cmd/ctrl-click opens them.
//!
//! Styling goes through `colored`, so it follows `--color` / `NO_COLOR` like
//! the rest of the CLI. Control characters in the source are dropped so a
//! note can't send escape sequences to the terminal.

use super::ui;
use crate::data::EntityRecord;
use crate::html::Catalog;
use colored::{Color, Colorize};
use pulldown_cmark::{
    Alignment, CodeBlockKind, CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
    TextMergeStream,
};
use regex::Regex;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Layout settings for one render.
#[derive(Clone, Copy, Debug)]
pub struct RenderOptions {
    /// Columns available for the body (excluding any left margin the caller adds).
    pub width: usize,
    /// Box-drawing characters and unicode glyphs; ASCII fallbacks otherwise.
    pub unicode: bool,
    /// Wrap links in OSC 8 escape sequences.
    pub hyperlinks: bool,
}

/// Where the document lives, so references and relative links resolve.
pub struct Doc<'a> {
    pub catalog: &'a Catalog,
    /// Repository root: relative links that leave it are not linked.
    pub root: &'a Path,
    /// Directory of the document (relative links resolve against it).
    pub dir: &'a Path,
    /// Entity title; a leading `# Title` that repeats it is skipped.
    pub title: Option<&'a str>,
}

/// Rendered lines (no trailing newlines) plus the entities the body links to.
pub struct Rendered {
    pub lines: Vec<String>,
    /// Known entity IDs referenced in the body, in order of first appearance.
    pub refs: Vec<String>,
}

/// Render `md` for the terminal.
pub fn render(md: &str, opts: &RenderOptions, doc: Option<&Doc>) -> Rendered {
    let md = sanitize(md);
    let md = OBSIDIAN_COMMENT_RE.replace_all(&md, "");
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    // Sanitise again after parsing: character references (`&#27;`) decode to
    // control characters in text and link destinations.
    let mut events: Vec<Event> = TextMergeStream::new(Parser::new_ext(&md, options))
        .map(sanitize_event)
        .collect();
    if let Some(title) = doc.and_then(|d| d.title) {
        skip_title_heading(&mut events, title);
    }

    let mut r = Renderer::new(opts, doc);
    for event in events {
        r.event(event);
    }
    r.flush();
    Rendered {
        lines: r.out,
        refs: r.refs,
    }
}

/// Drop control characters (ESC, BEL, C1 controls, ...) but keep newlines and tabs.
pub fn sanitize(s: &str) -> String {
    if !has_control(s) {
        return s.to_string();
    }
    s.chars()
        .filter(|&c| !c.is_control() || c == '\n' || c == '\t')
        .collect()
}

fn has_control(s: &str) -> bool {
    s.chars().any(|c| c.is_control() && c != '\n' && c != '\t')
}

fn clean(s: CowStr<'_>) -> CowStr<'_> {
    if has_control(&s) {
        sanitize(&s).into()
    } else {
        s
    }
}

/// `event` with control characters removed from every string it carries.
fn sanitize_event(event: Event<'_>) -> Event<'_> {
    match event {
        Event::Text(t) => Event::Text(clean(t)),
        Event::Code(t) => Event::Code(clean(t)),
        Event::InlineMath(t) => Event::InlineMath(clean(t)),
        Event::DisplayMath(t) => Event::DisplayMath(clean(t)),
        Event::Html(t) => Event::Html(clean(t)),
        Event::InlineHtml(t) => Event::InlineHtml(clean(t)),
        Event::FootnoteReference(t) => Event::FootnoteReference(clean(t)),
        Event::Start(tag) => Event::Start(match tag {
            Tag::CodeBlock(CodeBlockKind::Fenced(info)) => {
                Tag::CodeBlock(CodeBlockKind::Fenced(clean(info)))
            }
            Tag::Link {
                link_type,
                dest_url,
                title,
                id,
            } => Tag::Link {
                link_type,
                dest_url: clean(dest_url),
                title: clean(title),
                id: clean(id),
            },
            Tag::Image {
                link_type,
                dest_url,
                title,
                id,
            } => Tag::Image {
                link_type,
                dest_url: clean(dest_url),
                title: clean(title),
                id: clean(id),
            },
            Tag::FootnoteDefinition(name) => Tag::FootnoteDefinition(clean(name)),
            other => other,
        }),
        other => other,
    }
}

/// `file://` URL for a path, percent-encoding everything outside the
/// unreserved set so the URL is plain ASCII.
pub fn file_url(path: &Path) -> String {
    let abs = normalize(&std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf()));
    let raw = abs.to_string_lossy().replace('\\', "/");
    let mut url = String::from("file://");
    if !raw.starts_with('/') {
        url.push('/');
    }
    for b in raw.bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    url
}

static WIKILINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[([^\]|]+)(?:\|([^\]]+))?\]\]").expect("static regex"));

static OBSIDIAN_COMMENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)%%.*?%%").expect("static regex"));

static HTML_TAG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->|<[^>]*>").expect("static regex"));

/// Remove a leading level-1 heading whose text equals the entity title (the
/// header above the body already shows it).
fn skip_title_heading(events: &mut Vec<Event>, title: &str) {
    if !matches!(
        events.first(),
        Some(Event::Start(Tag::Heading {
            level: HeadingLevel::H1,
            ..
        }))
    ) {
        return;
    }
    let Some(end) = events
        .iter()
        .position(|e| matches!(e, Event::End(TagEnd::Heading(_))))
    else {
        return;
    };
    let text: String = events[1..end]
        .iter()
        .filter_map(|e| match e {
            Event::Text(t) | Event::Code(t) => Some(t.as_ref()),
            _ => None,
        })
        .collect();
    let norm = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    if norm(&text) == norm(title) {
        events.drain(..=end);
    }
}

// ---------------------------------------------------------------------------
// Inline spans and word wrapping
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Style {
    bold: bool,
    italic: bool,
    strike: bool,
    underline: bool,
    dim: bool,
    color: Option<Color>,
}

impl Style {
    fn dim() -> Self {
        Style {
            dim: true,
            ..Style::default()
        }
    }
}

/// A run of text with one style and an optional link target. `\n` in the
/// text is a hard line break.
#[derive(Clone, Debug, PartialEq)]
struct Span {
    text: String,
    style: Style,
    link: Option<String>,
}

impl Span {
    fn new(text: impl Into<String>, style: Style) -> Self {
        Span {
            text: text.into(),
            style,
            link: None,
        }
    }
    fn same_format(&self, other: &Span) -> bool {
        self.style == other.style && self.link == other.link
    }
}

fn char_width(c: char) -> usize {
    c.width().unwrap_or(0)
}

/// Append `span` to `line`, merging it into the last span when formatted alike.
fn push_merged(line: &mut Vec<Span>, span: Span) {
    match line.last_mut() {
        Some(last) if last.same_format(&span) => last.text.push_str(&span.text),
        _ => line.push(span),
    }
}

/// A word: the text between spaces, possibly mixing styles (`**bold**,`).
#[derive(Default)]
struct Word {
    spans: Vec<Span>,
    width: usize,
    /// Format of the space before this word (so links keep spaces inside them).
    gap: Option<Span>,
    /// A hard line break precedes this word.
    brk: bool,
}

fn words(spans: &[Span]) -> Vec<Word> {
    let mut out = Vec::new();
    let mut cur = Word::default();
    let mut gap: Option<Span> = None;
    let mut brk = false;
    for span in spans {
        for c in span.text.chars() {
            if c == '\n' {
                if !cur.spans.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                brk = true;
                gap = None;
                continue;
            }
            if c == ' ' || c == '\t' {
                if cur.width > 0 || !cur.spans.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                if gap.is_none() {
                    gap = Some(Span {
                        text: " ".into(),
                        style: span.style,
                        link: span.link.clone(),
                    });
                }
                continue;
            }
            if cur.spans.is_empty() {
                cur.gap = gap.take();
                cur.brk = std::mem::take(&mut brk);
            }
            push_merged(
                &mut cur.spans,
                Span {
                    text: c.to_string(),
                    style: span.style,
                    link: span.link.clone(),
                },
            );
            cur.width += char_width(c);
        }
    }
    if !cur.spans.is_empty() {
        out.push(cur);
    }
    out
}

/// Word-wrap spans to `width` columns. Words wider than a line are split.
fn wrap(spans: &[Span], width: usize) -> Vec<Vec<Span>> {
    let width = width.max(1);
    let mut lines: Vec<Vec<Span>> = Vec::new();
    let mut line: Vec<Span> = Vec::new();
    let mut w = 0;
    for word in words(spans) {
        if word.brk || (w > 0 && w + 1 + word.width > width) {
            lines.push(std::mem::take(&mut line));
            w = 0;
        }
        if w > 0 {
            let gap = word
                .gap
                .clone()
                .unwrap_or_else(|| Span::new(" ", Style::default()));
            push_merged(&mut line, gap);
            w += 1;
        }
        if word.width <= width - w {
            for s in word.spans {
                push_merged(&mut line, s);
            }
            w += word.width;
            continue;
        }
        // Too long for any line (URLs, paths): break it by characters.
        for s in word.spans {
            for c in s.text.chars() {
                let cw = char_width(c);
                if w + cw > width && w > 0 {
                    lines.push(std::mem::take(&mut line));
                    w = 0;
                }
                push_merged(
                    &mut line,
                    Span {
                        text: c.to_string(),
                        style: s.style,
                        link: s.link.clone(),
                    },
                );
                w += cw;
            }
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| s.text.width()).sum()
}

fn paint(text: &str, st: Style) -> String {
    if st == Style::default() {
        return text.to_string();
    }
    let mut c = text.normal();
    if let Some(color) = st.color {
        c = c.color(color);
    }
    if st.bold {
        c = c.bold();
    }
    if st.italic {
        c = c.italic();
    }
    if st.underline {
        c = c.underline();
    }
    if st.strike {
        c = c.strikethrough();
    }
    if st.dim {
        c = c.dimmed();
    }
    c.to_string()
}

fn paint_line(spans: &[Span], hyperlinks: bool) -> String {
    let mut out = String::new();
    // Consecutive spans with the same target share one hyperlink.
    for group in spans.chunk_by(|a, b| a.link == b.link) {
        let text: String = group.iter().map(|s| paint(&s.text, s.style)).collect();
        match &group[0].link {
            Some(url) if hyperlinks => out.push_str(&ui::hyperlink(url, &text)),
            _ => out.push_str(&text),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Glyphs
// ---------------------------------------------------------------------------

struct Marks {
    bullets: [&'static str; 3],
    todo: &'static str,
    done: &'static str,
    quote: &'static str,
    rule: &'static str,
    h1: &'static str,
    h2: &'static str,
    image: &'static str,
    /// Code box: top-left, vertical, bottom-left, horizontal.
    code: [&'static str; 4],
    /// Table: `[left, mid, right]` for top, separator, bottom rows.
    top: [&'static str; 3],
    sep: [&'static str; 3],
    head_sep: [&'static str; 3],
    bottom: [&'static str; 3],
    h: &'static str,
    head_h: &'static str,
    v: &'static str,
}

const UNICODE_MARKS: Marks = Marks {
    bullets: ["•", "◦", "▪"],
    todo: "☐",
    done: "☑",
    quote: "│",
    rule: "─",
    h1: "━",
    h2: "─",
    image: "▣",
    code: ["╭", "│", "╰", "─"],
    top: ["┌", "┬", "┐"],
    sep: ["├", "┼", "┤"],
    head_sep: ["├", "┼", "┤"],
    bottom: ["└", "┴", "┘"],
    h: "─",
    head_h: "─",
    v: "│",
};

const ASCII_MARKS: Marks = Marks {
    bullets: ["-", "*", "+"],
    todo: "[ ]",
    done: "[x]",
    quote: "|",
    rule: "-",
    h1: "=",
    h2: "-",
    image: "[img]",
    code: ["+", "|", "+", "-"],
    top: ["+", "+", "+"],
    sep: ["+", "+", "+"],
    head_sep: ["+", "+", "+"],
    bottom: ["+", "+", "+"],
    h: "-",
    head_h: "=",
    v: "|",
};

// ---------------------------------------------------------------------------
// Block layout
// ---------------------------------------------------------------------------

enum Container {
    Quote,
    Item {
        marker: String,
        width: usize,
        used: bool,
        checked: bool,
    },
}

struct LinkCtx {
    url: Option<String>,
    entity: bool,
    /// Shown after the link text when hyperlinks are off (`(https://…)`).
    suffix: Option<String>,
    /// Index into the inline buffer where the link text starts.
    start: usize,
}

struct TableState {
    aligns: Vec<Alignment>,
    rows: Vec<Vec<Vec<Span>>>,
    row: Vec<Vec<Span>>,
    in_head: bool,
}

struct Renderer<'a> {
    opts: &'a RenderOptions,
    marks: &'static Marks,
    doc: Option<&'a Doc<'a>>,
    by_id: HashMap<&'a str, &'a EntityRecord>,
    out: Vec<String>,
    refs: Vec<String>,
    inline: Vec<Span>,
    containers: Vec<Container>,
    lists: Vec<Option<u64>>,
    /// A blank line is owed before the next block.
    blank: bool,
    bold: usize,
    italic: usize,
    strike: usize,
    heading: Option<HeadingLevel>,
    links: Vec<LinkCtx>,
    code: Option<(String, String)>,
    html: Option<String>,
    table: Option<TableState>,
}

impl<'a> Renderer<'a> {
    fn new(opts: &'a RenderOptions, doc: Option<&'a Doc<'a>>) -> Self {
        let by_id = doc
            .map(|d| {
                d.catalog
                    .records
                    .iter()
                    .map(|r| (r.id.as_str(), r))
                    .collect()
            })
            .unwrap_or_default();
        Renderer {
            opts,
            marks: if opts.unicode {
                &UNICODE_MARKS
            } else {
                &ASCII_MARKS
            },
            doc,
            by_id,
            out: Vec::new(),
            refs: Vec::new(),
            inline: Vec::new(),
            containers: Vec::new(),
            lists: Vec::new(),
            blank: false,
            bold: 0,
            italic: 0,
            strike: 0,
            heading: None,
            links: Vec::new(),
            code: None,
            html: None,
            table: None,
        }
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.text(&t),
            Event::Code(t) => {
                let mut st = self.style();
                st.color = Some(Color::Yellow);
                st.underline = false;
                self.push(&t, st);
            }
            Event::InlineMath(t) | Event::DisplayMath(t) => self.text(&t),
            Event::Html(h) => match self.html.as_mut() {
                Some(buf) => buf.push_str(&h),
                None => self.inline_html(&h),
            },
            Event::InlineHtml(h) => self.inline_html(&h),
            // Keep the author's line breaks, like Obsidian and the old view:
            // `**Owner:** x` lines would otherwise run together.
            Event::SoftBreak | Event::HardBreak => {
                self.inline.push(Span::new("\n", Style::default()))
            }
            Event::Rule => {
                self.flush();
                self.open_block();
                let w = self.opts.width.saturating_sub(self.prefix_width()).max(3);
                let line = self.marks.rule.repeat(w).dimmed().to_string();
                self.emit(line);
                self.blank = true;
            }
            Event::TaskListMarker(checked) => self.task_marker(checked),
            Event::FootnoteReference(name) => self.push(&format!("[^{name}]"), Style::dim()),
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                self.flush();
                self.open_block();
            }
            Tag::Heading { level, .. } => {
                self.flush();
                self.open_block();
                self.heading = Some(level);
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.open_block();
                self.containers.push(Container::Quote);
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                self.open_block();
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_string()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::HtmlBlock => {
                self.flush();
                self.html = Some(String::new());
            }
            Tag::List(start) => {
                self.flush();
                self.open_block();
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush();
                let depth = self.lists.len().saturating_sub(1);
                let (marker, width) = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}.");
                        *n += 1;
                        let w = m.len() + 1;
                        (format!("{} ", m.cyan()), w)
                    }
                    _ => {
                        let b = self.marks.bullets[depth % 3];
                        (format!("{} ", b.cyan()), b.width() + 1)
                    }
                };
                self.containers.push(Container::Item {
                    marker,
                    width,
                    used: false,
                    checked: false,
                });
            }
            Tag::Table(aligns) => {
                self.flush();
                self.open_block();
                self.table = Some(TableState {
                    aligns,
                    rows: Vec::new(),
                    row: Vec::new(),
                    in_head: false,
                });
            }
            Tag::TableHead => {
                if let Some(t) = self.table.as_mut() {
                    t.in_head = true;
                    t.row.clear();
                }
            }
            Tag::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    t.row.clear();
                }
            }
            Tag::TableCell => self.inline.clear(),
            Tag::Emphasis => self.italic += 1,
            Tag::Strong => self.bold += 1,
            Tag::Strikethrough => self.strike += 1,
            Tag::Link { dest_url, .. } => self.open_link(&dest_url, None),
            Tag::Image { dest_url, .. } => {
                let glyph = self.marks.image;
                self.open_link(&dest_url, Some(glyph));
            }
            Tag::FootnoteDefinition(name) => {
                self.flush();
                self.open_block();
                self.push(&format!("[^{name}]: "), Style::dim());
            }
            Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::MetadataBlock(_) => {
                self.flush();
            }
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::FootnoteDefinition => {
                self.flush();
                self.blank = true;
            }
            TagEnd::Heading(level) => {
                let w = self.flush();
                self.heading = None;
                let rule = match level {
                    HeadingLevel::H1 => Some(self.marks.h1.repeat(w).cyan().to_string()),
                    HeadingLevel::H2 => Some(self.marks.h2.repeat(w).dimmed().to_string()),
                    _ => None,
                };
                if let Some(rule) = rule.filter(|_| w > 0) {
                    self.emit(rule);
                }
                self.blank = true;
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.containers.pop();
                self.blank = true;
            }
            TagEnd::CodeBlock => {
                if let Some((lang, text)) = self.code.take() {
                    self.code_block(&lang, &text);
                }
                self.blank = true;
            }
            TagEnd::HtmlBlock => {
                let raw = self.html.take().unwrap_or_default();
                let text = HTML_TAG_RE.replace_all(&raw, "");
                let lines: Vec<&str> = text
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .collect();
                if !lines.is_empty() {
                    self.open_block();
                    for (i, l) in lines.iter().enumerate() {
                        if i > 0 {
                            self.inline.push(Span::new("\n", Style::default()));
                        }
                        self.push(&decode_entities(l), Style::dim());
                    }
                    self.flush();
                    self.blank = true;
                }
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                // Tight nested lists continue their parent item without a gap.
                self.blank = !self
                    .containers
                    .iter()
                    .any(|c| matches!(c, Container::Item { .. }));
            }
            TagEnd::Item => {
                self.flush();
                if let Some(Container::Item { used: false, .. }) = self.containers.last() {
                    // An empty item still shows its bullet.
                    self.emit(String::new());
                }
                self.containers.pop();
            }
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    self.render_table(t);
                }
                self.blank = true;
            }
            TagEnd::TableHead => {
                if let Some(t) = self.table.as_mut() {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push(row);
                    t.in_head = false;
                }
            }
            TagEnd::TableRow => {
                if let Some(t) = self.table.as_mut() {
                    let row = std::mem::take(&mut t.row);
                    t.rows.push(row);
                }
            }
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.inline);
                if let Some(t) = self.table.as_mut() {
                    t.row.push(cell);
                }
            }
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Link | TagEnd::Image => self.close_link(),
            TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::MetadataBlock(_) => {
                self.flush();
                self.blank = true;
            }
        }
    }

    /// Style for text at the current position.
    fn style(&self) -> Style {
        let mut st = Style {
            bold: self.bold > 0,
            italic: self.italic > 0,
            strike: self.strike > 0,
            ..Style::default()
        };
        if let Some(level) = self.heading {
            st.bold = true;
            if level <= HeadingLevel::H2 {
                st.color = Some(Color::Cyan);
            }
        }
        if self.table.as_ref().is_some_and(|t| t.in_head) {
            st.bold = true;
        }
        if let Some(link) = self.links.last() {
            if link.entity {
                st.color = Some(Color::Cyan);
            } else if link.url.is_some() {
                st.color = Some(Color::Blue);
                st.underline = true;
            }
        }
        if self
            .containers
            .iter()
            .any(|c| matches!(c, Container::Item { checked: true, .. }))
        {
            st.dim = true;
        }
        st
    }

    fn push(&mut self, text: &str, style: Style) {
        let link = self.links.last().and_then(|l| l.url.clone());
        push_merged(
            &mut self.inline,
            Span {
                text: text.to_string(),
                style,
                link,
            },
        );
    }

    fn text(&mut self, text: &str) {
        if let Some((_, buf)) = self.code.as_mut() {
            buf.push_str(text);
            return;
        }
        if let Some(buf) = self.html.as_mut() {
            buf.push_str(text);
            return;
        }
        let style = self.style();
        match self.doc {
            Some(doc) if self.links.is_empty() => {
                let mut last = 0;
                for caps in WIKILINK_RE.captures_iter(text) {
                    let m = caps.get(0).expect("match");
                    self.text_with_ids(&text[last..m.start()], doc, style);
                    let alias = caps.get(2).map(|a| a.as_str().trim());
                    self.wikilink(caps[1].trim(), alias, doc, style);
                    last = m.end();
                }
                self.text_with_ids(&text[last..], doc, style);
            }
            _ => self.push(text, style),
        }
    }

    /// Plain text with bare entity IDs turned into references.
    fn text_with_ids(&mut self, text: &str, doc: &Doc, style: Style) {
        let mut last = 0;
        for m in doc.catalog.id_regex().find_iter(text) {
            self.push(&text[last..m.start()], style);
            let id = m.as_str();
            match self.entity_url(id) {
                Some(url) => {
                    let mut st = style;
                    st.color = Some(Color::Cyan);
                    self.push_link(id, st, url);
                }
                None => {
                    let mut st = style;
                    st.dim = true;
                    self.push(id, st);
                }
            }
            last = m.end();
        }
        self.push(&text[last..], style);
    }

    /// `[[TASK-069]]` → `TASK-069 Title`; `[[TASK-069|alias]]` → `alias`.
    fn wikilink(&mut self, target: &str, alias: Option<&str>, doc: &Doc, style: Style) {
        let id = doc.catalog.canonical_id(target);
        let url = id.and_then(|id| self.entity_url(id));
        let (Some(id), Some(url)) = (id, url) else {
            let mut st = style;
            st.dim = true;
            self.push(alias.unwrap_or(target), st);
            return;
        };
        let mut st = style;
        st.color = Some(Color::Cyan);
        match alias {
            Some(alias) => self.push_link(alias, st, url),
            None => {
                let mut id_st = st;
                id_st.bold = true;
                self.push_link(id, id_st, url.clone());
                if let Some(name) = doc.catalog.name(id).filter(|n| *n != id) {
                    self.push_link(&format!(" {name}"), st, url);
                }
            }
        }
    }

    fn push_link(&mut self, text: &str, style: Style, url: String) {
        push_merged(
            &mut self.inline,
            Span {
                text: text.to_string(),
                style,
                link: Some(url),
            },
        );
    }

    /// File URL of a known entity, recording it as a reference.
    fn entity_url(&mut self, id: &str) -> Option<String> {
        let rec = self.by_id.get(id)?;
        if !self.refs.iter().any(|r| r == id) {
            self.refs.push(id.to_string());
        }
        Some(file_url(&rec.source_path))
    }

    fn open_link(&mut self, dest: &str, glyph: Option<&str>) {
        let dest = dest.trim();
        let (url, entity) = self.resolve(dest);
        let web = matches!(url_scheme(dest), Some("http" | "https" | "mailto"));
        let suffix = (web && !self.opts.hyperlinks).then(|| dest.to_string());
        self.links.push(LinkCtx {
            url,
            entity,
            suffix,
            start: self.inline.len(),
        });
        if let Some(g) = glyph {
            let st = self.style();
            self.push(&format!("{g} "), st);
        }
    }

    fn close_link(&mut self) {
        let Some(link) = self.links.pop() else {
            return;
        };
        if let Some(suffix) = link.suffix {
            let text: String = self.inline[link.start.min(self.inline.len())..]
                .iter()
                .map(|s| s.text.as_str())
                .collect();
            let bare = suffix.strip_prefix("mailto:").unwrap_or(&suffix);
            if text.trim() != suffix && text.trim() != bare {
                self.push(&format!(" ({suffix})"), Style::dim());
            }
        }
    }

    /// Resolve a link destination to a URL; `true` when it names an entity.
    fn resolve(&mut self, dest: &str) -> (Option<String>, bool) {
        if dest.is_empty() || dest.starts_with('#') {
            return (None, false);
        }
        match url_scheme(dest) {
            // Same policy as the dashboard: web and mail links only, no `file:`.
            Some("http" | "https" | "mailto") => return (Some(dest.to_string()), false),
            Some(_) => return (None, false),
            None => {}
        }
        let Some(doc) = self.doc else {
            return (None, false);
        };
        let path = dest.split(['#', '?']).next().unwrap_or_default();
        let decoded = percent_decode(path);
        let target = normalize(&doc.dir.join(&decoded));
        if let Some(id) = doc.catalog.id_for_path(&target).map(str::to_string) {
            return (self.entity_url(&id), true);
        }
        if target.starts_with(normalize(doc.root)) && target.exists() {
            (Some(file_url(&target)), false)
        } else {
            (None, false)
        }
    }

    fn inline_html(&mut self, html: &str) {
        let tag = html.trim().to_ascii_lowercase().replace(' ', "");
        if matches!(tag.as_str(), "<br>" | "<br/>") {
            self.inline.push(Span::new("\n", Style::default()));
        }
    }

    fn task_marker(&mut self, checked: bool) {
        let m = self.marks;
        if let Some(Container::Item {
            marker,
            width,
            checked: c,
            ..
        }) = self
            .containers
            .iter_mut()
            .rev()
            .find(|c| matches!(c, Container::Item { .. }))
        {
            let glyph = if checked { m.done } else { m.todo };
            *width = glyph.width() + 1;
            *marker = if checked {
                format!("{} ", glyph.green())
            } else {
                format!("{} ", glyph.dimmed())
            };
            *c = checked;
        }
    }

    // -- output -------------------------------------------------------------

    fn prefix_width(&self) -> usize {
        self.containers
            .iter()
            .map(|c| match c {
                Container::Quote => 2,
                Container::Item { width, .. } => *width,
            })
            .sum()
    }

    /// Prefix for the next line; the first line of an item shows its marker.
    fn take_prefix(&mut self) -> String {
        let quote = self.marks.quote;
        let mut out = String::new();
        for c in &mut self.containers {
            match c {
                Container::Quote => out.push_str(&format!("{} ", quote.dimmed())),
                Container::Item {
                    marker,
                    width,
                    used,
                    ..
                } => {
                    if *used {
                        out.push_str(&" ".repeat(*width));
                    } else {
                        out.push_str(marker);
                        *used = true;
                    }
                }
            }
        }
        out
    }

    fn emit(&mut self, body: String) {
        let prefix = self.take_prefix();
        self.out
            .push(format!("{prefix}{body}").trim_end().to_string());
    }

    /// Emit the blank line owed by the previous block, if any.
    fn open_block(&mut self) {
        if self.blank && !self.out.is_empty() {
            let mut line = String::new();
            for c in &self.containers {
                match c {
                    Container::Quote => line.push_str(&format!("{} ", self.marks.quote.dimmed())),
                    Container::Item { width, .. } => line.push_str(&" ".repeat(*width)),
                }
            }
            self.out.push(line.trim_end().to_string());
        }
        self.blank = false;
    }

    /// Wrap and emit the pending inline text. Returns the widest line.
    fn flush(&mut self) -> usize {
        if self.inline.is_empty() {
            return 0;
        }
        let spans = std::mem::take(&mut self.inline);
        if spans.iter().all(|s| s.text.trim().is_empty()) {
            return 0;
        }
        self.open_block();
        let avail = self.opts.width.saturating_sub(self.prefix_width()).max(8);
        let mut widest = 0;
        for line in wrap(&spans, avail) {
            widest = widest.max(spans_width(&line));
            let body = paint_line(&line, self.opts.hyperlinks);
            self.emit(body);
        }
        widest
    }

    fn code_block(&mut self, lang: &str, text: &str) {
        let [tl, v, bl, h] = self.marks.code;
        let lines: Vec<String> = text
            .strip_suffix('\n')
            .unwrap_or(text)
            .split('\n')
            .map(|l| l.replace('\t', "    "))
            .collect();
        let avail = self.opts.width.saturating_sub(self.prefix_width()).max(8);
        let inner = lines.iter().map(|l| l.width()).max().unwrap_or(0);
        let label = if lang.is_empty() {
            String::new()
        } else {
            format!(" {lang} ")
        };
        let box_w = (inner + 2).max(label.width() + 4).min(avail).max(2);
        let top_fill = box_w.saturating_sub(2 + label.width());
        let top = format!(
            "{}{}{}{}",
            tl.dimmed(),
            h.dimmed(),
            label.dimmed(),
            h.repeat(top_fill).dimmed()
        );
        self.emit(top);
        for line in &lines {
            let body = format!("{} {}", v.dimmed(), line.yellow());
            self.emit(body);
        }
        let bottom = format!("{}{}", bl.dimmed(), h.repeat(box_w - 1).dimmed());
        self.emit(bottom);
    }

    fn render_table(&mut self, t: TableState) {
        let m = self.marks;
        let cols = t
            .rows
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or(0)
            .max(t.aligns.len());
        if cols == 0 {
            return;
        }
        let rows: Vec<Vec<Vec<Span>>> = t
            .rows
            .into_iter()
            .map(|mut r| {
                r.resize_with(cols, Vec::new);
                r
            })
            .collect();
        let mut natural = vec![1; cols];
        let mut longest = vec![1; cols];
        for row in &rows {
            for (i, cell) in row.iter().enumerate() {
                let ws = words(cell);
                let w: usize =
                    ws.iter().map(|w| w.width).sum::<usize>() + ws.len().saturating_sub(1);
                natural[i] = natural[i].max(w);
                longest[i] = longest[i].max(ws.iter().map(|w| w.width).max().unwrap_or(0));
            }
        }
        let avail = self.opts.width.saturating_sub(self.prefix_width());
        let widths = fit_columns(&natural, &longest, avail.saturating_sub(3 * cols + 1));

        let border = |ends: [&str; 3], fill: &str| -> String {
            let segs: Vec<String> = widths.iter().map(|w| fill.repeat(w + 2)).collect();
            format!("{}{}{}", ends[0], segs.join(ends[1]), ends[2])
                .dimmed()
                .to_string()
        };
        let laid: Vec<Vec<Vec<Vec<Span>>>> = rows
            .iter()
            .map(|row| {
                row.iter()
                    .zip(&widths)
                    .map(|(cell, w)| {
                        if cell.is_empty() {
                            vec![Vec::new()]
                        } else {
                            wrap(cell, *w)
                        }
                    })
                    .collect()
            })
            .collect();
        let tall = laid
            .iter()
            .skip(1)
            .any(|row| row.iter().any(|c| c.len() > 1));

        let v = m.v.dimmed().to_string();
        let mut lines = vec![border(m.top, m.h)];
        for (r, row) in laid.iter().enumerate() {
            if r == 1 {
                lines.push(border(m.head_sep, m.head_h));
            } else if r > 1 && tall {
                lines.push(border(m.sep, m.h));
            }
            let height = row.iter().map(Vec::len).max().unwrap_or(1);
            for k in 0..height {
                let mut line = v.clone();
                for (i, cell) in row.iter().enumerate() {
                    let empty = Vec::new();
                    let part = cell.get(k).unwrap_or(&empty);
                    let text = paint_line(part, self.opts.hyperlinks);
                    let pad = widths[i].saturating_sub(spans_width(part));
                    let align = t.aligns.get(i).copied().unwrap_or(Alignment::None);
                    let (l, rgt) = match align {
                        Alignment::Right => (pad, 0),
                        Alignment::Center => (pad / 2, pad - pad / 2),
                        Alignment::Left | Alignment::None => (0, pad),
                    };
                    line.push_str(&format!(" {}{text}{} {v}", " ".repeat(l), " ".repeat(rgt)));
                }
                lines.push(line);
            }
        }
        lines.push(border(m.bottom, m.h));
        for line in lines {
            self.emit(line);
        }
    }
}

/// Column widths that fit `avail` (cells only, borders excluded). Columns
/// keep their natural width when everything fits; otherwise narrow columns
/// stay whole, wide ones keep room for their longest word (capped), and the
/// rest is shared in proportion to what each column still lacks.
fn fit_columns(natural: &[usize], longest: &[usize], avail: usize) -> Vec<usize> {
    const MIN: usize = 3;
    const WORD_CAP: usize = 14;
    if natural.iter().sum::<usize>() <= avail {
        return natural.to_vec();
    }
    // Columns narrower than an even share stay whole; the rest start at
    // their longest word.
    let fair = avail / natural.len().max(1);
    let mut widths: Vec<usize> = natural
        .iter()
        .zip(longest)
        .map(|(&n, &l)| {
            if n <= fair {
                n
            } else {
                n.min(l.clamp(MIN, WORD_CAP))
            }
        })
        .collect();
    // Still too wide: shave the widest columns down to the minimum.
    while widths.iter().sum::<usize>() > avail {
        let Some((i, _)) = widths
            .iter()
            .enumerate()
            .filter(|(_, &w)| w > MIN)
            .max_by_key(|(_, &w)| w)
        else {
            break;
        };
        widths[i] -= 1;
    }
    let mut spare = avail.saturating_sub(widths.iter().sum());
    let lacking: Vec<usize> = natural.iter().zip(&widths).map(|(n, w)| n - w).collect();
    let total: usize = lacking.iter().sum();
    if total > 0 && spare > 0 {
        let share = spare;
        for (w, &lack) in widths.iter_mut().zip(&lacking) {
            let add = (share * lack / total).min(lack);
            *w += add;
            spare -= add;
        }
        while spare > 0 {
            let Some((i, _)) = natural
                .iter()
                .zip(&widths)
                .enumerate()
                .filter(|(_, (n, w))| w < n)
                .max_by_key(|(_, (n, w))| *n - *w)
            else {
                break;
            };
            widths[i] += 1;
            spare -= 1;
        }
    }
    widths
}

/// The scheme of a URL (`https` in `https://x`), if it has one.
fn url_scheme(url: &str) -> Option<&str> {
    let end = url.find(':')?;
    let path_start = url.find(['/', '?', '#']).unwrap_or(usize::MAX);
    let scheme = &url[..end];
    (end < path_start
        && end > 1
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)))
    .then_some(scheme)
}

/// Lexically resolve `.` and `..` components.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
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

/// Decode the handful of HTML entities common in pasted HTML blocks.
fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{load_config, RepoMode};
    use crate::entity::EntityKind;

    /// Strip ANSI styling and OSC 8 links (colours depend on the environment).
    fn plain(s: &str) -> String {
        static RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r"\x1b\]8;;[^\x1b]*\x1b\\|\x1b\[[0-9;]*m").expect("static regex")
        });
        RE.replace_all(s, "").into_owned()
    }

    fn opts(width: usize, unicode: bool) -> RenderOptions {
        RenderOptions {
            width,
            unicode,
            hyperlinks: false,
        }
    }

    fn lines(md: &str, width: usize, unicode: bool) -> Vec<String> {
        render(md, &opts(width, unicode), None)
            .lines
            .iter()
            .map(|l| plain(l))
            .collect()
    }

    /// A catalog with two tasks, plus a scratch dir holding a `notes.md` file.
    fn fixture() -> (tempfile::TempDir, Catalog) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::write(dir.path().join("config/config.yml"), "site:\n  name: T\n").unwrap();
        std::fs::write(dir.path().join("notes.md"), "hi").unwrap();
        let cfg = load_config(dir.path(), RepoMode::Standalone).unwrap();
        let rec = |id: &str, title: &str| EntityRecord {
            kind: EntityKind::Task,
            id: id.to_string(),
            frontmatter: serde_yaml::from_str(&format!("title: {title}\nstatus: todo")).unwrap(),
            body: String::new(),
            source_path: dir.path().join(format!("tasks/todo/{id}-x.md")),
        };
        let catalog = Catalog::from_records(
            vec![rec("TASK-001", "Fix login"), rec("TASK-002", "Write docs")],
            Vec::new(),
            &cfg,
        );
        (dir, catalog)
    }

    #[test]
    fn headings_are_underlined_and_title_is_skipped() {
        let out = lines("# Big\n\n## Section\n\ntext", 40, true);
        assert_eq!(
            out,
            vec!["Big", "━━━", "", "Section", "───────", "", "text"]
        );
        let ascii = lines("## Section", 40, false);
        assert_eq!(ascii, vec!["Section", "-------"]);

        let (dir, catalog) = fixture();
        let doc = Doc {
            catalog: &catalog,
            root: dir.path(),
            dir: dir.path(),
            title: Some("My  Task"),
        };
        let r = render("# my task\n\nbody", &opts(40, true), Some(&doc));
        assert_eq!(r.lines, vec!["body"]);
    }

    #[test]
    fn paragraphs_wrap_to_width_and_keep_line_breaks() {
        let md = "Grüße aus Köln — ein recht langer Satz mit Umlauten, der umbrechen muss.\n**Owner:** Jürgen";
        let out = lines(md, 24, true);
        for l in &out {
            assert!(l.width() <= 24, "too wide: {l:?}");
        }
        assert!(out.len() > 3, "{out:?}");
        assert_eq!(out.last().unwrap(), "Owner: Jürgen");
        // Wide characters count double.
        let cjk = lines("日本語のテキスト 日本語のテキスト", 10, true);
        assert!(cjk.iter().all(|l| l.width() <= 10), "{cjk:?}");
    }

    #[test]
    fn long_words_are_split() {
        let out = lines(
            "see https://example.com/a/very/long/path/that/never/ends",
            20,
            true,
        );
        assert!(out.iter().all(|l| l.width() <= 20), "{out:?}");
        assert_eq!(
            out.concat().replace(' ', ""),
            "seehttps://example.com/a/very/long/path/that/never/ends"
        );
    }

    #[test]
    fn nested_and_ordered_lists() {
        let out = lines("- a\n  - b\n    - c\n- d\n\n3. three\n4. four", 40, true);
        assert_eq!(
            out,
            vec!["• a", "  ◦ b", "    ▪ c", "• d", "", "3. three", "4. four"]
        );
        let ascii = lines("- a\n  - b", 40, false);
        assert_eq!(ascii, vec!["- a", "  * b"]);
        // Continuation lines hang under the item text.
        let wrapped = lines("- one two three four five six seven", 14, true);
        assert_eq!(wrapped[..2], ["• one two", "  three four"]);
    }

    #[test]
    fn task_lists_use_checkboxes() {
        let md = "- [ ] open\n- [x] closed";
        assert_eq!(lines(md, 40, true), vec!["☐ open", "☑ closed"]);
        assert_eq!(lines(md, 40, false), vec!["[ ] open", "[x] closed"]);
    }

    #[test]
    fn quotes_get_a_bar_and_rules_span_the_width() {
        let out = lines("> quoted\n>\n> more\n\n---", 10, true);
        assert_eq!(out, vec!["│ quoted", "│", "│ more", "", "──────────"]);
    }

    #[test]
    fn code_blocks_are_boxed_and_never_wrapped() {
        let long = "x".repeat(50);
        let md = format!("```rust\nfn main() {{}}\n\t{long}\n```");
        let out = lines(&md, 30, true);
        assert!(out[0].starts_with("╭─ rust "), "{out:?}");
        assert_eq!(out[1], "│ fn main() {}");
        assert_eq!(out[2], format!("│     {long}"));
        assert!(out[3].starts_with("╰─"));
        let ascii = lines("```\ncode\n```", 30, false);
        assert_eq!(ascii[1], "| code");
        assert!(ascii[0].starts_with("+-"));
    }

    #[test]
    fn tables_align_columns() {
        let md = "| Left | Mid | Right |\n|:-----|:---:|------:|\n| a | b | c |\n| **bold** | `x` | 10 |";
        let out = lines(md, 60, true);
        assert_eq!(
            out,
            vec![
                "┌──────┬─────┬───────┐",
                "│ Left │ Mid │ Right │",
                "├──────┼─────┼───────┤",
                "│ a    │  b  │     c │",
                "│ bold │  x  │    10 │",
                "└──────┴─────┴───────┘",
            ]
        );
        let ascii = lines(md, 60, false);
        assert_eq!(ascii[0], "+------+-----+-------+");
        assert_eq!(ascii[2], "+======+=====+=======+");
        assert_eq!(ascii[3], "| a    |  b  |     c |");
    }

    #[test]
    fn narrow_tables_wrap_cells_to_fit() {
        let md = "| Name | Description |\n|---|---|\n| Grüße | A rather long description that will need several lines here |\n| B | short |";
        let out = lines(md, 32, true);
        for l in &out {
            assert!(l.width() <= 32, "too wide: {l:?}\n{out:#?}");
        }
        assert!(out.len() > 7, "{out:#?}");
        // Wrapped rows are separated so they stay readable.
        assert_eq!(
            out.iter().filter(|l| l.starts_with('├')).count(),
            2,
            "{out:#?}"
        );
        assert!(out.iter().any(|l| l.contains("│ Grüße │")), "{out:#?}");
    }

    #[test]
    fn fit_columns_shares_space_by_need() {
        assert_eq!(fit_columns(&[5, 10], &[5, 4], 20), vec![5, 10]);
        let w = fit_columns(&[10, 80, 40], &[8, 12, 9], 60);
        assert_eq!(w.iter().sum::<usize>(), 60);
        assert!(w[1] > w[2] && w[0] == 10, "{w:?}");
    }

    #[test]
    fn entity_references_link_to_files() {
        let (dir, catalog) = fixture();
        let doc = Doc {
            catalog: &catalog,
            root: dir.path(),
            dir: dir.path(),
            title: None,
        };
        let md = "See TASK-002, [[TASK-001]], [[TASK-002|the docs]] and TASK-999.";
        let on = RenderOptions {
            hyperlinks: true,
            ..opts(80, true)
        };
        let r = render(md, &on, Some(&doc));
        let text = r.lines.join("\n");
        let url = file_url(&dir.path().join("tasks/todo/TASK-001-x.md"));
        assert!(text.contains(&format!("\x1b]8;;{url}\x1b\\")), "{text:?}");
        assert!(url.starts_with("file:///") && !url.contains(".."), "{url}");
        assert_eq!(r.refs, vec!["TASK-002", "TASK-001"]);
        assert_eq!(
            plain(&text),
            "See TASK-002, TASK-001 Fix login, the docs and TASK-999."
        );
        // The unknown ID is not linked; `TASK-001 Fix login` is one link.
        assert_eq!(text.matches("\x1b]8;;file").count(), 3, "{text:?}");

        let off = render(md, &opts(80, true), Some(&doc));
        assert!(!off.lines.join("\n").contains("\x1b]8"));
        assert_eq!(off.refs, r.refs);
    }

    #[test]
    fn links_resolve_relative_files_and_show_urls_without_osc8() {
        let (dir, catalog) = fixture();
        let doc = Doc {
            catalog: &catalog,
            root: dir.path(),
            dir: &dir.path().join("tasks/todo"),
            title: None,
        };
        let md = "[notes](../../notes.md), [task](TASK-002-x.md), [gone](nope.md), [site](https://example.com)";
        let on = RenderOptions {
            hyperlinks: true,
            ..opts(100, true)
        };
        let r = render(md, &on, Some(&doc));
        let text = r.lines.join("\n");
        assert!(
            text.contains(&file_url(&dir.path().join("notes.md"))),
            "{text:?}"
        );
        assert!(text.contains("\x1b]8;;https://example.com\x1b\\"));
        assert_eq!(r.refs, vec!["TASK-002"]);
        assert_eq!(plain(&text), "notes, task, gone, site");

        let off = render(md, &opts(100, true), Some(&doc));
        assert_eq!(
            plain(&off.lines.join("\n")),
            "notes, task, gone, site (https://example.com)"
        );
    }

    #[test]
    fn control_characters_and_html_are_neutralised() {
        let out = render(
            "evil \x1b]0;title\x07 \x1b[31mred\n\n<div>block <b>html</b></div>\n\nline<br>break %% hidden %%",
            &opts(80, true),
            None,
        );
        let text = out.lines.join("\n");
        assert!(
            !plain(&text).contains('\x1b') && !text.contains('\x07'),
            "{text:?}"
        );
        let text = plain(&text);
        assert!(text.contains("block html"), "{text}");
        assert!(text.contains("line\nbreak"), "{text}");
        assert!(!text.contains("hidden"));
    }

    #[test]
    fn character_references_cannot_smuggle_control_characters() {
        let (dir, catalog) = fixture();
        let doc = Doc {
            catalog: &catalog,
            root: dir.path(),
            dir: dir.path(),
            title: None,
        };
        let md =
            "x &#27;]0;PWNED&#7; &#x1b;[2J y `&#27;` [a](https://x.com/&#x1b;]0;T&#7; \"t&#7;\") \
                  ![alt &#27;](https://x.com/i.png)\n\n```&#27;\ncode\n```\n\n<b>&#27;</b>";
        // Only SGR styling and well-formed OSC 8 links may carry ESC.
        let allowed = Regex::new(r"\x1b\]8;;[^\x1b\x07]*\x1b\\|\x1b\[[0-9;]*m").unwrap();
        for hyperlinks in [false, true] {
            let on = RenderOptions {
                hyperlinks,
                ..opts(80, true)
            };
            let text = render(md, &on, Some(&doc)).lines.join("\n");
            let rest = allowed.replace_all(&text, "").into_owned();
            assert!(
                !rest.chars().any(|c| c.is_control() && c != '\n'),
                "{text:?}"
            );
            assert!(rest.contains("x ]0;PWNED [2J y"), "{rest:?}");
        }
        // The hyperlink helper itself never lets a control character through.
        let link = ui::hyperlink("https://x/\x1b]0;T\x07", "t");
        assert_eq!(link, "\x1b]8;;https://x/%1B]0;T%07\x1b\\t\x1b]8;;\x1b\\");
    }

    #[test]
    fn file_links_stay_inside_the_repo() {
        let (dir, catalog) = fixture();
        let doc = Doc {
            catalog: &catalog,
            root: &dir.path().join("tasks"),
            dir: &dir.path().join("tasks/todo"),
            title: None,
        };
        // notes.md exists but sits outside this (narrowed) root.
        let md = "[a](../../notes.md) [b](file:///etc/passwd) [c](../../../../../../etc/passwd)";
        let on = RenderOptions {
            hyperlinks: true,
            ..opts(100, true)
        };
        let text = render(md, &on, Some(&doc)).lines.join("\n");
        assert!(!text.contains("\x1b]8"), "{text:?}");
        assert_eq!(plain(&text), "a b c");
    }

    #[test]
    fn file_urls_are_percent_encoded() {
        assert_eq!(
            file_url(Path::new("/a b/Grüße.md")),
            "file:///a%20b/Gr%C3%BC%C3%9Fe.md"
        );
    }
}
