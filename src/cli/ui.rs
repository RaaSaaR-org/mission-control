//! Terminal presentation: capability detection (color, unicode, width, TTY),
//! status/priority styling, adaptive tables and small message helpers.
//!
//! Decoration (status glyphs, rules, hints, truncation) only applies when stdout
//! is an interactive terminal, and every glyph falls back to ASCII when stdout
//! is not a terminal, so piped output stays plain and awk-friendly. `--json`
//! is the stable machine interface.

use colored::*;
use dialoguer::console;
use std::borrow::Cow;
use std::io::IsTerminal;
use std::sync::OnceLock;
use std::time::SystemTime;
use unicode_width::UnicodeWidthChar;

/// `--color` setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

/// Resolved terminal capabilities for this process.
#[derive(Clone, Debug)]
pub struct Ui {
    /// ANSI colors are emitted.
    pub color: bool,
    /// Unicode glyphs and box-drawing characters may be used (stdout is a
    /// terminal with a UTF-8 locale and `MC_ASCII`/`TERM=dumb` are unset).
    pub unicode: bool,
    /// stdout is a terminal (enables glyphs in tables, hints, truncation).
    pub interactive: bool,
    /// Width to lay out for; `None` means unbounded (no truncation).
    pub width: Option<usize>,
    /// Emit machine-readable JSON instead of human output.
    pub json: bool,
}

static UI: OnceLock<Ui> = OnceLock::new();

/// Detect capabilities and apply the color override. Call once at startup.
pub fn init(color: ColorChoice, json: bool) -> &'static Ui {
    let ui = detect(color, json);
    colored::control::set_override(ui.color);
    let _ = UI.set(ui);
    get()
}

/// Current UI settings (auto-detected with defaults if `init` was never called).
pub fn get() -> &'static Ui {
    UI.get_or_init(|| detect(ColorChoice::Auto, false))
}

fn env_set(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| !v.is_empty() && v != "0")
}

fn detect(choice: ColorChoice, json: bool) -> Ui {
    let interactive = std::io::stdout().is_terminal();
    let color = !json
        && match choice {
            ColorChoice::Always => true,
            ColorChoice::Never => false,
            ColorChoice::Auto => auto_color(
                colored::control::SHOULD_COLORIZE.should_colorize(),
                env_set("CLICOLOR_FORCE"),
                term_dumb(),
            ),
        };
    Ui {
        color,
        unicode: interactive && detect_unicode(),
        interactive: interactive && !json,
        width: detect_width(interactive),
        json,
    }
}

/// `--color auto`: what `colored` decided (NO_COLOR, CLICOLOR, a TTY), but
/// off for `TERM=dumb`, which `colored` ignores, unless CLICOLOR_FORCE is set.
fn auto_color(should_colorize: bool, clicolor_force: bool, term_dumb: bool) -> bool {
    should_colorize && (clicolor_force || !term_dumb)
}

fn term_dumb() -> bool {
    std::env::var("TERM").is_ok_and(|t| t == "dumb")
}

fn detect_unicode() -> bool {
    if env_set("MC_ASCII") || term_dumb() {
        return false;
    }
    for var in ["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                let v = v.to_ascii_lowercase();
                return v.contains("utf-8") || v.contains("utf8");
            }
        }
    }
    true
}

fn detect_width(interactive: bool) -> Option<usize> {
    if let Some(w) = std::env::var("MC_WIDTH")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
    {
        return (w > 0).then_some(w.max(20));
    }
    if !interactive {
        return None;
    }
    let from_term = console::Term::stdout()
        .size_checked()
        .map(|(_, cols)| cols as usize)
        .filter(|c| *c > 0);
    let from_env = || {
        std::env::var("COLUMNS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
    };
    Some(from_term.or_else(from_env).unwrap_or(100).max(20))
}

/// Whether decorative glyphs should be drawn (unicode terminal, interactive).
pub fn fancy() -> bool {
    let ui = get();
    ui.unicode && ui.interactive
}

/// Whether to emit OSC 8 hyperlinks: on for an interactive, coloured terminal
/// unless `TERM=dumb`; `MC_HYPERLINKS=0`/`1` forces them off or on.
pub fn hyperlinks() -> bool {
    match std::env::var("MC_HYPERLINKS")
        .ok()
        .as_deref()
        .map(str::trim)
    {
        Some("0" | "false" | "no" | "off") => return false,
        Some("1" | "true" | "yes" | "on") => return true,
        _ => {}
    }
    if term_dumb() {
        return false;
    }
    let ui = get();
    ui.interactive && ui.color
}

/// `text` as an OSC 8 hyperlink to `url` (terminals without support show the
/// text). Control characters in `url` are percent-encoded so it can't end the
/// sequence early; `text` is passed through, as it carries the caller's styling.
pub fn hyperlink(url: &str, text: &str) -> String {
    let url: String = url
        .chars()
        .map(|c| {
            if c.is_control() {
                let mut buf = [0; 4];
                c.encode_utf8(&mut buf)
                    .bytes()
                    .map(|b| format!("%{b:02X}"))
                    .collect()
            } else {
                c.to_string()
            }
        })
        .collect();
    format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\")
}

/// Terminal height in rows, when stdout is an interactive terminal.
pub fn height() -> Option<usize> {
    if !get().interactive {
        return None;
    }
    console::Term::stdout()
        .size_checked()
        .map(|(rows, _)| rows as usize)
        .filter(|r| *r > 0)
}

// ---------------------------------------------------------------------------
// Glyphs
// ---------------------------------------------------------------------------

/// Symbol set with an ASCII fallback.
pub struct Glyphs {
    pub ok: &'static str,
    pub err: &'static str,
    pub warn: &'static str,
    pub info: &'static str,
    pub arrow: &'static str,
    pub bullet: &'static str,
    pub sep: &'static str,
    pub brand: &'static str,
    pub ellipsis: &'static str,
    pub rule: &'static str,
    pub heavy: &'static str,
    pub bar_fill: &'static str,
    pub bar_track: &'static str,
    pub quote: &'static str,
}

const UNICODE: Glyphs = Glyphs {
    ok: "✓",
    err: "✗",
    warn: "▲",
    info: "›",
    arrow: "→",
    bullet: "•",
    sep: "·",
    brand: "◆",
    ellipsis: "…",
    rule: "─",
    heavy: "━",
    bar_fill: "━",
    bar_track: "─",
    quote: "│",
};

const ASCII: Glyphs = Glyphs {
    ok: "ok",
    err: "x",
    warn: "!",
    info: ">",
    arrow: "->",
    bullet: "-",
    sep: "|",
    brand: "*",
    ellipsis: "...",
    rule: "-",
    heavy: "=",
    bar_fill: "#",
    bar_track: ".",
    quote: "|",
};

pub fn glyphs() -> &'static Glyphs {
    if get().unicode {
        &UNICODE
    } else {
        &ASCII
    }
}

// ---------------------------------------------------------------------------
// Status & priority styling
// ---------------------------------------------------------------------------

/// Semantic color class of a status value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Good,
    Bad,
    Warn,
    Info,
    Muted,
    Neutral,
}

pub fn status_tone(status: &str) -> Tone {
    match status {
        "active" | "completed" | "final" | "done" | "accepted" => Tone::Good,
        "inactive" | "cancelled" | "churned" | "outdated" | "rejected" | "withdrawn" => Tone::Bad,
        "on-hold" | "draft" | "in-progress" | "review" | "planning" | "proposed" => Tone::Warn,
        "prospect" | "scheduled" | "todo" => Tone::Info,
        "superseded" | "backlog" => Tone::Muted,
        _ => Tone::Neutral,
    }
}

pub fn tint(s: &str, tone: Tone) -> ColoredString {
    match tone {
        Tone::Good => s.green(),
        Tone::Bad => s.red(),
        Tone::Warn => s.yellow(),
        Tone::Info => s.blue(),
        Tone::Muted => s.dimmed(),
        Tone::Neutral => s.normal(),
    }
}

/// Single-width glyph that hints at where a status sits in its lifecycle.
pub fn status_glyph(status: &str) -> &'static str {
    match status {
        "done" | "completed" | "final" | "accepted" => "✓",
        "active" => "●",
        "in-progress" | "review" | "on-hold" | "planning" | "proposed" => "◐",
        "todo" | "scheduled" | "prospect" => "○",
        "backlog" | "draft" => "◌",
        "cancelled" | "rejected" | "withdrawn" | "churned" => "✗",
        "inactive" | "outdated" | "superseded" => "◌",
        _ => "·",
    }
}

/// Colored status, prefixed with a glyph when decoration is enabled.
pub fn status(status: &str) -> String {
    if status.is_empty() {
        return String::new();
    }
    let status = &*clean(status);
    let tone = status_tone(status);
    if fancy() {
        format!(
            "{} {}",
            tint(status_glyph(status), tone),
            tint(status, tone)
        )
    } else {
        tint(status, tone).to_string()
    }
}

pub fn priority_label(p: u32) -> &'static str {
    match p {
        1 => "critical",
        2 => "high",
        3 => "medium",
        4 => "low",
        _ => "unknown",
    }
}

/// Compact `P1`..`P4` badge colored by urgency.
pub fn priority(p: u32) -> String {
    let s = format!("P{p}");
    match p {
        1 => s.red().bold().to_string(),
        2 => s.yellow().bold().to_string(),
        4 => s.dimmed().to_string(),
        _ => s,
    }
}

// ---------------------------------------------------------------------------
// Untrusted text
// ---------------------------------------------------------------------------

/// A file value (title, owner, status, ...) made safe to print on one line:
/// control characters are dropped and line breaks and tabs become spaces, so
/// a file can't send escape sequences to the terminal. Apply it before styling.
pub fn clean(s: &str) -> Cow<'_, str> {
    if !s.chars().any(char::is_control) {
        return Cow::Borrowed(s);
    }
    Cow::Owned(
        s.chars()
            .filter_map(|c| match c {
                '\n' | '\r' | '\t' => Some(' '),
                c if c.is_control() => None,
                c => Some(c),
            })
            .collect(),
    )
}

/// Safety net for text that mixes the CLI's own styling with file values:
/// keeps SGR colour codes (`ESC [ ... m`), OSC 8 hyperlinks and newlines,
/// and drops every other control character, so a value that slipped through
/// unclean can't set the window title, clear the screen or write the
/// clipboard. Tables, truncation and the message helpers apply it.
pub fn scrub(s: &str) -> Cow<'_, str> {
    if !s.chars().any(|c| c.is_control() && c != '\n') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        if c == '\x1b' {
            if let Some(n) = allowed_escape(rest) {
                out.push_str(&rest[..n]);
                rest = &rest[n..];
                continue;
            }
        }
        if !c.is_control() || c == '\n' {
            out.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    Cow::Owned(out)
}

/// Length of the SGR sequence or OSC 8 hyperlink at the start of `s`, if any.
fn allowed_escape(s: &str) -> Option<usize> {
    if let Some(params) = s.strip_prefix("\x1b[") {
        let end = params.find(|c: char| !(c.is_ascii_digit() || c == ';' || c == ':'))?;
        return params[end..].starts_with('m').then_some(2 + end + 1);
    }
    // `ESC ] 8 ; params ; uri` ended by ST or BEL, with a printable payload.
    let body = s.strip_prefix("\x1b]8;")?;
    let end = body.find(char::is_control)?;
    let term = if body[end..].starts_with("\x1b\\") {
        2
    } else if body[end..].starts_with('\x07') {
        1
    } else {
        return None;
    };
    Some(4 + end + term)
}

/// [`scrub`] for a table cell, which must also stay on one line.
fn scrub_cell(cell: String) -> String {
    let cell = match scrub(&cell) {
        Cow::Borrowed(_) => cell,
        Cow::Owned(s) => s,
    };
    if cell.contains('\n') {
        cell.replace('\n', " ")
    } else {
        cell
    }
}

// ---------------------------------------------------------------------------
// Text measuring
// ---------------------------------------------------------------------------

/// Display width, ignoring ANSI escapes (including OSC 8 hyperlinks) and
/// accounting for wide characters.
pub fn width_of(s: &str) -> usize {
    if s.contains("\x1b]") {
        console::measure_text_width(&strip_osc(s))
    } else {
        console::measure_text_width(s)
    }
}

/// Remove OSC sequences (`ESC ] ... ST` or `... BEL`), which `console` doesn't.
fn strip_osc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("\x1b]") {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let end = [
            tail.find("\x1b\\").map(|i| i + 2),
            tail.find('\x07').map(|i| i + 1),
        ]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(tail.len());
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Truncate to `width` display columns with an ellipsis (ANSI-safe).
pub fn truncate(s: &str, width: usize) -> String {
    let s = &*scrub(s);
    if width_of(s) <= width {
        return s.to_string();
    }
    let tail = glyphs().ellipsis;
    if width <= width_of(tail) {
        return console::truncate_str(s, width, "").into_owned();
    }
    console::truncate_str(s, width, tail).into_owned()
}

/// Pad (ANSI-safe) to `width` display columns.
pub fn pad(s: &str, width: usize) -> String {
    let w = width_of(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - w))
    }
}

pub fn pad_left(s: &str, width: usize) -> String {
    let w = width_of(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{}{s}", " ".repeat(width - w))
    }
}

/// Keep the end of `s` (plain text) within `width` columns, with a leading
/// ellipsis: `…/tasks/todo/TASK-001-x.md`. For paths, whose tail matters most.
pub fn truncate_start(s: &str, width: usize) -> String {
    if width_of(s) <= width {
        return s.to_string();
    }
    let tail = glyphs().ellipsis;
    let budget = width.saturating_sub(width_of(tail));
    let mut used = 0;
    let mut start = s.len();
    for (i, c) in s.char_indices().rev() {
        let w = c.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        used += w;
        start = i;
    }
    format!("{tail}{}", &s[start..])
}

/// Word-wrap `text` (may contain ANSI) to `width` columns. Continuation lines
/// are prefixed with `hang` spaces. Plain words longer than a line are split;
/// styled ones are kept whole.
pub fn wrap(text: &str, width: usize, hang: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    let mut cur_w = 0;
    let indent = " ".repeat(hang);
    let limit = |lines: &Vec<String>| {
        if lines.is_empty() {
            width
        } else {
            width.saturating_sub(hang).max(1)
        }
    };
    for word in text.split(' ') {
        let w = width_of(word);
        if cur_w > 0 && cur_w + 1 + w > limit(&lines) {
            lines.push(std::mem::take(&mut cur));
            cur_w = 0;
        }
        if cur_w > 0 {
            cur.push(' ');
            cur_w += 1;
        }
        if w > limit(&lines) && !word.contains('\x1b') {
            for c in word.chars() {
                let cw = c.width().unwrap_or(0);
                if cur_w > 0 && cur_w + cw > limit(&lines) {
                    lines.push(std::mem::take(&mut cur));
                    cur_w = 0;
                }
                cur.push(c);
                cur_w += cw;
            }
            continue;
        }
        cur.push_str(word);
        cur_w += w;
    }
    lines.push(cur);
    for l in lines.iter_mut().skip(1) {
        *l = format!("{indent}{l}");
    }
    lines
}

/// Width available for content after a left indent, capped for readability.
pub fn content_width(indent: usize, cap: usize) -> usize {
    get()
        .width
        .map(|w| w.saturating_sub(indent).clamp(20, cap))
        .unwrap_or(cap)
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// An error message in the CLI's `error: lower-case, no period` style.
/// Messages shared with the API and dashboard are written as sentences
/// ("There is no checklist item 5."); acronyms and IDs keep their case.
pub fn error_message(msg: &str) -> String {
    let first_word = msg.split(' ').next().unwrap_or_default();
    let mut chars = first_word.chars();
    let sentence = chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && chars.all(|c| c.is_lowercase() || c == '\'' || c == '’');
    let msg = &*scrub(msg);
    let mut out = if sentence {
        let mut c = msg.chars();
        c.next()
            .map(|f| f.to_lowercase().collect::<String>() + c.as_str())
            .unwrap_or_default()
    } else {
        msg.to_string()
    };
    if out.ends_with('.') && !out.ends_with("..") && !out.contains(". ") {
        out.pop();
    }
    out
}

pub fn success(msg: impl std::fmt::Display) {
    println!("{} {}", glyphs().ok.green().bold(), scrub(&msg.to_string()));
}

pub fn info(msg: impl std::fmt::Display) {
    println!(
        "{} {}",
        glyphs().info.blue().bold(),
        scrub(&msg.to_string())
    );
}

pub fn warn(msg: impl std::fmt::Display) {
    println!(
        "{} {}",
        glyphs().warn.yellow().bold(),
        scrub(&msg.to_string())
    );
}

/// Next-step suggestion. Only shown on interactive terminals.
pub fn hint(msg: impl std::fmt::Display) {
    if get().interactive {
        println!(
            "  {} {}",
            glyphs().arrow.dimmed(),
            scrub(&msg.to_string()).dimmed()
        );
    }
}

/// Render a command reference like `mc task next` for use inside hints.
pub fn cmd(s: &str) -> String {
    s.cyan().to_string()
}

/// Horizontal rule of `width` columns.
pub fn rule(width: usize) -> String {
    glyphs().rule.repeat(width).dimmed().to_string()
}

/// Section title in the house style: uppercase, bold, letter-spaced feel.
pub fn section(title: &str) -> String {
    title.to_uppercase().bold().to_string()
}

/// Proportional bar of `width` cells, `fraction` in 0..=1.
pub fn bar(fraction: f64, width: usize, tone: Tone) -> String {
    let fraction = fraction.clamp(0.0, 1.0);
    let mut filled = ((fraction * width as f64).round() as usize).min(width);
    if fraction > 0.0 && filled == 0 {
        filled = width.min(1);
    }
    let g = glyphs();
    format!(
        "{}{}",
        tint(&g.bar_fill.repeat(filled), tone),
        g.bar_track.repeat(width - filled).dimmed()
    )
}

/// Human-friendly age like `5m ago`, `3d ago`, falling back to a date.
pub fn relative_time(t: SystemTime) -> String {
    let secs = SystemTime::now()
        .duration_since(t)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        86_400..=1_209_599 => format!("{}d ago", secs / 86_400),
        1_209_600..=5_183_999 => format!("{}w ago", secs / 604_800),
        _ => chrono::DateTime::<chrono::Local>::from(t)
            .format("%Y-%m-%d")
            .to_string(),
    }
}

/// Pluralize a count: `1 task`, `3 tasks`.
pub fn count(n: usize, singular: &str, plural: &str) -> String {
    if n == 1 {
        format!("{n} {singular}")
    } else {
        format!("{n} {plural}")
    }
}

// ---------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------

/// Column definition for [`Table`].
#[derive(Clone, Debug)]
pub struct Col {
    header: &'static str,
    min: usize,
    max: usize,
    flex: bool,
    drop: u8,
    right: bool,
}

impl Col {
    pub fn new(header: &'static str) -> Self {
        Col {
            header,
            min: header.len().max(4),
            max: 0,
            flex: false,
            drop: 0,
            right: false,
        }
    }
    /// Absorbs leftover width and shrinks first when space is tight.
    pub fn flex(mut self, min: usize) -> Self {
        self.flex = true;
        self.min = min;
        self
    }
    /// Upper bound on width when the terminal width is known.
    pub fn max(mut self, max: usize) -> Self {
        self.max = max;
        self
    }
    /// Hide this column on narrow terminals; higher values are hidden first.
    pub fn drop(mut self, priority: u8) -> Self {
        self.drop = priority;
        self
    }
    pub fn right(mut self) -> Self {
        self.right = true;
        self
    }
    /// Never shrink below the widest cell (identifiers must stay readable).
    pub fn fixed(mut self) -> Self {
        self.min = usize::MAX;
        self
    }
}

const INDENT: usize = 2;
const GAP: usize = 2;
/// Narrowest a flexible column gets when nothing else can give way.
const FLEX_FLOOR: usize = 4;

/// Simple adaptive table: hides empty columns, drops low-priority columns
/// and truncates flexible ones to fit the terminal.
pub struct Table {
    cols: Vec<Col>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(cols: Vec<Col>) -> Self {
        Table {
            cols,
            rows: Vec::new(),
        }
    }

    /// Add a row. Cells are scrubbed of control characters other than the
    /// caller's own styling (see [`scrub`]).
    pub fn row(&mut self, cells: Vec<String>) {
        debug_assert_eq!(cells.len(), self.cols.len());
        self.rows.push(cells.into_iter().map(scrub_cell).collect());
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Compute visible column indices and their widths for `width`.
    fn layout(&self, width: Option<usize>) -> Vec<(usize, usize)> {
        let natural: Vec<usize> = self
            .cols
            .iter()
            .enumerate()
            .map(|(i, c)| {
                self.rows
                    .iter()
                    .map(|r| width_of(&r[i]))
                    .max()
                    .unwrap_or(0)
                    .max(c.header.len())
            })
            .collect();

        // Hide columns where every cell is empty.
        let mut visible: Vec<usize> = (0..self.cols.len())
            .filter(|&i| self.rows.iter().any(|r| width_of(&r[i]) > 0))
            .collect();

        let Some(total) = width else {
            return visible.into_iter().map(|i| (i, natural[i])).collect();
        };
        let avail = total.saturating_sub(INDENT);

        let mut widths = natural.clone();

        let need = |vis: &[usize], widths: &[usize]| -> usize {
            vis.iter().map(|&i| widths[i]).sum::<usize>() + GAP * vis.len().saturating_sub(1)
        };
        // Caps only kick in when the natural layout does not fit.
        if need(&visible, &widths) > avail {
            for (w, c) in widths.iter_mut().zip(&self.cols) {
                if c.max > 0 {
                    *w = (*w).min(c.max);
                }
            }
        }

        // A flexible column deserves a fair share of the line (~40%) before
        // optional columns are kept around.
        let fair = (avail * 2 / 5).max(1);
        let want = |vis: &[usize], widths: &[usize]| -> usize {
            vis.iter()
                .map(|&i| {
                    if self.cols[i].flex {
                        widths[i].min(fair.max(self.cols[i].min))
                    } else {
                        widths[i]
                    }
                })
                .sum::<usize>()
                + GAP * vis.len().saturating_sub(1)
        };

        while want(&visible, &widths) > avail {
            let victim = visible
                .iter()
                .enumerate()
                .filter(|(_, &i)| self.cols[i].drop > 0)
                .max_by_key(|(pos, &i)| (self.cols[i].drop, *pos))
                .map(|(pos, _)| pos);
            match victim {
                Some(pos) => {
                    visible.remove(pos);
                }
                None => break,
            }
        }

        // Shrink flexible columns, then the widest remaining ones.
        let mut over = need(&visible, &widths).saturating_sub(avail);
        for &i in &visible {
            if over == 0 {
                break;
            }
            if self.cols[i].flex {
                let room = widths[i].saturating_sub(self.cols[i].min);
                let take = room.min(over);
                widths[i] -= take;
                over -= take;
            }
        }
        while over > 0 {
            let Some(&i) = visible
                .iter()
                .filter(|&&i| widths[i] > self.cols[i].min.min(natural[i]).max(4))
                .max_by_key(|&&i| widths[i])
            else {
                break;
            };
            widths[i] -= 1;
            over -= 1;
        }
        // Very narrow terminals: flexible columns give up their minimum too,
        // rather than every line wrapping.
        for &i in &visible {
            if over == 0 {
                break;
            }
            if self.cols[i].flex {
                let take = widths[i].saturating_sub(FLEX_FLOOR).min(over);
                widths[i] -= take;
                over -= take;
            }
        }

        visible.into_iter().map(|i| (i, widths[i])).collect()
    }

    /// Render into lines (without trailing newlines).
    pub fn render(&self, width: Option<usize>) -> Vec<String> {
        let layout = self.layout(width);
        let truncating = width.is_some();
        let indent = " ".repeat(INDENT);
        let mut out = Vec::with_capacity(self.rows.len() + 2);

        let fmt_row = |cells: Vec<String>| -> String {
            let last = layout.len().saturating_sub(1);
            let mut line = indent.clone();
            for (pos, ((i, w), cell)) in layout.iter().zip(cells).enumerate() {
                let cell = if truncating {
                    truncate(&cell, *w)
                } else {
                    cell
                };
                let cell = if self.cols[*i].right {
                    pad_left(&cell, *w)
                } else if pos == last {
                    cell
                } else {
                    pad(&cell, *w)
                };
                line.push_str(&cell);
                if pos != last {
                    line.push_str(&" ".repeat(GAP));
                }
            }
            line.trim_end().to_string()
        };

        let header: Vec<String> = layout
            .iter()
            .map(|(i, _)| self.cols[*i].header.to_uppercase().dimmed().to_string())
            .collect();
        out.push(fmt_row(header));
        if get().interactive {
            let total: usize = layout.iter().map(|(_, w)| *w).sum::<usize>()
                + GAP * layout.len().saturating_sub(1);
            out.push(format!("{indent}{}", rule(total)));
        }
        let empty = "-".dimmed().to_string();
        for row in &self.rows {
            let cells = layout
                .iter()
                .map(|(i, _)| {
                    if width_of(&row[*i]) == 0 {
                        empty.clone()
                    } else {
                        row[*i].clone()
                    }
                })
                .collect();
            out.push(fmt_row(cells));
        }
        out
    }

    pub fn print(&self) {
        for line in self.render(get().width) {
            println!("{line}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Table {
        let mut t = Table::new(vec![
            Col::new("ID"),
            Col::new("Title").flex(16),
            Col::new("Owner").drop(2),
            Col::new("Sprint").drop(3),
            Col::new("Empty").drop(1),
        ]);
        t.row(vec![
            "TASK-001".into(),
            "A fairly long task title that will not fit".into(),
            "alice".into(),
            "2026-W05".into(),
            String::new(),
        ]);
        t.row(vec![
            "TASK-002".into(),
            "Short".into(),
            String::new(),
            String::new(),
            String::new(),
        ]);
        t
    }

    #[test]
    fn unbounded_width_keeps_everything_but_empty_columns() {
        let lines = sample().render(None);
        assert!(lines[0].contains("SPRINT"));
        assert!(!lines[0].contains("EMPTY"));
        assert!(lines
            .iter()
            .any(|l| l.contains("A fairly long task title that will not fit")));
        // Empty cells get a placeholder so columns stay aligned for awk.
        assert!(lines.last().unwrap().contains('-'));
    }

    #[test]
    fn narrow_width_drops_optional_columns_then_truncates() {
        let lines = sample().render(Some(40));
        for l in &lines {
            assert!(width_of(l) <= 40, "line too wide: {l:?}");
        }
        assert!(
            !lines[0].contains("SPRINT"),
            "sprint should be dropped first"
        );
        assert!(lines[0].contains("TITLE"));
    }

    #[test]
    fn very_narrow_width_shrinks_flex_columns_below_their_minimum() {
        // The `mc list tasks` layout: fixed ID, priority, status, flexible title.
        let mut t = Table::new(vec![
            Col::new("ID").fixed(),
            Col::new("Pri"),
            Col::new("Status"),
            Col::new("Title").flex(18),
            Col::new("Owner").max(16).drop(3),
        ]);
        t.row(vec![
            "TASK-023".into(),
            "P1".into(),
            "todo".into(),
            "AP2: Hardware-Integration und Tests".into(),
            "alice".into(),
        ]);
        for width in [40, 30] {
            let lines = t.render(Some(width));
            for l in &lines {
                assert!(width_of(l) <= width, "{width}: line too wide: {l:?}");
            }
            assert!(lines.last().unwrap().contains("TASK-023"), "{lines:?}");
        }
    }

    #[test]
    fn table_cells_and_truncation_drop_foreign_escapes() {
        let evil = "Evil \x1b]0;PWNED\x07 \x1b[2Jx\x1b]52;c;aGk=\x07 \u{9b}31m";
        let mut t = Table::new(vec![Col::new("ID"), Col::new("Title").flex(8)]);
        t.row(vec!["\x1b[36mTASK-1\x1b[0m".into(), evil.into()]);
        let lines = t.render(None);
        let row = lines.last().unwrap();
        // The caller's own colour survives; the title's escapes don't.
        assert!(row.contains("\x1b[36mTASK-1\x1b[0m"), "{row:?}");
        assert_eq!(
            console::strip_ansi_codes(row).matches('\x1b').count(),
            0,
            "{row:?}"
        );
        assert!(!row.contains('\x07') && !row.contains('\u{9b}'), "{row:?}");
        assert!(row.contains("Evil ]0;PWNED [2Jx]52;c;aGk= 31m"), "{row:?}");
        let cut = truncate(evil, 12);
        assert!(!cut.contains('\x1b') && !cut.contains('\x07'), "{cut:?}");
    }

    #[test]
    fn scrub_keeps_styling_and_hyperlinks_only() {
        let link = hyperlink("file:///tmp/a.md", "a");
        assert_eq!(scrub(&link), link);
        assert_eq!(scrub("\x1b[1;31mred\x1b[0m"), "\x1b[1;31mred\x1b[0m");
        assert_eq!(scrub("a\nb"), "a\nb");
        assert_eq!(scrub("t\x1b]0;title\x07x"), "t]0;titlex");
        assert_eq!(scrub("\x1b]8;;x\x1b[2J"), "]8;;x[2J");
        assert_eq!(clean("Own\x1b[2Jer\nnext\ttab"), "Own[2Jer next tab");
        assert!(matches!(clean("plain"), Cow::Borrowed(_)));
        assert_eq!(status("\x1b]0;x\x07done"), status("]0;xdone"));
        assert!(!error_message("bad '\x1b]0;x\x07'").contains('\x1b'));
    }

    #[test]
    fn dumb_terminals_get_no_color_unless_forced() {
        assert!(auto_color(true, false, false));
        assert!(!auto_color(true, false, true));
        assert!(auto_color(true, true, true));
        assert!(!auto_color(false, false, false));
    }

    #[test]
    fn fixed_columns_are_never_truncated() {
        let mut t = Table::new(vec![
            Col::new("ID").fixed(),
            Col::new("Status"),
            Col::new("Title").flex(16),
        ]);
        t.row(vec![
            "TASK-001".into(),
            "in-progress".into(),
            "Implement auth middleware".into(),
        ]);
        let lines = t.render(Some(30));
        assert!(lines.last().unwrap().contains("TASK-001"), "{lines:?}");
    }

    #[test]
    fn bar_never_overflows() {
        assert_eq!(width_of(&bar(1.0, 0, Tone::Good)), 0);
        assert_eq!(width_of(&bar(0.01, 5, Tone::Good)), 5);
        assert_eq!(width_of(&bar(2.0, 5, Tone::Good)), 5);
    }

    #[test]
    fn truncate_is_width_aware() {
        assert_eq!(width_of(&truncate("日本語テスト", 5)), 5);
        assert_eq!(truncate("hello", 10), "hello");
        let t = truncate("hello world", 8);
        assert!(width_of(&t) <= 8);
    }

    #[test]
    fn wrap_respects_width_and_hangs() {
        let lines = wrap("- one two three four five six", 12, 2);
        assert!(lines.len() > 1);
        for l in &lines {
            assert!(width_of(l) <= 12, "{l:?}");
        }
        assert!(lines[1].starts_with("  "));
    }

    #[test]
    fn wrap_splits_long_plain_words_and_paths_truncate_from_the_start() {
        let lines = wrap("tags: hightech-agenda-deutschland, x", 12, 0);
        assert!(lines.iter().all(|l| width_of(l) <= 12), "{lines:?}");
        assert_eq!(
            lines.concat().replace(' ', ""),
            "tags:hightech-agenda-deutschland,x"
        );
        let t = truncate_start("tasks/todo/TASK-001-a-long-name.md", 20);
        assert_eq!(width_of(&t), 20);
        assert!(t.ends_with("-a-long-name.md"), "{t}");
        assert_eq!(truncate_start("short.md", 20), "short.md");
    }

    #[test]
    fn error_messages_follow_the_cli_style() {
        assert_eq!(
            error_message("There is no checklist item 9."),
            "there is no checklist item 9"
        );
        assert_eq!(
            error_message("A comment needs some text."),
            "a comment needs some text"
        );
        assert_eq!(
            error_message("task TASK-9 not found"),
            "task TASK-9 not found"
        );
        assert_eq!(
            error_message("TASK-001 is a sprint."),
            "TASK-001 is a sprint"
        );
        assert_eq!(error_message("MCP failed"), "MCP failed");
        assert_eq!(
            error_message("Invalid status 'x'. Valid: a, b."),
            "invalid status 'x'. Valid: a, b."
        );
    }

    #[test]
    fn width_ignores_hyperlinks() {
        let link = hyperlink("file:///tmp/a.md", "\x1b[36mTASK-001\x1b[0m");
        assert_eq!(width_of(&link), 8);
        assert_eq!(width_of(&format!("a {link} b")), 12);
        assert_eq!(strip_osc("x\x1b]8;;u\x07y\x1b]8;;\x07"), "xy");
    }

    #[test]
    fn pad_handles_ansi() {
        let colored = "\x1b[31mred\x1b[0m";
        assert_eq!(width_of(&pad(colored, 6)), 6);
        assert_eq!(width_of(&pad_left(colored, 6)), 6);
    }

    #[test]
    fn status_tones() {
        assert_eq!(status_tone("done"), Tone::Good);
        assert_eq!(status_tone("cancelled"), Tone::Bad);
        assert_eq!(status_tone("in-progress"), Tone::Warn);
        assert_eq!(status_tone("todo"), Tone::Info);
        assert_eq!(status_tone("backlog"), Tone::Muted);
        assert_eq!(status_tone("whatever"), Tone::Neutral);
    }

    #[test]
    fn count_pluralizes() {
        assert_eq!(count(1, "task", "tasks"), "1 task");
        assert_eq!(count(3, "task", "tasks"), "3 tasks");
        assert_eq!(count(0, "sprint", "sprints"), "0 sprints");
    }
}
