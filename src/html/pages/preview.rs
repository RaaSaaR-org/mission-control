//! Preview cards: the small summary shown when hovering or focusing a link
//! to an entity. Served as an HTML fragment by `GET /api/preview/{id}`.

use crate::checklist;
use crate::comments::{self, Comment};
use crate::data::{self, EntityRecord};
use crate::entity::EntityKind;
use crate::frontmatter;
use crate::html::catalog::{display_name, split_wikilink};
use crate::html::components::{
    icon, id_chip, lamp, owner_html, priority_html, progress_bar, status_badge,
};
use crate::html::format::{
    capitalize, due_short, due_title, entity_href, escape_html, fmt_day, fmt_day_relative,
    parse_date, plural,
};
use crate::html::markdown::{strip_leading_h1, OBSIDIAN_COMMENT_RE, WIKILINK_RE};
use crate::html::pages::tasks::due_state;
use crate::html::{is_closed, Page};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// Longest excerpt, in characters, before it's cut with an ellipsis. The
/// card clamps it to three lines; this only bounds the payload.
const EXCERPT_CHARS: usize = 280;

/// Longest ID echoed back in the not-found card.
const MAX_ECHOED_ID: usize = 40;

/// The preview card for `id`, or `None` when no entity has that ID.
/// Slugged references (`CONT-003-jane-doe`) resolve to their entity.
pub fn preview_card(page: &Page, id: &str) -> Option<String> {
    let catalog = page.catalog;
    let id = catalog.canonical_id(id).unwrap_or(id);
    let rec = catalog.records.iter().find(|r| r.id == id)?;
    Some(card(rec, page))
}

/// The card shown for a reference that doesn't resolve.
pub fn preview_not_found(id: &str) -> String {
    let shown: String = id.chars().take(MAX_ECHOED_ID).collect();
    format!(
        r#"<div class="pv-card is-missing"><div class="pv-head"><span class="pv-kind">{}Not found</span>{}</div><p class="pv-text">Nothing in this repo has this ID. The file may have been renamed, moved or deleted.</p></div>"#,
        lamp("negative", ""),
        id_chip(&shown),
    )
}

fn card(rec: &EntityRecord, page: &Page) -> String {
    let fm = &rec.frontmatter;
    let status = frontmatter::get_str_or(fm, "status", "");
    let href = entity_href(&rec.id);
    let rows: String = fields(rec, page)
        .into_iter()
        .map(|(label, value)| {
            format!(r#"<div class="pv-field"><dt>{label}</dt><dd>{value}</dd></div>"#)
        })
        .collect();
    let rows = if rows.is_empty() {
        String::new()
    } else {
        format!(r#"<dl class="pv-fields">{rows}</dl>"#)
    };
    // Comments are shown on the page, not in the glance: the excerpt and
    // checklist come from the notes alone, like the detail page's.
    let (notes, comment_list) = if comments::is_commentable(rec.kind) {
        comments::split(&rec.body)
    } else {
        (rec.body.clone(), Vec::new())
    };
    let excerpt = frontmatter::get_str(fm, "summary")
        .map(|s| clamp_chars(s.trim(), EXCERPT_CHARS))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| excerpt(&notes, page));
    let excerpt = if excerpt.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="pv-text">{}</p>"#, escape_html(&excerpt))
    };
    let (done, total) = checklist::progress(&checklist::body_items(rec.kind, &rec.body));
    let checks = if total == 0 {
        String::new()
    } else {
        format!(
            r#"<div class="pv-checks">{}<span class="progress-caption">{done} of {total} done</span></div>"#,
            progress_bar(done, total)
        )
    };
    let discussion = comments_line(&comment_list, page);
    let open = format!(
        r#"<a class="icon-btn pv-open" href="{href}" aria-label="Open {id}" title="Open {id}">{}</a>"#,
        icon("chevron"),
        id = escape_html(&rec.id),
    );
    format!(
        r#"<div class="pv-card{state}"><div class="pv-head"><span class="pv-kind">{kind}</span>{id}{badge}{open}</div><p class="pv-title"><a href="{href}">{title}</a></p>{rows}{excerpt}{checks}{discussion}</div>"#,
        state = match due_state(rec, page.today) {
            Some(s) if rec.kind == EntityKind::Task => format!(" is-{s}"),
            _ => String::new(),
        },
        kind = capitalize(rec.kind.label()),
        id = id_chip(&rec.id),
        badge = status_badge(status),
        title = escape_html(display_name(rec)),
    )
}

/// The fields worth a glance for each kind, as (label, trusted HTML) pairs.
fn fields(rec: &EntityRecord, page: &Page) -> Vec<(&'static str, String)> {
    let fm = &rec.frontmatter;
    let catalog = page.catalog;
    let str_of = |key: &str| {
        frontmatter::get_str(fm, key)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let refs = |key: &str| {
        let items = frontmatter::get_string_list(fm, key);
        Some(catalog.refs_html_compact(&items)).filter(|h| !h.is_empty())
    };
    // `customer: CUST-001` and `customers: [...]` both occur in real repos.
    let refs_either = |one: &str, many: &str| refs(many).or_else(|| refs(one));
    let day = |key: &str| {
        str_of(key)
            .and_then(parse_date)
            .map(|d| time(d, &fmt_day(d, page.today)))
    };
    let text = |key: &str| str_of(key).map(escape_html);

    let mut out: Vec<(&'static str, Option<String>)> = Vec::new();
    match rec.kind {
        EntityKind::Task => {
            let status = str_of("status").unwrap_or("");
            out.push((
                "Priority",
                data::get_number(fm, "priority").map(priority_html),
            ));
            out.push(("Owner", str_of("owner").map(owner_html)));
            out.push((
                "Due",
                str_of("due_date").and_then(|raw| due(raw, status, page)),
            ));
            out.push(("Sprint", str_of("sprint").map(|s| catalog.ref_html(s))));
            out.push(("Project", refs_either("project", "projects")));
            out.push(("Customer", refs_either("customer", "customers")));
        }
        EntityKind::Meeting => {
            out.push(("When", meeting_when(rec, page)));
            out.push(("Attendees", attendees(rec, page)));
            out.push(("Where", text("location")));
            out.push(("Customer", refs_either("customer", "customers")));
            out.push(("Project", refs_either("project", "projects")));
        }
        EntityKind::Project => {
            out.push(("Owner", str_of("owner").map(owner_html)));
            out.push(("Customer", refs_either("customer", "customers")));
            out.push(("Start", day("start_date")));
            out.push(("Target", day("target_date").or_else(|| day("end_date"))));
        }
        EntityKind::Sprint => {
            out.push(("Start", day("start_date")));
            out.push(("End", day("end_date")));
            out.push(("Goal", text("goal")));
        }
        EntityKind::Contact => {
            out.push(("Role", text("role")));
            out.push(("Customer", refs_either("customer", "customers")));
            out.push(("Email", text("email")));
        }
        EntityKind::Customer => {
            out.push(("Owner", str_of("owner").map(owner_html)));
            out.push(("Industry", text("industry")));
            out.push(("Location", text("location")));
        }
        EntityKind::Research | EntityKind::Proposal => {
            out.push(("Owner", str_of("owner").map(owner_html)));
            out.push(("Date", day("date")));
            out.push(("Due", str_of("due_date").and_then(|raw| due(raw, "", page))));
            out.push(("Customer", refs_either("customer", "customers")));
            out.push(("Project", refs_either("project", "projects")));
        }
    }
    out.into_iter()
        .filter_map(|(label, value)| value.map(|v| (label, v)))
        .collect()
}

fn time(d: chrono::NaiveDate, text: &str) -> String {
    format!(
        r#"<time datetime="{}">{}</time>"#,
        d.format("%Y-%m-%d"),
        escape_html(text)
    )
}

/// Due date with the late / due-soon readout for open work, a plain date
/// once it's finished.
fn due(raw: &str, status: &str, page: &Page) -> Option<String> {
    let d = parse_date(raw)?;
    let date = time(d, &fmt_day(d, page.today));
    if is_closed(status) {
        return Some(format!(r#"<span class="muted">{date}</span>"#));
    }
    let (text, class) = due_short(d, page.today);
    if class == "later" {
        return Some(date);
    }
    Some(format!(
        r#"<span class="due due-{class}" title="{}">{text}</span><span class="pv-sub">{date}</span>"#,
        escape_html(&due_title(d, page.today))
    ))
}

/// "Today, 14:00 · 1h": today in the route colour, like the flight plan.
fn meeting_when(rec: &EntityRecord, page: &Page) -> Option<String> {
    let fm = &rec.frontmatter;
    let d = parse_date(frontmatter::get_str(fm, "date")?)?;
    let mut text = fmt_day_relative(d, page.today);
    for key in ["time", "duration"] {
        if let Some(v) = frontmatter::get_str(fm, key).filter(|s| !s.trim().is_empty()) {
            text.push_str(if key == "time" { ", " } else { " · " });
            text.push_str(v.trim());
        }
    }
    let html = time(d, &text);
    Some(if d == page.today {
        format!(r#"<span class="pv-today">{html}</span>"#)
    } else {
        html
    })
}

/// Attendee count, then the first few names.
fn attendees(rec: &EntityRecord, page: &Page) -> Option<String> {
    const SHOWN: usize = 3;
    let items = frontmatter::get_string_list(&rec.frontmatter, "attendees");
    let names: Vec<String> = items
        .iter()
        .map(|raw| {
            let (target, alias) = split_wikilink(raw);
            page.catalog
                .canonical_id(target)
                .and_then(|id| page.catalog.name(id))
                .or(alias)
                .unwrap_or(target)
                .to_string()
        })
        .filter(|n| !n.trim().is_empty())
        .collect();
    if names.is_empty() {
        return None;
    }
    let mut shown = names[..names.len().min(SHOWN)].join(", ");
    if names.len() > SHOWN {
        shown.push_str(&format!(" +{}", names.len() - SHOWN));
    }
    Some(format!(
        r#"<span class="pv-count">{}</span> <span class="pv-sub" title="{}">{}</span>"#,
        plural(names.len(), "person", "people"),
        escape_html(&names.join(", ")),
        escape_html(&shown)
    ))
}

/// The opening prose of a note as plain text: paragraphs and list items,
/// without headings, tables, code or raw HTML. Wikilinks show their alias or
/// the entity's name.
fn excerpt(body: &str, page: &Page) -> String {
    let md = OBSIDIAN_COMMENT_RE.replace_all(strip_leading_h1(body.trim()), "");
    let md = WIKILINK_RE.replace_all(&md, |caps: &regex::Captures| {
        let target = caps[1].trim();
        caps.get(2)
            .map(|a| a.as_str().trim())
            .or_else(|| {
                page.catalog
                    .canonical_id(target)
                    .and_then(|id| page.catalog.name(id))
            })
            .unwrap_or(target)
            .chars()
            // Names are text, not Markdown: `<b>` or `*` must survive parsing.
            .flat_map(|c| {
                (c.is_ascii_punctuation().then_some('\\'))
                    .into_iter()
                    .chain([c])
            })
            .collect::<String>()
    });
    let mut out = String::new();
    // Depth inside blocks whose text is skipped (headings, tables, code).
    let mut skip = 0usize;
    let mut events =
        Parser::new_ext(&md, Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS).peekable();
    while let Some(event) = events.next() {
        // One `**Key:** value` per line reads as separate fields, not a sentence.
        let field_break = match &event {
            Event::HardBreak => true,
            Event::SoftBreak => matches!(events.peek(), Some(Event::Start(Tag::Strong))),
            _ => false,
        };
        match event {
            Event::Start(Tag::Heading { .. } | Tag::Table(_) | Tag::CodeBlock(_)) => skip += 1,
            Event::End(TagEnd::Heading(_) | TagEnd::Table | TagEnd::CodeBlock) => {
                skip = skip.saturating_sub(1)
            }
            Event::Text(t) | Event::Code(t) if skip == 0 => out.push_str(&t),
            Event::SoftBreak | Event::HardBreak if skip == 0 && !field_break => out.push(' '),
            Event::SoftBreak | Event::HardBreak | Event::End(TagEnd::Paragraph | TagEnd::Item)
                if skip == 0 =>
            {
                // Blocks run on as one line, list items joined by a dot.
                let trimmed = out.trim_end().len();
                out.truncate(trimmed);
                if !out.is_empty() && !out.ends_with(['.', ':', '!', '?', '·']) {
                    out.push_str(" ·");
                }
                if !out.is_empty() {
                    out.push(' ');
                }
            }
            _ => {}
        }
        if out.chars().count() > EXCERPT_CHARS {
            break;
        }
    }
    clamp_chars(out.trim().trim_end_matches('·').trim_end(), EXCERPT_CHARS)
}

/// Cut `s` to at most `max` characters, at a word boundary, with an ellipsis.
fn clamp_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    let cut = match cut.rfind(' ') {
        Some(i) if i > max / 2 => &cut[..i],
        _ => &cut,
    };
    format!("{}…", cut.trim_end_matches([' ', ',', ';', '·']))
}

/// "3 comments · latest Mon 5 Oct by Jane", or nothing without comments.
fn comments_line(list: &[Comment], page: &Page) -> String {
    let Some(last) = list.last() else {
        return String::new();
    };
    let count = match list.len() {
        1 => "1 comment".to_string(),
        n => format!("{n} comments"),
    };
    let when = last
        .date
        .as_deref()
        .and_then(parse_date)
        .map(|d| format!(" · latest {}", fmt_day_relative(d, page.today)))
        .unwrap_or_default();
    let by = last
        .author
        .as_deref()
        .map(|a| format!(" by {}", escape_html(a)))
        .unwrap_or_default();
    format!(r#"<p class="pv-comments">{count}{when}{by}</p>"#)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::catalog::tests::{catalog, rec, test_config};

    fn with_body(mut r: EntityRecord, body: &str) -> EntityRecord {
        r.body = body.to_string();
        r
    }

    #[test]
    fn task_card_shows_fields_excerpt_and_checklist() {
        let (_d, cfg) = test_config();
        let today = chrono::Local::now().date_naive();
        let late = (today - chrono::Duration::days(3)).format("%Y-%m-%d");
        let task = with_body(
            rec(
                EntityKind::Task,
                "TASK-007",
                &format!("title: Ship it\nstatus: todo\npriority: 1\nowner: Jane Doe\ndue_date: {late}\nsprint: SPR-001\nprojects: ['[[PROJ-001|Robots]]']"),
            ),
            "# Ship it\n\n## Steps\n\nFirst **bold** line with [[PROJ-001]].\n\n| a | b |\n|---|---|\n| x | y |\n\n- [x] one\n- [ ] two\n- [x] three\n",
        );
        let cat = catalog(
            vec![
                task,
                rec(EntityKind::Sprint, "SPR-001", "title: Alpha"),
                rec(EntityKind::Project, "PROJ-001", "name: Robot Arm"),
            ],
            &cfg,
        );
        let page = Page::new(&cfg, &cat, "");
        let html = preview_card(&page, "TASK-007").unwrap();
        assert!(html.contains(r#"<span class="pv-kind">Task</span>"#));
        assert!(html.contains(">Ship it</a>"));
        assert!(html.contains("pri-critical"));
        assert!(html.contains(r#"class="due due-overdue""#));
        assert!(html.contains(">3d late<"));
        assert!(html.contains(">Alpha</a>"));
        assert!(html.contains(">Robot Arm</a>"));
        assert!(html.contains("First bold line with Robot Arm."), "{html}");
        assert!(!html.contains("Steps"), "headings are skipped");
        assert!(!html.contains("| a"), "tables are skipped");
        assert!(html.contains("2 of 3 done"));
        assert!(html.contains(r#"href="/entity/TASK-007""#));
    }

    #[test]
    fn meeting_card_shows_when_and_attendees() {
        let (_d, cfg) = test_config();
        let today = chrono::Local::now().date_naive().format("%Y-%m-%d");
        let cat = catalog(
            vec![
                rec(
                    EntityKind::Meeting,
                    "MTG-001",
                    &format!("title: Jour fixe\ndate: {today}\ntime: '14:00'\nduration: 1h\nattendees: ['[[CONT-001-ann|Ann]]', Bob, Cy, Di]"),
                ),
                rec(EntityKind::Contact, "CONT-001", "name: Ann Example"),
            ],
            &cfg,
        );
        let page = Page::new(&cfg, &cat, "");
        let html = preview_card(&page, "MTG-001").unwrap();
        assert!(html.contains("pv-today"));
        assert!(html.contains("Today, 14:00 · 1h"));
        assert!(html.contains("4 people"));
        assert!(html.contains("Ann Example, Bob, Cy +1"));
    }

    #[test]
    fn hostile_values_are_escaped() {
        let (_d, cfg) = test_config();
        let task = with_body(
            rec(
                EntityKind::Task,
                "TASK-001",
                "title: \"<img src=x onerror=alert(1)>\"\nstatus: \"<b>\"\nowner: \"<script>\"",
            ),
            "<script>alert(1)</script> and <b>raw</b> &amp; text",
        );
        let cat = catalog(vec![task], &cfg);
        let page = Page::new(&cfg, &cat, "");
        let html = preview_card(&page, "TASK-001").unwrap();
        assert!(!html.contains("<img"));
        assert!(!html.contains("<script"));
        assert!(!html.contains("<b>"));
        assert!(html.contains("&lt;img src=x onerror=alert(1)&gt;"));
        let missing = preview_not_found("<x>\"");
        assert!(missing.contains("&lt;x&gt;&quot;"));
        assert!(!missing.contains("<x>"));
    }

    #[test]
    fn unknown_and_slugged_ids() {
        let (_d, cfg) = test_config();
        let cat = catalog(
            vec![rec(EntityKind::Contact, "CONT-003", "name: Jane")],
            &cfg,
        );
        let page = Page::new(&cfg, &cat, "");
        assert!(preview_card(&page, "CONT-999").is_none());
        assert!(preview_card(&page, "CONT-003-jane").is_some());
        assert!(preview_not_found(&"X".repeat(100)).contains(&"X".repeat(MAX_ECHOED_ID)));
        assert!(!preview_not_found(&"X".repeat(100)).contains(&"X".repeat(MAX_ECHOED_ID + 1)));
    }

    #[test]
    fn excerpt_and_clamp() {
        assert_eq!(clamp_chars("short", 10), "short");
        assert_eq!(clamp_chars("one two three four", 12), "one two…");
        let (_d, cfg) = test_config();
        let cat = catalog(Vec::new(), &cfg);
        let page = Page::new(&cfg, &cat, "");
        assert_eq!(
            excerpt(
                "**Termin:** Do., 14:00\n**Ort:** Teams\nwrapped\nline\n\nNext",
                &page
            ),
            "Termin: Do., 14:00 · Ort: Teams wrapped line · Next"
        );
    }

    #[test]
    fn comments_stay_out_of_excerpt_and_checklist() {
        let (_d, cfg) = test_config();
        let task = with_body(
            rec(EntityKind::Task, "TASK-001", "title: Ship\nstatus: todo"),
            "Notes first.\n\n- [ ] a\n- [X] b\n* [x] c\n\n## Comments\n\n### 2026-10-05 09:00 · Jane\n\n- [ ] not part of it\n\nSecret comment text.\n",
        );
        let cat = catalog(vec![task], &cfg);
        let mut page = Page::new(&cfg, &cat, "");
        page.today = chrono::NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
        let html = preview_card(&page, "TASK-001").unwrap();
        assert!(html.contains("2 of 3 done"), "{html}");
        assert!(!html.contains("Secret comment"), "{html}");
        assert!(
            html.contains("1 comment · latest Mon 5 Oct by Jane"),
            "{html}"
        );
    }
}
