use crate::cli::ui;
use crate::commands::task;
use crate::config::{RepoMode, ResolvedConfig};
use crate::data::{self, EntityRecord, StatusCounts, TaskFilter};
use crate::entity::EntityKind;
use crate::error::McResult;
use crate::frontmatter;
use colored::*;
use serde_json::{json, Value as JsonValue};

const KINDS: [EntityKind; 9] = [
    EntityKind::Customer,
    EntityKind::Project,
    EntityKind::Contact,
    EntityKind::Meeting,
    EntityKind::Research,
    EntityKind::Task,
    EntityKind::Sprint,
    EntityKind::Milestone,
    EntityKind::Proposal,
];

const RECENT: usize = 5;
const FOCUS: usize = 5;
/// How many days ahead "Coming up" looks for meetings.
const UPCOMING_DAYS: i64 = 7;

/// Width of the per-status proportion bars.
const BAR: usize = 24;

pub fn run(cfg: &ResolvedConfig) -> McResult<()> {
    // Tasks and meetings are read once and reused for counts and focus.
    let tasks = if cfg.entity_available(&EntityKind::Task) {
        data::collect_tasks(cfg)?
    } else {
        Vec::new()
    };
    let meetings = if cfg.entity_available(&EntityKind::Meeting) {
        data::collect_entities(EntityKind::Meeting, cfg)?
    } else {
        Vec::new()
    };
    let mut sections: Vec<(EntityKind, StatusCounts)> = Vec::new();
    for kind in KINDS {
        if !cfg.entity_available(&kind) {
            continue;
        }
        let counts = if kind == EntityKind::Task {
            data::status_counts_of(kind, &tasks)
        } else if kind == EntityKind::Meeting {
            data::status_counts_of(kind, &meetings)
        } else {
            data::count_by_status(kind, cfg)?
        };
        sections.push((kind, counts));
    }
    let recent = data::recent_activity(cfg, RECENT)?;
    let today = crate::util::today_str();
    let in_progress: Vec<&EntityRecord> = tasks
        .iter()
        .filter(|t| frontmatter::get_str(&t.frontmatter, "status") == Some("in-progress"))
        .collect();
    let overdue: Vec<&EntityRecord> = tasks
        .iter()
        .filter(|t| is_open(t))
        .filter(|t| {
            frontmatter::get_str(&t.frontmatter, "due_date")
                .is_some_and(|d| !d.is_empty() && d < today.as_str())
        })
        .collect();
    let next = task::actionable_in(&tasks, &TaskFilter::all())
        .0
        .into_iter()
        .next();
    let agenda = Agenda::of(&meetings, &today);

    if ui::get().json {
        return print_json(
            cfg,
            &sections,
            &recent,
            Focus {
                in_progress: &in_progress,
                overdue: &overdue,
                next,
                agenda: &agenda,
            },
        );
    }

    print_banner(cfg);
    print_counts(&sections);
    print_focus(&in_progress, &overdue, next, &today);
    print_agenda(&agenda, &today);
    print_recent(cfg, &recent);
    Ok(())
}

/// Meetings for the "Coming up" section.
struct Agenda<'a> {
    /// Open meetings from today through [`UPCOMING_DAYS`] ahead (or else the
    /// next one), by date and time.
    upcoming: Vec<&'a EntityRecord>,
    /// Meetings dated before today that are still `scheduled`.
    past_scheduled: usize,
}

impl<'a> Agenda<'a> {
    fn of(meetings: &'a [EntityRecord], today: &str) -> Self {
        fn get<'r>(m: &'r EntityRecord, key: &str) -> &'r str {
            frontmatter::get_str_or(&m.frontmatter, key, "")
        }
        let until = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d")
            .map(|d| {
                (d + chrono::Duration::days(UPCOMING_DAYS))
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .unwrap_or_else(|_| today.to_string());
        let mut ahead: Vec<&EntityRecord> = meetings
            .iter()
            .filter(|m| !matches!(get(m, "status"), "completed" | "cancelled"))
            .filter(|m| get(m, "date") >= today)
            .collect();
        ahead.sort_by_key(|m| (get(m, "date"), get(m, "time")));
        // The next week's meetings; if there are none, just the next one.
        let in_window = ahead
            .iter()
            .take_while(|m| get(m, "date") <= until.as_str())
            .count();
        ahead.truncate(in_window.max(1));
        let upcoming = ahead;
        let past_scheduled = meetings
            .iter()
            .filter(|m| get(m, "status") == "scheduled")
            .filter(|m| !get(m, "date").is_empty() && get(m, "date") < today)
            .count();
        Agenda {
            upcoming,
            past_scheduled,
        }
    }
}

fn is_open(t: &EntityRecord) -> bool {
    !matches!(
        frontmatter::get_str_or(&t.frontmatter, "status", ""),
        "done" | "cancelled"
    )
}

fn repo_name(cfg: &ResolvedConfig) -> String {
    let brand = cfg.brand.name.trim();
    if !brand.is_empty() && brand != "MissionControl" {
        return brand.to_string();
    }
    cfg.root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "MissionControl".into())
}

fn mode_label(cfg: &ResolvedConfig) -> &'static str {
    match cfg.mode {
        RepoMode::Embedded => "embedded",
        RepoMode::Standalone => "standalone",
    }
}

fn print_banner(cfg: &ResolvedConfig) {
    let g = ui::glyphs();
    println!();
    println!(
        "  {} {}  {} {} {}",
        g.brand.cyan().bold(),
        ui::section("Mission Control"),
        repo_name(cfg).bold(),
        g.sep.dimmed(),
        mode_label(cfg).dimmed()
    );
    println!("  {}", ui::rule(ui::content_width(2, 64)));
    println!();
}

fn print_counts(sections: &[(EntityKind, StatusCounts)]) {
    let label_w = sections
        .iter()
        .flat_map(|(_, c)| c.by_status.iter().map(|(s, _)| ui::width_of(s)))
        .max()
        .unwrap_or(8)
        .max(8)
        + if ui::fancy() { 2 } else { 0 };
    let count_w = sections
        .iter()
        .map(|(_, c)| c.total.to_string().len())
        .max()
        .unwrap_or(1)
        .max(3);
    // Shrink bars on narrow terminals: indent + label + count + gaps + pct.
    let bar_w = ui::get()
        .width
        .map(|w| w.saturating_sub(4 + label_w + 2 + count_w + 2 + 6))
        .unwrap_or(BAR)
        .clamp(6, BAR);

    let mut empty = Vec::new();
    for (kind, counts) in sections {
        if counts.total == 0 {
            empty.push(kind.label_plural());
            continue;
        }
        println!(
            "  {}  {}",
            ui::section(kind.label_plural()),
            counts.total.to_string().bold()
        );
        for (status, n) in &counts.by_status {
            let tone = ui::status_tone(status);
            let frac = *n as f64 / counts.total as f64;
            println!(
                "    {}  {}  {}  {}",
                ui::pad(&ui::status(status), label_w),
                ui::pad_left(&n.to_string(), count_w),
                ui::bar(frac, bar_w, tone),
                ui::pad_left(&format!("{:.0}%", frac * 100.0), 4).dimmed()
            );
        }
        println!();
    }
    if empty.len() == sections.len() {
        ui::info("Nothing tracked yet.");
        ui::hint(format!(
            "start with {} or {}",
            ui::cmd("mc new task \"...\""),
            ui::cmd("mc new meeting \"...\"")
        ));
        println!();
        return;
    }
    if !empty.is_empty() {
        println!(
            "  {} {}",
            "empty".dimmed(),
            empty.join(&format!(" {} ", ui::glyphs().sep)).dimmed()
        );
        println!();
    }
}

fn task_line(t: &EntityRecord, extra: String) -> String {
    let title = ui::clean(frontmatter::get_str_or(&t.frontmatter, "title", ""));
    let line = format!("{}  {}  {}", ui::clean(&t.id).cyan(), title, extra);
    let line = line.trim_end().to_string();
    match ui::get().width {
        Some(w) => ui::truncate(&line, w.saturating_sub(6)),
        None => line,
    }
}

fn print_focus(
    in_progress: &[&EntityRecord],
    overdue: &[&EntityRecord],
    next: Option<&EntityRecord>,
    today: &str,
) {
    if in_progress.is_empty() && overdue.is_empty() && next.is_none() {
        return;
    }
    let g = ui::glyphs();
    println!("  {}", ui::section("Focus"));
    for t in overdue.iter().take(FOCUS) {
        let due = crate::commands::list::due_cell(t, today);
        println!(
            "    {} {}",
            g.warn.red().bold(),
            task_line(t, format!("{} {due}", "due".dimmed()))
        );
    }
    let in_progress: Vec<&&EntityRecord> = in_progress
        .iter()
        .filter(|t| !overdue.iter().any(|o| o.id == t.id))
        .collect();
    for t in in_progress.iter().take(FOCUS) {
        let owner = ui::clean(frontmatter::get_str_or(&t.frontmatter, "owner", ""));
        let extra = if owner.is_empty() {
            String::new()
        } else {
            format!("@{owner}").dimmed().to_string()
        };
        println!(
            "    {} {}",
            ui::tint(if ui::get().unicode { "◐" } else { "~" }, ui::Tone::Warn),
            task_line(t, extra)
        );
    }
    if let Some(t) = next {
        println!(
            "    {} {}",
            g.arrow.green().bold(),
            task_line(t, "next up".dimmed().to_string())
        );
    }
    let hidden = overdue.len().saturating_sub(FOCUS) + in_progress.len().saturating_sub(FOCUS);
    if hidden > 0 {
        println!("    {}", format!("+{hidden} more").dimmed());
    }
    println!();
}

fn print_agenda(agenda: &Agenda, today: &str) {
    if agenda.upcoming.is_empty() && agenda.past_scheduled == 0 {
        return;
    }
    println!("  {}", ui::section("Coming up"));
    let tomorrow = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d")
        .map(|d| {
            (d + chrono::Duration::days(1))
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_default();
    let when: Vec<String> = agenda
        .upcoming
        .iter()
        .map(|m| {
            let date = frontmatter::get_str_or(&m.frontmatter, "date", "");
            let day = if date == today {
                "today"
            } else if date == tomorrow {
                "tomorrow"
            } else {
                date
            };
            let time = frontmatter::get_str_or(&m.frontmatter, "time", "");
            ui::clean(format!("{day} {time}").trim_end()).into_owned()
        })
        .collect();
    let when_w = when.iter().map(|w| ui::width_of(w)).max().unwrap_or(0);
    for (m, when) in agenda.upcoming.iter().zip(&when) {
        let title = ui::clean(frontmatter::get_str_or(&m.frontmatter, "title", ""));
        let line = format!(
            "    {}  {}  {}",
            ui::pad(when, when_w),
            ui::clean(&m.id).cyan(),
            title
        );
        match ui::get().width {
            Some(w) => println!("{}", ui::truncate(line.trim_end(), w)),
            None => println!("{}", line.trim_end()),
        }
    }
    if agenda.past_scheduled > 0 {
        println!(
            "    {}",
            format!(
                "{} still scheduled",
                ui::count(agenda.past_scheduled, "past meeting", "past meetings")
            )
            .dimmed()
        );
        ui::hint(format!(
            "review them with {}",
            ui::cmd("mc list meetings --status scheduled")
        ));
    }
    println!();
}

fn print_recent(cfg: &ResolvedConfig, recent: &[data::RecentFile]) {
    if recent.is_empty() {
        return;
    }
    println!("  {}", ui::section("Recent"));
    let ages: Vec<String> = recent
        .iter()
        .map(|f| ui::relative_time(f.modified))
        .collect();
    let age_w = ages.iter().map(|a| a.len()).max().unwrap_or(0);
    for (f, age) in recent.iter().zip(ages) {
        let label = if f.id.is_empty() {
            let path = f
                .path
                .strip_prefix(&cfg.root)
                .unwrap_or(&f.path)
                .display()
                .to_string();
            ui::clean(&path).dimmed().to_string()
        } else {
            ui::clean(&f.id).cyan().to_string()
        };
        let line = format!(
            "    {}  {}  {}",
            ui::pad_left(&age, age_w).dimmed(),
            label,
            ui::clean(&f.name)
        );
        match ui::get().width {
            Some(w) => println!("{}", ui::truncate(line.trim_end(), w)),
            None => println!("{}", line.trim_end()),
        }
    }
    println!();
}

/// The focus lists for `--json`.
struct Focus<'a> {
    in_progress: &'a [&'a EntityRecord],
    overdue: &'a [&'a EntityRecord],
    next: Option<&'a EntityRecord>,
    agenda: &'a Agenda<'a>,
}

fn print_json(
    cfg: &ResolvedConfig,
    sections: &[(EntityKind, StatusCounts)],
    recent: &[data::RecentFile],
    focus: Focus,
) -> McResult<()> {
    let mut counts = serde_json::Map::new();
    for (kind, c) in sections {
        let by_status: serde_json::Map<String, JsonValue> = c
            .by_status
            .iter()
            .map(|(s, n)| (s.clone(), JsonValue::from(*n)))
            .collect();
        counts.insert(
            kind.label_plural().to_string(),
            json!({ "total": c.total, "by_status": by_status }),
        );
    }
    let rel = |p: &std::path::Path| p.strip_prefix(&cfg.root).unwrap_or(p).display().to_string();
    let recent: Vec<JsonValue> = recent
        .iter()
        .map(|f| {
            json!({
                "id": f.id,
                "name": f.name,
                "path": rel(&f.path),
                "modified": chrono::DateTime::<chrono::Utc>::from(f.modified).to_rfc3339(),
            })
        })
        .collect();
    let ids = |v: &[&EntityRecord]| v.iter().map(|t| t.id.clone()).collect::<Vec<_>>();
    let upcoming: Vec<JsonValue> = focus
        .agenda
        .upcoming
        .iter()
        .map(|m| {
            let get = |key: &str| frontmatter::get_str_or(&m.frontmatter, key, "");
            json!({
                "id": m.id,
                "title": get("title"),
                "date": get("date"),
                "time": get("time"),
            })
        })
        .collect();
    let out = json!({
        "repo": {
            "name": repo_name(cfg),
            "mode": mode_label(cfg),
            "root": cfg.root.display().to_string(),
        },
        "counts": counts,
        "focus": {
            "in_progress": ids(focus.in_progress),
            "overdue": ids(focus.overdue),
            "next": focus.next.map(|t| t.id.clone()),
            "upcoming_meetings": upcoming,
            "past_scheduled_meetings": focus.agenda.past_scheduled,
        },
        "recent": recent,
    });
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meeting(id: &str, date: &str, time: &str, status: &str) -> EntityRecord {
        EntityRecord {
            kind: EntityKind::Meeting,
            id: id.into(),
            frontmatter: serde_yaml::from_str(&format!(
                "id: {id}\ndate: {date}\ntime: '{time}'\nstatus: {status}"
            ))
            .unwrap(),
            body: String::new(),
            source_path: format!("/m/{id}.md").into(),
        }
    }

    #[test]
    fn agenda_lists_the_next_week_and_counts_stale_meetings() {
        let meetings = vec![
            meeting("MTG-001", "2026-10-01", "10:00", "scheduled"),
            meeting("MTG-002", "2026-10-02", "10:00", "completed"),
            meeting("MTG-003", "2026-10-09", "09:00", "scheduled"),
            meeting("MTG-004", "2026-10-07", "15:00", "scheduled"),
            meeting("MTG-005", "2026-10-07", "08:30", "scheduled"),
            meeting("MTG-006", "2026-10-08", "10:00", "cancelled"),
            meeting("MTG-007", "2026-10-20", "10:00", "scheduled"),
        ];
        let agenda = Agenda::of(&meetings, "2026-10-07");
        let ids: Vec<&str> = agenda.upcoming.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["MTG-005", "MTG-004", "MTG-003"]);
        assert_eq!(agenda.past_scheduled, 1);

        // A quiet week still shows the next meeting.
        let agenda = Agenda::of(&meetings, "2026-10-10");
        let ids: Vec<&str> = agenda.upcoming.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["MTG-007"]);
        assert_eq!(agenda.past_scheduled, 4);
    }
}
