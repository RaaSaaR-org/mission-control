use crate::config::{RepoMode, ResolvedBrand, ResolvedConfig, DEFAULT_ACCENT, DEFAULT_PRIMARY};
use crate::data::{self, EntityRecord, RecentFile, StatusCounts};
use crate::entity::EntityKind;
use crate::frontmatter;
use chrono::NaiveDate;
use regex::Regex;
use serde_yaml::Value;
use std::collections::{BTreeSet, HashSet, HashMap};

/// A section of related entities to display on a detail page.
pub struct RelatedSection {
    pub title: String,
    pub kind: EntityKind,
    pub entities: Vec<EntityRecord>,
}

/// Filter options derived from all tasks for populating dropdowns.
pub struct TaskFilterOptions {
    pub owners: Vec<String>,
    pub projects: Vec<String>,
    pub sprints: Vec<String>,
}

impl TaskFilterOptions {
    pub fn from_tasks(tasks: &[EntityRecord]) -> Self {
        let mut owners = BTreeSet::new();
        let mut projects = BTreeSet::new();
        let mut sprints = BTreeSet::new();
        for t in tasks {
            let owner = frontmatter::get_str_or(&t.frontmatter, "owner", "");
            if !owner.is_empty() {
                owners.insert(owner.to_string());
            }
            for p in frontmatter::get_link_list(&t.frontmatter, "projects") {
                projects.insert(p);
            }
            let sprint = frontmatter::get_str_or(&t.frontmatter, "sprint", "");
            let sprint = frontmatter::strip_wikilink(sprint);
            if !sprint.is_empty() {
                sprints.insert(sprint.to_string());
            }
        }
        Self {
            owners: owners.into_iter().collect(),
            projects: projects.into_iter().collect(),
            sprints: sprints.into_iter().collect(),
        }
    }
}

static SIMPLE_CSS: &str = include_str!("assets/simple.min.css");
static CUSTOM_CSS: &str = include_str!("assets/custom.css");

/// Rewrite absolute URLs in generated HTML to include a base path prefix.
/// No-op when base_path is empty.
pub fn prefix_base_path(html: &str, base_path: &str) -> String {
    if base_path.is_empty() {
        return html.to_string();
    }
    html.replace("href=\"/", &format!("href=\"{}/", base_path))
        .replace("src=\"/", &format!("src=\"{}/", base_path))
        .replace("action=\"/", &format!("action=\"{}/", base_path))
        .replace("url(\"/", &format!("url(\"{}/", base_path))
}

/// Wrap body HTML in a full HTML page (default brand, standalone mode).
pub fn layout(title: &str, active_nav: &str, body_html: &str) -> String {
    layout_branded(
        title,
        active_nav,
        body_html,
        &RepoMode::Standalone,
        &HashSet::new(),
        &default_brand(),
        "",
    )
}

fn default_brand() -> ResolvedBrand {
    ResolvedBrand {
        name: "MissionControl".into(),
        tagline: String::new(),
        fonts_dir: None,
        font_name: "LiberationSans".into(),
        primary_color: DEFAULT_PRIMARY,
        accent_color: DEFAULT_ACCENT,
        logo: None,
        custom_css: None,
    }
}

/// Generate CSS overrides from brand colors.
pub fn brand_css(brand: &ResolvedBrand) -> String {
    if brand.primary_color == DEFAULT_PRIMARY && brand.accent_color == DEFAULT_ACCENT {
        return String::new();
    }

    let [pr, pg, pb] = brand.primary_color;
    let [ar, ag, ab] = brand.accent_color;

    // Darken for text variants
    let darken = |r: u8, g: u8, b: u8, f: f32| -> (u8, u8, u8) {
        (
            (r as f32 * f) as u8,
            (g as f32 * f) as u8,
            (b as f32 * f) as u8,
        )
    };
    // Lighten for dark mode
    let lighten = |r: u8, g: u8, b: u8, f: f32| -> (u8, u8, u8) {
        let l = |v: u8| -> u8 { (v as f32 + (255.0 - v as f32) * f) as u8 };
        (l(r), l(g), l(b))
    };

    let (pdr, pdg, pdb) = darken(pr, pg, pb, 0.7);
    let (plr, plg, plb) = lighten(pr, pg, pb, 0.35);
    let (adr, adg, adb) = darken(ar, ag, ab, 0.7);
    let (alr, alg, alb) = lighten(ar, ag, ab, 0.35);

    format!(
        r#"<style>
:root {{
  --mc-blue: rgb({pr},{pg},{pb});
  --mc-blue-bg: rgba({pr},{pg},{pb},0.1);
  --mc-blue-text: rgb({pdr},{pdg},{pdb});
  --mc-amber: rgb({ar},{ag},{ab});
  --mc-amber-bg: rgba({ar},{ag},{ab},0.1);
  --mc-amber-text: rgb({adr},{adg},{adb});
}}
@media (prefers-color-scheme: dark) {{
  :root {{
    --mc-blue: rgb({plr},{plg},{plb});
    --mc-blue-bg: rgba({pr},{pg},{pb},0.15);
    --mc-blue-text: rgb({plr},{plg},{plb});
    --mc-amber: rgb({alr},{alg},{alb});
    --mc-amber-bg: rgba({ar},{ag},{ab},0.15);
    --mc-amber-text: rgb({alr},{alg},{alb});
  }}
}}
</style>"#
    )
}

/// Generate @font-face CSS for brand fonts.
pub fn font_face_css(brand: &ResolvedBrand) -> String {
    let fonts_dir = match &brand.fonts_dir {
        Some(d) => d,
        None => return String::new(),
    };

    let mut faces = String::new();
    let font_name = &brand.font_name;

    if let Ok(entries) = std::fs::read_dir(fonts_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let lower = name.to_lowercase();
            if !lower.ends_with(".ttf") && !lower.ends_with(".woff2") {
                continue;
            }
            let (weight, style) = if lower.contains("bolditalic") {
                ("700", "italic")
            } else if lower.contains("bold") {
                ("700", "normal")
            } else if lower.contains("italic") {
                ("400", "italic")
            } else {
                ("400", "normal")
            };
            let format = if lower.ends_with(".woff2") {
                "woff2"
            } else {
                "truetype"
            };
            faces.push_str(&format!(
                r#"@font-face {{ font-family: "{font_name}"; src: url("/brand/fonts/{name}") format("{format}"); font-weight: {weight}; font-style: {style}; font-display: swap; }}
"#
            ));
        }
    }

    if faces.is_empty() {
        return String::new();
    }

    format!(
        r#"<style>
{faces}body {{ font-family: "{font_name}", -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif; }}
</style>"#
    )
}

/// Full branded layout with mode, brand, and optional custom CSS content.
#[allow(clippy::too_many_arguments)]
pub fn layout_branded(
    title: &str,
    active_nav: &str,
    body_html: &str,
    mode: &RepoMode,
    configured_entities: &HashSet<String>,
    brand: &ResolvedBrand,
    custom_css_content: &str,
) -> String {
    let all_nav_items: Vec<(&str, &str, Option<EntityKind>)> = vec![
        ("Dashboard", "/", None),
        ("Customers", "/customers", Some(EntityKind::Customer)),
        ("Projects", "/projects", Some(EntityKind::Project)),
        ("Meetings", "/meetings", Some(EntityKind::Meeting)),
        ("Research", "/research", Some(EntityKind::Research)),
        ("Tasks", "/tasks", Some(EntityKind::Task)),
        ("Sprints", "/sprints", Some(EntityKind::Sprint)),
        ("Proposals", "/proposals", Some(EntityKind::Proposal)),
        ("Contacts", "/contacts", Some(EntityKind::Contact)),
    ];

    // Build a temporary ResolvedConfig-like check using mode + configured_entities
    let check_available = |kind: &EntityKind| -> bool {
        if *mode == RepoMode::Embedded {
            return matches!(
                kind,
                EntityKind::Task
                    | EntityKind::Meeting
                    | EntityKind::Research
                    | EntityKind::Sprint
                    | EntityKind::Proposal
            );
        }
        if configured_entities.is_empty() {
            return true;
        }
        let plural = kind.label_plural();
        let singular = kind.label();
        configured_entities.contains(plural) || configured_entities.contains(singular)
    };

    let nav_links: String = all_nav_items
        .iter()
        .filter(|(_, _, kind)| match kind {
            None => true,
            Some(k) => check_available(k),
        })
        .map(|(label, href, _)| {
            let class = if *href == active_nav {
                " class=\"active\""
            } else {
                ""
            };
            format!(r#"<a href="{}"{}>{}</a>"#, href, class, label)
        })
        .collect::<Vec<_>>()
        .join("\n          ");

    let brand_name = escape_html(&brand.name);
    let brand_css_block = brand_css(brand);
    let font_css_block = font_face_css(brand);
    let custom_css_block = if custom_css_content.is_empty() {
        String::new()
    } else {
        format!("<style>{}</style>", custom_css_content)
    };

    let logo_html = if brand.logo.is_some() {
        r#"<img src="/brand/logo" alt="" class="brand-logo">"#
    } else {
        ""
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <link rel="icon" href="data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'><text y='.9em' font-size='90'>📋</text></svg>">
  <title>{title} - {brand_name}</title>
  <style>{SIMPLE_CSS}</style>
  <style>{CUSTOM_CSS}</style>
  {brand_css_block}
  {font_css_block}
  {custom_css_block}
</head>
<body>
  <header>
    <h1>{logo_html}{brand_name}</h1>
    <input type="checkbox" id="nav-toggle" class="nav-toggle" aria-hidden="true">
    <label for="nav-toggle" class="nav-toggle-label" aria-label="Toggle navigation"><span></span></label>
    <nav>
      <ul>
        {nav_links}
      </ul>
    </nav>
  </header>
  <main>
    {body_html}
  </main>
  <footer>
    <p>{brand_name} &mdash; <code>mc serve</code></p>
  </footer>
</body>
</html>"#,
    )
}

/// Render a colored status badge.
pub fn status_badge(status: &str) -> String {
    let class = match status {
        "active" | "completed" | "final" | "done" => format!("badge badge-{}", status),
        "inactive" | "cancelled" | "churned" | "outdated" => format!("badge badge-{}", status),
        "on-hold" | "draft" | "in-progress" | "review" | "planning" => {
            format!("badge badge-{}", status)
        }
        "prospect" | "scheduled" | "todo" => format!("badge badge-{}", status),
        "backlog" => format!("badge badge-{}", status),
        _ => "badge".to_string(),
    };
    format!(r#"<span class="{}">{}</span>"#, class, escape_html(status))
}

/// Render tag badges.
pub fn tag_badges(tags: &[String]) -> String {
    if tags.is_empty() {
        return String::new();
    }
    tags.iter()
        .map(|t| format!(r#"<span class="tag">{}</span>"#, escape_html(t)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Render a link to an entity by ID.
pub fn entity_link(id: &str) -> String {
    format!(
        r#"<a href="/entity/{}" class="entity-link">{}</a>"#,
        escape_html(id),
        escape_html(id)
    )
}

/// Render markdown to HTML, auto-linking entity IDs.
pub fn render_markdown(md: &str, prefixes: &[&str]) -> String {
    use pulldown_cmark::{html, Options, Parser};

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(md, options);
    let mut html_output = String::new();
    html::push_html(&mut html_output, parser);

    // Auto-link entity IDs (e.g., CUST-001, PROJ-002)
    auto_link_entity_ids(&html_output, prefixes)
}

/// Replace entity ID patterns in HTML with links (but not inside existing <a> tags).
fn auto_link_entity_ids(html: &str, prefixes: &[&str]) -> String {
    if prefixes.is_empty() {
        return html.to_string();
    }
    let prefix_pattern = prefixes
        .iter()
        .map(|p| regex::escape(p))
        .collect::<Vec<_>>()
        .join("|");
    let pattern = format!(r"\b(?:{})-\d{{3}}\b", prefix_pattern);
    let re = Regex::new(&pattern).expect("regex with escaped prefixes is always valid");

    // Simple approach: split by <a...>...</a> tags to avoid linking inside existing links
    let link_re = Regex::new(r"<a[^>]*>.*?</a>").expect("static regex pattern is always valid");
    let mut result = String::new();
    let mut last_end = 0;

    for m in link_re.find_iter(html) {
        // Process text before this link
        let before = &html[last_end..m.start()];
        result.push_str(&re.replace_all(before, |caps: &regex::Captures| {
            let id = &caps[0];
            format!(r#"<a href="/entity/{}" class="entity-link">{}</a>"#, id, id)
        }));
        // Keep the existing link as-is
        result.push_str(m.as_str());
        last_end = m.end();
    }
    // Process remaining text after the last link
    let remaining = &html[last_end..];
    result.push_str(&re.replace_all(remaining, |caps: &regex::Captures| {
        let id = &caps[0];
        format!(r#"<a href="/entity/{}" class="entity-link">{}</a>"#, id, id)
    }));

    result
}

/// CSS class for a status bar segment.
fn segment_class(status: &str) -> String {
    match status {
        "active" | "completed" | "final" | "done" | "inactive" | "cancelled" | "churned"
        | "outdated" | "on-hold" | "draft" | "in-progress" | "review" | "prospect"
        | "scheduled" | "todo" | "backlog" | "planning" => format!("seg-{}", status),
        _ => "seg-unknown".to_string(),
    }
}

/// Extract initials from a name (e.g. "John Doe" -> "JD", "alice" -> "A").
fn initials(name: &str) -> String {
    let parts: Vec<&str> = name.split_whitespace().collect();
    match parts.len() {
        0 => String::new(),
        1 => parts[0]
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default(),
        _ => {
            let first = parts[0]
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default();
            let last = parts
                .last()
                .unwrap()
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default();
            format!("{}{}", first, last)
        }
    }
}

/// Insights extracted from tasks for the dashboard.
pub struct TaskInsights {
    pub overdue: Vec<(String, String, String)>,
    pub due_this_week: Vec<(String, String, String)>,
    pub in_progress: usize,
    pub in_review: usize,
    pub done: usize,
    pub total_active: usize,
}

impl TaskInsights {
    pub fn from_tasks(tasks: &[EntityRecord]) -> Self {
        let today = chrono::Local::now().date_naive();
        let week_end = today + chrono::Duration::days(7);

        let mut overdue = Vec::new();
        let mut due_this_week = Vec::new();
        let mut in_progress: usize = 0;
        let mut in_review: usize = 0;
        let mut done: usize = 0;
        let mut total_active: usize = 0;

        for task in tasks {
            let status = frontmatter::get_str_or(&task.frontmatter, "status", "");
            let title = frontmatter::get_str_or(&task.frontmatter, "title", "");
            let due_date = frontmatter::get_str_or(&task.frontmatter, "due_date", "");

            if status == "cancelled" {
                continue;
            }
            total_active += 1;

            match status {
                "in-progress" => in_progress += 1,
                "review" => in_review += 1,
                "done" => done += 1,
                _ => {}
            }

            if status == "done" {
                continue;
            }

            if let Ok(due) = NaiveDate::parse_from_str(due_date, "%Y-%m-%d") {
                let id = frontmatter::get_str_or(&task.frontmatter, "id", "");
                let entry = (id.to_string(), title.to_string(), due_date.to_string());
                if due < today {
                    overdue.push(entry);
                } else if due <= week_end {
                    due_this_week.push(entry);
                }
            }
        }

        overdue.truncate(5);
        due_this_week.truncate(5);

        Self {
            overdue,
            due_this_week,
            in_progress,
            in_review,
            done,
            total_active,
        }
    }
}

/// Render the dashboard page.
pub fn dashboard_page(
    counts: &[StatusCounts],
    recent: &[RecentFile],
    task_insights: &TaskInsights,
    cfg: &ResolvedConfig,
    custom_css: &str,
) -> String {
    let mode = &cfg.mode;
    let brand = &cfg.brand;
    let repo_root = &cfg.root;
    let mut body = String::new();

    // Summary card grid — with inline mini stacked bar
    let total_entities: usize = counts.iter().map(|c| c.total).sum();
    body.push_str(&format!(
        r#"<div class="dashboard-hero"><span class="hero-count">{}</span> entities tracked</div>"#,
        total_entities
    ));
    body.push('\n');
    body.push_str(r#"<div class="summary-grid">"#);
    body.push('\n');
    for sc in counts {
        let muted = if sc.total == 0 {
            " summary-card-empty"
        } else {
            ""
        };
        let icon = match sc.label.as_str() {
            "customers" => "👥",
            "projects" => "📐",
            "meetings" => "📅",
            "research" => "🔬",
            "tasks" => "✅",
            "sprints" => "🏃",
            "proposals" => "💡",
            "contacts" => "📇",
            _ => "📋",
        };
        body.push_str(&format!(
            r#"<a href="/{}" class="summary-card card-type-{}{}">
  <div class="summary-card-icon">{}</div>
  <div class="summary-card-body">
    <div class="summary-card-label">{}</div>
    <div class="summary-card-count">{}</div>
  </div>"#,
            sc.label,
            sc.label,
            muted,
            icon,
            capitalize(&sc.label),
            sc.total
        ));
        // Mini stacked bar inside card
        if sc.total > 0 && !sc.by_status.is_empty() {
            body.push_str(r#"<div class="card-bar">"#);
            for (status, count) in &sc.by_status {
                if *count == 0 {
                    continue;
                }
                let pct = (*count as f64 / sc.total as f64 * 100.0).max(2.0);
                body.push_str(&format!(
                    r#"<div class="card-bar-seg {}" style="width:{:.1}%" title="{}: {}"></div>"#,
                    segment_class(status),
                    pct,
                    escape_html(status),
                    count
                ));
            }
            body.push_str("</div>\n");
            // Compact breakdown
            body.push_str(r#"<div class="summary-card-breakdown">"#);
            for (status, count) in &sc.by_status {
                body.push_str(&format!(
                    r#"<span class="badge badge-{}">{} {}</span>"#,
                    escape_html(status),
                    count,
                    escape_html(status)
                ));
            }
            body.push_str("</div>\n");
        }
        body.push_str("</a>\n");
    }
    body.push_str("</div>\n");

    // Task insights section
    if task_insights.total_active > 0 {
        body.push_str("<div class=\"dashboard-insights\">\n");

        // Progress bar
        let pct = if task_insights.total_active > 0 {
            (task_insights.done as f64 / task_insights.total_active as f64 * 100.0).round() as u32
        } else {
            0
        };
        body.push_str(&format!(
            r#"<div class="insights-progress">
  <div class="insights-progress-label">
    <span>Task Progress</span>
    <span>{}/{} done</span>
  </div>
  <div class="insights-bar">
    <div class="insights-bar-fill" style="width:{}%"></div>
  </div>
</div>
"#,
            task_insights.done, task_insights.total_active, pct
        ));

        // Status pills
        body.push_str("<div class=\"insights-statuses\">\n");
        body.push_str(&format!(
            "<span class=\"insights-pill pill-progress\">{} in progress</span>\n",
            task_insights.in_progress
        ));
        body.push_str(&format!(
            "<span class=\"insights-pill pill-review\">{} in review</span>\n",
            task_insights.in_review
        ));
        body.push_str("</div>\n");

        // Overdue tasks
        if !task_insights.overdue.is_empty() {
            body.push_str("<div class=\"insights-section insights-overdue\">\n");
            body.push_str("<h4>\u{26a0} Overdue</h4>\n<ul>\n");
            for (id, title, due_date) in &task_insights.overdue {
                body.push_str(&format!(
                    "<li><a href=\"/entity/{}\">{}</a> {} <span class=\"due-date date-overdue\">{}</span></li>\n",
                    escape_html(id),
                    escape_html(id),
                    escape_html(title),
                    escape_html(due_date)
                ));
            }
            body.push_str("</ul>\n</div>\n");
        }

        // Due this week
        if !task_insights.due_this_week.is_empty() {
            body.push_str("<div class=\"insights-section insights-upcoming\">\n");
            body.push_str("<h4>\u{1f4c5} Due This Week</h4>\n<ul>\n");
            for (id, title, due_date) in &task_insights.due_this_week {
                body.push_str(&format!(
                    "<li><a href=\"/entity/{}\">{}</a> {} <span class=\"due-date date-soon\">{}</span></li>\n",
                    escape_html(id),
                    escape_html(id),
                    escape_html(title),
                    escape_html(due_date)
                ));
            }
            body.push_str("</ul>\n</div>\n");
        }

        body.push_str("</div>\n");
    }

    // Recent activity as timeline
    body.push_str(r#"<div class="dashboard-activity">"#);
    body.push_str(&format!(
        r#"<h3>Recent Activity <span class="activity-count">{}</span></h3>"#,
        recent.len()
    ));
    body.push('\n');
    if recent.is_empty() {
        body.push_str(r#"<p style="color:var(--mc-text-muted);font-size:0.875rem;">No recent files found.</p>"#);
        body.push('\n');
    } else {
        body.push_str(r#"<ul class="activity-timeline">"#);
        body.push('\n');
        for f in recent {
            // Detect entity type from ID prefix or fall back to path heuristics
            let entity_type = detect_entity_type(&f.id, &f.path, cfg);
            let type_badge = format!(
                r#"<span class="activity-type-badge badge-type-{}">{}</span>"#,
                entity_type, entity_type
            );

            // Primary display: entity link or name
            let primary = if !f.id.is_empty() {
                entity_link(&f.id)
            } else if !f.name.is_empty() {
                format!(
                    r#"<span class="activity-name">{}</span>"#,
                    escape_html(&f.name)
                )
            } else {
                let rel = f
                    .path
                    .strip_prefix(repo_root)
                    .unwrap_or(&f.path)
                    .display()
                    .to_string();
                format!(
                    r#"<span class="activity-path">{}</span>"#,
                    escape_html(&rel)
                )
            };

            // Secondary: name + optional path context (e.g. customer name)
            let mut secondary = String::new();
            if !f.id.is_empty() && !f.name.is_empty() {
                secondary.push_str(&format!(
                    r#" <span class="activity-name">{}</span>"#,
                    escape_html(&f.name)
                ));
            }
            let ctx = extract_path_context(&f.path, repo_root);
            if !ctx.is_empty() {
                secondary.push_str(&format!(
                    r#" <span class="activity-context">{}</span>"#,
                    escape_html(&ctx)
                ));
            }

            body.push_str(&format!(
                "<li>{} {}{}</li>\n",
                type_badge, primary, secondary
            ));
        }
        body.push_str("</ul>\n");
    }
    body.push_str("</div>\n");

    layout_branded("Dashboard", "/", &body, mode, &cfg.configured_entities, brand, custom_css)
}

/// Detect entity type label from an entity ID and file path, using configured prefixes.
fn detect_entity_type(id: &str, path: &std::path::Path, cfg: &ResolvedConfig) -> &'static str {
    if !id.is_empty() {
        // Match against configured prefixes (longest prefix wins to avoid e.g. "TASK" matching "TASK-001" before "TSK-001")
        let candidates: &[(&str, &'static str)] = &[
            (&cfg.id_prefixes.customer, EntityKind::Customer.label()),
            (&cfg.id_prefixes.project, EntityKind::Project.label()),
            (&cfg.id_prefixes.meeting, EntityKind::Meeting.label()),
            (&cfg.id_prefixes.research, EntityKind::Research.label()),
            (&cfg.id_prefixes.task, EntityKind::Task.label()),
            (&cfg.id_prefixes.sprint, EntityKind::Sprint.label()),
            (&cfg.id_prefixes.proposal, EntityKind::Proposal.label()),
            (&cfg.id_prefixes.contact, EntityKind::Contact.label()),
        ];
        // Sort candidates by prefix length descending so longer prefixes match first
        let mut sorted = candidates.to_vec();
        sorted.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        for (prefix, label) in &sorted {
            let needle = format!("{}-", prefix);
            if id.starts_with(needle.as_str()) {
                return label;
            }
        }
    }
    // Fall back to path-based detection using standard directory segment names
    let path_str = path.to_string_lossy();
    if path_str.contains("/contacts/") || path_str.contains("/team/") {
        "contact"
    } else if path_str.contains("/customers/") {
        "customer"
    } else if path_str.contains("/projects/") {
        "project"
    } else if path_str.contains("/meetings/") {
        "meeting"
    } else if path_str.contains("/research/") {
        "research"
    } else if path_str.contains("/tasks/")
        || path_str.contains("/todo/")
        || path_str.contains("/done/")
    {
        "task"
    } else if path_str.contains("/sprints/") {
        "sprint"
    } else if path_str.contains("/proposals/") {
        "proposal"
    } else {
        "file"
    }
}

/// Extract a short context string from a path (e.g. parent customer name).
fn extract_path_context(path: &std::path::Path, root: &std::path::Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let rel_str = rel.to_string_lossy();
    let parts: Vec<&str> = rel_str.split('/').collect();

    // For files inside customer directories, extract the customer slug
    // e.g. customers/CUST-001-thyssenkrupp/contacts/foo.md → "thyssenkrupp"
    for (i, part) in parts.iter().enumerate() {
        if *part == "customers" || *part == "projects" {
            if let Some(parent) = parts.get(i + 1) {
                // Extract readable name: strip ID prefix (e.g. "CUST-001-" or "PROJ-002-")
                let name = strip_id_prefix(parent);
                if !name.is_empty() {
                    return name.replace('-', " ");
                }
            }
        }
    }
    String::new()
}

/// Strip entity ID prefix from a directory name (e.g. "CUST-001-thyssenkrupp" → "thyssenkrupp").
fn strip_id_prefix(dirname: &str) -> String {
    // Pattern: PREFIX-NNN-slug
    let re = Regex::new(r"^[A-Z]+-\d+-(.+)$").expect("static regex");
    if let Some(caps) = re.captures(dirname) {
        caps[1].to_string()
    } else {
        dirname.to_string()
    }
}

/// Sort entities by a frontmatter field.
pub fn sort_entities(entities: &mut [EntityRecord], field: &str, dir: &str) {
    let descending = dir == "desc";
    entities.sort_by(|a, b| {
        let va = frontmatter::get_str_or(&a.frontmatter, field, "");
        let vb = frontmatter::get_str_or(&b.frontmatter, field, "");
        let cmp = va.to_lowercase().cmp(&vb.to_lowercase());
        if descending {
            cmp.reverse()
        } else {
            cmp
        }
    });
}

/// Render a list page for a given entity kind.
#[allow(clippy::too_many_arguments)]
pub fn list_page(
    kind_plural: &str,
    entities: &[EntityRecord],
    status_filter: Option<&str>,
    tag_filter: Option<&str>,
    valid_statuses: &[String],
    sort_field: Option<&str>,
    sort_dir: &str,
    mode: &RepoMode,
    configured_entities: &HashSet<String>,
    brand: &ResolvedBrand,
    custom_css: &str,
) -> String {
    let nav_path = format!("/{}", kind_plural);
    let mut body = String::new();

    body.push_str(&format!("<h2 class=\"page-title\">{}</h2>\n", capitalize(kind_plural)));

    // Filter form
    body.push_str(&format!(
        r#"<form class="filter-form" method="get" action="/{}">
  <div>
    <label for="status">Status</label>
    <select name="status" id="status">
      <option value="">All</option>
"#,
        kind_plural
    ));
    for s in valid_statuses {
        let selected = if status_filter == Some(s.as_str()) {
            " selected"
        } else {
            ""
        };
        body.push_str(&format!(
            "      <option value=\"{}\"{}>{}</option>\n",
            escape_html(s),
            selected,
            escape_html(s)
        ));
    }
    body.push_str("    </select>\n  </div>\n  <div>\n");
    body.push_str(&format!(
        r#"    <label for="tag">Tag</label>
    <input type="text" name="tag" id="tag" placeholder="filter by tag" value="{}">
  </div>
  <div>
    <button type="submit">Filter</button>
  </div>
  <a href="/{}" class="reset-link">Reset</a>
</form>
"#,
        escape_html(tag_filter.unwrap_or("")),
        kind_plural,
    ));

    if entities.is_empty() {
        body.push_str(r#"<div class="empty-state"><span class="empty-state-icon">~</span>No entities match your filters.</div>"#);
        body.push('\n');
        return layout_branded(
            &capitalize(kind_plural),
            &nav_path,
            &body,
            mode,
            configured_entities,
            brand,
            custom_css,
        );
    }

    // Determine which columns to show based on kind
    let is_meeting = kind_plural == "meetings";
    let is_sprint = kind_plural == "sprints";

    // Helper to build sortable header params
    let base_params = build_filter_params(status_filter, tag_filter);

    body.push_str(r#"<div class="table-wrap"><table>"#);
    body.push_str("\n<thead><tr>");
    body.push_str(&sortable_th(
        "ID",
        "id",
        sort_field,
        sort_dir,
        kind_plural,
        &base_params,
    ));
    if is_meeting {
        body.push_str(&sortable_th(
            "Title",
            "title",
            sort_field,
            sort_dir,
            kind_plural,
            &base_params,
        ));
        body.push_str(&sortable_th(
            "Date",
            "date",
            sort_field,
            sort_dir,
            kind_plural,
            &base_params,
        ));
        body.push_str("<th>Time</th>");
        body.push_str(&sortable_th(
            "Status",
            "status",
            sort_field,
            sort_dir,
            kind_plural,
            &base_params,
        ));
        body.push_str("<th>Tags</th>");
    } else if is_sprint {
        body.push_str(&sortable_th(
            "Title",
            "title",
            sort_field,
            sort_dir,
            kind_plural,
            &base_params,
        ));
        body.push_str(&sortable_th(
            "Status",
            "status",
            sort_field,
            sort_dir,
            kind_plural,
            &base_params,
        ));
        body.push_str(&sortable_th(
            "Start",
            "start_date",
            sort_field,
            sort_dir,
            kind_plural,
            &base_params,
        ));
        body.push_str(&sortable_th(
            "End",
            "end_date",
            sort_field,
            sort_dir,
            kind_plural,
            &base_params,
        ));
        body.push_str(&sortable_th(
            "Owner",
            "owner",
            sort_field,
            sort_dir,
            kind_plural,
            &base_params,
        ));
        body.push_str("<th>Tags</th>");
    } else {
        body.push_str(&sortable_th(
            "Name",
            "name",
            sort_field,
            sort_dir,
            kind_plural,
            &base_params,
        ));
        body.push_str(&sortable_th(
            "Status",
            "status",
            sort_field,
            sort_dir,
            kind_plural,
            &base_params,
        ));
        body.push_str(&sortable_th(
            "Owner",
            "owner",
            sort_field,
            sort_dir,
            kind_plural,
            &base_params,
        ));
        body.push_str("<th>Tags</th>");
    }
    body.push_str("</tr></thead>\n<tbody>\n");

    for e in entities {
        let id = frontmatter::get_str_or(&e.frontmatter, "id", "");
        let tags = frontmatter::get_string_list(&e.frontmatter, "tags");
        let status = frontmatter::get_str_or(&e.frontmatter, "status", "");

        body.push_str("<tr>");
        body.push_str(&format!("<td>{}</td>", entity_link(id)));

        if is_meeting {
            let title = frontmatter::get_str_or(&e.frontmatter, "title", "");
            let date = frontmatter::get_str_or(&e.frontmatter, "date", "");
            let time = frontmatter::get_str_or(&e.frontmatter, "time", "");
            body.push_str(&format!(
                "<td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td>",
                escape_html(title),
                escape_html(date),
                escape_html(time),
                status_badge(status),
                tag_badges(&tags)
            ));
        } else if is_sprint {
            let title = frontmatter::get_str_or(&e.frontmatter, "title", "");
            let start = frontmatter::get_str_or(&e.frontmatter, "start_date", "");
            let end = frontmatter::get_str_or(&e.frontmatter, "end_date", "");
            let owner = frontmatter::get_str_or(&e.frontmatter, "owner", "");
            body.push_str(&format!(
                "<td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td>",
                escape_html(title),
                status_badge(status),
                escape_html(start),
                escape_html(end),
                escape_html(owner),
                tag_badges(&tags)
            ));
        } else {
            let name = frontmatter::get_str_or(&e.frontmatter, "name", "").to_string();
            let name = if name.is_empty() {
                frontmatter::get_str_or(&e.frontmatter, "title", "").to_string()
            } else {
                name
            };
            let owner = frontmatter::get_str_or(&e.frontmatter, "owner", "");
            body.push_str(&format!(
                "<td>{}</td><td>{}</td><td>{}</td><td>{}</td>",
                escape_html(&name),
                status_badge(status),
                escape_html(owner),
                tag_badges(&tags)
            ));
        }

        body.push_str("</tr>\n");
    }

    body.push_str("</tbody></table></div>\n");
    body.push_str(&format!(
        r#"<p class="table-count"><strong>{}</strong> {} total</p>"#,
        entities.len(),
        kind_plural
    ));

    layout_branded(
        &capitalize(kind_plural),
        &nav_path,
        &body,
        mode,
        configured_entities,
        brand,
        custom_css,
    )
}

/// Build query string for filter params (to preserve when sorting).
fn build_filter_params(status: Option<&str>, tag: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(s) = status {
        parts.push(format!("status={}", escape_html(s)));
    }
    if let Some(t) = tag {
        parts.push(format!("tag={}", escape_html(t)));
    }
    if parts.is_empty() {
        String::new()
    } else {
        parts.join("&")
    }
}

/// Render a sortable table header cell.
fn sortable_th(
    label: &str,
    field: &str,
    current_sort: Option<&str>,
    current_dir: &str,
    kind_plural: &str,
    base_params: &str,
) -> String {
    let is_active = current_sort == Some(field);
    let next_dir = if is_active && current_dir == "asc" {
        "desc"
    } else {
        "asc"
    };
    let arrow = if is_active {
        if current_dir == "asc" {
            " &#9650;"
        } else {
            " &#9660;"
        }
    } else {
        ""
    };
    let sep = if base_params.is_empty() { "" } else { "&" };
    let href = format!(
        "/{}?{}{}sort={}&dir={}",
        kind_plural, base_params, sep, field, next_dir
    );
    let class = if is_active { " class=\"sorted\"" } else { "" };
    format!(
        r#"<th{}><a href="{}" class="sort-link">{}{}</a></th>"#,
        class, href, label, arrow
    )
}

/// Map a numeric priority value (1-4) to a label string.
fn priority_label(priority: u64) -> &'static str {
    match priority {
        1 => "Critical",
        2 => "High",
        3 => "Medium",
        4 => "Low",
        _ => "Unknown",
    }
}

/// CSS class suffix for priority color (maps to existing `pri-*` classes in kanban).
fn priority_class(priority: u64) -> &'static str {
    match priority {
        1 => "critical",
        2 => "high",
        3 => "medium",
        4 => "low",
        _ => "low",
    }
}

/// Render the body HTML for a task detail page (two-column layout).
fn task_detail_body(
    entity: &EntityRecord,
    prefixes: &[&str],
    related: &[RelatedSection],
    cfg: &ResolvedConfig,
) -> String {
    let mut body = String::new();

    let display_name = frontmatter::get_str(&entity.frontmatter, "title")
        .or_else(|| frontmatter::get_str(&entity.frontmatter, "name"))
        .unwrap_or(&entity.id);
    let status = frontmatter::get_str_or(&entity.frontmatter, "status", "");

    // Read task-specific metadata
    let priority_raw = entity
        .frontmatter
        .get("priority")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let owner = frontmatter::get_str_or(&entity.frontmatter, "owner", "");
    let sprint_raw = frontmatter::get_str_or(&entity.frontmatter, "sprint", "");
    let sprint = frontmatter::strip_wikilink(sprint_raw);
    let due_date = frontmatter::get_str_or(&entity.frontmatter, "due_date", "");
    let created = frontmatter::get_str_or(&entity.frontmatter, "created", "");
    let updated = frontmatter::get_str_or(&entity.frontmatter, "updated", "");

    let tags: Vec<String> = entity
        .frontmatter
        .get("tags")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let projects: Vec<String> = entity
        .frontmatter
        .get("projects")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| {
                    v.as_str()
                        .map(|s| frontmatter::strip_wikilink(s).to_string())
                })
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let customers: Vec<String> = entity
        .frontmatter
        .get("customers")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| {
                    v.as_str()
                        .map(|s| frontmatter::strip_wikilink(s).to_string())
                })
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let depends_on: Vec<String> = entity
        .frontmatter
        .get("depends_on")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    // ── Breadcrumb ────────────────────────────────────────────────────
    let kind_plural = entity.kind.label_plural();
    body.push_str(&format!(
        r#"<div class="breadcrumb"><a href="/">Dashboard</a><span class="sep">/</span><a href="/{}">{}</a><span class="sep">/</span>{}</div>"#,
        kind_plural,
        capitalize(kind_plural),
        escape_html(&entity.id),
    ));

    // ── Priority accent header bar ────────────────────────────────────
    let pri_cls = if priority_raw > 0 {
        priority_class(priority_raw)
    } else {
        "low"
    };
    body.push_str(&format!(
        r#"<div class="task-detail-card pri-{}">"#,
        pri_cls
    ));

    // ── Hero ──────────────────────────────────────────────────────────
    body.push_str(r#"<div class="task-detail-hero">"#);
    body.push_str(&format!(
        r#"<div class="task-detail-title-row">
  <h2 class="task-detail-title">{}</h2>
  <span class="entity-id task-detail-id">{}</span>
</div>
<div class="task-detail-badges">"#,
        escape_html(display_name),
        escape_html(&entity.id),
    ));

    // Status badge
    if !status.is_empty() {
        body.push_str(&status_badge(status));
    }

    // Priority badge
    if priority_raw > 0 {
        body.push_str(&format!(
            r#" <span class="badge task-pri-badge task-pri-{}">{}</span>"#,
            pri_cls,
            priority_label(priority_raw),
        ));
    }

    // Tags inline
    if !tags.is_empty() {
        body.push(' ');
        body.push_str(&tag_badges(&tags));
    }

    body.push_str("</div>\n"); // .task-detail-badges
    body.push_str("</div>\n"); // .task-detail-hero

    // ── Two-column layout: main body + sidebar ────────────────────────
    body.push_str(r#"<div class="task-detail-layout">"#);

    // ── Main column: description ──────────────────────────────────────
    body.push_str(r#"<div class="task-detail-main">"#);
    if !entity.body.trim().is_empty() {
        // Strip leading H1 that duplicates the page title
        let md = strip_leading_h1(entity.body.trim());
        body.push_str(r#"<div class="detail-body task-detail-description">"#);
        body.push_str(&render_markdown(md, prefixes));
        body.push_str("</div>\n");
    } else {
        body.push_str(r#"<p class="task-detail-no-desc">No description provided.</p>"#);
    }
    body.push_str("</div>\n"); // .task-detail-main

    // ── Sidebar: metadata ─────────────────────────────────────────────
    body.push_str(r#"<aside class="task-detail-sidebar">"#);

    // Owner
    if !owner.is_empty() {
        let init = initials(owner);
        body.push_str(&format!(
            r#"<div class="task-meta-block">
  <div class="task-meta-label">Owner</div>
  <div class="task-meta-value task-owner-row">
    <span class="owner-initials">{}</span>
    <span>{}</span>
  </div>
</div>"#,
            escape_html(&init),
            escape_html(owner),
        ));
    }

    // Project links
    if !projects.is_empty() {
        let links: Vec<String> = projects.iter().map(|id| entity_link(id)).collect();
        body.push_str(&format!(
            r#"<div class="task-meta-block">
  <div class="task-meta-label">{}</div>
  <div class="task-meta-value">{}</div>
</div>"#,
            if projects.len() == 1 {
                "Project"
            } else {
                "Projects"
            },
            links.join(", "),
        ));
    }

    // Customer links
    if !customers.is_empty() {
        let links: Vec<String> = customers.iter().map(|id| entity_link(id)).collect();
        body.push_str(&format!(
            r#"<div class="task-meta-block">
  <div class="task-meta-label">{}</div>
  <div class="task-meta-value">{}</div>
</div>"#,
            if customers.len() == 1 {
                "Customer"
            } else {
                "Customers"
            },
            links.join(", "),
        ));
    }

    // Sprint
    if !sprint.is_empty() {
        body.push_str(&format!(
            r#"<div class="task-meta-block">
  <div class="task-meta-label">Sprint</div>
  <div class="task-meta-value">{}</div>
</div>"#,
            entity_link(sprint),
        ));
    }

    // Depends on
    if !depends_on.is_empty() {
        let links: Vec<String> = depends_on
            .iter()
            .map(|id| {
                let stripped = id
                    .strip_prefix("[[")
                    .and_then(|s| s.strip_suffix("]]"))
                    .unwrap_or(id);
                entity_link(stripped)
            })
            .collect();
        body.push_str(&format!(
            r#"<div class="task-meta-block">
  <div class="task-meta-label">Depends on</div>
  <div class="task-meta-value">{}</div>
</div>"#,
            links.join(", "),
        ));
    }

    // Dates block
    let has_dates = !due_date.is_empty() || !created.is_empty() || !updated.is_empty();
    if has_dates {
        body.push_str(r#"<div class="task-meta-block task-meta-dates">"#);
        if !due_date.is_empty() {
            let urgency_class = due_date_urgency_class(due_date);
            body.push_str(&format!(
                r#"<div class="task-meta-date-row">
  <span class="task-meta-label">Due</span>
  <span class="task-date-value {}">{}</span>
</div>"#,
                urgency_class,
                escape_html(due_date),
            ));
        }
        if !created.is_empty() {
            body.push_str(&format!(
                r#"<div class="task-meta-date-row">
  <span class="task-meta-label">Created</span>
  <span class="task-date-value">{}</span>
</div>"#,
                escape_html(created),
            ));
        }
        if !updated.is_empty() {
            body.push_str(&format!(
                r#"<div class="task-meta-date-row">
  <span class="task-meta-label">Updated</span>
  <span class="task-date-value">{}</span>
</div>"#,
                escape_html(updated),
            ));
        }
        body.push_str("</div>\n"); // .task-meta-dates
    }

    // Source path — collapsed, subtle
    let source_display = entity
        .source_path
        .strip_prefix(&cfg.root)
        .unwrap_or(&entity.source_path)
        .display()
        .to_string();
    body.push_str(&format!(
        r#"<div class="task-meta-source"><code>{}</code></div>"#,
        escape_html(&source_display)
    ));

    body.push_str("</aside>\n"); // .task-detail-sidebar
    body.push_str("</div>\n"); // .task-detail-layout
    body.push_str("</div>\n"); // .task-detail-card

    // Related entities sections
    if !related.is_empty() {
        body.push_str(r#"<hr class="detail-separator">"#);
        body.push('\n');
        for section in related {
            body.push_str(&render_related_section(section));
        }
    }

    body
}

/// Strip a leading H1 heading from markdown that duplicates the page title.
fn strip_leading_h1(md: &str) -> &str {
    let mut rest = md;
    // Skip an optional leading `# Title` line (H1)
    if let Some(stripped) = rest.strip_prefix("# ") {
        if let Some(newline_pos) = stripped.find('\n') {
            rest = stripped[newline_pos..].trim_start_matches('\n');
        } else {
            // Entire string is the H1 with no body
            rest = "";
        }
    }
    rest
}

/// Return a CSS class for due date urgency ("", "date-overdue", "date-soon").
fn due_date_urgency_class(due_date: &str) -> &'static str {
    // Parse YYYY-MM-DD
    let parts: Vec<&str> = due_date.split('-').collect();
    if parts.len() != 3 {
        return "";
    }
    let (Ok(y), Ok(m), Ok(d)) = (
        parts[0].parse::<i32>(),
        parts[1].parse::<u32>(),
        parts[2].parse::<u32>(),
    ) else {
        return "";
    };

    // Get today's date from the system
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let today_days = secs / 86400; // days since epoch

    // days since epoch for due date (simplified Gregorian)
    let due_days = gregorian_to_days(y, m, d);
    let today_days = today_days as i64;

    let diff = due_days - today_days;
    if diff < 0 {
        "date-overdue"
    } else if diff <= 7 {
        "date-soon"
    } else {
        ""
    }
}

/// Approximate days since Unix epoch for a Gregorian date (accurate enough for date comparison).
fn gregorian_to_days(y: i32, m: u32, d: u32) -> i64 {
    // Days from 1970-01-01 using a simple formula
    let y = y as i64;
    let m = m as i64;
    let d = d as i64;

    let adj_y = y - (14 - m) / 12;
    let adj_m = m + 12 * ((14 - m) / 12) - 3;

    let jdn =
        d + (153 * adj_m + 2) / 5 + 365 * adj_y + adj_y / 4 - adj_y / 100 + adj_y / 400 + 1721119;
    // JDN for 1970-01-01 = 2440588
    jdn - 2440588
}

/// Render a detail page for a single entity.
pub fn detail_page(
    entity: &EntityRecord,
    prefixes: &[&str],
    related: &[RelatedSection],
    cfg: &ResolvedConfig,
    custom_css: &str,
) -> String {
    // Dispatch to the task-specific layout
    if entity.kind == EntityKind::Task {
        let body = task_detail_body(entity, prefixes, related, cfg);
        let display_name = frontmatter::get_str(&entity.frontmatter, "title")
            .or_else(|| frontmatter::get_str(&entity.frontmatter, "name"))
            .unwrap_or(&entity.id);
        return layout_branded(
            &format!("{} - {}", display_name, entity.id),
            "/tasks",
            &body,
            &cfg.mode,
            &cfg.configured_entities,
            &cfg.brand,
            custom_css,
        );
    }

    let mut body = String::new();

    let display_name = frontmatter::get_str(&entity.frontmatter, "name")
        .or_else(|| frontmatter::get_str(&entity.frontmatter, "title"))
        .unwrap_or(&entity.id);

    let status = frontmatter::get_str_or(&entity.frontmatter, "status", "");

    // Breadcrumb
    let kind_plural = entity.kind.label_plural();
    body.push_str(&format!(
        r#"<div class="breadcrumb"><a href="/">Dashboard</a><span class="sep">/</span><a href="/{}">{}</a><span class="sep">/</span>{}</div>"#,
        kind_plural,
        capitalize(kind_plural),
        escape_html(&entity.id),
    ));

    // ── Card container ────────────────────────────────────────────────
    body.push_str(r#"<div class="detail-card">"#);

    // Hero heading with inline status
    body.push_str(r#"<div class="detail-hero">"#);
    body.push_str(&format!(
        "<h2>{}</h2>{}",
        escape_html(display_name),
        if !status.is_empty() {
            format!(" {}", status_badge(status))
        } else {
            String::new()
        },
    ));
    body.push_str("</div>\n");

    // ── Two-column layout: sidebar of metadata + main body ────────────
    body.push_str(r#"<div class="detail-layout">"#);

    // ── Main: markdown body ───────────────────────────────────────────
    body.push_str(r#"<div class="detail-main">"#);
    if !entity.body.trim().is_empty() {
        let md = strip_leading_h1(entity.body.trim());
        body.push_str(r#"<div class="detail-body">"#);
        body.push_str(&render_markdown(md, prefixes));
        body.push_str("</div>\n");
    } else {
        body.push_str(r#"<p style="color:var(--mc-text-muted);font-style:italic;">No description provided.</p>"#);
    }
    body.push_str("</div>\n"); // .detail-main

    // ── Sidebar: frontmatter fields ───────────────────────────────────
    body.push_str(r#"<aside class="detail-sidebar">"#);

    // Emit entity-id block
    body.push_str(&format!(
        r#"<div class="detail-meta-block">
  <div class="detail-meta-label">ID</div>
  <div class="detail-meta-value">{}</div>
</div>"#,
        entity_link(&entity.id),
    ));

    // Emit remaining frontmatter fields as sidebar blocks
    if let Some(map) = entity.frontmatter.as_mapping() {
        for (key, value) in map {
            let key_str = key.as_str().unwrap_or("");
            if key_str.starts_with('_') {
                continue;
            }
            // Skip fields shown elsewhere
            if matches!(
                key_str,
                "id" | "status" | "name" | "title" | "contacts" | "slug"
            ) {
                continue;
            }
            // Skip empty values
            let value_html = match key_str {
                "tags" => {
                    let tags = value
                        .as_sequence()
                        .map(|seq| {
                            seq.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    if tags.is_empty() {
                        continue;
                    }
                    tag_badges(&tags)
                }
                "customers" | "projects" | "customer" => {
                    let ids = value
                        .as_sequence()
                        .map(|seq| {
                            seq.iter()
                                .filter_map(|v| {
                                    v.as_str()
                                        .map(|s| frontmatter::strip_wikilink(s).to_string())
                                })
                                .filter(|s| !s.is_empty())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    if ids.is_empty() {
                        if let Some(s) = value.as_str() {
                            let s = frontmatter::strip_wikilink(s);
                            if s.is_empty() {
                                continue;
                            }
                            entity_link(s)
                        } else {
                            continue;
                        }
                    } else {
                        let links: Vec<String> = ids.iter().map(|id| entity_link(id)).collect();
                        links.join(", ")
                    }
                }
                _ => {
                    let v = format_value_html(value, prefixes);
                    if v.contains("(empty)") || v.contains("(none)") || v.contains("(null)") {
                        continue;
                    }
                    v
                }
            };
            // Pretty-print key label
            let label = key_str.replace('_', " ");
            body.push_str(&format!(
                r#"<div class="detail-meta-block">
  <div class="detail-meta-label">{}</div>
  <div class="detail-meta-value">{}</div>
</div>"#,
                escape_html(&label),
                value_html,
            ));
        }
    }

    // Source path — subtle, at bottom
    let source_display = entity
        .source_path
        .strip_prefix(&cfg.root)
        .unwrap_or(&entity.source_path)
        .display()
        .to_string();
    body.push_str(&format!(
        r#"<div class="detail-meta-source"><code>{}</code></div>"#,
        escape_html(&source_display)
    ));

    body.push_str("</aside>\n"); // .detail-sidebar
    body.push_str("</div>\n"); // .detail-layout
    body.push_str("</div>\n"); // .detail-card

    // Related entities sections
    if !related.is_empty() {
        for section in related {
            body.push_str(&render_related_section(section));
        }
    }

    let nav_path = format!("/{}", entity.kind.label_plural());

    layout_branded(
        &format!("{} - {}", display_name, entity.id),
        &nav_path,
        &body,
        &cfg.mode,
        &cfg.configured_entities,
        &cfg.brand,
        custom_css,
    )
}

/// Render a related entities section on a detail page.
fn render_related_section(section: &RelatedSection) -> String {
    let mut html = String::new();
    html.push_str(&format!(
        r#"<div class="related-section"><h3>{} <span class="related-count">{}</span></h3>"#,
        escape_html(&section.title),
        section.entities.len()
    ));

    // Progress bar for tasks
    if section.kind == EntityKind::Task && !section.entities.is_empty() {
        let done = section
            .entities
            .iter()
            .filter(|e| {
                let s = frontmatter::get_str_or(&e.frontmatter, "status", "");
                s == "done" || s == "completed"
            })
            .count();
        let pct = (done as f64 / section.entities.len() as f64 * 100.0) as u32;
        html.push_str(&format!(
            r#"<div class="progress-bar"><div class="progress-fill" style="width:{}%"></div></div>
<span class="progress-label">{}/{} done ({}%)</span>"#,
            pct,
            done,
            section.entities.len(),
            pct
        ));
    }

    html.push_str(r#"<div class="table-wrap"><table class="related-table"><tbody>"#);
    for e in &section.entities {
        let id = frontmatter::get_str_or(&e.frontmatter, "id", "");
        let name = frontmatter::get_str(&e.frontmatter, "name")
            .or_else(|| frontmatter::get_str(&e.frontmatter, "title"))
            .unwrap_or("");
        let status = frontmatter::get_str_or(&e.frontmatter, "status", "");
        html.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td></tr>\n",
            entity_link(id),
            escape_html(name),
            status_badge(status),
        ));
    }
    html.push_str("</tbody></table></div></div>\n");
    html
}

/// Render a tasks list page.
#[allow(clippy::too_many_arguments)]
pub fn tasks_list_page(
    entities: &[EntityRecord],
    status_filter: Option<&str>,
    priority_filter: Option<u32>,
    owner_filter: Option<&str>,
    project_filter: Option<&str>,
    sprint_filter: Option<&str>,
    valid_statuses: &[String],
    filter_options: &TaskFilterOptions,
    cfg: &ResolvedConfig,
    custom_css: &str,
) -> String {
    let mut body = String::new();
    body.push_str(r#"<div class="page-header"><h2 class="page-title">Tasks</h2><div class="view-toggle"><a href="/tasks">Board</a><a href="/tasks/list" class="active">List</a></div></div>"#);
    body.push('\n');

    // Filter form with all dimensions
    body.push_str(
        r#"<form class="filter-form" method="get" action="/tasks/list">
  <div>
    <label for="status">Status</label>
    <select name="status" id="status">
      <option value="">All</option>
"#,
    );
    for s in valid_statuses {
        let selected = if status_filter == Some(s.as_str()) {
            " selected"
        } else {
            ""
        };
        body.push_str(&format!(
            "      <option value=\"{}\"{}>{}</option>\n",
            escape_html(s),
            selected,
            escape_html(s)
        ));
    }
    body.push_str("    </select>\n  </div>\n");

    // Priority filter
    body.push_str(
        r#"  <div>
    <label for="priority">Priority</label>
    <select name="priority" id="priority">
      <option value="">All</option>
"#,
    );
    for (val, label) in [(1, "Critical"), (2, "High"), (3, "Medium"), (4, "Low")] {
        let selected = if priority_filter == Some(val) {
            " selected"
        } else {
            ""
        };
        body.push_str(&format!(
            "      <option value=\"{}\"{}>{}</option>\n",
            val, selected, label
        ));
    }
    body.push_str("    </select>\n  </div>\n");

    // Owner filter
    body.push_str(
        r#"  <div>
    <label for="owner">Owner</label>
    <select name="owner" id="owner">
      <option value="">All</option>
"#,
    );
    for o in &filter_options.owners {
        let selected = if owner_filter == Some(o.as_str()) {
            " selected"
        } else {
            ""
        };
        body.push_str(&format!(
            "      <option value=\"{}\"{}>{}</option>\n",
            escape_html(o),
            selected,
            escape_html(o)
        ));
    }
    body.push_str("    </select>\n  </div>\n");

    // Project filter
    if !filter_options.projects.is_empty() {
        body.push_str(
            r#"  <div>
    <label for="project">Project</label>
    <select name="project" id="project">
      <option value="">All</option>
"#,
        );
        for p in &filter_options.projects {
            let selected = if project_filter == Some(p.as_str()) {
                " selected"
            } else {
                ""
            };
            body.push_str(&format!(
                "      <option value=\"{}\"{}>{}</option>\n",
                escape_html(p),
                selected,
                escape_html(p)
            ));
        }
        body.push_str("    </select>\n  </div>\n");
    }

    // Sprint filter
    if !filter_options.sprints.is_empty() {
        body.push_str(
            r#"  <div>
    <label for="sprint">Sprint</label>
    <select name="sprint" id="sprint">
      <option value="">All</option>
"#,
        );
        for s in &filter_options.sprints {
            let selected = if sprint_filter == Some(s.as_str()) {
                " selected"
            } else {
                ""
            };
            body.push_str(&format!(
                "      <option value=\"{}\"{}>{}</option>\n",
                escape_html(s),
                selected,
                escape_html(s)
            ));
        }
        body.push_str("    </select>\n  </div>\n");
    }

    body.push_str(
        r#"  <div>
    <button type="submit">Filter</button>
  </div>
  <a href="/tasks/list" class="reset-link">Reset</a>
</form>
"#,
    );

    if entities.is_empty() {
        body.push_str(r#"<div class="empty-state"><span class="empty-state-icon">~</span>No tasks found.</div>"#);
        body.push('\n');
        return layout_branded(
            "Tasks",
            "/tasks",
            &body,
            &cfg.mode,
            &cfg.configured_entities,
            &cfg.brand,
            custom_css,
        );
    }

    body.push_str(r#"<div class="table-wrap"><table>"#);
    body.push_str("\n<thead><tr>");
    body.push_str("<th>ID</th><th>Title</th><th>Status</th><th>Pri</th><th>Owner</th><th>Project</th><th>Sprint</th><th>Tags</th>");
    body.push_str("</tr></thead>\n<tbody>\n");

    for e in entities {
        let id = frontmatter::get_str_or(&e.frontmatter, "id", "");
        let title = frontmatter::get_str_or(&e.frontmatter, "title", "");
        let status = frontmatter::get_str_or(&e.frontmatter, "status", "");
        let owner = frontmatter::get_str_or(&e.frontmatter, "owner", "");
        let sprint_raw = frontmatter::get_str_or(&e.frontmatter, "sprint", "");
        let sprint = frontmatter::strip_wikilink(sprint_raw);
        let priority = data::get_number(&e.frontmatter, "priority").unwrap_or(3);
        let tags = frontmatter::get_string_list(&e.frontmatter, "tags");
        let projects = frontmatter::get_link_list(&e.frontmatter, "projects");

        let pri_label = match priority {
            1 => "Critical",
            2 => "High",
            3 => "Medium",
            4 => "Low",
            _ => "?",
        };

        body.push_str("<tr>");
        body.push_str(&format!("<td>{}</td>", entity_link(id)));
        body.push_str(&format!("<td>{}</td>", escape_html(title)));
        body.push_str(&format!("<td>{}</td>", status_badge(status)));
        body.push_str(&format!("<td>{}</td>", escape_html(pri_label)));
        body.push_str(&format!("<td>{}</td>", escape_html(owner)));
        body.push_str(&format!(
            "<td>{}</td>",
            projects
                .iter()
                .map(|p| entity_link(p))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        body.push_str(&format!("<td>{}</td>", escape_html(sprint)));
        body.push_str(&format!("<td>{}</td>", tag_badges(&tags)));
        body.push_str("</tr>\n");
    }

    body.push_str("</tbody></table></div>\n");
    body.push_str(&format!(
        r#"<p class="table-count"><strong>{}</strong> tasks total</p>"#,
        entities.len()
    ));

    layout_branded(
        "Tasks",
        "/tasks",
        &body,
        &cfg.mode,
        &cfg.configured_entities,
        &cfg.brand,
        custom_css,
    )
}

/// Render a kanban board page for tasks.
pub fn board_page(tasks: &[EntityRecord], cfg: &ResolvedConfig, custom_css: &str) -> String {
    let mut body = String::new();
    body.push_str(r#"<div class="page-header"><h2 class="page-title">Task Board</h2><div class="view-toggle"><a href="/tasks" class="active">Board</a><a href="/tasks/list">List</a></div></div>"#);
    body.push('\n');

    let columns = ["backlog", "todo", "in-progress", "review", "done"];
    let mut grouped: HashMap<&str, Vec<&EntityRecord>> = HashMap::new();
    for col in &columns {
        grouped.insert(col, Vec::new());
    }
    for task in tasks {
        let status = frontmatter::get_str_or(&task.frontmatter, "status", "backlog");
        if let Some(col) = grouped.get_mut(status) {
            col.push(task);
        }
    }
    // Sort each column by priority
    for col in grouped.values_mut() {
        col.sort_by(|a, b| {
            let pa = data::get_number(&a.frontmatter, "priority").unwrap_or(3);
            let pb = data::get_number(&b.frontmatter, "priority").unwrap_or(3);
            pa.cmp(&pb).then(a.id.cmp(&b.id))
        });
    }

    body.push_str(r#"<div class="kanban-board">"#);
    body.push('\n');

    for col in &columns {
        let tasks_in_col = &grouped[col];
        body.push_str(&format!(
            r#"<div class="kanban-column col-{}">
<h3>{} <span class="kanban-count">{}</span></h3>
"#,
            col,
            escape_html(&col.to_uppercase()),
            tasks_in_col.len()
        ));

        if tasks_in_col.is_empty() {
            body.push_str(&format!(
                r#"<div class="kanban-empty">No {} tasks</div>"#,
                col.replace('-', " ")
            ));
        }

        for task in tasks_in_col {
            let id = frontmatter::get_str_or(&task.frontmatter, "id", "");
            let title = frontmatter::get_str_or(&task.frontmatter, "title", "");
            let owner = frontmatter::get_str_or(&task.frontmatter, "owner", "");
            let priority = data::get_number(&task.frontmatter, "priority").unwrap_or(3);
            let pri_class = match priority {
                1 => "pri-critical",
                2 => "pri-high",
                3 => "pri-medium",
                _ => "pri-low",
            };
            // Non-empty only for critical/high/medium; low gets no dot
            let has_pri_dot = matches!(priority, 1..=3);

            // Project name (strip wikilink)
            let projects = frontmatter::get_link_list(&task.frontmatter, "projects");
            let project_html = if let Some(p) = projects.first() {
                format!(
                    r#"<span class="kanban-card-project">{}</span>"#,
                    escape_html(p)
                )
            } else {
                String::new()
            };

            // Tags (max 2)
            let tags = frontmatter::get_string_list(&task.frontmatter, "tags");
            let tags_html = if tags.is_empty() {
                String::new()
            } else {
                let tag_spans: String = tags
                    .iter()
                    .take(2)
                    .map(|t| format!(r#"<span class="kanban-tag">{}</span>"#, escape_html(t)))
                    .collect::<Vec<_>>()
                    .join("");
                format!(r#"<div class="kanban-card-tags">{}</div>"#, tag_spans)
            };

            let owner_html = if owner.is_empty() {
                String::new()
            } else {
                let ini = initials(owner);
                format!(
                    r#"<span class="kanban-card-avatar" title="{}"><span class="owner-initials">{}</span></span>"#,
                    escape_html(owner),
                    escape_html(&ini),
                )
            };

            let pri_html = if has_pri_dot {
                format!(
                    r#"<span class="kanban-pri {}" title="{}"></span>"#,
                    pri_class,
                    priority_label(priority as u64)
                )
            } else {
                String::new()
            };

            body.push_str(&format!(
                r#"<a href="/entity/{}" class="kanban-card {}">
  <div class="kanban-card-title">{}</div>
  <div class="kanban-card-meta">
    <span class="kanban-card-id">{}</span>{}{}</div>
  <div class="kanban-card-footer">{}{}</div>
</a>
"#,
                escape_html(id),
                pri_class,
                escape_html(title),
                escape_html(id),
                pri_html,
                project_html,
                tags_html,
                owner_html,
            ));
        }

        body.push_str("</div>\n");
    }

    body.push_str("</div>\n");

    layout_branded(
        "Task Board",
        "/tasks",
        &body,
        &cfg.mode,
        &cfg.configured_entities,
        &cfg.brand,
        custom_css,
    )
}

/// Render a 500 error page.
pub fn error_page(message: &str) -> String {
    let body = format!(
        r#"<div class="empty-state"><span class="empty-state-icon">!</span><h2>Error</h2><p>{}</p><p><a href="/">Back to Dashboard</a></p></div>"#,
        escape_html(message)
    );
    layout("Error", "", &body)
}

/// Render a 404 page.
pub fn not_found_page(path: &str, cfg: &ResolvedConfig, custom_css: &str) -> String {
    let body = format!(
        r#"<div class="empty-state"><span class="empty-state-icon">?</span><h2>Not Found</h2><p>The page <code>{}</code> was not found.</p><p><a href="/">Back to Dashboard</a></p></div>"#,
        escape_html(path)
    );
    layout_branded(
        "Not Found",
        "",
        &body,
        &cfg.mode,
        &cfg.configured_entities,
        &cfg.brand,
        custom_css,
    )
}

/// Format a YAML value as HTML.
fn format_value_html(value: &Value, prefixes: &[&str]) -> String {
    match value {
        Value::String(s) => {
            let s = frontmatter::strip_wikilink(s);
            if s.is_empty() {
                "<em>(empty)</em>".to_string()
            } else {
                // Check if it looks like an entity ID
                let looks_like_id = prefixes.iter().any(|p| s.starts_with(&format!("{}-", p)));
                if looks_like_id {
                    entity_link(s)
                } else {
                    escape_html(s)
                }
            }
        }
        Value::Sequence(seq) => {
            if seq.is_empty() {
                "<em>(none)</em>".to_string()
            } else {
                let items: Vec<String> =
                    seq.iter().map(|v| format_value_html(v, prefixes)).collect();
                items.join(", ")
            }
        }
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Null => "<em>(null)</em>".to_string(),
        Value::Mapping(_) => "<em>(object)</em>".to_string(),
        Value::Tagged(t) => format_value_html(&t.value, prefixes),
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
