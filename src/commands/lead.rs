//! The lead pipeline (`mc lead`): field updates, stage moves, the
//! interaction log and conversion into a contact.
//!
//! Like `commands::new` and `commands::task`, these functions are the one
//! write path for leads: the CLI, the MCP tools, the REST API and the
//! dashboard all call them, and each holds the repo write lock across its
//! read-modify-write. See `docs/crm.md` for the data model.

use crate::cli::suggest;
use crate::cli::ui;
use crate::cli::LeadSubcommand;
use crate::commands::new::{self as new_cmd, LEAD_SOURCES, LEAD_WARMTH};
use crate::commands::task::single_line;
use crate::config::ResolvedConfig;
use crate::data::{self, EntityRecord};
use crate::entity::EntityKind;
use crate::error::{McError, McResult};
use crate::frontmatter;
use crate::util;
use colored::*;
use serde_json::Value as JsonValue;
use serde_yaml::Value;
use std::path::PathBuf;

/// Kinds of interaction in a lead's log.
pub const INTERACTION_TYPES: [&str; 7] = [
    "call", "email", "meeting", "event", "linkedin", "intro", "note",
];

/// Pipeline stages that end a lead's lifecycle: they never show up as due
/// for a follow-up.
pub const CLOSED_LEAD_STATUSES: &[&str] = &["converted", "lost"];

/// Whether a lead in `status` is out of the pipeline (converted or lost).
pub fn is_closed_lead_status(status: &str) -> bool {
    CLOSED_LEAD_STATUSES.contains(&status)
}

/// Heading of the interaction log section.
pub const INTERACTIONS_SECTION: &str = "Interactions";

// ---------------------------------------------------------------------------
// Body sections
// ---------------------------------------------------------------------------

/// Whether a line is a heading of level 1 or 2 (`# X`, `## X`).
fn is_top_heading(line: &str) -> bool {
    let t = line.trim_end();
    t.starts_with("# ") || t.starts_with("## ") || t == "#" || t == "##"
}

/// Whether a line is mc's `%% mc-links: ... %%` footer.
fn is_links_footer(line: &str) -> bool {
    line.trim_start().starts_with("%% mc-links:")
}

/// The lines (with their line breaks) of the `## <heading>` section:
/// `(heading line index, end index)`, where the section ends at the next
/// level 1/2 heading, mc's link footer or the end of the body. Headings
/// inside fenced code are ignored; the heading matches case-insensitively.
fn find_section(lines: &[&str], heading: &str) -> Option<(usize, usize)> {
    let wanted = format!("## {heading}").to_lowercase();
    let mut in_code = false;
    let mut start = None;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        match start {
            None if t.to_lowercase() == wanted => start = Some(i),
            Some(s) if is_top_heading(line) || is_links_footer(line) => return Some((s, i)),
            _ => {}
        }
    }
    start.map(|s| (s, lines.len()))
}

fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// `lines` joined, with a line break after the last one.
fn join_lines(lines: &[&str]) -> String {
    let mut out = lines.concat();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// `body` with `text` added at the end of its `## <heading>` section (after
/// a blank line), creating the section at the end of the body when it is
/// missing.
pub(crate) fn append_to_section(body: &str, heading: &str, text: &str) -> String {
    let text = text.trim_end();
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    let Some((start, end)) = find_section(&lines, heading) else {
        let trimmed = body.trim_end();
        let sep = if trimmed.is_empty() { "" } else { "\n\n" };
        return format!("{trimmed}{sep}## {heading}\n\n{text}\n");
    };
    let last = (start + 1..end)
        .rev()
        .find(|&i| !is_blank(lines[i]))
        .unwrap_or(start);
    let mut out = join_lines(&lines[..=last]);
    out.push('\n');
    out.push_str(text);
    out.push('\n');
    let rest = &lines[last + 1..];
    if rest.first().is_some_and(|l| !is_blank(l)) {
        out.push('\n');
    }
    out.push_str(&rest.concat());
    out
}

/// The date at the start of an interaction line (`- 2026-11-26 · call · …`).
fn entry_date(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("- ")?;
    let date = rest.get(..10)?;
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .map(|_| date)
}

/// `body` with the interaction `line` in the `## Interactions` section,
/// newest first: before the first entry dated `date` or earlier (so the
/// latest entry of a day comes first), else after the last entry. The
/// section is created before `## Notes` (or at the end) when missing.
pub(crate) fn insert_interaction(body: &str, date: &str, line: &str) -> String {
    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    let Some((start, end)) = find_section(&lines, INTERACTIONS_SECTION) else {
        let section = format!("## {INTERACTIONS_SECTION}\n\n{line}\n\n");
        return match find_section(&lines, "Notes") {
            Some((notes, _)) => {
                let mut out = join_lines(&lines[..notes]);
                if !out.is_empty() && !out.ends_with("\n\n") {
                    out.push('\n');
                }
                out.push_str(&section);
                out.push_str(&lines[notes..].concat());
                out
            }
            None => append_to_section(body, INTERACTIONS_SECTION, line),
        };
    };
    let entries: Vec<usize> = (start + 1..end)
        .filter(|&i| lines[i].starts_with("- "))
        .collect();
    let at = match entries
        .iter()
        .find(|&&i| entry_date(lines[i]).is_none_or(|d| d <= date))
    {
        Some(&i) => i,
        None => match entries.last() {
            Some(&i) => i + 1,
            // No entries yet: after the section's content (e.g. a comment).
            None => {
                return append_to_section(body, INTERACTIONS_SECTION, line);
            }
        },
    };
    let mut out = join_lines(&lines[..at]);
    out.push_str(line);
    out.push('\n');
    out.push_str(&lines[at..].concat());
    out
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Leads must be enabled and `id` must be a lead ID (a usage error names
/// the kind it is instead).
fn require_lead_id(cfg: &ResolvedConfig, id: &str) -> McResult<()> {
    new_cmd::check_mode(EntityKind::Lead, cfg)?;
    let kind = EntityKind::from_id(id, cfg)?;
    if kind != EntityKind::Lead {
        return Err(McError::usage(
            format!("{id} is a {}, not a lead", kind.label()),
            Some(format!("lead IDs look like {}-001", cfg.id_prefixes.lead)),
        ));
    }
    Ok(())
}

/// The lead `id` names (canonical ID), or a usage error for other kinds.
fn find_lead(cfg: &ResolvedConfig, id: &str) -> McResult<EntityRecord> {
    require_lead_id(cfg, id)?;
    data::find_entity_by_id(id, cfg)
}

fn str_field<'a>(rec: &'a EntityRecord, key: &str) -> &'a str {
    frontmatter::get_str_or(&rec.frontmatter, key, "").trim()
}

fn yaml_seq(items: &[String]) -> Value {
    Value::Sequence(items.iter().map(|i| Value::String(i.clone())).collect())
}

/// Trimmed, deduplicated, non-empty tags.
fn clean_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tags.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        if !out.iter().any(|o| o == t) {
            out.push(t.to_string());
        }
    }
    out
}

/// Lower-cased, whitespace-collapsed form for matching names.
fn name_key(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

// ---------------------------------------------------------------------------
// Field updates and stage moves
// ---------------------------------------------------------------------------

/// Changes for [`update_lead`]; `None` leaves a field alone, an empty string
/// clears it (except `name` and `status`, which can't be empty), an empty
/// list clears a list.
#[derive(Debug, Clone, Default)]
pub struct LeadUpdate {
    pub name: Option<String>,
    pub company: Option<String>,
    pub role: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub linkedin: Option<String>,
    pub website: Option<String>,
    /// One of [`LEAD_SOURCES`].
    pub source: Option<String>,
    /// Event ID (must exist), stored as `[[EVT-NNN|Title]]`.
    pub event: Option<String>,
    /// Customer IDs (must exist), replacing the list.
    pub introduce_to: Option<Vec<String>>,
    /// One of [`LEAD_WARMTH`].
    pub warmth: Option<String>,
    /// `YYYY-MM-DD`.
    pub next_follow_up: Option<String>,
    /// `YYYY-MM-DD`.
    pub last_contact: Option<String>,
    pub owner: Option<String>,
    /// A configured lead status (`statuses.lead`), matched leniently.
    pub status: Option<String>,
    pub tags: Option<Vec<String>>,
}

impl LeadUpdate {
    pub fn is_empty(&self) -> bool {
        self.name.is_none()
            && self.company.is_none()
            && self.role.is_none()
            && self.email.is_none()
            && self.phone.is_none()
            && self.linkedin.is_none()
            && self.website.is_none()
            && self.source.is_none()
            && self.event.is_none()
            && self.introduce_to.is_none()
            && self.warmth.is_none()
            && self.next_follow_up.is_none()
            && self.last_contact.is_none()
            && self.owner.is_none()
            && self.status.is_none()
            && self.tags.is_none()
    }
}

/// The outcome of [`update_lead`] / [`move_lead`].
#[derive(Debug, Clone)]
pub struct LeadUpdated {
    pub id: String,
    /// Fields whose value actually changed, `status` last. Empty when the
    /// lead already had every given value (the file is left alone then).
    pub changed: Vec<&'static str>,
    pub old_status: String,
    pub new_status: String,
    pub path: PathBuf,
}

impl LeadUpdated {
    /// `{id, changed, old_status, new_status, path}`, `path` repo-relative.
    pub fn to_json(&self, cfg: &ResolvedConfig) -> JsonValue {
        serde_json::json!({
            "id": self.id,
            "changed": self.changed,
            "old_status": self.old_status,
            "new_status": self.new_status,
            "path": crate::mcp::repo_relative(cfg, &self.path),
        })
    }
}

/// A lead status in its configured spelling, or a usage error.
fn canonical_lead_status(cfg: &ResolvedConfig, status: &str) -> McResult<String> {
    if status.trim().is_empty() {
        return Err(McError::usage(
            "A lead needs a status",
            Some(format!("valid statuses: {}", cfg.statuses.lead.join(", "))),
        ));
    }
    Ok(
        new_cmd::canonical_status(&Some(status.to_string()), EntityKind::Lead, cfg)?
            .unwrap_or_default(),
    )
}

/// Validate a fixed-vocabulary update value; blank clears.
fn choice_update(value: &Option<String>, field: &str, valid: &[&str]) -> McResult<Option<String>> {
    Ok(match value {
        None => None,
        Some(v) if v.trim().is_empty() => Some(String::new()),
        Some(_) => new_cmd::canonical_choice(value, field, valid)?,
    })
}

/// Validate a date update value; blank clears.
fn date_update(value: &Option<String>, field: &str) -> McResult<Option<String>> {
    let Some(v) = value.as_deref().map(str::trim) else {
        return Ok(None);
    };
    if !v.is_empty() {
        new_cmd::validate_date(v, field)?;
    }
    Ok(Some(v.to_string()))
}

/// Change a lead's fields (and stage). Shared by `mc lead set`/`move`, MCP
/// `update_lead`/`move_lead`, `PATCH /v1/leads/{id}` and the dashboard.
///
/// `id` must be a lead ID (callers resolve loose input first). Every value
/// is checked before the file is touched: references must name existing
/// entities and are stored as `[[ID|Name]]` links, `source`, `warmth` and
/// dates must be valid. A new name also replaces the body's leading
/// `# Old name` heading; the folder keeps its name. Unchanged values leave
/// the file alone; otherwise `updated` is set to today.
pub fn update_lead(cfg: &ResolvedConfig, id: &str, update: &LeadUpdate) -> McResult<LeadUpdated> {
    require_lead_id(cfg, id)?;
    if update.is_empty() {
        return Err(McError::usage(
            "Nothing to change",
            Some(
                "give at least one field, e.g. status, warmth, next_follow_up or introduce_to"
                    .into(),
            ),
        ));
    }

    // Validate everything first, so a bad value changes nothing.
    let name = update
        .name
        .as_deref()
        .map(|n| single_line("name", n))
        .transpose()?;
    if name.as_deref() == Some("") {
        return Err(McError::usage("A lead needs a name", None));
    }
    let mut texts: Vec<(&'static str, String)> = Vec::new();
    for (key, value) in [
        ("company", &update.company),
        ("role", &update.role),
        ("email", &update.email),
        ("phone", &update.phone),
        ("linkedin", &update.linkedin),
        ("website", &update.website),
        ("owner", &update.owner),
    ] {
        if let Some(v) = value {
            texts.push((key, single_line(key, v)?));
        }
    }
    if let Some(v) = choice_update(&update.source, "lead source", &LEAD_SOURCES)? {
        texts.push(("source", v));
    }
    if let Some(v) = choice_update(&update.warmth, "warmth", &LEAD_WARMTH)? {
        texts.push(("warmth", v));
    }
    if let Some(v) = date_update(&update.next_follow_up, "follow-up date")? {
        texts.push(("next_follow_up", v));
    }
    if let Some(v) = date_update(&update.last_contact, "last contact date")? {
        let today = util::today_str();
        if v > today {
            return Err(McError::usage(
                format!("The last contact date {v} is in the future"),
                Some(format!("today is {today}")),
            ));
        }
        texts.push(("last_contact", v));
    }
    let status = update
        .status
        .as_deref()
        .map(|s| canonical_lead_status(cfg, s))
        .transpose()?;
    let event = update
        .event
        .as_deref()
        .map(|e| new_cmd::resolve_ref(cfg, EntityKind::Event, e))
        .transpose()?;
    let introduce_to = update
        .introduce_to
        .as_deref()
        .map(|l| new_cmd::resolve_refs(cfg, EntityKind::Customer, l))
        .transpose()?;
    let tags = update.tags.as_deref().map(clean_tags);

    let _lock = crate::lock::acquire(cfg)?;
    let lead = find_lead(cfg, id)?;
    let fm = &lead.frontmatter;
    let old_name = str_field(&lead, "name").to_string();
    let old_status = str_field(&lead, "status").to_string();

    // Only values that differ from the file are written.
    let name = name.filter(|n| *n != old_name);
    let texts: Vec<(&'static str, String)> = texts
        .into_iter()
        .filter(|(k, v)| frontmatter::get_str(fm, k).unwrap_or("").trim() != v)
        .collect();
    let event = event.filter(|e| frontmatter::get_link_str(fm, "event").unwrap_or("").trim() != e);
    let introduce_to =
        introduce_to.filter(|l| frontmatter::get_link_list(fm, "introduce_to") != *l);
    let tags = tags.filter(|l| frontmatter::get_string_list(fm, "tags") != *l);
    let status = status.filter(|s| *s != old_status);
    if let Some(s) = &status {
        check_converted_stage(cfg, &lead, s)?;
    }

    let mut changed: Vec<&'static str> = Vec::new();
    if name.is_some() {
        changed.push("name");
    }
    changed.extend(texts.iter().map(|(k, _)| *k));
    if event.is_some() {
        changed.push("event");
    }
    if introduce_to.is_some() {
        changed.push("introduce_to");
    }
    if tags.is_some() {
        changed.push("tags");
    }
    if status.is_some() {
        changed.push("status");
    }

    if !changed.is_empty() {
        let event_link = event.as_deref().map(|e| new_cmd::labeled_link(cfg, e));
        let intro_links: Option<Vec<String>> = introduce_to
            .as_ref()
            .map(|l| l.iter().map(|c| new_cmd::labeled_link(cfg, c)).collect());
        frontmatter::update_file(&lead.source_path, |fm| {
            if let Some(name) = &name {
                frontmatter::set_str(fm, "name", name);
            }
            for (key, value) in &texts {
                frontmatter::set_str(fm, key, value);
            }
            if let Some(link) = &event_link {
                frontmatter::set_str(fm, "event", link);
            }
            if let Some(status) = &status {
                frontmatter::set_str(fm, "status", status);
            }
            if let Some(map) = fm.as_mapping_mut() {
                if let Some(links) = &intro_links {
                    map.insert(Value::String("introduce_to".into()), yaml_seq(links));
                }
                if let Some(tags) = &tags {
                    map.insert(Value::String("tags".into()), yaml_seq(tags));
                }
            }
            frontmatter::set_str(fm, "updated", &util::today_str());
        })?;
        if let Some(name) = name.as_deref().filter(|_| !old_name.is_empty()) {
            crate::commands::task::retitle_body(&lead.source_path, &old_name, name)?;
        }
    }

    Ok(LeadUpdated {
        id: lead.id,
        changed,
        new_status: status.unwrap_or_else(|| old_status.clone()),
        old_status,
        path: lead.source_path,
    })
}

/// The `converted` stage and `converted_to` go together: only
/// [`convert_lead`] moves a lead into `converted` (it creates or links the
/// contact), and a lead linked to a contact can't be moved out of it.
/// Without a configured `converted` stage there is nothing to check.
fn check_converted_stage(cfg: &ResolvedConfig, lead: &EntityRecord, status: &str) -> McResult<()> {
    if !cfg.statuses.lead.iter().any(|s| s == "converted") {
        return Ok(());
    }
    let converted_to = frontmatter::get_link_str(&lead.frontmatter, "converted_to")
        .unwrap_or("")
        .trim();
    if status == "converted" && converted_to.is_empty() {
        return Err(McError::usage(
            format!("{} can't be moved to converted without a contact", lead.id),
            Some(format!(
                "convert it instead (`mc lead convert {}`, MCP convert_lead), which creates the contact",
                lead.id
            )),
        ));
    }
    if status != "converted" && !converted_to.is_empty() {
        return Err(McError::conflict(
            format!(
                "{} is converted to {converted_to}, so it stays in converted",
                lead.id
            ),
            Some("to undo the conversion, clear converted_to in the lead's file first".into()),
        ));
    }
    Ok(())
}

/// Move a lead to another pipeline stage (validated against
/// `statuses.lead`, matched leniently: `Intro Planned` -> `intro-planned`).
pub fn move_lead(cfg: &ResolvedConfig, id: &str, status: &str) -> McResult<LeadUpdated> {
    let status = canonical_lead_status(cfg, status)?;
    update_lead(
        cfg,
        id,
        &LeadUpdate {
            status: Some(status),
            ..Default::default()
        },
    )
}

// ---------------------------------------------------------------------------
// Interaction log
// ---------------------------------------------------------------------------

/// One interaction for [`log_interaction`].
#[derive(Debug, Clone, Default)]
pub struct Interaction {
    /// `YYYY-MM-DD`, defaults to today.
    pub date: Option<String>,
    /// One of [`INTERACTION_TYPES`].
    pub kind: String,
    /// What happened; line breaks become spaces.
    pub note: String,
    /// Also set the next follow-up date (`YYYY-MM-DD`).
    pub next_follow_up: Option<String>,
}

/// The outcome of [`log_interaction`].
#[derive(Debug, Clone)]
pub struct Logged {
    pub id: String,
    pub date: String,
    pub kind: String,
    pub note: String,
    /// The line written to `## Interactions`.
    pub line: String,
    pub last_contact: String,
    pub next_follow_up: String,
    pub old_status: String,
    pub new_status: String,
    pub path: PathBuf,
}

impl Logged {
    /// `{id, date, type, note, line, last_contact, next_follow_up,
    /// old_status, new_status, path}`, `path` repo-relative.
    pub fn to_json(&self, cfg: &ResolvedConfig) -> JsonValue {
        serde_json::json!({
            "id": self.id,
            "date": self.date,
            "type": self.kind,
            "note": self.note,
            "line": self.line,
            "last_contact": self.last_contact,
            "next_follow_up": self.next_follow_up,
            "old_status": self.old_status,
            "new_status": self.new_status,
            "path": crate::mcp::repo_relative(cfg, &self.path),
        })
    }
}

/// The note as one line: line breaks and tabs become spaces, other control
/// characters are dropped.
fn one_line(note: &str) -> String {
    let cleaned: String = note
        .chars()
        .map(|c| {
            if c == '\n' || c == '\r' || c == '\t' {
                ' '
            } else {
                c
            }
        })
        .filter(|c| !c.is_control())
        .collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Log an interaction: prepends `- YYYY-MM-DD · <type> · <note>` to the
/// lead's `## Interactions` section (newest first, see
/// [`insert_interaction`]), and
/// - sets `last_contact` to the date unless the type is `note` (it never
///   moves backwards when an older interaction is logged),
/// - sets `next_follow_up` when given,
/// - moves a lead still in `new` to `contacted` (when configured) unless the
///   type is `note`,
/// - bumps `updated`.
pub fn log_interaction(cfg: &ResolvedConfig, id: &str, input: &Interaction) -> McResult<Logged> {
    new_cmd::check_mode(EntityKind::Lead, cfg)?;
    let kind = new_cmd::canonical_choice(
        &Some(input.kind.clone()),
        "interaction type",
        &INTERACTION_TYPES,
    )?
    .ok_or_else(|| {
        McError::usage(
            "An interaction needs a type",
            Some(format!("one of {}", INTERACTION_TYPES.join(", "))),
        )
    })?;
    let note = one_line(&input.note);
    if note.is_empty() {
        return Err(McError::usage("An interaction needs a note", None));
    }
    let date = match new_cmd::present(&input.date) {
        Some(d) => {
            new_cmd::validate_date(d, "interaction date")?;
            // Dates are YYYY-MM-DD, so string order is date order. A future
            // date would pin last_contact (it never moves backwards).
            let today = util::today_str();
            if d > today.as_str() {
                return Err(McError::usage(
                    format!("The interaction date {d} is in the future"),
                    Some(format!("log what already happened (today is {today}); plan the next step with next_follow_up")),
                ));
            }
            d.to_string()
        }
        None => util::today_str(),
    };
    let follow_up = date_update(&input.next_follow_up, "follow-up date")?.filter(|d| !d.is_empty());
    let line = format!("- {date} · {kind} · {note}");

    let _lock = crate::lock::acquire(cfg)?;
    let lead = find_lead(cfg, id)?;
    let (mut fm, body) = frontmatter::parse_file(&lead.source_path)?;
    let old_status = frontmatter::get_str_or(&fm, "status", "")
        .trim()
        .to_string();
    let mut last_contact = frontmatter::get_str_or(&fm, "last_contact", "")
        .trim()
        .to_string();
    let mut new_status = old_status.clone();
    if kind != "note" {
        if last_contact.is_empty() || last_contact.as_str() < date.as_str() {
            last_contact = date.clone();
            frontmatter::set_str(&mut fm, "last_contact", &last_contact);
        }
        let first = cfg.statuses.lead.first().map(String::as_str);
        let contacted = cfg.statuses.lead.iter().any(|s| s == "contacted");
        if old_status == "new" && first == Some("new") && contacted {
            new_status = "contacted".into();
            frontmatter::set_str(&mut fm, "status", &new_status);
        }
    }
    if let Some(d) = &follow_up {
        frontmatter::set_str(&mut fm, "next_follow_up", d);
    }
    frontmatter::set_str(&mut fm, "updated", &util::today_str());
    let next_follow_up = frontmatter::get_str_or(&fm, "next_follow_up", "")
        .trim()
        .to_string();
    let body = insert_interaction(&body, &date, &line);
    util::atomic_write(
        &lead.source_path,
        frontmatter::serialize_document(&fm, &body).as_bytes(),
    )?;

    Ok(Logged {
        id: lead.id,
        date,
        kind,
        note,
        line,
        last_contact,
        next_follow_up,
        old_status,
        new_status,
        path: lead.source_path,
    })
}

// ---------------------------------------------------------------------------
// Conversion into a contact
// ---------------------------------------------------------------------------

/// Where [`convert_lead`] puts the new contact. At most one of the two;
/// with neither, the customer named like the lead's `company` is used.
#[derive(Debug, Clone, Default)]
pub struct ConvertInput {
    /// Existing customer ID (loose forms ok).
    pub customer: Option<String>,
    /// Create this customer (status `prospect` when configured) first.
    pub new_customer_name: Option<String>,
    /// When the customer already has a contact for this person (same email,
    /// or else same name), link the lead to it instead of refusing.
    pub link_existing: bool,
}

/// The outcome of [`convert_lead`].
#[derive(Debug, Clone)]
pub struct Converted {
    pub id: String,
    pub contact: String,
    pub contact_path: PathBuf,
    /// Whether the contact was created (false: linked to an existing one).
    pub contact_created: bool,
    pub customer: String,
    /// Whether the customer was created by this conversion.
    pub customer_created: bool,
    pub old_status: String,
    pub new_status: String,
    pub path: PathBuf,
}

impl Converted {
    /// `{id, contact, contact_path, contact_created, customer,
    /// customer_created, old_status, new_status, path}`, paths repo-relative.
    pub fn to_json(&self, cfg: &ResolvedConfig) -> JsonValue {
        serde_json::json!({
            "id": self.id,
            "contact": self.contact,
            "contact_path": crate::mcp::repo_relative(cfg, &self.contact_path),
            "contact_created": self.contact_created,
            "customer": self.customer,
            "customer_created": self.customer_created,
            "old_status": self.old_status,
            "new_status": self.new_status,
            "path": crate::mcp::repo_relative(cfg, &self.path),
        })
    }
}

/// Customers whose name matches `name` (case and spacing ignored).
fn customers_named(cfg: &ResolvedConfig, name: &str) -> McResult<Vec<EntityRecord>> {
    let key = name_key(name);
    Ok(data::collect_entities(EntityKind::Customer, cfg)?
        .into_iter()
        .filter(|c| name_key(frontmatter::get_str_or(&c.frontmatter, "name", "")) == key)
        .collect())
}

/// The contact under `customer` that is the same person as a lead: the one
/// with the same email (case-insensitive), or else the same name (case and
/// spacing ignored).
fn matching_contact(
    cfg: &ResolvedConfig,
    customer: &str,
    name: &str,
    email: &str,
) -> McResult<Option<EntityRecord>> {
    let contacts: Vec<EntityRecord> = data::collect_entities(EntityKind::Contact, cfg)?
        .into_iter()
        .filter(|c| {
            frontmatter::get_link_str(&c.frontmatter, "customer")
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case(customer)
        })
        .collect();
    let email = email.trim().to_lowercase();
    if !email.is_empty() {
        if let Some(c) = contacts.iter().find(|c| {
            frontmatter::get_str_or(&c.frontmatter, "email", "")
                .trim()
                .to_lowercase()
                == email
        }) {
            return Ok(Some(c.clone()));
        }
    }
    let key = name_key(name);
    Ok(contacts
        .into_iter()
        .find(|c| name_key(frontmatter::get_str_or(&c.frontmatter, "name", "")) == key))
}

/// Turn a lead into a contact (`CONT-NNN`) under a customer:
/// - the customer is `input.customer`, a new prospect named
///   `input.new_customer_name`, or else the one customer whose name matches
///   the lead's `company` (refused when there is none or several);
/// - the contact gets the lead's name, role, email, phone and tags, and a
///   note pointing back to the lead (with company, LinkedIn and website);
/// - the lead gets `converted_to: "[[CONT-NNN|Name]]"`, status `converted`
///   (when configured), an interaction line and a new `updated` date.
///
/// A lead that already has `converted_to` is refused (conflict). So is a
/// lead whose customer already has a contact with the same email (or else
/// the same name), unless `input.link_existing` is set: then the lead is
/// linked to that contact, which is left untouched. If a later write fails,
/// the contact (and a customer this call created) is removed again.
pub fn convert_lead(cfg: &ResolvedConfig, id: &str, input: &ConvertInput) -> McResult<Converted> {
    new_cmd::check_mode(EntityKind::Lead, cfg)?;
    new_cmd::check_mode(EntityKind::Contact, cfg)?;
    let customer_in = new_cmd::present(&input.customer).map(str::to_string);
    let new_name = new_cmd::present(&input.new_customer_name)
        .map(|n| single_line("customer name", n))
        .transpose()?;
    if customer_in.is_some() && new_name.is_some() {
        return Err(McError::usage(
            "Give either an existing customer or a new customer name, not both",
            None,
        ));
    }
    let customer_in = customer_in
        .map(|c| new_cmd::resolve_ref(cfg, EntityKind::Customer, &c))
        .transpose()?;

    let _lock = crate::lock::acquire(cfg)?;
    let lead = find_lead(cfg, id)?;
    let converted = frontmatter::get_link_str(&lead.frontmatter, "converted_to")
        .unwrap_or("")
        .trim()
        .to_string();
    if !converted.is_empty() {
        return Err(McError::conflict(
            format!("{} is already converted to {converted}", lead.id),
            Some(format!("see `mc show {converted}`")),
        ));
    }
    let name = str_field(&lead, "name").to_string();
    let company = str_field(&lead, "company").to_string();
    // Contacts (and customers) need their templates: fail before anything
    // is written rather than leave a half-done conversion behind.
    crate::template::load_template(&cfg.templates_dir, "contact")?;
    if new_name.is_some() {
        crate::template::load_template(&cfg.templates_dir, "customer")?;
    }

    let (customer, customer_created, customer_dir) = match (customer_in, new_name) {
        (Some(c), _) => (c, false, None),
        (None, Some(new_name)) => {
            if let Some(existing) = customers_named(cfg, &new_name)?.first() {
                return Err(McError::usage(
                    format!(
                        "A customer named '{new_name}' already exists ({})",
                        existing.id
                    ),
                    Some(format!("convert into it with customer {}", existing.id)),
                ));
            }
            let status = cfg
                .statuses
                .customer
                .iter()
                .find(|s| *s == "prospect")
                .cloned();
            let owner = Some(str_field(&lead, "owner").to_string()).filter(|o| !o.is_empty());
            let created = new_cmd::create_customer(
                cfg,
                &new_cmd::CustomerInput {
                    name: new_name,
                    owner,
                    status,
                    tags: Vec::new(),
                },
            )?;
            (created.id, true, Some(created.path))
        }
        (None, None) => {
            let hint = format!(
                "pass customer (e.g. CUST-001) or new_customer_name{}",
                if company.is_empty() {
                    String::new()
                } else {
                    format!(" (e.g. \"{company}\")")
                }
            );
            if company.is_empty() {
                return Err(McError::usage(
                    format!(
                        "{} has no company, so there is no customer to convert it into",
                        lead.id
                    ),
                    Some(hint),
                ));
            }
            match customers_named(cfg, &company)?.as_slice() {
                [one] => (one.id.clone(), false, None),
                [] => {
                    return Err(McError::usage(
                        format!(
                            "No customer is named '{company}' (the company of {})",
                            lead.id
                        ),
                        Some(hint),
                    ))
                }
                many => {
                    let ids: Vec<&str> = many.iter().map(|c| c.id.as_str()).collect();
                    return Err(McError::usage(
                        format!(
                            "Several customers are named '{company}': {}",
                            ids.join(", ")
                        ),
                        Some("pass customer with the right ID".into()),
                    ));
                }
            }
        }
    };

    let opt = |key: &str| Some(str_field(&lead, key).to_string()).filter(|v| !v.is_empty());
    if !customer_created {
        if let Some(existing) = matching_contact(cfg, &customer, &name, str_field(&lead, "email"))?
        {
            let existing_name = frontmatter::get_str_or(&existing.frontmatter, "name", "")
                .trim()
                .to_string();
            if !input.link_existing {
                return Err(McError::conflict(
                    format!(
                        "{customer} already has a contact for {name}: {} ({existing_name})",
                        existing.id
                    ),
                    Some(format!(
                        "link the lead to {} instead with link_existing (CLI: --link-existing)",
                        existing.id
                    )),
                ));
            }
            let contact_name = if existing_name.is_empty() {
                name.clone()
            } else {
                existing_name
            };
            let (old_status, new_status) = mark_converted(
                cfg,
                &lead,
                &existing.id,
                &contact_name,
                &customer,
                "Linked to",
            )?;
            return Ok(Converted {
                id: lead.id,
                contact: existing.id,
                contact_path: existing.source_path,
                contact_created: false,
                customer,
                customer_created,
                old_status,
                new_status,
                path: lead.source_path,
            });
        }
    }
    let contact = new_cmd::create_contact(
        cfg,
        &new_cmd::ContactInput {
            name: name.clone(),
            customer: customer.clone(),
            role: opt("role"),
            email: opt("email"),
            phone: opt("phone"),
            status: None,
            tags: frontmatter::get_string_list(&lead.frontmatter, "tags"),
        },
    );
    let contact = match contact {
        Ok(c) => c,
        Err(e) => {
            // Don't leave the customer this conversion just made behind.
            if let Some(dir) = customer_dir {
                let _ = std::fs::remove_dir_all(dir);
            }
            return Err(e);
        }
    };

    // From here on a failure must not leave the new contact (or customer)
    // behind: a retry would otherwise create a second one.
    let finish = || -> McResult<(String, String)> {
        write_contact_note(&contact.path, &lead, &name, &opt)?;
        mark_converted(cfg, &lead, &contact.id, &name, &customer, "Converted to")
    };
    let (old_status, new_status) = match finish() {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_file(&contact.path);
            if let Some(dir) = customer_dir {
                let _ = std::fs::remove_dir_all(dir);
            }
            return Err(e);
        }
    };

    Ok(Converted {
        id: lead.id,
        contact: contact.id,
        contact_path: contact.path,
        contact_created: true,
        customer,
        customer_created,
        old_status,
        new_status,
        path: lead.source_path,
    })
}

/// Point a new contact back at the lead in its `## Notes`; its frontmatter
/// stays the standard contact schema.
fn write_contact_note(
    path: &std::path::Path,
    lead: &EntityRecord,
    name: &str,
    opt: &dyn Fn(&str) -> Option<String>,
) -> McResult<()> {
    let today = util::today_str();
    let mut note = format!(
        "Converted from {} on {today}.",
        frontmatter::wrap_wikilink_labeled(&lead.id, name)
    );
    for (label, key) in [
        ("Company", "company"),
        ("LinkedIn", "linkedin"),
        ("Website", "website"),
    ] {
        if let Some(v) = opt(key) {
            note.push_str(&format!("\n- {label}: {v}"));
        }
    }
    let (cfm, cbody) = frontmatter::parse_file(path)?;
    let cbody = append_to_section(&cbody, "Notes", &note);
    util::atomic_write(
        path,
        frontmatter::serialize_document(&cfm, &cbody).as_bytes(),
    )
}

/// Set the lead's `converted_to`, status `converted` (when configured) and
/// `updated`, and log `<verb> [[CONT|Name]] at [[CUST|Name]]`. Returns
/// `(old_status, new_status)`.
fn mark_converted(
    cfg: &ResolvedConfig,
    lead: &EntityRecord,
    contact: &str,
    contact_name: &str,
    customer: &str,
    verb: &str,
) -> McResult<(String, String)> {
    let today = util::today_str();
    let contact_link = frontmatter::wrap_wikilink_labeled(contact, contact_name);
    let customer_link = new_cmd::labeled_link(cfg, customer);
    let (mut fm, body) = frontmatter::parse_file(&lead.source_path)?;
    let old_status = frontmatter::get_str_or(&fm, "status", "")
        .trim()
        .to_string();
    let new_status = if cfg.statuses.lead.iter().any(|s| s == "converted") {
        "converted".to_string()
    } else {
        old_status.clone()
    };
    frontmatter::set_str(&mut fm, "converted_to", &contact_link);
    frontmatter::set_str(&mut fm, "status", &new_status);
    frontmatter::set_str(&mut fm, "updated", &today);
    let line = format!("- {today} · note · {verb} {contact_link} at {customer_link}");
    let body = insert_interaction(&body, &today, &line);
    util::atomic_write(
        &lead.source_path,
        frontmatter::serialize_document(&fm, &body).as_bytes(),
    )?;
    Ok((old_status, new_status))
}

// ---------------------------------------------------------------------------
// CLI: `mc lead move|log|convert|set`
// ---------------------------------------------------------------------------

/// The lead that loose CLI input (`lead-1`, `1`) names.
fn resolve_cli_lead(cfg: &ResolvedConfig, id: &str) -> McResult<EntityRecord> {
    new_cmd::check_mode(EntityKind::Lead, cfg)?;
    let lead = suggest::find_entity(id, cfg, Some(EntityKind::Lead))?;
    if lead.kind != EntityKind::Lead {
        return Err(McError::usage(
            format!("{} is a {}, not a lead", lead.id, lead.kind.label()),
            Some(format!("lead IDs look like {}-001", cfg.id_prefixes.lead)),
        ));
    }
    Ok(lead)
}

fn print_json(value: &JsonValue) -> McResult<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn status_change(old: &str, new: &str) -> String {
    format!(
        "{} {} {}",
        ui::status(old),
        ui::glyphs().arrow.dimmed(),
        ui::status(new)
    )
}

pub fn run(subcmd: &LeadSubcommand, cfg: &ResolvedConfig) -> McResult<()> {
    match subcmd {
        LeadSubcommand::Move { id, status } => {
            let lead = resolve_cli_lead(cfg, id)?;
            let updated = move_lead(cfg, &lead.id, status)?;
            if ui::get().json {
                return print_json(&updated.to_json(cfg));
            }
            if updated.changed.is_empty() {
                ui::info(format!(
                    "{} is already {} {}",
                    updated.id.cyan().bold(),
                    ui::status(&updated.new_status),
                    "(nothing changed)".dimmed()
                ));
            } else {
                ui::success(format!(
                    "{} {}",
                    updated.id.cyan().bold(),
                    ui::clean(str_field(&lead, "name")).bold()
                ));
                println!(
                    "  {}",
                    status_change(&updated.old_status, &updated.new_status)
                );
            }
            Ok(())
        }
        LeadSubcommand::Log {
            id,
            interaction,
            note,
            date,
            next_follow_up,
        } => {
            let lead = resolve_cli_lead(cfg, id)?;
            let logged = log_interaction(
                cfg,
                &lead.id,
                &Interaction {
                    date: date.clone(),
                    kind: interaction.clone(),
                    note: note.clone(),
                    next_follow_up: next_follow_up.clone(),
                },
            )?;
            if ui::get().json {
                return print_json(&logged.to_json(cfg));
            }
            ui::success(format!(
                "Logged {} on {} {}",
                logged.kind.bold(),
                logged.id.cyan().bold(),
                ui::clean(str_field(&lead, "name")).bold()
            ));
            println!("  {}", ui::clean(&logged.line).dimmed());
            if logged.old_status != logged.new_status {
                println!(
                    "  {}",
                    status_change(&logged.old_status, &logged.new_status)
                );
            }
            if !logged.next_follow_up.is_empty() {
                println!("  next follow-up {}", logged.next_follow_up.cyan());
            }
            Ok(())
        }
        LeadSubcommand::Convert {
            id,
            customer,
            new_customer,
            link_existing,
        } => {
            let lead = resolve_cli_lead(cfg, id)?;
            let converted = convert_lead(
                cfg,
                &lead.id,
                &ConvertInput {
                    customer: customer.clone(),
                    new_customer_name: new_customer.clone(),
                    link_existing: *link_existing,
                },
            )?;
            if ui::get().json {
                return print_json(&converted.to_json(cfg));
            }
            ui::success(format!(
                "{} {} {} contact {} at {}{}",
                if converted.contact_created {
                    "Converted"
                } else {
                    "Linked"
                },
                converted.id.cyan().bold(),
                if converted.contact_created {
                    "into"
                } else {
                    "to existing"
                },
                converted.contact.cyan().bold(),
                converted.customer.cyan(),
                if converted.customer_created {
                    " (new prospect)".dimmed().to_string()
                } else {
                    String::new()
                }
            ));
            println!(
                "  {}",
                crate::mcp::repo_relative(cfg, &converted.contact_path).dimmed()
            );
            Ok(())
        }
        LeadSubcommand::Set {
            id,
            name,
            company,
            role,
            email,
            phone,
            linkedin,
            website,
            source,
            event,
            introduce_to,
            warmth,
            next_follow_up,
            last_contact,
            owner,
            status,
            tags,
        } => {
            let lead = resolve_cli_lead(cfg, id)?;
            let list = |v: &Option<String>| v.as_deref().map(util::parse_comma_list);
            let update = LeadUpdate {
                name: name.clone(),
                company: company.clone(),
                role: role.clone(),
                email: email.clone(),
                phone: phone.clone(),
                linkedin: linkedin.clone(),
                website: website.clone(),
                source: source.clone(),
                event: event.clone(),
                introduce_to: list(introduce_to),
                warmth: warmth.clone(),
                next_follow_up: next_follow_up.clone(),
                last_contact: last_contact.clone(),
                owner: owner.clone(),
                status: status.clone(),
                tags: list(tags),
            };
            let updated = update_lead(cfg, &lead.id, &update)?;
            if ui::get().json {
                return print_json(&updated.to_json(cfg));
            }
            if updated.changed.is_empty() {
                ui::info(format!(
                    "{} already has these values {}",
                    updated.id.cyan().bold(),
                    "(nothing changed)".dimmed()
                ));
            } else {
                ui::success(format!(
                    "Updated {} ({})",
                    updated.id.cyan().bold(),
                    updated.changed.join(", ")
                ));
                if updated.old_status != updated.new_status {
                    println!(
                        "  {}",
                        status_change(&updated.old_status, &updated.new_status)
                    );
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{init, new};
    use crate::config;
    use tempfile::TempDir;

    fn setup_repo() -> (TempDir, ResolvedConfig) {
        let tmp = TempDir::new().unwrap();
        init::run(tmp.path(), false, false, Some("CRM"), false, true).unwrap();
        let cfg = config::load_config(tmp.path(), config::RepoMode::Standalone).unwrap();
        (tmp, cfg)
    }

    fn lead_file(cfg: &ResolvedConfig, id: &str) -> (Value, String) {
        let rec = data::find_entity_by_id(id, cfg).unwrap();
        frontmatter::parse_file(&rec.source_path).unwrap()
    }

    fn get<'a>(fm: &'a Value, key: &str) -> &'a str {
        frontmatter::get_str_or(fm, key, "")
    }

    #[test]
    fn append_to_section_keeps_following_sections() {
        let body = "\n# X\n\n## Notes\n\n<!-- hi -->\n\n## Other\n\ntext\n";
        let out = append_to_section(body, "Notes", "first");
        assert_eq!(
            out,
            "\n# X\n\n## Notes\n\n<!-- hi -->\n\nfirst\n\n## Other\n\ntext\n"
        );
        let out = append_to_section("\n# X\n\n## Notes\n", "Notes", "a");
        assert_eq!(out, "\n# X\n\n## Notes\n\na\n");
        let out = append_to_section("\n# X\n", "Notes", "a");
        assert_eq!(out, "\n# X\n\n## Notes\n\na\n");
        // Headings in code don't count; the footer ends the section.
        let body = "## Notes\n\n```\n## Notes\n```\n%% mc-links: [[A-1]] %%\n";
        let out = append_to_section(body, "Notes", "z");
        assert_eq!(
            out,
            "## Notes\n\n```\n## Notes\n```\n\nz\n\n%% mc-links: [[A-1]] %%\n"
        );
    }

    #[test]
    fn interactions_are_newest_first() {
        let body = "# L\n\n## Interactions\n\n<!-- log -->\n\n## Notes\n";
        let b = insert_interaction(body, "2026-11-20", "- 2026-11-20 · call · a");
        assert_eq!(
            b,
            "# L\n\n## Interactions\n\n<!-- log -->\n\n- 2026-11-20 · call · a\n\n## Notes\n"
        );
        let b = insert_interaction(&b, "2026-11-25", "- 2026-11-25 · email · b");
        let b = insert_interaction(&b, "2026-11-01", "- 2026-11-01 · event · c");
        let b = insert_interaction(&b, "2026-11-25", "- 2026-11-25 · note · d");
        let entries: Vec<&str> = b.lines().filter(|l| l.starts_with("- ")).collect();
        assert_eq!(
            entries,
            vec![
                "- 2026-11-25 · note · d",
                "- 2026-11-25 · email · b",
                "- 2026-11-20 · call · a",
                "- 2026-11-01 · event · c",
            ]
        );
        assert!(b.ends_with("- 2026-11-01 · event · c\n\n## Notes\n"), "{b}");
        // A missing section goes before Notes.
        let b = insert_interaction(
            "# L\n\n## Notes\n\nx\n",
            "2026-01-01",
            "- 2026-01-01 · call · y",
        );
        assert_eq!(
            b,
            "# L\n\n## Interactions\n\n- 2026-01-01 · call · y\n\n## Notes\n\nx\n"
        );
    }

    #[test]
    fn create_lead_and_event_with_builtin_templates() {
        let (_tmp, cfg) = setup_repo();
        // Repos from before leads existed have no templates.
        std::fs::remove_file(cfg.templates_dir.join("lead.md")).unwrap();
        std::fs::remove_file(cfg.templates_dir.join("event.md")).unwrap();
        new::create_customer(&cfg, &new::CustomerInput::new("Zema")).unwrap();
        let mut ev = new::EventInput::new("Hannover Messe");
        ev.date = Some("2026-04-20".into());
        ev.event_type = Some("Trade Fair".into());
        let ev = new::create_event(&cfg, &ev).unwrap();
        assert_eq!(ev.id, "EVT-001");
        let (efm, ebody) = frontmatter::parse_file(&ev.path.join("EVT-001.md")).unwrap();
        assert_eq!(get(&efm, "type"), "trade-fair");
        assert_eq!(get(&efm, "status"), "planned");
        assert!(ebody.contains("## Goals") && ebody.contains("## Debrief"));

        let mut input = new::LeadInput::new("Jane Doe");
        input.company = Some("Acme GmbH".into());
        input.event = Some("evt-1".into());
        input.introduce_to = vec!["CUST-001".into()];
        input.warmth = Some("HOT".into());
        input.notes = Some("Met at the booth".into());
        let lead = new::create_lead(&cfg, &input).unwrap();
        assert_eq!(lead.id, "LEAD-001");
        assert!(lead.path.ends_with("LEAD-001-jane-doe"));
        let (fm, body) = lead_file(&cfg, "LEAD-001");
        let keys: Vec<&str> = fm
            .as_mapping()
            .unwrap()
            .keys()
            .filter_map(|k| k.as_str())
            .collect();
        assert_eq!(
            keys,
            vec![
                "id",
                "aliases",
                "name",
                "slug",
                "company",
                "role",
                "email",
                "phone",
                "linkedin",
                "website",
                "status",
                "owner",
                "source",
                "event",
                "introduce_to",
                "warmth",
                "next_follow_up",
                "last_contact",
                "converted_to",
                "tags",
                "created",
                "updated"
            ]
        );
        assert_eq!(get(&fm, "status"), "new");
        assert_eq!(get(&fm, "source"), "event");
        assert_eq!(get(&fm, "warmth"), "hot");
        assert_eq!(get(&fm, "event"), "[[EVT-001|Hannover Messe]]");
        assert_eq!(
            frontmatter::get_string_list(&fm, "introduce_to"),
            vec!["[[CUST-001|Zema]]"]
        );
        assert!(body.contains("## Summary") && body.contains("## Interactions"));
        assert!(body.contains("## Notes\n\nMet at the booth\n"), "{body}");
    }

    #[test]
    fn create_lead_validates_input() {
        let (_tmp, cfg) = setup_repo();
        let mut input = new::LeadInput::new("X");
        input.warmth = Some("lukewarm".into());
        assert!(matches!(
            new::create_lead(&cfg, &input),
            Err(McError::Usage { .. })
        ));
        let mut input = new::LeadInput::new("X");
        input.event = Some("EVT-009".into());
        assert!(new::create_lead(&cfg, &input).is_err());
        let mut input = new::LeadInput::new("X");
        input.next_follow_up = Some("2026-1-5".into());
        assert!(new::create_lead(&cfg, &input).is_err());
        let mut ev = new::EventInput::new("E");
        ev.date = Some("2026-05-02".into());
        ev.end_date = Some("2026-05-01".into());
        assert!(new::create_event(&cfg, &ev).is_err());
        // Nothing was allocated.
        assert!(data::collect_entities(EntityKind::Lead, &cfg)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn update_and_move_lead() {
        let (_tmp, cfg) = setup_repo();
        new::create_customer(&cfg, &new::CustomerInput::new("Zema")).unwrap();
        new::create_lead(&cfg, &new::LeadInput::new("Jane Doe")).unwrap();
        let updated = update_lead(
            &cfg,
            "LEAD-001",
            &LeadUpdate {
                name: Some("Jane Q. Doe".into()),
                warmth: Some("warm".into()),
                introduce_to: Some(vec!["cust-1".into()]),
                next_follow_up: Some("2026-12-01".into()),
                tags: Some(vec!["robotics".into(), "robotics".into()]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            updated.changed,
            vec!["name", "warmth", "next_follow_up", "introduce_to", "tags"]
        );
        let (fm, body) = lead_file(&cfg, "LEAD-001");
        assert_eq!(get(&fm, "name"), "Jane Q. Doe");
        assert!(body.contains("# Jane Q. Doe"));
        assert_eq!(frontmatter::get_string_list(&fm, "tags"), vec!["robotics"]);
        assert_eq!(
            frontmatter::get_string_list(&fm, "introduce_to"),
            vec!["[[CUST-001|Zema]]"]
        );

        // Same values again change nothing.
        let again = update_lead(
            &cfg,
            "LEAD-001",
            &LeadUpdate {
                warmth: Some("Warm".into()),
                introduce_to: Some(vec!["CUST-001".into()]),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(again.changed.is_empty());

        let moved = move_lead(&cfg, "LEAD-001", "Intro Planned").unwrap();
        assert_eq!(moved.old_status, "new");
        assert_eq!(moved.new_status, "intro-planned");
        assert!(matches!(
            move_lead(&cfg, "LEAD-001", "won"),
            Err(McError::Usage { .. })
        ));
        assert!(update_lead(&cfg, "LEAD-001", &LeadUpdate::default()).is_err());
        assert!(update_lead(
            &cfg,
            "LEAD-001",
            &LeadUpdate {
                source: Some("fax".into()),
                ..Default::default()
            }
        )
        .is_err());
        // Clearing a field.
        update_lead(
            &cfg,
            "LEAD-001",
            &LeadUpdate {
                warmth: Some(String::new()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(get(&lead_file(&cfg, "LEAD-001").0, "warmth"), "");
        // Other kinds are refused.
        assert!(update_lead(&cfg, "CUST-001", &LeadUpdate::default()).is_err());
    }

    #[test]
    fn log_interaction_updates_dates_and_stage() {
        let (_tmp, cfg) = setup_repo();
        new::create_lead(&cfg, &new::LeadInput::new("Jane")).unwrap();
        let note = log_interaction(
            &cfg,
            "LEAD-001",
            &Interaction {
                date: Some("2026-11-20".into()),
                kind: "note".into(),
                note: "Looks promising".into(),
                next_follow_up: None,
            },
        )
        .unwrap();
        assert_eq!(note.new_status, "new");
        assert_eq!(note.last_contact, "");

        let call = log_interaction(
            &cfg,
            "LEAD-001",
            &Interaction {
                date: Some("2026-11-21".into()),
                kind: "Call".into(),
                note: "Talked\nabout a pilot".into(),
                next_follow_up: Some("2026-12-01".into()),
            },
        )
        .unwrap();
        assert_eq!(call.line, "- 2026-11-21 · call · Talked about a pilot");
        assert_eq!(call.old_status, "new");
        assert_eq!(call.new_status, "contacted");
        assert_eq!(call.last_contact, "2026-11-21");
        assert_eq!(call.next_follow_up, "2026-12-01");

        // An older interaction doesn't move last_contact back.
        log_interaction(
            &cfg,
            "LEAD-001",
            &Interaction {
                date: Some("2026-11-02".into()),
                kind: "event".into(),
                note: "Met at the fair".into(),
                next_follow_up: None,
            },
        )
        .unwrap();
        let (fm, body) = lead_file(&cfg, "LEAD-001");
        assert_eq!(get(&fm, "last_contact"), "2026-11-21");
        assert_eq!(get(&fm, "status"), "contacted");
        let entries: Vec<&str> = body.lines().filter(|l| l.starts_with("- ")).collect();
        assert_eq!(
            entries,
            vec![
                "- 2026-11-21 · call · Talked about a pilot",
                "- 2026-11-20 · note · Looks promising",
                "- 2026-11-02 · event · Met at the fair",
            ]
        );
        for bad in [
            Interaction {
                kind: "fax".into(),
                note: "x".into(),
                ..Default::default()
            },
            Interaction {
                kind: "call".into(),
                note: " \n ".into(),
                ..Default::default()
            },
            Interaction {
                kind: "call".into(),
                note: "x".into(),
                date: Some("yesterday".into()),
                ..Default::default()
            },
        ] {
            assert!(log_interaction(&cfg, "LEAD-001", &bad).is_err());
        }
    }

    #[test]
    fn convert_lead_into_existing_new_and_matching_customer() {
        let (_tmp, cfg) = setup_repo();
        new::create_customer(&cfg, &new::CustomerInput::new("Acme GmbH")).unwrap();
        let mut a = new::LeadInput::new("Jane Doe");
        a.company = Some("acme  gmbh".into());
        a.email = Some("jane@acme.example".into());
        a.role = Some("CTO".into());
        a.linkedin = Some("https://linkedin.example/jane".into());
        a.tags = vec!["robotics".into()];
        new::create_lead(&cfg, &a).unwrap();
        let mut b = new::LeadInput::new("Max Muster");
        b.company = Some("Nowhere AG".into());
        new::create_lead(&cfg, &b).unwrap();

        // Matched by company name.
        let c = convert_lead(&cfg, "LEAD-001", &ConvertInput::default()).unwrap();
        assert_eq!(c.customer, "CUST-001");
        assert!(!c.customer_created);
        assert_eq!(c.contact, "CONT-001");
        assert_eq!(c.new_status, "converted");
        let contact = data::find_entity_by_id("CONT-001", &cfg).unwrap();
        assert_eq!(get(&contact.frontmatter, "email"), "jane@acme.example");
        assert_eq!(get(&contact.frontmatter, "role"), "CTO");
        // Same frontmatter as `mc new contact`: a real slug and a labelled
        // customer link.
        assert_eq!(get(&contact.frontmatter, "slug"), "jane-doe");
        assert_eq!(
            get(&contact.frontmatter, "customer"),
            "[[CUST-001|Acme GmbH]]"
        );
        assert!(contact.source_path.ends_with("CONT-001-jane-doe.md"));
        assert_eq!(
            frontmatter::get_string_list(&contact.frontmatter, "tags"),
            vec!["robotics"]
        );
        assert!(contact
            .body
            .contains("Converted from [[LEAD-001|Jane Doe]]"));
        assert!(contact
            .body
            .contains("LinkedIn: https://linkedin.example/jane"));
        let (fm, body) = lead_file(&cfg, "LEAD-001");
        assert_eq!(get(&fm, "converted_to"), "[[CONT-001|Jane Doe]]");
        assert_eq!(get(&fm, "status"), "converted");
        assert!(
            body.contains("· note · Converted to [[CONT-001|Jane Doe]] at [[CUST-001|Acme GmbH]]")
        );

        // Twice is a conflict.
        assert!(matches!(
            convert_lead(&cfg, "LEAD-001", &ConvertInput::default()),
            Err(McError::Conflict { .. })
        ));
        // No matching customer: refused, nothing created.
        assert!(matches!(
            convert_lead(&cfg, "LEAD-002", &ConvertInput::default()),
            Err(McError::Usage { .. })
        ));
        assert_eq!(data::collect_contacts(&cfg).unwrap().len(), 1);
        // New customer by name, but not a duplicate one.
        assert!(convert_lead(
            &cfg,
            "LEAD-002",
            &ConvertInput {
                new_customer_name: Some("Acme GmbH".into()),
                ..Default::default()
            }
        )
        .is_err());
        let c = convert_lead(
            &cfg,
            "LEAD-002",
            &ConvertInput {
                new_customer_name: Some("Nowhere AG".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(c.customer_created);
        assert_eq!(c.customer, "CUST-002");
        let cust = data::find_entity_by_id("CUST-002", &cfg).unwrap();
        assert_eq!(get(&cust.frontmatter, "status"), "prospect");
        assert!(crate::commands::validate::validate_programmatic(&cfg)
            .unwrap()
            .iter()
            .all(|i| !i.is_error()));
    }

    #[test]
    fn convert_without_contact_template_writes_nothing() {
        let (_tmp, cfg) = setup_repo();
        let mut input = new::LeadInput::new("Jane");
        input.company = Some("Acme".into());
        new::create_lead(&cfg, &input).unwrap();
        std::fs::remove_file(cfg.templates_dir.join("contact.md")).unwrap();
        let err = convert_lead(
            &cfg,
            "LEAD-001",
            &ConvertInput {
                new_customer_name: Some("Acme".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, McError::TemplateNotFound(_)), "{err}");
        assert!(data::collect_entities(EntityKind::Customer, &cfg)
            .unwrap()
            .is_empty());
        assert_eq!(get(&lead_file(&cfg, "LEAD-001").0, "status"), "new");
    }

    #[test]
    fn convert_with_explicit_customer_and_both_is_refused() {
        let (_tmp, cfg) = setup_repo();
        new::create_customer(&cfg, &new::CustomerInput::new("Zema")).unwrap();
        new::create_lead(&cfg, &new::LeadInput::new("Jane")).unwrap();
        assert!(convert_lead(
            &cfg,
            "LEAD-001",
            &ConvertInput {
                customer: Some("CUST-001".into()),
                new_customer_name: Some("Other".into()),
                ..Default::default()
            }
        )
        .is_err());
        let c = convert_lead(
            &cfg,
            "LEAD-001",
            &ConvertInput {
                customer: Some("cust-1".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(c.customer, "CUST-001");
    }

    #[test]
    fn convert_refuses_a_duplicate_contact_unless_linking() {
        let (_tmp, cfg) = setup_repo();
        new::create_customer(&cfg, &new::CustomerInput::new("Zema")).unwrap();
        let mut existing = new::ContactInput::new("Paul Motzki", "CUST-001");
        existing.email = Some("Paul@Zema.example".into());
        new::create_contact(&cfg, &existing).unwrap();
        let before = std::fs::read_to_string(
            data::find_entity_by_id("CONT-001", &cfg)
                .unwrap()
                .source_path,
        )
        .unwrap();
        // Same name, other spacing and case.
        let mut a = new::LeadInput::new("paul  motzki");
        a.company = Some("Zema".into());
        new::create_lead(&cfg, &a).unwrap();
        // Different name, same email.
        let mut b = new::LeadInput::new("P. Motzki");
        b.email = Some("paul@zema.example".into());
        new::create_lead(&cfg, &b).unwrap();
        // Someone else.
        let mut c = new::LeadInput::new("Jane Doe");
        c.company = Some("Zema".into());
        new::create_lead(&cfg, &c).unwrap();

        let err = convert_lead(&cfg, "LEAD-001", &ConvertInput::default()).unwrap_err();
        assert!(matches!(err, McError::Conflict { .. }), "{err}");
        assert!(err.to_string().contains("CONT-001"), "{err}");
        let err = convert_lead(
            &cfg,
            "LEAD-002",
            &ConvertInput {
                customer: Some("CUST-001".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(matches!(err, McError::Conflict { .. }), "{err}");
        assert_eq!(data::collect_contacts(&cfg).unwrap().len(), 1);
        assert_eq!(get(&lead_file(&cfg, "LEAD-001").0, "status"), "new");

        // Linking uses the existing contact and leaves it untouched.
        let linked = convert_lead(
            &cfg,
            "LEAD-001",
            &ConvertInput {
                link_existing: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(linked.contact, "CONT-001");
        assert!(!linked.contact_created);
        assert_eq!(linked.new_status, "converted");
        let (fm, body) = lead_file(&cfg, "LEAD-001");
        assert_eq!(get(&fm, "converted_to"), "[[CONT-001|Paul Motzki]]");
        assert!(body.contains("· note · Linked to [[CONT-001|Paul Motzki]] at [[CUST-001|Zema]]"));
        let after = std::fs::read_to_string(
            data::find_entity_by_id("CONT-001", &cfg)
                .unwrap()
                .source_path,
        )
        .unwrap();
        assert_eq!(before, after);
        assert_eq!(data::collect_contacts(&cfg).unwrap().len(), 1);

        // No match: link_existing still creates the contact.
        let made = convert_lead(
            &cfg,
            "LEAD-003",
            &ConvertInput {
                link_existing: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(made.contact_created);
        assert_eq!(made.contact, "CONT-002");
    }

    #[cfg(unix)]
    #[test]
    fn convert_removes_the_contact_when_the_lead_write_fails() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, cfg) = setup_repo();
        new::create_customer(&cfg, &new::CustomerInput::new("Zema")).unwrap();
        let mut input = new::LeadInput::new("Jane");
        input.company = Some("Zema".into());
        new::create_lead(&cfg, &input).unwrap();
        let lead = data::find_entity_by_id("LEAD-001", &cfg).unwrap();
        let dir = lead.source_path.parent().unwrap().to_path_buf();
        // A read-only folder: the atomic write can't create its temp file.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = convert_lead(&cfg, "LEAD-001", &ConvertInput::default());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(result.is_err());
        assert!(data::collect_contacts(&cfg).unwrap().is_empty());
        assert_eq!(get(&lead_file(&cfg, "LEAD-001").0, "converted_to"), "");
        // A retry then works and makes exactly one contact.
        let c = convert_lead(&cfg, "LEAD-001", &ConvertInput::default()).unwrap();
        assert_eq!(c.contact, "CONT-001");
        assert_eq!(data::collect_contacts(&cfg).unwrap().len(), 1);
    }

    #[test]
    fn converted_stage_needs_a_contact_and_keeps_it() {
        let (_tmp, cfg) = setup_repo();
        new::create_customer(&cfg, &new::CustomerInput::new("Zema")).unwrap();
        let mut input = new::LeadInput::new("Jane");
        input.company = Some("Zema".into());
        new::create_lead(&cfg, &input).unwrap();
        new::create_lead(&cfg, &new::LeadInput::new("Max")).unwrap();

        // Moving to converted by hand is refused: convert creates the contact.
        let err = move_lead(&cfg, "LEAD-002", "converted").unwrap_err();
        assert!(matches!(err, McError::Usage { .. }), "{err}");
        assert_eq!(get(&lead_file(&cfg, "LEAD-002").0, "status"), "new");
        assert!(update_lead(
            &cfg,
            "LEAD-002",
            &LeadUpdate {
                status: Some("Converted".into()),
                ..Default::default()
            }
        )
        .is_err());

        convert_lead(&cfg, "LEAD-001", &ConvertInput::default()).unwrap();
        // A converted lead stays converted, but its other fields can change.
        for stage in ["qualified", "lost"] {
            let err = move_lead(&cfg, "LEAD-001", stage).unwrap_err();
            assert!(matches!(err, McError::Conflict { .. }), "{err}");
        }
        update_lead(
            &cfg,
            "LEAD-001",
            &LeadUpdate {
                warmth: Some("hot".into()),
                status: Some("converted".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let fm = lead_file(&cfg, "LEAD-001").0;
        assert_eq!(get(&fm, "status"), "converted");
        assert_eq!(get(&fm, "warmth"), "hot");
    }

    #[test]
    fn future_interaction_and_last_contact_dates_are_refused() {
        let (_tmp, cfg) = setup_repo();
        new::create_lead(&cfg, &new::LeadInput::new("Jane")).unwrap();
        let tomorrow = (chrono::Local::now().date_naive() + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let err = log_interaction(
            &cfg,
            "LEAD-001",
            &Interaction {
                date: Some(tomorrow.clone()),
                kind: "call".into(),
                note: "Typo in the year".into(),
                next_follow_up: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, McError::Usage { .. }), "{err}");
        let (fm, body) = lead_file(&cfg, "LEAD-001");
        assert_eq!(get(&fm, "last_contact"), "");
        assert!(!body.contains("Typo"));
        // Today is fine.
        log_interaction(
            &cfg,
            "LEAD-001",
            &Interaction {
                date: Some(util::today_str()),
                kind: "call".into(),
                note: "Called".into(),
                next_follow_up: Some(tomorrow.clone()),
            },
        )
        .unwrap();
        assert!(update_lead(
            &cfg,
            "LEAD-001",
            &LeadUpdate {
                last_contact: Some(tomorrow),
                ..Default::default()
            }
        )
        .is_err());
    }
}
