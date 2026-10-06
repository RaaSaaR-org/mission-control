//! String helpers: escaping, casing, dates and URLs.

use chrono::{Datelike, Local, NaiveDate};
use std::time::SystemTime;

/// Escape text for use in HTML element content and quoted attribute values.
pub(crate) fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

pub(crate) fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

/// A status value as a sentence-case label: `in-progress` → `In progress`.
pub(crate) fn status_label(status: &str) -> String {
    capitalize(&status.replace('-', " "))
}

/// A CSS-class-safe version of a status value.
pub(crate) fn status_slug(status: &str) -> String {
    status
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

/// Initials for an owner avatar ("Jane Doe" → "JD").
pub(crate) fn initials(name: &str) -> String {
    let parts: Vec<&str> = name
        .split(|c: char| c.is_whitespace() || c == '+' || c == ',')
        .filter(|p| !p.is_empty())
        .collect();
    let first = |s: &str| s.chars().next().map(|c| c.to_uppercase().to_string());
    match parts.as_slice() {
        [] => String::new(),
        [one] => first(one).unwrap_or_default(),
        [a, .., b] => format!(
            "{}{}",
            first(a).unwrap_or_default(),
            first(b).unwrap_or_default()
        ),
    }
}

pub(crate) fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

// ── Dates ───────────────────────────────────────────────────────────────

pub(crate) fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()
}

/// Format a date compactly: "27 Jan" this year, "27 Jan 2025" otherwise.
pub(crate) fn fmt_date(d: NaiveDate, today: NaiveDate) -> String {
    if d.year() == today.year() {
        d.format("%-d %b").to_string()
    } else {
        d.format("%-d %b %Y").to_string()
    }
}

/// Format a date with its weekday: "Thu 15 Oct", or "Thu 15 Oct 2027" in another year.
pub(crate) fn fmt_day(d: NaiveDate, today: NaiveDate) -> String {
    if d.year() == today.year() {
        d.format("%a %-d %b").to_string()
    } else {
        d.format("%a %-d %b %Y").to_string()
    }
}

/// "Today", "Tomorrow" or the weekday form of [`fmt_day`].
pub(crate) fn fmt_day_relative(d: NaiveDate, today: NaiveDate) -> String {
    match (d - today).num_days() {
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        _ => fmt_day(d, today),
    }
}

/// ISO week label, e.g. "W41".
pub(crate) fn iso_week(d: NaiveDate) -> String {
    format!("W{}", d.iso_week().week())
}

/// Describe a due date relative to today, with an urgency class.
pub(crate) fn due_phrase(due: NaiveDate, today: NaiveDate) -> (String, &'static str) {
    let days = (due - today).num_days();
    match days {
        d if d < -1 => (format!("{} days overdue", -d), "overdue"),
        -1 => ("1 day overdue".into(), "overdue"),
        0 => ("Due today".into(), "soon"),
        1 => ("Due tomorrow".into(), "soon"),
        2..=7 => (format!("Due in {days} days"), "soon"),
        _ => (format!("Due {}", fmt_date(due, today)), "later"),
    }
}

/// Compact due label for dense contexts: "5d late", "Today", "in 4d", "16 Oct".
pub(crate) fn due_short(due: NaiveDate, today: NaiveDate) -> (String, &'static str) {
    let days = (due - today).num_days();
    match days {
        d if d < 0 => (format!("{}d late", -d), "overdue"),
        0 => ("Today".into(), "soon"),
        1 => ("Tomorrow".into(), "soon"),
        2..=7 => (format!("in {days}d"), "soon"),
        _ => (fmt_date(due, today), "later"),
    }
}

/// Full tooltip for a due date: "Due Fri 9 Oct 2026, in 4 days".
pub(crate) fn due_title(due: NaiveDate, today: NaiveDate) -> String {
    let full = due.format("%a %-d %b %Y");
    let (phrase, class) = due_phrase(due, today);
    if class == "later" {
        return format!("Due {full}");
    }
    let rel = phrase.strip_prefix("Due ").unwrap_or(&phrase);
    format!("Due {full}, {rel}")
}

/// "3 h ago" style description of a modification time.
pub(crate) fn time_ago(t: SystemTime, now: SystemTime) -> String {
    let secs = now.duration_since(t).map(|d| d.as_secs()).unwrap_or(0);
    match secs {
        0..60 => "Just now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86400 => format!("{} h ago", secs / 3600),
        86400..172800 => "Yesterday".into(),
        172800..604800 => format!("{} days ago", secs / 86400),
        _ => {
            let dt: chrono::DateTime<Local> = t.into();
            fmt_date(dt.date_naive(), Local::now().date_naive())
        }
    }
}

// ── URLs ────────────────────────────────────────────────────────────────

/// Percent-encode a query string from key/value pairs (HTML-escaped for attributes).
/// Pairs with empty values are left out.
pub(crate) fn query_string(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v)))
        .collect::<Vec<_>>()
        .join("&amp;")
}

pub(crate) fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// `path?query`, or just `path` when every value is empty.
pub(crate) fn href_with(path: &str, pairs: &[(&str, &str)]) -> String {
    let qs = query_string(pairs);
    if qs.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{qs}")
    }
}

/// Link to an entity's detail page (ID percent-encoded).
pub(crate) fn entity_href(id: &str) -> String {
    format!("/entity/{}", url_encode(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn escape_html_covers_all_specials() {
        assert_eq!(
            escape_html(r#"<a href="x">'&'</a>"#),
            "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"
        );
    }

    #[test]
    fn status_label_is_sentence_case() {
        assert_eq!(status_label("in-progress"), "In progress");
        assert_eq!(status_label("todo"), "Todo");
    }

    #[test]
    fn due_phrase_describes_relative_dates() {
        let today = d(2026, 3, 10);
        assert_eq!(
            due_phrase(d(2026, 3, 7), today),
            ("3 days overdue".into(), "overdue")
        );
        assert_eq!(
            due_phrase(d(2026, 3, 9), today),
            ("1 day overdue".into(), "overdue")
        );
        assert_eq!(
            due_phrase(d(2026, 3, 10), today),
            ("Due today".into(), "soon")
        );
        assert_eq!(
            due_phrase(d(2026, 3, 11), today),
            ("Due tomorrow".into(), "soon")
        );
        assert_eq!(
            due_phrase(d(2026, 3, 14), today),
            ("Due in 4 days".into(), "soon")
        );
        assert_eq!(
            due_phrase(d(2026, 3, 30), today),
            ("Due 30 Mar".into(), "later")
        );
    }

    #[test]
    fn due_short_is_compact() {
        let today = d(2026, 10, 5);
        let at = |n: i64| due_short(today + chrono::Duration::days(n), today);
        assert_eq!(at(-127), ("127d late".into(), "overdue"));
        assert_eq!(at(-1), ("1d late".into(), "overdue"));
        assert_eq!(at(0), ("Today".into(), "soon"));
        assert_eq!(at(1), ("Tomorrow".into(), "soon"));
        assert_eq!(at(4), ("in 4d".into(), "soon"));
        assert_eq!(at(30), ("4 Nov".into(), "later"));
        assert_eq!(
            due_short(d(2027, 1, 4), today),
            ("4 Jan 2027".into(), "later")
        );
    }

    #[test]
    fn due_title_combines_date_and_phrase() {
        let today = d(2026, 10, 5);
        assert_eq!(
            due_title(d(2026, 10, 9), today),
            "Due Fri 9 Oct 2026, in 4 days"
        );
        assert_eq!(
            due_title(d(2026, 10, 4), today),
            "Due Sun 4 Oct 2026, 1 day overdue"
        );
        assert_eq!(due_title(d(2026, 12, 1), today), "Due Tue 1 Dec 2026");
    }

    #[test]
    fn fmt_date_omits_current_year() {
        let today = d(2026, 3, 10);
        assert_eq!(fmt_date(d(2026, 1, 27), today), "27 Jan");
        assert_eq!(fmt_date(d(2025, 1, 27), today), "27 Jan 2025");
        assert_eq!(fmt_day(d(2026, 10, 15), today), "Thu 15 Oct");
        assert_eq!(fmt_day_relative(d(2026, 3, 11), today), "Tomorrow");
    }

    #[test]
    fn iso_week_labels() {
        assert_eq!(iso_week(d(2026, 10, 5)), "W41");
        assert_eq!(iso_week(d(2027, 1, 1)), "W53");
    }

    #[test]
    fn initials_handles_multiple_owners() {
        assert_eq!(initials("Jane Doe"), "JD");
        assert_eq!(initials("alice"), "A");
        assert_eq!(initials("Jane Doe + Max Mustermann"), "JM");
        assert_eq!(initials(""), "");
    }

    #[test]
    fn query_string_encodes_values() {
        assert_eq!(
            query_string(&[("tag", "a b&c"), ("status", ""), ("sort", "id")]),
            "tag=a%20b%26c&amp;sort=id"
        );
        assert_eq!(href_with("/x", &[("a", "")]), "/x");
        assert_eq!(entity_href("TASK-1\"x"), "/entity/TASK-1%22x");
    }

    #[test]
    fn plural_picks_form() {
        assert_eq!(plural(1, "meeting", "meetings"), "1 meeting");
        assert_eq!(plural(0, "meeting", "meetings"), "0 meetings");
    }
}
