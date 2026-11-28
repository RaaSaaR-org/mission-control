//! The CRM pages: the lead pipeline board and table, quick capture, the
//! lead detail page (stage route, interaction timeline, research, business
//! card), the events list and the event summary, plus the overview's
//! follow-up and pipeline panels.
//!
//! Every write control starts `hidden` (or lives in a hidden panel) and is
//! only rendered when the server accepts edits; the script posts the forms to
//! the guarded JSON endpoints under `/api/leads` and `/api/events`.

use super::detail::rail;
use crate::commands::lead::{is_closed_lead_status, INTERACTIONS_SECTION, INTERACTION_TYPES};
use crate::commands::new::{EVENT_TYPES, LEAD_SOURCES, LEAD_WARMTH};
use crate::data::EntityRecord;
use crate::entity::EntityKind;
use crate::frontmatter;
use crate::html::catalog::display_name;
use crate::html::components::{
    data_table, empty_state, icon, kbd, link_button, page_header, panel, row_filter, section_title,
    status_badge, status_badge_lg, status_lamp, table_count, th, view_switch, Btn, Sort, View,
};
use crate::html::edit::{move_button, move_menu_template_for};
use crate::html::format::{
    capitalize, due_short, entity_href, escape_html, files_href, fmt_date, fmt_day,
    fmt_day_relative, href_with, initials, parse_date, plural, status_label, status_slug,
    url_encode,
};
use crate::html::layout::layout;
use crate::html::markdown::{render_markdown, render_markdown_in, strip_leading_h1, DocContext};
use crate::html::notes::{self, Notes};
use crate::html::Page;
use chrono::NaiveDate;
use std::collections::BTreeSet;

/// Cards shown in a closed lane (converted, lost) before the rest collapse.
const CLOSED_VISIBLE: usize = 8;
/// Customers shown as avatars on a card before "+N".
const INTRO_AVATARS: usize = 3;
/// Rows in the overview's follow-up panel.
const FOLLOW_UP_ROWS: usize = 8;
/// How far ahead the follow-up panel looks.
const FOLLOW_UP_HORIZON_DAYS: i64 = 7;
/// Statuses that mean an introduction is under way.
const INTRO_STATUSES: &[&str] = &["intro-planned", "introduced"];
/// Business card photo names looked for next to a lead.
const CARD_FILES: &[&str] = &["card.jpg", "card.jpeg", "card.png", "card.webp"];

// ── Field helpers ───────────────────────────────────────────────────────

fn field<'a>(r: &'a EntityRecord, key: &str) -> &'a str {
    frontmatter::get_str_or(&r.frontmatter, key, "").trim()
}

fn status_of(r: &EntityRecord) -> &str {
    field(r, "status")
}

/// The event a lead was met at (bare ID), if any.
fn event_of(r: &EntityRecord) -> Option<&str> {
    frontmatter::get_link_str(&r.frontmatter, "event")
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn intros_of(r: &EntityRecord) -> Vec<String> {
    frontmatter::get_link_list(&r.frontmatter, "introduce_to")
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Sort rank of a warmth: hot first, unset last.
fn warmth_rank(w: &str) -> u8 {
    match w.to_ascii_lowercase().as_str() {
        "hot" => 1,
        "warm" => 2,
        "cold" => 3,
        _ => 4,
    }
}

fn is_closed_lead(r: &EntityRecord) -> bool {
    is_closed_lead_status(status_of(r))
}

/// Where an open lead's next follow-up stands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FollowUp {
    Overdue,
    Today,
    Soon,
    Later,
}

impl FollowUp {
    fn class(self) -> &'static str {
        match self {
            FollowUp::Overdue => "overdue",
            FollowUp::Today => "today",
            FollowUp::Soon => "soon",
            FollowUp::Later => "later",
        }
    }
}

/// The follow-up date of an open lead and how urgent it is.
pub(crate) fn follow_up(r: &EntityRecord, today: NaiveDate) -> Option<(NaiveDate, FollowUp)> {
    if is_closed_lead(r) {
        return None;
    }
    let d = parse_date(field(r, "next_follow_up"))?;
    let state = match (d - today).num_days() {
        n if n < 0 => FollowUp::Overdue,
        0 => FollowUp::Today,
        1..=FOLLOW_UP_HORIZON_DAYS => FollowUp::Soon,
        _ => FollowUp::Later,
    };
    Some((d, state))
}

// ── Small components ────────────────────────────────────────────────────

/// A thermal gauge plus the word: three bars, lit by warmth.
pub(crate) fn warmth_chip(warmth: &str) -> String {
    let w = warmth.trim().to_ascii_lowercase();
    if !LEAD_WARMTH.contains(&w.as_str()) {
        return String::new();
    }
    format!(
        r#"<span class="warmth warmth-{w}" title="{label} lead"><span class="thermo" aria-hidden="true"><i></i><i></i><i></i></span><span class="warmth-label">{label}</span></span>"#,
        label = capitalize(&w)
    )
}

/// The follow-up readout: red when overdue, amber today, quiet otherwise.
fn follow_up_badge(d: NaiveDate, state: FollowUp, today: NaiveDate) -> String {
    let text = match state {
        FollowUp::Overdue => format!("{}d overdue", (today - d).num_days()),
        _ => due_short(d, today).0,
    };
    format!(
        r#"<time class="fu fu-{}" datetime="{}" title="Follow up {}">{}{}</time>"#,
        state.class(),
        d.format("%Y-%m-%d"),
        escape_html(&fmt_day(d, today)),
        icon("clock"),
        escape_html(&text)
    )
}

/// The event a lead was met at, as a compact chip (no link: it sits inside
/// the card's link).
fn event_chip(page: &Page, id: &str) -> String {
    let name = page.catalog.name(id).unwrap_or(id);
    format!(
        r#"<span class="evt-chip" title="Met at {0}">{1}<span class="evt-chip-name">{0}</span></span>"#,
        escape_html(name),
        icon("flag")
    )
}

/// Initials of the customers a lead is to be introduced to.
fn intro_avatars(page: &Page, ids: &[String]) -> String {
    if ids.is_empty() {
        return String::new();
    }
    let names: Vec<&str> = ids
        .iter()
        .map(|id| page.catalog.name(id).unwrap_or(id))
        .collect();
    let mut html = format!(
        r#"<span class="intro-stack" title="Introduce to {}">"#,
        escape_html(&names.join(", "))
    );
    for name in names.iter().take(INTRO_AVATARS) {
        html.push_str(&format!(
            r#"<span class="intro-av" aria-hidden="true">{}</span>"#,
            escape_html(&initials(name))
        ));
    }
    if names.len() > INTRO_AVATARS {
        html.push_str(&format!(
            r#"<span class="intro-av intro-more" aria-hidden="true">+{}</span>"#,
            names.len() - INTRO_AVATARS
        ));
    }
    html.push_str(&format!(
        r#"<span class="sr-only">Introduce to {}</span></span>"#,
        escape_html(&names.join(", "))
    ));
    html
}

/// "CTO at Acme GmbH", or whichever half is known.
fn org_line(r: &EntityRecord) -> String {
    match (field(r, "role"), field(r, "company")) {
        ("", "") => String::new(),
        (role, "") => escape_html(role),
        ("", company) => escape_html(company),
        (role, company) => format!("{} at {}", escape_html(role), escape_html(company)),
    }
}

// ── Board ───────────────────────────────────────────────────────────────

/// Filters of the lead board and table.
#[derive(Default)]
pub struct LeadQuery<'a> {
    pub event: Option<&'a str>,
    pub warmth: Option<&'a str>,
    pub owner: Option<&'a str>,
    pub tag: Option<&'a str>,
    pub status: Option<&'a str>,
    /// Only open leads whose follow-up is today or earlier.
    pub due: bool,
    /// Table sort field (`name`, `company`, `status`, `warmth`, `event`,
    /// `next_follow_up`, `last_contact`).
    pub sort: Option<&'a str>,
    pub dir: &'a str,
}

impl LeadQuery<'_> {
    fn is_empty(&self) -> bool {
        self.event.is_none()
            && self.warmth.is_none()
            && self.owner.is_none()
            && self.tag.is_none()
            && self.status.is_none()
            && !self.due
    }

    fn href(&self, path: &str, extra: &[(&str, &str)]) -> String {
        let mut pairs = vec![
            ("event", self.event.unwrap_or("")),
            ("warmth", self.warmth.unwrap_or("")),
            ("owner", self.owner.unwrap_or("")),
            ("tag", self.tag.unwrap_or("")),
            ("status", self.status.unwrap_or("")),
            ("due", if self.due { "1" } else { "" }),
        ];
        pairs.extend_from_slice(extra);
        href_with(path, &pairs)
    }

    fn matches(&self, r: &EntityRecord, today: NaiveDate) -> bool {
        let eq = |key: &str, want: Option<&str>| {
            want.is_none_or(|w| field(r, key).eq_ignore_ascii_case(w))
        };
        eq("warmth", self.warmth)
            && eq("owner", self.owner)
            && eq("status", self.status)
            && self.event.is_none_or(|e| event_of(r) == Some(e))
            && self.tag.is_none_or(|t| {
                frontmatter::get_string_list(&r.frontmatter, "tags")
                    .iter()
                    .any(|x| x.eq_ignore_ascii_case(t))
            })
            && (!self.due
                || follow_up(r, today)
                    .is_some_and(|(_, s)| matches!(s, FollowUp::Overdue | FollowUp::Today)))
    }
}

fn leads<'a>(page: &'a Page) -> impl Iterator<Item = &'a EntityRecord> {
    page.catalog.of_kind(EntityKind::Lead)
}

fn filtered<'a>(page: &'a Page, q: &LeadQuery) -> Vec<&'a EntityRecord> {
    leads(page).filter(|r| q.matches(r, page.today)).collect()
}

/// Board order inside a lane: soonest follow-up, then warmest, then ID.
fn lane_key(r: &EntityRecord) -> (NaiveDate, u8, String) {
    (
        parse_date(field(r, "next_follow_up")).unwrap_or(NaiveDate::MAX),
        warmth_rank(field(r, "warmth")),
        r.id.clone(),
    )
}

/// Pipeline stages in configured order, plus any stage a lead uses that
/// the config doesn't list.
fn stages<'a>(page: &Page, recs: impl Iterator<Item = &'a EntityRecord>) -> Vec<String> {
    let mut lanes: Vec<String> = EntityKind::Lead.statuses(page.cfg).to_vec();
    for r in recs {
        let s = status_of(r);
        if !s.is_empty() && !lanes.iter().any(|l| l == s) {
            lanes.push(s.to_string());
        }
    }
    lanes
}

fn new_lead_button(page: &Page, event: Option<&str>) -> String {
    if !page.editable {
        return String::new();
    }
    let href = href_with("/leads/new", &[("event", event.unwrap_or(""))]);
    format!(
        r#"<a class="btn btn-primary btn-sm" href="{href}" data-new-lead>{}New lead</a>"#,
        icon("plus")
    )
}

fn lead_view_toggle(q: &LeadQuery, board: bool) -> String {
    let board_href = q.href("/leads", &[]);
    let list_href = q.href("/leads/list", &[]);
    view_switch(
        "Lead view",
        &[
            View {
                href: &board_href,
                icon: "board",
                label: "Pipeline",
                active: board,
                key: "b",
            },
            View {
                href: &list_href,
                icon: "list",
                label: "Table",
                active: !board,
                key: "l",
            },
        ],
    )
}

/// One labelled select, like `components::filter_cell` but with the values
/// as given (no "All" duplication).
fn filter_select(
    name: &str,
    label: &str,
    options: &[(String, String)],
    selected: Option<&str>,
) -> String {
    crate::html::components::filter_cell(name, label, options, selected)
}

fn lead_toolbar(
    page: &Page,
    action: &str,
    q: &LeadQuery,
    with_status: bool,
    target: &str,
) -> String {
    let all: Vec<&EntityRecord> = leads(page).collect();
    let mut cells = String::new();
    if with_status {
        let statuses: Vec<(String, String)> = stages(page, all.iter().copied())
            .into_iter()
            .map(|s| {
                let l = status_label(&s);
                (s, l)
            })
            .collect();
        cells.push_str(&filter_select("status", "Stage", &statuses, q.status));
    }
    let events: Vec<(String, String)> = sorted_events(page)
        .into_iter()
        .map(|e| (e.id.clone(), display_name(e).to_string()))
        .collect();
    if !events.is_empty() {
        cells.push_str(&filter_select("event", "Event", &events, q.event));
    }
    let warmth: Vec<(String, String)> = LEAD_WARMTH
        .iter()
        .map(|w| (w.to_string(), capitalize(w)))
        .collect();
    cells.push_str(&filter_select("warmth", "Warmth", &warmth, q.warmth));
    let owners: BTreeSet<&str> = all
        .iter()
        .map(|r| field(r, "owner"))
        .filter(|o| !o.is_empty())
        .collect();
    if !owners.is_empty() {
        let owners: Vec<(String, String)> = owners
            .into_iter()
            .map(|o| (o.to_string(), o.to_string()))
            .collect();
        cells.push_str(&filter_select("owner", "Owner", &owners, q.owner));
    }
    let tags: BTreeSet<String> = all
        .iter()
        .flat_map(|r| frontmatter::get_string_list(&r.frontmatter, "tags"))
        .filter(|t| !t.trim().is_empty())
        .collect();
    if !tags.is_empty() {
        let tags: Vec<(String, String)> = tags.into_iter().map(|t| (t.clone(), t)).collect();
        cells.push_str(&filter_select("tag", "Tag", &tags, q.tag));
    }
    let mut html = format!(
        r#"<form class="toolbar filter-form" method="get" action="{action}" autocomplete="off"><div class="filter-group">{cells}</div>"#
    );
    // A toggle link, so the state is always in the URL.
    let due_href = LeadQuery {
        due: !q.due,
        ..clone_query(q)
    }
    .href(action, &sort_pairs(q));
    let due_count = all
        .iter()
        .filter(|r| {
            follow_up(r, page.today)
                .is_some_and(|(_, s)| matches!(s, FollowUp::Overdue | FollowUp::Today))
        })
        .count();
    html.push_str(&format!(
        r#"<a class="chip due-toggle{}" href="{due_href}" aria-pressed="{}">{}Follow-up due<span class="chip-count">{due_count}</span></a>"#,
        if q.due { " chip-active" } else { "" },
        q.due,
        icon("clock"),
    ));
    if q.due {
        html.push_str(r#"<input type="hidden" name="due" value="1">"#);
    }
    if let Some(s) = q.sort {
        html.push_str(&format!(
            r#"<input type="hidden" name="sort" value="{}"><input type="hidden" name="dir" value="{}">"#,
            escape_html(s),
            escape_html(q.dir)
        ));
    }
    html.push_str(
        r#"<noscript><button type="submit" class="btn btn-sm">Apply</button></noscript>"#,
    );
    if !q.is_empty() {
        html.push_str(&link_button(
            action,
            "Clear filters",
            Btn::Ghost,
            "btn-sm reset-link",
        ));
    }
    html.push_str(&row_filter(target, "Search leads"));
    html.push_str("</form>");
    html
}

fn clone_query<'a>(q: &LeadQuery<'a>) -> LeadQuery<'a> {
    LeadQuery {
        event: q.event,
        warmth: q.warmth,
        owner: q.owner,
        tag: q.tag,
        status: q.status,
        due: q.due,
        sort: q.sort,
        dir: q.dir,
    }
}

fn sort_pairs<'a>(q: &LeadQuery<'a>) -> Vec<(&'static str, &'a str)> {
    match q.sort {
        Some(s) => vec![("sort", s), ("dir", q.dir)],
        None => Vec::new(),
    }
}

fn no_leads_yet(page: &Page) -> String {
    let hint = if page.editable {
        r#"Capture the first person you met with <a href="/leads/new">New lead</a>, or run <code>mc new lead "Name" --company "Acme"</code>."#
    } else {
        r#"Create one with <code>mc new lead "Name" --company "Acme"</code>."#
    };
    empty_state("No leads yet", hint, "")
}

/// The pipeline board: one lane per configured lead stage.
pub fn leads_board_page(page: &Page, q: &LeadQuery) -> String {
    let all_count = leads(page).count();
    let shown = filtered(page, q);
    let lanes = stages(page, leads(page));
    let open = shown.iter().filter(|r| !is_closed_lead(r)).count();
    let due = shown
        .iter()
        .filter(|r| {
            follow_up(r, page.today)
                .is_some_and(|(_, s)| matches!(s, FollowUp::Overdue | FollowUp::Today))
        })
        .count();
    let meta = format!(
        "{} open, {} due",
        open,
        plural(due, "follow-up", "follow-ups")
    );
    let mut body = page_header(
        "Leads",
        &meta,
        &format!(
            "{}{}",
            new_lead_button(page, q.event),
            lead_view_toggle(q, true)
        ),
    );
    if all_count == 0 {
        body.push_str(&no_leads_yet(page));
        return layout(page, "Lead pipeline", "/leads", "", &body);
    }
    body.push_str(&lead_toolbar(page, "/leads", q, false, "board"));

    let open_total = shown.iter().filter(|r| !is_closed_lead(r)).count().max(1);
    body.push_str(&format!(
        r#"<div class="kanban-board lead-board" id="board" data-api="/leads" data-order="due" data-late-word="overdue" style="--lanes:{};--open-lanes:{};--closed-lanes:{}">"#,
        lanes.len(),
        lanes.iter().filter(|l| !is_closed_lead_status(l)).count(),
        lanes.iter().filter(|l| is_closed_lead_status(l)).count(),
    ));
    for lane in &lanes {
        let mut items: Vec<&EntityRecord> = shown
            .iter()
            .copied()
            .filter(|r| status_of(r) == lane)
            .collect();
        items.sort_by_cached_key(|r| lane_key(r));
        body.push_str(&lead_lane(page, lane, &items, open_total));
    }
    body.push_str("</div>");
    if page.editable {
        body.push_str(&move_menu_template_for(page, EntityKind::Lead));
    }
    layout(page, "Lead pipeline", "/leads", "", &body)
}

fn lead_lane(page: &Page, status: &str, items: &[&EntityRecord], open_total: usize) -> String {
    let slug = status_slug(status);
    let label = status_label(status);
    let closed = is_closed_lead_status(status);
    let overdue = items
        .iter()
        .filter(|r| follow_up(r, page.today).is_some_and(|(_, s)| s == FollowUp::Overdue))
        .count();
    let late = if overdue > 0 {
        format!(r#"<span class="kanban-late">{overdue} overdue</span>"#)
    } else {
        String::new()
    };
    // The share of open leads at this stage, as a thin funnel meter.
    let share = if closed {
        String::new()
    } else {
        format!(
            r#"<span class="lane-share" style="--share:{}%" aria-hidden="true"></span>"#,
            items.len() * 100 / open_total
        )
    };
    let mut html = format!(
        r#"<section class="kanban-column lead-lane col-{slug}{closed_class}" data-status="{st}"{compact} aria-labelledby="lane-{slug}"><h2 class="kanban-head" id="lane-{slug}">{lamp}<span class="kanban-name">{name}</span><span class="kanban-count readout" data-lane-count>{n}</span>{late}{share}</h2><div class="kanban-cards" data-status="{st}">"#,
        closed_class = if closed { " is-closed" } else { "" },
        compact = if closed { " data-compact" } else { "" },
        st = escape_html(status),
        lamp = status_lamp(status),
        name = escape_html(&label),
        n = items.len(),
    );
    if items.is_empty() {
        html.push_str(&format!(
            r#"<div class="kanban-empty">No leads {}.</div>"#,
            if closed {
                escape_html(&label.to_lowercase())
            } else {
                format!("at {}", escape_html(&label.to_lowercase()))
            }
        ));
    }
    for (i, r) in items.iter().enumerate() {
        if closed && i == CLOSED_VISIBLE {
            html.push_str(&format!(
                r#"<details class="kanban-more"><summary>Show {} more</summary><div class="kanban-more-cards">"#,
                items.len() - CLOSED_VISIBLE
            ));
        }
        html.push_str(&lead_item(page, r, closed));
    }
    if closed && items.len() > CLOSED_VISIBLE {
        html.push_str("</div></details>");
    }
    html.push_str("</div></section>");
    html
}

/// A lead's board item as it renders in its own lane (also sent back after
/// a move so the page can swap it in).
pub fn lead_card(page: &Page, r: &EntityRecord) -> String {
    lead_item(page, r, is_closed_lead(r))
}

fn lead_item(page: &Page, r: &EntityRecord, compact: bool) -> String {
    let card = if compact {
        compact_lead_card(page, r)
    } else {
        full_lead_card(page, r)
    };
    let mv = if page.editable {
        move_button(&r.id)
    } else {
        String::new()
    };
    format!(
        r#"<div class="kanban-item lead-item" data-row data-id="{}" data-status="{}" data-pri="{}" data-due="{}">{card}{mv}</div>"#,
        escape_html(&r.id),
        escape_html(status_of(r)),
        warmth_rank(field(r, "warmth")),
        escape_html(field(r, "next_follow_up")),
    )
}

fn full_lead_card(page: &Page, r: &EntityRecord) -> String {
    let fu = follow_up(r, page.today);
    let state = match fu {
        Some((_, FollowUp::Overdue)) => " is-overdue",
        Some((_, FollowUp::Today)) => " is-soon",
        _ => "",
    };
    let warmth = field(r, "warmth").to_ascii_lowercase();
    let org = org_line(r);
    let org = if org.is_empty() {
        String::new()
    } else {
        format!(r#"<span class="lead-card-org">{org}</span>"#)
    };
    let mut meta = warmth_chip(&warmth);
    if let Some(e) = event_of(r) {
        meta.push_str(&event_chip(page, e));
    }
    let meta = if meta.is_empty() {
        String::new()
    } else {
        format!(r#"<span class="lead-card-meta">{meta}</span>"#)
    };
    let fu_html = fu
        .map(|(d, s)| follow_up_badge(d, s, page.today))
        .unwrap_or_default();
    let intros = intro_avatars(page, &intros_of(r));
    let footer = if fu_html.is_empty() && intros.is_empty() {
        String::new()
    } else {
        format!(r#"<span class="kanban-card-footer">{fu_html}{intros}</span>"#)
    };
    format!(
        r#"<a class="kanban-card lead-card{state}{w}" href="{href}"><span class="kanban-card-title">{name}</span>{org}{meta}{footer}</a>"#,
        w = if warmth.is_empty() {
            String::new()
        } else {
            format!(" warmth-{}", status_slug(&warmth))
        },
        href = entity_href(&r.id),
        name = escape_html(display_name(r)),
    )
}

fn compact_lead_card(page: &Page, r: &EntityRecord) -> String {
    let name = display_name(r);
    let company = field(r, "company");
    let converted = frontmatter::get_link_str(&r.frontmatter, "converted_to")
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let tail = match converted {
        Some(c) => format!(
            r#"<span class="lead-converted" title="Converted to {} ({c})">{}{}</span>"#,
            escape_html(page.catalog.name(c).unwrap_or(c)),
            icon("check"),
            escape_html(if company.is_empty() { c } else { company }),
        ),
        None if !company.is_empty() => {
            format!(
                r#"<span class="lead-card-co">{}</span>"#,
                escape_html(company)
            )
        }
        None => String::new(),
    };
    format!(
        r#"<a class="kanban-card lead-card is-compact" href="{}" title="{}"><span class="kanban-card-title">{}</span>{tail}</a>"#,
        entity_href(&r.id),
        escape_html(&format!("{} {}", r.id, name)),
        escape_html(name),
    )
}

// ── Table ───────────────────────────────────────────────────────────────

/// Sort leads for the table; empty values always sort last.
fn sort_leads(page: &Page, recs: &mut [&EntityRecord], field_name: &str, dir: &str) {
    let order = EntityKind::Lead.statuses(page.cfg);
    let key = |r: &EntityRecord| -> (bool, String) {
        let v = match field_name {
            "name" => display_name(r).to_lowercase(),
            "status" => {
                let s = status_of(r);
                order
                    .iter()
                    .position(|o| o == s)
                    .map(|i| format!("{i:03}"))
                    .unwrap_or_else(|| format!("999{s}"))
            }
            "warmth" => {
                let rank = warmth_rank(field(r, "warmth"));
                if rank == 4 {
                    String::new()
                } else {
                    rank.to_string()
                }
            }
            "event" => event_of(r)
                .map(|e| page.catalog.name(e).unwrap_or(e).to_lowercase())
                .unwrap_or_default(),
            other => field(r, other).to_lowercase(),
        };
        (v.is_empty(), v)
    };
    recs.sort_by_cached_key(|r| key(r));
    if dir == "desc" {
        // Keep empty values last in both directions.
        let split = recs.iter().position(|r| key(r).0).unwrap_or(recs.len());
        recs[..split].reverse();
    }
}

/// The sortable lead table.
pub fn leads_list_page(page: &Page, q: &LeadQuery) -> String {
    let total = leads(page).count();
    let mut shown = filtered(page, q);
    let meta = if q.is_empty() {
        format!("{total} total")
    } else {
        format!("{} of {total} shown", shown.len())
    };
    let mut body = page_header(
        "Leads",
        &meta,
        &format!(
            "{}{}",
            new_lead_button(page, q.event),
            lead_view_toggle(q, false)
        ),
    );
    if total == 0 {
        body.push_str(&no_leads_yet(page));
        return layout(page, "Leads", "/leads", "", &body);
    }
    body.push_str(&lead_toolbar(page, "/leads/list", q, true, "rows"));
    if shown.is_empty() {
        body.push_str(&empty_state(
            "No leads match these filters",
            "Clear the filters to see every lead again.",
            &link_button("/leads/list", "Clear filters", Btn::Secondary, ""),
        ));
        return layout(page, "Leads", "/leads", "", &body);
    }
    match q.sort {
        Some(f) => sort_leads(page, &mut shown, f, q.dir),
        None => shown.sort_by_cached_key(|r| (is_closed_lead(r), lane_key(r))),
    }
    let sort = Sort {
        field: q.sort,
        dir: q.dir,
    };
    let sort_href = |f: &str, d: &str| q.href("/leads/list", &[("sort", f), ("dir", d)]);
    let sorted = |label: &str, class: &str, f: &str| {
        th(
            label,
            class,
            Some((f, &sort, &sort_href as &dyn Fn(&str, &str) -> String)),
        )
    };
    let head = [
        sorted("Name", "col-name", "name"),
        sorted("Company", "col-company", "company"),
        sorted("Stage", "col-status", "status"),
        sorted("Warmth", "col-warmth", "warmth"),
        sorted("Event", "col-event", "event"),
        th("Introduce to", "col-intro", None),
        sorted("Next follow-up", "col-date", "next_follow_up"),
        sorted("Last contact", "col-date", "last_contact"),
    ]
    .concat();
    let rows: String = shown
        .iter()
        .map(|r| {
            let fu = follow_up(r, page.today)
                .map(|(d, s)| follow_up_badge(d, s, page.today))
                .or_else(|| {
                    parse_date(field(r, "next_follow_up")).map(|d| {
                        format!(
                            r#"<time class="muted" datetime="{}">{}</time>"#,
                            d.format("%Y-%m-%d"),
                            fmt_date(d, page.today)
                        )
                    })
                })
                .unwrap_or_default();
            let last = parse_date(field(r, "last_contact"))
                .map(|d| {
                    format!(
                        r#"<time datetime="{}">{}</time>"#,
                        d.format("%Y-%m-%d"),
                        fmt_date(d, page.today)
                    )
                })
                .unwrap_or_default();
            let event = event_of(r)
                .map(|e| page.catalog.ref_html(e))
                .unwrap_or_default();
            let role = field(r, "role");
            let role = if role.is_empty() {
                String::new()
            } else {
                format!(r#"<span class="cell-sub">{}</span>"#, escape_html(role))
            };
            format!(
                r#"<tr data-row data-id="{id}"><td class="col-name"><a class="name-link" href="{href}">{name}</a>{role}</td><td class="col-company">{company}</td><td class="col-status">{status}</td><td class="col-warmth">{warmth}</td><td class="col-event">{event}</td><td class="col-intro">{intro}</td><td class="col-date">{fu}</td><td class="col-date">{last}</td></tr>"#,
                id = escape_html(&r.id),
                href = entity_href(&r.id),
                name = escape_html(display_name(r)),
                company = escape_html(field(r, "company")),
                status = status_badge(status_of(r)),
                warmth = warmth_chip(field(r, "warmth")),
                intro = page.catalog.refs_html_compact(&intros_of(r)),
            )
        })
        .collect();
    body.push_str(&data_table("lead-table", &head, &rows));
    body.push_str(&table_count(shown.len(), total));
    layout(page, "Leads", "/leads", "", &body)
}

// ── Quick capture ───────────────────────────────────────────────────────

/// Events, newest first.
fn sorted_events<'a>(page: &'a Page) -> Vec<&'a EntityRecord> {
    let mut events: Vec<&EntityRecord> = page.catalog.of_kind(EntityKind::Event).collect();
    events.sort_by_key(|e| std::cmp::Reverse((field(e, "date").to_string(), e.id.clone())));
    events
}

/// The event new leads most likely come from: one happening today, else the
/// latest one that already started, else the next upcoming one (planned or
/// attended, never cancelled).
pub(crate) fn default_event<'a>(page: &'a Page) -> Option<&'a EntityRecord> {
    let today = page.today;
    let usable: Vec<(NaiveDate, NaiveDate, &EntityRecord)> = page
        .catalog
        .of_kind(EntityKind::Event)
        .filter(|e| matches!(status_of(e), "planned" | "attended"))
        .filter_map(|e| {
            let start = parse_date(field(e, "date"))?;
            let end = parse_date(field(e, "end_date")).unwrap_or(start).max(start);
            Some((start, end, e))
        })
        .collect();
    let ongoing = usable
        .iter()
        .filter(|(s, e, _)| *s <= today && today <= *e)
        .max_by_key(|(s, _, _)| *s);
    let past = || {
        usable
            .iter()
            .filter(|(s, _, _)| *s <= today)
            .max_by_key(|(s, _, _)| *s)
    };
    let next = || {
        usable
            .iter()
            .filter(|(s, _, _)| *s > today)
            .min_by_key(|(s, _, _)| *s)
    };
    ongoing.or_else(past).or_else(next).map(|(_, _, e)| *e)
}

fn input(name: &str, kind: &str, value: &str, extra: &str) -> String {
    format!(
        r#"<input type="{kind}" name="{name}" value="{}"{extra}>"#,
        escape_html(value)
    )
}

fn labelled(label: &str, control: &str, class: &str) -> String {
    format!(
        r#"<label class="form-field{class}"><span class="form-label">{label}</span>{control}</label>"#
    )
}

fn options(opts: &[(String, String)], selected: &str, none: Option<&str>) -> String {
    let mut html = none
        .map(|n| format!(r#"<option value="">{}</option>"#, escape_html(n)))
        .unwrap_or_default();
    for (v, l) in opts {
        html.push_str(&format!(
            r#"<option value="{}"{}>{}</option>"#,
            escape_html(v),
            if v == selected { " selected" } else { "" },
            escape_html(l)
        ));
    }
    if !selected.is_empty() && !opts.iter().any(|(v, _)| v == selected) {
        html.push_str(&format!(
            r#"<option value="{0}" selected>{0}</option>"#,
            escape_html(selected)
        ));
    }
    html
}

fn event_options(page: &Page) -> Vec<(String, String)> {
    sorted_events(page)
        .into_iter()
        .filter(|e| status_of(e) != "cancelled")
        .map(|e| {
            let date = parse_date(field(e, "date"))
                .map(|d| format!(" ({})", fmt_date(d, page.today)))
                .unwrap_or_default();
            (e.id.clone(), format!("{}{date}", display_name(e)))
        })
        .collect()
}

/// Segmented radio chips (warmth, interaction type).
fn radio_chips(name: &str, values: &[&str], selected: &str, with_none: Option<&str>) -> String {
    let mut html = format!(r#"<div class="seg-radio" role="radiogroup" data-name="{name}">"#);
    if let Some(label) = with_none {
        html.push_str(&format!(
            r#"<label class="seg-opt"><input type="radio" name="{name}" value=""{}><span>{}</span></label>"#,
            if selected.is_empty() { " checked" } else { "" },
            escape_html(label)
        ));
    }
    for v in values {
        html.push_str(&format!(
            r#"<label class="seg-opt seg-{v}"><input type="radio" name="{name}" value="{v}"{}><span>{}</span></label>"#,
            if *v == selected { " checked" } else { "" },
            escape_html(&type_label(v))
        ));
    }
    html.push_str("</div>");
    html
}

/// `/leads/new`: a short, thumb-friendly form. Only the name is required.
pub fn new_lead_page(page: &Page, event: Option<&str>) -> String {
    let mut body = String::from(r#"<div class="capture">"#);
    body.push_str(
        r#"<nav class="breadcrumb" aria-label="Breadcrumb"><a href="/leads">Leads</a><span class="sep" aria-hidden="true">/</span><span class="crumb-id">New</span></nav><h1 class="page-title capture-title">Who did you meet?</h1>"#,
    );
    if !page.editable {
        body.push_str(&empty_state(
            "This dashboard is read-only",
            r#"Restart <code>mc serve</code> without <code>--read-only</code> to capture leads here, or run <code>mc new lead "Name"</code>."#,
            "",
        ));
        body.push_str("</div>");
        return layout(page, "New lead", "/leads", "", &body);
    }
    let preset = event
        .filter(|e| page.catalog.of_kind(EntityKind::Event).any(|r| r.id == *e))
        .map(str::to_string)
        .or_else(|| default_event(page).map(|e| e.id.clone()))
        .unwrap_or_default();
    let events = event_options(page);
    let event_field = if events.is_empty() {
        String::new()
    } else {
        labelled(
            "Met at",
            &format!(
                r#"<select name="event">{}</select>"#,
                options(&events, &preset, Some("No event"))
            ),
            "",
        )
    };
    body.push_str(&format!(
        r#"<form class="form capture-form" data-lead-form="create" data-endpoint="/leads" data-method="POST" autocomplete="off">
  {name}
  {company}
  <div class="form-grid capture-grid">{email}{phone}</div>
  {event_field}
  <div class="form-field"><span class="form-label">Warmth</span>{warmth}</div>
  {note}
  <details class="capture-more"><summary>More details</summary><div class="form-grid capture-grid">{role}{linkedin}{website}{follow}</div></details>
  <p class="form-error" role="alert" hidden></p>
  <div class="form-actions capture-actions"><label class="form-check"><input type="checkbox" name="another" checked> Add another after this</label><button type="submit" class="btn btn-primary btn-lg">Save lead {k}</button></div>
</form>"#,
        name = labelled(
            "Name",
            r#"<input type="text" name="name" required maxlength="120" autocomplete="off" autocapitalize="words" placeholder="Jane Doe" autofocus>"#,
            " form-field-wide"
        ),
        company = labelled(
            "Company",
            r#"<input type="text" name="company" maxlength="120" autocomplete="off" autocapitalize="words" placeholder="Acme GmbH">"#,
            " form-field-wide"
        ),
        email = labelled(
            "Email",
            r#"<input type="email" name="email" maxlength="160" autocomplete="off" inputmode="email" placeholder="jane@acme.example">"#,
            ""
        ),
        phone = labelled(
            "Phone",
            r#"<input type="tel" name="phone" maxlength="60" autocomplete="off" inputmode="tel" placeholder="+49 …">"#,
            ""
        ),
        warmth = radio_chips("warmth", &LEAD_WARMTH, "", Some("Not sure")),
        note = labelled(
            "Note",
            r#"<textarea name="notes" rows="3" maxlength="4000" placeholder="What did you talk about? What do they need?"></textarea>"#,
            ""
        ),
        role = labelled("Role", &input("role", "text", "", r#" maxlength="120""#), ""),
        linkedin = labelled(
            "LinkedIn",
            &input("linkedin", "url", "", r#" maxlength="300" placeholder="https://www.linkedin.com/in/…""#),
            ""
        ),
        website = labelled(
            "Website",
            &input("website", "url", "", r#" maxlength="300" placeholder="https://""#),
            ""
        ),
        follow = labelled("Follow up on", &input("next_follow_up", "date", "", ""), ""),
        k = kbd("⌘↵"),
    ));
    body.push_str("</div>");
    layout(page, "New lead", "/leads", "", &body)
}

// ── Lead detail ─────────────────────────────────────────────────────────

/// One line of the `## Interactions` log.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Interaction {
    pub date: String,
    pub kind: String,
    pub note: String,
}

/// Parse `- YYYY-MM-DD · type · note` (the type and note are optional, so a
/// hand-written `- 2026-11-20 Met at the booth` still shows).
pub(crate) fn parse_interaction(line: &str) -> Option<Interaction> {
    let rest = line
        .trim()
        .strip_prefix("- ")
        .or_else(|| line.trim().strip_prefix("* "))?;
    let date = rest.get(..10)?;
    parse_date(date)?;
    let rest = rest[10..].trim_start_matches([' ', '·', '-']);
    let (kind, note) = match rest.split_once('·') {
        Some((k, n)) if INTERACTION_TYPES.contains(&k.trim().to_ascii_lowercase().as_str()) => {
            (k.trim().to_ascii_lowercase(), n.trim().to_string())
        }
        _ => {
            let word = rest
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if INTERACTION_TYPES.contains(&word.as_str()) && rest.trim().len() == word.len() {
                (word, String::new())
            } else {
                ("note".to_string(), rest.trim().to_string())
            }
        }
    };
    Some(Interaction {
        date: date.to_string(),
        kind,
        note,
    })
}

/// `body` split into the contents of its `## <heading>` section and the
/// body without that section. Headings in fenced code are ignored.
pub(crate) fn take_section(body: &str, heading: &str) -> (Option<String>, String) {
    let wanted = format!("## {heading}").to_lowercase();
    let mut in_code = false;
    let mut section: Option<String> = None;
    let mut rest = String::new();
    let mut inside = false;
    for line in body.split_inclusive('\n') {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_code = !in_code;
        }
        if !in_code {
            let top = t.starts_with("# ") || t.starts_with("## ") || t.starts_with("%% mc-links");
            if inside && top {
                inside = false;
            } else if !inside && section.is_none() && t.to_lowercase() == wanted {
                inside = true;
                section = Some(String::new());
                continue;
            }
        }
        if inside {
            if let Some(s) = section.as_mut() {
                s.push_str(line);
            }
        } else {
            rest.push_str(line);
        }
    }
    (section, rest)
}

/// `md` without `## ` sections that hold nothing but whitespace or comments
/// (a template's empty `## Notes`).
pub(crate) fn drop_empty_sections(md: &str) -> String {
    let mut out = String::new();
    let mut pending: Option<String> = None;
    let mut buf = String::new();
    let flush = |out: &mut String, pending: &mut Option<String>, buf: &mut String| {
        match pending.take() {
            Some(head) if has_content(buf) => {
                out.push_str(&head);
                out.push_str(buf);
            }
            Some(_) => {}
            None => out.push_str(buf),
        }
        buf.clear();
    };
    for line in md.split_inclusive('\n') {
        if line.starts_with("## ") || line.starts_with("# ") {
            flush(&mut out, &mut pending, &mut buf);
            if line.starts_with("## ") {
                pending = Some(line.to_string());
                continue;
            }
        }
        buf.push_str(line);
    }
    flush(&mut out, &mut pending, &mut buf);
    out
}

/// Whether Markdown has visible content (not just whitespace or comments).
fn has_content(md: &str) -> bool {
    let mut s = md.to_string();
    // HTML comments and Obsidian `%% … %%` comments (mc's link footer) are invisible.
    for (open, close) in [("<!--", "-->"), ("%%", "%%")] {
        while let Some(start) = s.find(open) {
            match s[start + open.len()..].find(close) {
                Some(end) => s.replace_range(start..start + open.len() + end + close.len(), ""),
                None => s.truncate(start),
            }
        }
    }
    !s.trim().is_empty()
}

fn type_label(kind: &str) -> String {
    match kind {
        "linkedin" => "LinkedIn".to_string(),
        other => capitalize(other),
    }
}

fn interaction_icon(kind: &str) -> &'static str {
    match kind {
        "call" => "phone",
        "email" => "mail",
        "meeting" => "users",
        "event" => "flag",
        "linkedin" => "linkedin",
        "intro" => "intro",
        _ => "note",
    }
}

fn timeline(page: &Page, entries: &[Interaction]) -> String {
    if entries.is_empty() {
        return r#"<p class="muted tl-empty">No interactions logged yet.</p>"#.to_string();
    }
    let mut html = String::from(r#"<ol class="timeline">"#);
    for e in entries {
        let d = parse_date(&e.date);
        let when = d
            .map(|d| {
                format!(
                    r#"<time datetime="{}" title="{}">{}</time>"#,
                    d.format("%Y-%m-%d"),
                    d.format("%a %-d %b %Y"),
                    escape_html(&fmt_day_relative(d, page.today))
                )
            })
            .unwrap_or_default();
        let note = if e.note.is_empty() {
            String::new()
        } else {
            format!(
                r#"<div class="tl-note">{}</div>"#,
                render_markdown(&e.note, page.catalog)
            )
        };
        html.push_str(&format!(
            r#"<li class="tl-item tl-{kind}"><span class="tl-mark" aria-hidden="true">{icon}</span><div class="tl-body"><div class="tl-head"><span class="tl-type">{label}</span>{when}</div>{note}</div></li>"#,
            kind = status_slug(&e.kind),
            icon = icon(interaction_icon(&e.kind)),
            label = escape_html(&type_label(&e.kind)),
        ));
    }
    html.push_str("</ol>");
    html
}

/// Quick dates offered next to follow-up inputs.
fn quick_dates(page: &Page, target: &str) -> String {
    let opts = [
        ("Tomorrow", 1),
        ("In 3 days", 3),
        ("In a week", 7),
        ("In 2 weeks", 14),
    ];
    let chips: String = opts
        .iter()
        .map(|(label, n)| {
            let d = page.today + chrono::Duration::days(*n);
            format!(
                r#"<button type="button" class="chip chip-btn" data-set-date="{}" data-target="{target}">{label}</button>"#,
                d.format("%Y-%m-%d")
            )
        })
        .collect();
    format!(r#"<div class="quick-dates">{chips}</div>"#)
}

fn composer(page: &Page, id: &str) -> String {
    let types: Vec<&str> = INTERACTION_TYPES.to_vec();
    format!(
        r#"<form class="composer lead-composer" id="lead-log" data-lead-form="log" data-endpoint="/leads/{eid}/interactions" data-method="POST" hidden>
  {types}
  <textarea name="note" rows="2" maxlength="2000" required placeholder="What happened? Who said what?" aria-label="Note"></textarea>
  <div class="composer-row"><label class="form-field composer-date"><span class="form-label">Date</span>{date}</label><label class="form-field composer-date"><span class="form-label">Next follow-up</span><input type="date" name="next_follow_up" id="log-fu"></label>{quick}</div>
  <p class="form-error" role="alert" hidden></p>
  <div class="form-actions"><button type="submit" class="btn btn-primary btn-sm">Log interaction {k}</button></div>
</form>"#,
        eid = url_encode(id),
        types = radio_chips("type", &types, "call", None),
        date = input(
            "date",
            "date",
            &page.today.format("%Y-%m-%d").to_string(),
            &format!(r#" required max="{}""#, page.today.format("%Y-%m-%d"))
        ),
        quick = quick_dates(page, "log-fu"),
        k = kbd("⌘↵"),
    )
}

/// The stage route: every pipeline stage as a waypoint, the current one lit.
/// Editable waypoints move the lead; "converted" opens the convert panel.
fn stage_route(page: &Page, r: &EntityRecord) -> String {
    let current = status_of(r);
    let all = EntityKind::Lead.statuses(page.cfg);
    let route: Vec<&String> = all.iter().filter(|s| *s != "lost").collect();
    let pos = route.iter().position(|s| *s == current);
    let lost = current == "lost";
    let converted = frontmatter::get_link_str(&r.frontmatter, "converted_to")
        .is_some_and(|s| !s.trim().is_empty());
    let mut html = format!(
        r#"<nav class="stage-route{}" aria-label="Pipeline stage" data-lead-id="{}"><ol>"#,
        if lost { " is-lost" } else { "" },
        escape_html(&r.id)
    );
    for (i, s) in route.iter().enumerate() {
        let state = match pos {
            Some(p) if i < p => " is-done",
            Some(p) if i == p => " is-current",
            _ => "",
        };
        let label = escape_html(&status_label(s));
        let current_attr = if state == " is-current" {
            r#" aria-current="step""#
        } else {
            ""
        };
        let inner = format!(
            r#"<span class="stage-dot" aria-hidden="true"></span><span class="stage-label">{label}</span>"#
        );
        // A lead linked to a contact stays converted (see lead::move_lead).
        let control = if page.editable && state != " is-current" && !converted {
            let opens = if s.as_str() == "converted" && !converted {
                r#" data-opens="lead-convert""#
            } else {
                ""
            };
            format!(
                r#"<button type="button" class="stage-btn" data-lead-stage="{}"{opens} title="Move to {label}">{inner}</button>"#,
                escape_html(s)
            )
        } else {
            format!(r#"<span class="stage-btn"{current_attr}>{inner}</span>"#)
        };
        html.push_str(&format!(r#"<li class="stage{state}">{control}</li>"#));
    }
    html.push_str("</ol>");
    if all.iter().any(|s| s == "lost") {
        if lost {
            html.push_str(r#"<span class="stage-lost" aria-current="step">Lost</span>"#);
        } else if page.editable && !converted {
            html.push_str(
                r#"<button type="button" class="btn btn-ghost btn-sm stage-lose" data-lead-stage="lost">Mark lost</button>"#,
            );
        }
    }
    html.push_str("</nav>");
    html
}

fn contact_links(r: &EntityRecord) -> String {
    let mut links = String::new();
    let email = field(r, "email");
    if !email.is_empty() {
        links.push_str(&format!(
            r#"<a class="contact-link" href="mailto:{0}">{1}<span>{0}</span></a>"#,
            escape_html(email),
            icon("mail")
        ));
    }
    let phone = field(r, "phone");
    if !phone.is_empty() {
        let tel: String = phone
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '+')
            .collect();
        links.push_str(&format!(
            r#"<a class="contact-link" href="tel:{}">{}<span>{}</span></a>"#,
            escape_html(&tel),
            icon("phone"),
            escape_html(phone)
        ));
    }
    let web = |url: &str| url.starts_with("https://") || url.starts_with("http://");
    let linkedin = field(r, "linkedin");
    if web(linkedin) {
        links.push_str(&format!(
            r#"<a class="contact-link" href="{}" target="_blank" rel="noopener">{}<span>LinkedIn</span></a>"#,
            escape_html(linkedin),
            icon("linkedin")
        ));
    }
    let website = field(r, "website");
    if web(website) {
        let shown = website
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("www.")
            .trim_end_matches('/');
        links.push_str(&format!(
            r#"<a class="contact-link" href="{}" target="_blank" rel="noopener">{}<span>{}</span></a>"#,
            escape_html(website),
            icon("globe"),
            escape_html(shown)
        ));
    }
    if links.is_empty() {
        String::new()
    } else {
        format!(r#"<div class="contact-links">{links}</div>"#)
    }
}

fn lead_title_block(page: &Page, r: &EntityRecord) -> String {
    let cell = |label: &str, value: &str| {
        format!(
            r#"<div class="tb-cell"><dt class="tb-label">{label}</dt><dd class="tb-value">{value}</dd></div>"#
        )
    };
    let none = r#"<span class="muted">None</span>"#;
    let mut cells = vec![cell("Stage", &status_badge_lg(status_of(r)))];
    let warmth = warmth_chip(field(r, "warmth"));
    cells.push(cell(
        "Warmth",
        if warmth.is_empty() { none } else { &warmth },
    ));
    let fu = match (
        follow_up(r, page.today),
        parse_date(field(r, "next_follow_up")),
    ) {
        (Some((d, s)), _) => format!(
            r#"<time datetime="{}">{}</time>{}"#,
            d.format("%Y-%m-%d"),
            escape_html(&fmt_day(d, page.today)),
            follow_up_badge(d, s, page.today)
        ),
        (None, Some(d)) => format!(
            r#"<time datetime="{}">{}</time>"#,
            d.format("%Y-%m-%d"),
            escape_html(&fmt_day(d, page.today))
        ),
        _ => none.to_string(),
    };
    let fu_edit = if page.editable {
        r#"<button type="button" class="tb-edit" data-toggle="lead-followup" aria-controls="lead-followup" aria-expanded="false" hidden>Change</button>"#
    } else {
        ""
    };
    cells.push(cell("Next follow-up", &format!("{fu}{fu_edit}")));
    let last = parse_date(field(r, "last_contact"))
        .map(|d| {
            let days = (page.today - d).num_days();
            let ago = match days {
                0 => "today".to_string(),
                1 => "yesterday".to_string(),
                n if n > 0 => format!("{n} days ago"),
                _ => String::new(),
            };
            format!(
                r#"<time datetime="{}">{}</time><span class="tb-sub muted">{ago}</span>"#,
                d.format("%Y-%m-%d"),
                escape_html(&fmt_day(d, page.today))
            )
        })
        .unwrap_or_else(|| none.to_string());
    cells.push(cell("Last contact", &last));
    if let Some(e) = event_of(r) {
        cells.push(cell("Met at", &page.catalog.ref_html(e)));
    }
    let converted = frontmatter::get_str_or(&r.frontmatter, "converted_to", "").trim();
    if !converted.is_empty() {
        cells.push(cell("Contact", &page.catalog.ref_html(converted)));
    }
    format!(r#"<dl class="title-block lead-tb">{}</dl>"#, cells.concat())
}

/// Hidden edit panel with every lead field.
fn edit_panel(page: &Page, r: &EntityRecord) -> String {
    let text = |label: &str, name: &str, kind: &str, max: u32| {
        labelled(
            label,
            &input(
                name,
                kind,
                field(r, name),
                &format!(r#" maxlength="{max}""#),
            ),
            "",
        )
    };
    let sources: Vec<(String, String)> = LEAD_SOURCES
        .iter()
        .map(|s| (s.to_string(), capitalize(s)))
        .collect();
    let event = event_of(r).unwrap_or("");
    let tags = frontmatter::get_string_list(&r.frontmatter, "tags").join(", ");
    format!(
        r#"<form class="edit-panel lead-edit" id="lead-edit" data-lead-form="edit" data-endpoint="/leads/{eid}" data-method="PATCH" data-diff hidden>
  <h2 class="section-title">Edit lead</h2>
  <div class="form-grid form-grid-edit">{name}{company}{role}{email}{phone}{linkedin}{website}{owner}
  {source}{event}{last}<div class="form-field"><span class="form-label">Warmth</span>{warmth}</div>{tags}</div>
  <p class="form-error" role="alert" hidden></p>
  <div class="form-actions"><button type="button" class="btn btn-ghost" data-toggle="lead-edit">Cancel</button><button type="submit" class="btn btn-primary">Save changes {k}</button></div>
</form>"#,
        eid = url_encode(&r.id),
        name = text("Name", "name", "text", 120),
        company = text("Company", "company", "text", 120),
        role = text("Role", "role", "text", 120),
        email = text("Email", "email", "email", 160),
        phone = text("Phone", "phone", "tel", 60),
        linkedin = text("LinkedIn", "linkedin", "url", 300),
        website = text("Website", "website", "url", 300),
        owner = labelled(
            "Owner",
            &input(
                "owner",
                "text",
                field(r, "owner"),
                r#" maxlength="80" list="mc-owners" autocomplete="off""#
            ),
            ""
        ),
        source = labelled(
            "Source",
            &format!(
                r#"<select name="source">{}</select>"#,
                options(
                    &sources,
                    &field(r, "source").to_ascii_lowercase(),
                    Some("Unknown")
                )
            ),
            ""
        ),
        event = labelled(
            "Event",
            &format!(
                r#"<select name="event">{}</select>"#,
                options(&event_options(page), event, Some("No event"))
            ),
            ""
        ),
        last = labelled(
            "Last contact",
            &input(
                "last_contact",
                "date",
                field(r, "last_contact"),
                &format!(r#" max="{}""#, page.today.format("%Y-%m-%d"))
            ),
            ""
        ),
        warmth = radio_chips(
            "warmth",
            &LEAD_WARMTH,
            &field(r, "warmth").to_ascii_lowercase(),
            Some("Not sure")
        ),
        tags = labelled(
            "Tags",
            &input(
                "tags",
                "text",
                &tags,
                r#" maxlength="300" placeholder="robotics, pilot" data-list"#
            ),
            ""
        ),
        k = kbd("⌘↵"),
    )
}

fn follow_up_panel(page: &Page, r: &EntityRecord) -> String {
    let current = parse_date(field(r, "next_follow_up"))
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    format!(
        r#"<form class="edit-panel lead-followup" id="lead-followup" data-lead-form="follow-up" data-endpoint="/leads/{eid}" data-method="PATCH" data-diff hidden>
  <h2 class="section-title">Next follow-up</h2>
  <div class="composer-row"><label class="form-field composer-date"><span class="form-label">Date</span><input type="date" name="next_follow_up" id="fu-date" value="{current}"></label>{quick}</div>
  <p class="form-error" role="alert" hidden></p>
  <div class="form-actions"><button type="button" class="btn btn-ghost" data-clear-date="fu-date">No follow-up</button><button type="button" class="btn btn-ghost" data-toggle="lead-followup">Cancel</button><button type="submit" class="btn btn-primary">Save follow-up</button></div>
</form>"#,
        eid = url_encode(&r.id),
        quick = quick_dates(page, "fu-date"),
    )
}

fn customers_sorted<'a>(page: &'a Page) -> Vec<&'a EntityRecord> {
    let mut customers: Vec<&EntityRecord> = page.catalog.of_kind(EntityKind::Customer).collect();
    customers.sort_by_key(|c| display_name(c).to_lowercase());
    customers
}

/// The customer whose name matches the lead's company, as convert would pick it.
fn company_customer<'a>(page: &'a Page, r: &EntityRecord) -> Option<&'a EntityRecord> {
    let key = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let company = key(field(r, "company"));
    if company.is_empty() {
        return None;
    }
    let mut hits = page
        .catalog
        .of_kind(EntityKind::Customer)
        .filter(|c| key(display_name(c)) == company);
    let first = hits.next()?;
    hits.next().is_none().then_some(first)
}

fn convert_panel(page: &Page, r: &EntityRecord) -> String {
    let customers = customers_sorted(page);
    let matched = company_customer(page, r);
    let opts: Vec<(String, String)> = customers
        .iter()
        .map(|c| (c.id.clone(), display_name(c).to_string()))
        .collect();
    let selected = matched.map(|c| c.id.as_str()).unwrap_or("");
    let new_default = if matched.is_none() {
        field(r, "company")
    } else {
        ""
    };
    let existing_checked = !customers.is_empty() && (matched.is_some() || new_default.is_empty());
    format!(
        r#"<form class="edit-panel lead-convert" id="lead-convert" data-lead-form="convert" data-endpoint="/leads/{eid}/convert" data-method="POST" hidden>
  <h2 class="section-title">Convert to contact</h2>
  <p class="muted">Creates a contact under a customer with {name}'s role, email, phone and tags. The lead stays as a record, marked converted.</p>
  <div class="convert-choice">
    <label class="convert-opt"><input type="radio" name="mode" value="existing"{ex_checked}{ex_disabled}><span><strong>Existing customer</strong><select name="customer"{ex_disabled}>{opts}</select></span></label>
    <label class="convert-opt"><input type="radio" name="mode" value="new"{new_checked}><span><strong>New customer</strong><input type="text" name="new_customer_name" maxlength="120" value="{new_default}" placeholder="Company name"><small class="muted">Created as a prospect.</small></span></label>
  </div>
  <label class="form-check"><input type="checkbox" name="link_existing" value="1"> If {name} is already a contact there, link the lead to that contact</label>
  <p class="form-error" role="alert" hidden></p>
  <div class="form-actions"><button type="button" class="btn btn-ghost" data-toggle="lead-convert">Cancel</button><button type="submit" class="btn btn-primary">Convert to contact</button></div>
</form>"#,
        eid = url_encode(&r.id),
        name = escape_html(display_name(r)),
        ex_checked = if existing_checked { " checked" } else { "" },
        ex_disabled = if customers.is_empty() {
            " disabled"
        } else {
            ""
        },
        new_checked = if existing_checked { "" } else { " checked" },
        opts = options(
            &opts,
            selected,
            if selected.is_empty() {
                Some("Choose a customer")
            } else {
                None
            }
        ),
        new_default = escape_html(new_default),
    )
}

/// The rail's "Introduce to" box: chips, plus a checkbox editor when editable.
fn intro_box(page: &Page, r: &EntityRecord) -> String {
    let ids = intros_of(r);
    let chips: String = ids
        .iter()
        .map(|id| {
            let name = page.catalog.name(id).unwrap_or(id);
            format!(
                r#"<a class="intro-chip" href="{}"><span class="intro-av" aria-hidden="true">{}</span>{}</a>"#,
                entity_href(id),
                escape_html(&initials(name)),
                escape_html(name)
            )
        })
        .collect();
    let shown = if chips.is_empty() {
        r#"<p class="muted intro-none">Nobody yet. Pick the partners this person should meet.</p>"#
            .to_string()
    } else {
        format!(r#"<div class="intro-chips">{chips}</div>"#)
    };
    let editor = if page.editable && page.cfg.entity_available(&EntityKind::Customer) {
        let boxes: String = customers_sorted(page)
            .iter()
            .map(|c| {
                format!(
                    r#"<label class="chip intro-pick"><input type="checkbox" name="introduce_to" value="{}"{}>{}</label>"#,
                    escape_html(&c.id),
                    if ids.contains(&c.id) { " checked" } else { "" },
                    escape_html(display_name(c))
                )
            })
            .collect();
        let boxes = if boxes.is_empty() {
            r#"<p class="muted">No customers yet.</p>"#.to_string()
        } else {
            boxes
        };
        format!(
            r#"<form class="intro-editor" id="lead-intro" data-lead-form="intro" data-endpoint="/leads/{}" data-method="PATCH" hidden><div class="intro-picks">{boxes}</div><p class="form-error" role="alert" hidden></p><div class="form-actions"><button type="button" class="btn btn-ghost btn-sm" data-toggle="lead-intro">Cancel</button><button type="submit" class="btn btn-primary btn-sm">Save</button></div></form>"#,
            url_encode(&r.id)
        )
    } else {
        String::new()
    };
    let edit = if editor.is_empty() {
        String::new()
    } else {
        r#"<button type="button" class="tb-edit" data-toggle="lead-intro" aria-controls="lead-intro" aria-expanded="false" hidden>Edit</button>"#.to_string()
    };
    format!(
        r#"<section class="rail-box intro-box"><div class="rail-box-head"><h2 class="rail-title">Introduce to</h2>{edit}</div>{shown}{editor}</section>"#
    )
}

/// Files next to the lead's own file.
fn lead_dir(r: &EntityRecord) -> Option<&std::path::Path> {
    r.source_path.parent()
}

fn card_image(page: &Page, r: &EntityRecord) -> String {
    let Some(dir) = lead_dir(r) else {
        return String::new();
    };
    let Some(path) = CARD_FILES.iter().map(|f| dir.join(f)).find(|p| p.is_file()) else {
        return String::new();
    };
    let Some(href) = path.strip_prefix(&page.cfg.root).ok().and_then(files_href) else {
        return String::new();
    };
    format!(
        r#"<figure class="biz-card"><a href="{href}" target="_blank" rel="noopener"><img src="{href}" alt="Business card of {}" loading="lazy"></a><figcaption>Business card</figcaption></figure>"#,
        escape_html(display_name(r))
    )
}

fn research_panel(page: &Page, r: &EntityRecord) -> String {
    let file = lead_dir(r).map(|d| d.join("research.md"));
    let content = file
        .as_ref()
        .filter(|p| p.is_file())
        .and_then(|p| std::fs::read_to_string(p).ok());
    let body = match (&content, &file) {
        (Some(md), Some(path)) if has_content(md) => {
            let md = frontmatter::split_frontmatter(md)
                .map(|(_, body)| body)
                .unwrap_or_else(|| md.clone());
            let dir = path.parent().unwrap_or(&page.cfg.root);
            let doc = DocContext {
                root: &page.cfg.root,
                dir,
            };
            let rel = path
                .strip_prefix(&page.cfg.root)
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            format!(
                r#"<div class="research-panel"><div class="prose research-body">{}</div><p class="research-source"><code>{}</code></p></div>"#,
                render_markdown_in(strip_leading_h1(md.trim()), page.catalog, Some(&doc)),
                escape_html(&rel)
            )
        }
        _ => empty_state(
            "No research yet",
            &format!(
                r#"Run <code>/enrich-lead {}</code> in Claude Code. It writes <code>research.md</code> next to this lead and it shows up here."#,
                escape_html(&r.id)
            ),
            "",
        ),
    };
    format!(
        r#"<section class="related-section research" id="research">{}{body}</section>"#,
        section_title("Research", None)
    )
}

/// The lead's detail page.
pub fn lead_detail_page(page: &Page, r: &EntityRecord) -> String {
    let name = display_name(r);
    let id = escape_html(&r.id);
    let mut body = format!(
        r#"<article class="detail lead-detail" data-entity-id="{id}" data-entity-kind="lead" data-entity-title="{}">"#,
        escape_html(name)
    );
    body.push_str(&format!(
        r#"<nav class="breadcrumb" aria-label="Breadcrumb"><a href="/leads">Leads</a><span class="sep" aria-hidden="true">/</span><span class="crumb-id">{id}</span>{}</nav>"#,
        crate::html::components::icon_button(
            "copy",
            "Copy ID",
            "copy-id",
            &format!(r#" data-copy="{id}" hidden"#)
        ),
    ));
    let converted = frontmatter::get_link_str(&r.frontmatter, "converted_to")
        .is_some_and(|s| !s.trim().is_empty());
    let actions = if page.editable {
        let convert = if converted {
            String::new()
        } else {
            format!(
                r#"<button type="button" class="btn btn-sm" data-toggle="lead-convert" aria-controls="lead-convert" aria-expanded="false" hidden>{}Convert</button>"#,
                icon("check")
            )
        };
        format!(
            r#"<div class="detail-actions"><button type="button" class="btn btn-primary btn-sm" data-toggle="lead-log" aria-controls="lead-log" aria-expanded="false" hidden>{}Log interaction</button><button type="button" class="btn btn-sm" data-toggle="lead-edit" aria-controls="lead-edit" aria-expanded="false" hidden>{}Edit {}</button>{convert}</div>"#,
            icon("plus"),
            icon("edit"),
            kbd("e"),
        )
    } else {
        String::new()
    };
    let org = org_line(r);
    let org = if org.is_empty() {
        String::new()
    } else {
        format!(r#"<p class="lead-org">{org}</p>"#)
    };
    body.push_str(&format!(
        r#"<header class="detail-hero lead-hero"><div class="lead-ident"><span class="lead-avatar" aria-hidden="true">{}</span><div><h1 class="detail-title">{}</h1>{org}{}</div></div>{actions}</header>"#,
        escape_html(&initials(name)),
        escape_html(name),
        contact_links(r),
    ));
    body.push_str(&stage_route(page, r));
    body.push_str(&lead_title_block(page, r));
    if page.editable {
        body.push_str(&follow_up_panel(page, r));
        body.push_str(&edit_panel(page, r));
        if !converted {
            body.push_str(&convert_panel(page, r));
        }
    }

    // The body: Summary first, then the interaction log, then the rest.
    let (summary, rest) = take_section(&r.body, "Summary");
    let (log, rest) = take_section(&rest, INTERACTIONS_SECTION);
    let entries: Vec<Interaction> = log
        .as_deref()
        .unwrap_or("")
        .lines()
        .filter_map(parse_interaction)
        .collect();

    body.push_str(r#"<div class="detail-layout"><div class="detail-main">"#);
    if let Some(summary) = summary.filter(|s| has_content(s)) {
        body.push_str(&format!(
            r#"<div class="lead-summary prose">{}</div>"#,
            render_markdown(summary.trim(), page.catalog)
        ));
    }
    body.push_str(&format!(
        r#"<section class="related-section interactions" id="interactions">{}{}{}</section>"#,
        section_title("Interactions", Some(entries.len())),
        if page.editable {
            composer(page, &r.id)
        } else {
            String::new()
        },
        timeline(page, &entries)
    ));
    let notes = Notes {
        md: drop_empty_sections(&rest),
        checks: Vec::new(),
        comments: Vec::new(),
    };
    if has_content(strip_leading_h1(notes.md.trim())) {
        if let Some(html) = notes::body_html(page, r, &notes) {
            body.push_str(&format!(r#"<div class="lead-notes">{html}</div>"#));
        }
    }
    body.push_str(&research_panel(page, r));
    body.push_str("</div>");

    body.push_str(r#"<div class="lead-rail">"#);
    body.push_str(&card_image(page, r));
    body.push_str(&intro_box(page, r));
    let shown = [
        "company",
        "role",
        "email",
        "phone",
        "linkedin",
        "website",
        "event",
        "introduce_to",
        "warmth",
        "next_follow_up",
        "last_contact",
        "converted_to",
    ];
    body.push_str(&rail(r, page, &shown));
    body.push_str("</div></div></article>");
    layout(page, &format!("{name} ({})", r.id), "/leads", "", &body)
}

// ── Events ──────────────────────────────────────────────────────────────

/// Lead counts per stage for a set of leads, in pipeline order.
fn stage_counts(page: &Page, recs: &[&EntityRecord]) -> Vec<(String, usize)> {
    stages(page, recs.iter().copied())
        .into_iter()
        .map(|s| {
            let n = recs.iter().filter(|r| status_of(r) == s).count();
            (s, n)
        })
        .collect()
}

/// A segmented bar of lead stages (only stages with leads).
fn stage_bar(counts: &[(String, usize)]) -> String {
    let total: usize = counts.iter().map(|(_, n)| n).sum();
    if total == 0 {
        return r#"<div class="status-bar status-bar-empty"></div>"#.to_string();
    }
    let segs: String = counts
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(s, n)| {
            format!(
                r#"<div class="seg tone-{} seg-{}" style="flex:{n}" title="{}: {n}"></div>"#,
                crate::html::status_tone(s),
                status_slug(s),
                escape_html(&status_label(s))
            )
        })
        .collect();
    format!(r#"<div class="status-bar stage-bar">{segs}</div>"#)
}

fn leads_of_event<'a>(page: &'a Page, event: &str) -> Vec<&'a EntityRecord> {
    leads(page).filter(|r| event_of(r) == Some(event)).collect()
}

fn event_dates(page: &Page, e: &EntityRecord) -> String {
    let Some(start) = parse_date(field(e, "date")) else {
        return String::new();
    };
    let end = parse_date(field(e, "end_date")).filter(|d| *d > start);
    let text = match end {
        Some(end) => format!(
            "{} – {}",
            fmt_date(start, page.today),
            fmt_date(end, page.today)
        ),
        None => fmt_day(start, page.today),
    };
    format!(
        r#"<time datetime="{}">{}</time>"#,
        start.format("%Y-%m-%d"),
        escape_html(&text)
    )
}

fn event_row(page: &Page, e: &EntityRecord) -> String {
    let event_leads = leads_of_event(page, &e.id);
    let n = event_leads.len();
    let converted = event_leads
        .iter()
        .filter(|r| status_of(r) == "converted")
        .count();
    let date = parse_date(field(e, "date"));
    let tile = date
        .map(|d| {
            format!(
                r#"<span class="evt-tile" aria-hidden="true"><span class="evt-tile-m">{}</span><span class="evt-tile-d">{}</span></span>"#,
                d.format("%b"),
                d.format("%-d")
            )
        })
        .unwrap_or_else(|| r#"<span class="evt-tile" aria-hidden="true"></span>"#.to_string());
    let mut sub = Vec::new();
    let dates = event_dates(page, e);
    if !dates.is_empty() {
        sub.push(dates);
    }
    for key in ["location", "type"] {
        let v = field(e, key);
        if !v.is_empty() {
            sub.push(escape_html(&capitalize(&v.replace('-', " "))));
        }
    }
    let stats = if n == 0 {
        r#"<span class="muted">No leads yet</span>"#.to_string()
    } else {
        format!(
            r#"<a class="evt-leads" href="{}"><span class="readout">{n}</span> {}{}</a>{}"#,
            href_with("/leads", &[("event", &e.id)]),
            if n == 1 { "lead" } else { "leads" },
            if converted > 0 {
                format!(r#", <span class="readout">{converted}</span> converted"#)
            } else {
                String::new()
            },
            stage_bar(&stage_counts(page, &event_leads))
        )
    };
    format!(
        r#"<li class="evt-row" data-row data-id="{id}">{tile}<div class="evt-main"><a class="name-link" href="{href}">{title}</a><div class="evt-sub">{sub}</div></div><div class="evt-status">{status}</div><div class="evt-stats">{stats}</div></li>"#,
        id = escape_html(&e.id),
        href = entity_href(&e.id),
        title = escape_html(display_name(e)),
        sub = sub.join(r#"<span class="sep" aria-hidden="true"></span>"#),
        status = status_badge(status_of(e)),
    )
}

/// `/events`: upcoming events first (soonest first), then past ones.
pub fn events_page(page: &Page) -> String {
    let today = page.today;
    let mut events: Vec<&EntityRecord> = page.catalog.of_kind(EntityKind::Event).collect();
    let actions = if page.editable {
        format!(
            r#"<a class="btn btn-primary btn-sm" href="/events/new">{}New event</a>"#,
            icon("plus")
        )
    } else {
        String::new()
    };
    let mut body = page_header("Events", &plural(events.len(), "event", "events"), &actions);
    if events.is_empty() {
        let hint = if page.editable {
            r#"Add the trade fairs and meetups you go to with <a href="/events/new">New event</a>, then attach the people you meet there."#
        } else {
            r#"Create one with <code>mc new event "Hannover Messe" --date 2026-04-20</code>."#
        };
        body.push_str(&empty_state("No events yet", hint, ""));
        return layout(page, "Events", "/events", "", &body);
    }
    let end = |e: &EntityRecord| {
        let start = parse_date(field(e, "date"));
        parse_date(field(e, "end_date")).or(start)
    };
    events.sort_by_key(|e| field(e, "date").to_string());
    let (upcoming, mut past): (Vec<&EntityRecord>, Vec<&EntityRecord>) = events
        .into_iter()
        .partition(|e| end(e).is_none_or(|d| d >= today));
    past.reverse();
    body.push_str(r#"<div id="rows">"#);
    for (title, list, empty) in [
        (
            "Upcoming",
            &upcoming,
            "Nothing planned. Add the next trade fair you're going to.",
        ),
        ("Past", &past, ""),
    ] {
        if list.is_empty() && empty.is_empty() {
            continue;
        }
        body.push_str(&format!(
            r#"<section class="related-section evt-section">{}"#,
            section_title(title, Some(list.len()))
        ));
        if list.is_empty() {
            body.push_str(&format!(r#"<p class="muted">{empty}</p>"#));
        } else {
            body.push_str(r#"<ul class="evt-list">"#);
            for e in list.iter() {
                body.push_str(&event_row(page, e));
            }
            body.push_str("</ul>");
        }
        body.push_str("</section>");
    }
    body.push_str("</div>");
    layout(page, "Events", "/events", "", &body)
}

/// `/events/new`.
pub fn new_event_page(page: &Page) -> String {
    let mut body = String::from(r#"<div class="capture">"#);
    body.push_str(r#"<nav class="breadcrumb" aria-label="Breadcrumb"><a href="/events">Events</a><span class="sep" aria-hidden="true">/</span><span class="crumb-id">New</span></nav><h1 class="page-title capture-title">New event</h1>"#);
    if !page.editable {
        body.push_str(&empty_state(
            "This dashboard is read-only",
            r#"Run <code>mc new event "Title" --date YYYY-MM-DD</code> instead."#,
            "",
        ));
        body.push_str("</div>");
        return layout(page, "New event", "/events", "", &body);
    }
    let types: Vec<(String, String)> = EVENT_TYPES
        .iter()
        .map(|t| (t.to_string(), capitalize(&t.replace('-', " "))))
        .collect();
    let statuses: Vec<(String, String)> = EntityKind::Event
        .statuses(page.cfg)
        .iter()
        .map(|s| (s.clone(), status_label(s)))
        .collect();
    let first = statuses.first().map(|s| s.0.clone()).unwrap_or_default();
    body.push_str(&format!(
        r#"<form class="form capture-form" data-lead-form="create-event" data-endpoint="/events" data-method="POST" autocomplete="off">
  {title}
  <div class="form-grid capture-grid">{date}{end}{location}{kind}{status}{url}</div>
  <p class="form-error" role="alert" hidden></p>
  <div class="form-actions capture-actions"><a class="btn btn-ghost" href="/events">Cancel</a><button type="submit" class="btn btn-primary btn-lg">Create event {k}</button></div>
</form>"#,
        title = labelled(
            "Title",
            r#"<input type="text" name="title" required maxlength="160" placeholder="Hannover Messe 2027" autofocus>"#,
            " form-field-wide"
        ),
        date = labelled(
            "First day",
            &input("date", "date", &page.today.format("%Y-%m-%d").to_string(), " required"),
            ""
        ),
        end = labelled("Last day", &input("end_date", "date", "", ""), ""),
        location = labelled(
            "Location",
            &input("location", "text", "", r#" maxlength="120" placeholder="Hannover""#),
            ""
        ),
        kind = labelled(
            "Type",
            &format!(r#"<select name="type">{}</select>"#, options(&types, "trade-fair", None)),
            ""
        ),
        status = labelled(
            "Status",
            &format!(r#"<select name="status">{}</select>"#, options(&statuses, &first, None)),
            ""
        ),
        url = labelled(
            "Website",
            &input("url", "url", "", r#" maxlength="300" placeholder="https://""#),
            ""
        ),
        k = kbd("⌘↵"),
    ));
    body.push_str("</div>");
    layout(page, "New event", "/events", "", &body)
}

fn stat(label: &str, value: &str, sub: &str) -> String {
    let sub = if sub.is_empty() {
        String::new()
    } else {
        format!(r#"<span class="stat-sub">{sub}</span>"#)
    };
    format!(
        r#"<div class="stat"><span class="stat-label">{label}</span><span class="stat-value readout">{value}</span>{sub}</div>"#
    )
}

/// The lead summary at the top of an event's page: counts, conversion, a
/// stage breakdown and the leads by stage.
pub(crate) fn event_summary(page: &Page, e: &EntityRecord) -> String {
    if !page.cfg.entity_available(&EntityKind::Lead) {
        return String::new();
    }
    let event_leads = leads_of_event(page, &e.id);
    let n = event_leads.len();
    let mut actions = link_button(
        &href_with("/leads", &[("event", &e.id)]),
        "Open on the board",
        Btn::Secondary,
        "btn-sm",
    );
    if page.editable {
        actions.push_str(&format!(
            r#"<a class="btn btn-primary btn-sm" href="{}">{}Add lead</a>"#,
            href_with("/leads/new", &[("event", &e.id)]),
            icon("plus")
        ));
    }
    let mut html = format!(
        r#"<section class="related-section event-leads"><div class="section-head">{}<div class="section-actions">{actions}</div></div>"#,
        section_title("People met", Some(n))
    );
    if n == 0 {
        html.push_str(&empty_state(
            "Nobody captured yet",
            "Leads whose event is this event show up here.",
            "",
        ));
        html.push_str("</section>");
        return html;
    }
    let count =
        |pred: &dyn Fn(&str) -> bool| event_leads.iter().filter(|r| pred(status_of(r))).count();
    let converted = count(&|s| s == "converted");
    let lost = count(&|s| s == "lost");
    let intros = count(&|s| INTRO_STATUSES.contains(&s));
    let due = event_leads
        .iter()
        .filter(|r| {
            follow_up(r, page.today)
                .is_some_and(|(_, s)| matches!(s, FollowUp::Overdue | FollowUp::Today))
        })
        .count();
    let rate = converted * 100 / n;
    html.push_str(&format!(
        r#"<div class="stats">{}{}{}{}</div>"#,
        stat(
            "Leads",
            &n.to_string(),
            &format!("{} open", n - converted - lost)
        ),
        stat(
            "Converted",
            &converted.to_string(),
            &format!("{rate}% of leads")
        ),
        stat("Intros", &intros.to_string(), "planned or made"),
        stat(
            "Follow-ups due",
            &due.to_string(),
            if due > 0 { "today or overdue" } else { "" }
        ),
    ));
    let counts = stage_counts(page, &event_leads);
    html.push_str(&stage_bar(&counts));
    let legend: String = counts
        .iter()
        .filter(|(_, c)| *c > 0)
        .map(|(s, c)| {
            format!(
                r#"<a class="legend-item" href="{}">{}{} {c}</a>"#,
                href_with("/leads/list", &[("event", &e.id), ("status", s)]),
                status_lamp(s),
                escape_html(&status_label(s))
            )
        })
        .collect();
    html.push_str(&format!(r#"<div class="board-legend">{legend}</div>"#));
    let mut sorted = event_leads.clone();
    let order = EntityKind::Lead.statuses(page.cfg);
    sorted.sort_by_cached_key(|r| {
        (
            order.iter().position(|o| o == status_of(r)).unwrap_or(99),
            lane_key(r),
        )
    });
    html.push_str(r#"<ul class="related-list lead-rows">"#);
    for r in sorted {
        let fu = follow_up(r, page.today)
            .map(|(d, s)| follow_up_badge(d, s, page.today))
            .unwrap_or_default();
        html.push_str(&format!(
            r#"<li><span>{}</span><span class="related-name"><a class="name-link" href="{}">{}</a><span class="muted">{}</span></span><span class="related-meta">{}{fu}</span></li>"#,
            status_badge(status_of(r)),
            entity_href(&r.id),
            escape_html(display_name(r)),
            org_line(r),
            warmth_chip(field(r, "warmth")),
        ));
    }
    html.push_str("</ul></section>");
    html
}

// ── Overview panels ─────────────────────────────────────────────────────

/// Follow-ups that are overdue, due today or due in the next seven days.
pub(crate) fn follow_ups_panel(page: &Page) -> String {
    if !page.cfg.entity_available(&EntityKind::Lead) {
        return String::new();
    }
    let mut due: Vec<(NaiveDate, FollowUp, &EntityRecord)> = leads(page)
        .filter_map(|r| follow_up(r, page.today).map(|(d, s)| (d, s, r)))
        .filter(|(_, s, _)| *s != FollowUp::Later)
        .collect();
    due.sort_by_key(|(d, _, r)| (*d, warmth_rank(field(r, "warmth")), r.id.clone()));
    let overdue = due
        .iter()
        .filter(|(_, s, _)| *s == FollowUp::Overdue)
        .count();
    let today = due.iter().filter(|(_, s, _)| *s == FollowUp::Today).count();
    let mut body = String::new();
    if leads(page).next().is_none() {
        body.push_str(&empty_state(
            "No leads yet",
            if page.editable {
                r#"Met someone? <a href="/leads/new">Capture them</a> while you remember."#
            } else {
                r#"Add one with <code>mc new lead "Name"</code>."#
            },
            "",
        ));
    } else if due.is_empty() {
        body.push_str(&empty_state(
            "No follow-ups this week",
            "Leads with a follow-up date in the next 7 days show up here.",
            "",
        ));
    } else {
        body.push_str(r#"<ul class="item-list">"#);
        for (d, s, r) in due.iter().take(FOLLOW_UP_ROWS) {
            let org = org_line(r);
            let org = if org.is_empty() {
                String::new()
            } else {
                format!(r#"<span class="item-context">{org}</span>"#)
            };
            body.push_str(&format!(
                r#"<li class="item"><span class="item-lead">{}</span><div class="item-main"><a class="name-link" href="{}">{}</a><div class="item-meta">{}{org}</div></div></li>"#,
                follow_up_badge(*d, *s, page.today),
                entity_href(&r.id),
                escape_html(display_name(r)),
                status_badge(status_of(r)),
            ));
        }
        body.push_str("</ul>");
        if due.len() > FOLLOW_UP_ROWS {
            body.push_str(&format!(
                r#"<p class="panel-more"><a href="/leads/list?sort=next_follow_up&amp;dir=asc">{} more on the lead table</a></p>"#,
                due.len() - FOLLOW_UP_ROWS
            ));
        }
    }
    let signal = if overdue > 0 {
        Some("negative")
    } else if today > 0 {
        Some("pending")
    } else {
        None
    };
    let link = format!(
        r#"<a class="panel-link" href="/leads?due=1">{overdue} overdue, {today} today</a>"#
    );
    panel("Follow-ups", &link, &body, signal)
}

/// Lead counts per stage plus introductions in progress.
pub(crate) fn pipeline_panel(page: &Page) -> String {
    if !page.cfg.entity_available(&EntityKind::Lead) {
        return String::new();
    }
    let all: Vec<&EntityRecord> = leads(page).collect();
    if all.is_empty() {
        return String::new();
    }
    let counts = stage_counts(page, &all);
    let max = counts.iter().map(|(_, n)| *n).max().unwrap_or(1).max(1);
    let rows: String = counts
        .iter()
        .map(|(s, n)| {
            format!(
                r#"<li class="funnel-row{}"><a href="{}"><span class="funnel-label">{}{}</span><span class="funnel-track"><span class="funnel-fill tone-{}" style="width:{}%"></span></span><span class="funnel-n readout">{n}</span></a></li>"#,
                if *n == 0 { " is-empty" } else { "" },
                href_with("/leads/list", &[("status", s)]),
                status_lamp(s),
                escape_html(&status_label(s)),
                crate::html::status_tone(s),
                n * 100 / max,
            )
        })
        .collect();
    let intros: Vec<&&EntityRecord> = all
        .iter()
        .filter(|r| INTRO_STATUSES.contains(&status_of(r)))
        .collect();
    let mut body = format!(r#"<ul class="funnel">{rows}</ul>"#);
    if !intros.is_empty() {
        let names: Vec<String> = intros
            .iter()
            .take(4)
            .map(|r| {
                let to: Vec<String> = intros_of(r)
                    .iter()
                    .map(|c| escape_html(page.catalog.name(c).unwrap_or(c)))
                    .collect();
                let to = if to.is_empty() {
                    String::new()
                } else {
                    format!(r#" <span class="muted">to {}</span>"#, to.join(", "))
                };
                format!(
                    r#"<a href="{}">{}</a>{to}"#,
                    entity_href(&r.id),
                    escape_html(display_name(r))
                )
            })
            .collect();
        body.push_str(&format!(
            r#"<p class="panel-more intros-line">{} in progress: {}</p>"#,
            plural(intros.len(), "intro", "intros"),
            names.join(", ")
        ));
    }
    let open = all.iter().filter(|r| !is_closed_lead(r)).count();
    let link = format!(r#"<a class="panel-link" href="/leads">{open} open</a>"#);
    let add = if page.editable {
        format!(
            r#"<span class="panel-actions">{link}<a class="icon-btn" href="/leads/new" aria-label="New lead" title="New lead">{}</a></span>"#,
            icon("plus")
        )
    } else {
        link
    };
    panel("Pipeline", &add, &body, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::catalog::tests::{catalog, rec, test_config};

    fn day(n: i64) -> String {
        (chrono::Local::now().date_naive() + chrono::Duration::days(n))
            .format("%Y-%m-%d")
            .to_string()
    }

    fn sample() -> Vec<EntityRecord> {
        vec![
            rec(
                EntityKind::Event,
                "EVT-001",
                &format!("title: Fair\nstatus: attended\ndate: {}", day(-3)),
            ),
            rec(EntityKind::Customer, "CUST-001", "name: Zema\nstatus: active"),
            rec(
                EntityKind::Lead,
                "LEAD-001",
                &format!(
                    "name: \"Ann <b>\"\ncompany: Acme\nrole: CTO\nstatus: new\nwarmth: hot\nevent: \"[[EVT-001|Fair]]\"\nintroduce_to: [\"[[CUST-001|Zema]]\"]\nnext_follow_up: {}",
                    day(-2)
                ),
            ),
            rec(
                EntityKind::Lead,
                "LEAD-002",
                &format!("name: Bob\nstatus: qualified\nwarmth: cold\nnext_follow_up: {}", day(0)),
            ),
            rec(
                EntityKind::Lead,
                "LEAD-003",
                "name: Cy\nstatus: converted\nconverted_to: \"[[CONT-001|Cy]]\"",
            ),
        ]
    }

    #[test]
    fn board_has_lanes_cards_and_hooks() {
        let (_d, cfg) = test_config();
        let cat = catalog(sample(), &cfg);
        let page = Page::new(&cfg, &cat, "").with_editable(true);
        let html = leads_board_page(&page, &LeadQuery::default());
        assert!(html.contains(r#"data-api="/leads""#));
        assert!(html.contains(r#"class="kanban-column lead-lane col-new" data-status="new""#));
        assert!(html.contains(r#"data-id="LEAD-001" data-status="new" data-pri="1""#));
        assert!(html.contains("lead-card is-overdue warmth-hot"));
        assert!(html.contains("lead-card is-soon warmth-cold"));
        assert!(html.contains("Ann &lt;b&gt;"));
        assert!(html.contains("2d overdue"));
        assert!(html.contains(r#"<span class="intro-av" aria-hidden="true">Z</span>"#));
        assert!(html.contains("evt-chip"));
        assert!(html.contains(r#"<span class="kanban-late">1 overdue</span>"#));
        assert!(html.contains("lead-card is-compact"));
        assert!(html.contains(r#"<template id="move-menu-tpl">"#));
        assert!(html.contains(r#"data-status="intro-planned""#));
    }

    #[test]
    fn filters_narrow_the_board() {
        let (_d, cfg) = test_config();
        let cat = catalog(sample(), &cfg);
        let page = Page::new(&cfg, &cat, "");
        let q = LeadQuery {
            due: true,
            ..Default::default()
        };
        let html = leads_board_page(&page, &q);
        assert!(html.contains(r#"data-id="LEAD-001""#));
        assert!(html.contains(r#"data-id="LEAD-002""#));
        assert!(!html.contains(r#"data-id="LEAD-003""#));
        let q = LeadQuery {
            warmth: Some("cold"),
            ..Default::default()
        };
        let html = leads_board_page(&page, &q);
        assert!(!html.contains(r#"data-id="LEAD-001""#));
        // Read-only: no move menu.
        assert!(!html.contains("move-menu-tpl"));
    }

    #[test]
    fn table_sorts_with_empty_values_last() {
        let (_d, cfg) = test_config();
        let cat = catalog(sample(), &cfg);
        let page = Page::new(&cfg, &cat, "");
        let q = LeadQuery {
            sort: Some("warmth"),
            dir: "desc",
            ..Default::default()
        };
        let html = leads_list_page(&page, &q);
        let pos = |id: &str| html.find(&format!(r#"data-id="{id}""#)).unwrap();
        assert!(pos("LEAD-002") < pos("LEAD-001"));
        assert!(pos("LEAD-001") < pos("LEAD-003"));
        assert!(html.contains(r#"aria-sort="descending""#));
    }

    #[test]
    fn interactions_parse_and_sections_split() {
        let i = parse_interaction("- 2026-11-20 · call · Talked about [[CUST-001]]").unwrap();
        assert_eq!(i.kind, "call");
        assert_eq!(i.note, "Talked about [[CUST-001]]");
        let i = parse_interaction("- 2026-11-20 Met at the booth").unwrap();
        assert_eq!(i.kind, "note");
        assert_eq!(i.note, "Met at the booth");
        assert!(parse_interaction("- someday · call · x").is_none());
        let body = "# X\n\n## Summary\n\nWho\n\n## Interactions\n\n- 2026-11-20 · call · A\n\n## Notes\n\nN\n";
        let (log, rest) = take_section(body, "Interactions");
        assert_eq!(log.unwrap().trim(), "- 2026-11-20 · call · A");
        assert!(rest.contains("## Summary") && rest.contains("## Notes"));
        assert!(!rest.contains("Interactions"));
        assert!(!has_content("\n<!-- placeholder -->\n"));
        assert!(!has_content("\n%% mc-links: [[EVT-001]] %%\n"));
        let kept = drop_empty_sections("# X\n\n## A\n\n<!-- c -->\n\n## B\n\ntext\n");
        assert!(!kept.contains("## A"));
        assert!(kept.contains("## B\n\ntext"));
    }

    #[test]
    fn detail_shows_route_timeline_and_research_hint() {
        let (_d, cfg) = test_config();
        let mut lead = sample().remove(2);
        lead.body = "# Ann\n\n## Summary\n\n<!-- x -->\n\n## Interactions\n\n- 2026-11-20 · email · Sent deck\n\n## Notes\n".into();
        let cat = catalog(sample(), &cfg);
        let page = Page::new(&cfg, &cat, "").with_editable(true);
        let html = lead_detail_page(&page, &lead);
        assert!(html.contains(r#"class="stage-route""#));
        assert!(html.contains(r#"data-lead-stage="contacted""#));
        assert!(html.contains(r#"data-lead-stage="converted" data-opens="lead-convert""#));
        assert!(html.contains("tl-item tl-email"));
        assert!(html.contains("Sent deck"));
        assert!(html.contains("/enrich-lead LEAD-001"));
        assert!(html.contains(r#"data-lead-form="log""#));
        assert!(html.contains(r#"data-lead-form="convert""#));
        assert!(html.contains("Role at") || html.contains("CTO at Acme"));
        // Customer matched by company is not preselected (no match), so "new" is.
        assert!(html.contains(r#"name="new_customer_name" maxlength="120" value="Acme""#));
    }

    #[test]
    fn default_event_prefers_the_latest_started_one() {
        let (_d, cfg) = test_config();
        let mut recs = sample();
        recs.push(rec(
            EntityKind::Event,
            "EVT-002",
            &format!("title: Later\nstatus: planned\ndate: {}", day(30)),
        ));
        recs.push(rec(
            EntityKind::Event,
            "EVT-003",
            &format!("title: Off\nstatus: cancelled\ndate: {}", day(-1)),
        ));
        let cat = catalog(recs, &cfg);
        let page = Page::new(&cfg, &cat, "").with_editable(true);
        assert_eq!(default_event(&page).unwrap().id, "EVT-001");
        let html = new_lead_page(&page, None);
        assert!(html.contains(r#"<option value="EVT-001" selected>"#));
        assert!(!html.contains("EVT-003"));
    }

    #[test]
    fn overview_panels_list_due_follow_ups_and_stages() {
        let (_d, cfg) = test_config();
        let cat = catalog(sample(), &cfg);
        let page = Page::new(&cfg, &cat, "");
        let fu = follow_ups_panel(&page);
        assert!(fu.contains("1 overdue, 1 today"));
        assert!(fu.contains("LEAD-001"));
        let pipe = pipeline_panel(&page);
        assert!(pipe.contains("funnel-row"));
        assert!(pipe.contains("2 open"));
    }
}
