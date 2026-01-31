use crate::cli::ListEntity;
use crate::config::ResolvedConfig;
use crate::data::{self, TaskFilter};
use crate::entity::EntityKind;
use crate::error::McResult;
use crate::frontmatter;
use colored::*;

pub fn run(entity: &ListEntity, cfg: &ResolvedConfig) -> McResult<()> {
    match entity {
        ListEntity::Tasks {
            status,
            tag,
            project,
            customer,
            priority,
            sprint,
            owner,
        } => list_tasks(cfg, status, tag, project, customer, priority, sprint, owner),
        _ => {
            let (kind, status_filter, tag_filter) = match entity {
                ListEntity::Customers { status, tag } => (EntityKind::Customer, status, tag),
                ListEntity::Projects { status, tag } => (EntityKind::Project, status, tag),
                ListEntity::Meetings { status, tag } => (EntityKind::Meeting, status, tag),
                ListEntity::Research { status, tag } => (EntityKind::Research, status, tag),
                ListEntity::Tasks { .. } => unreachable!(),
            };
            list_standard(kind, cfg, status_filter, tag_filter)
        }
    }
}

/// Truncate a string to `max` chars, appending "..." if needed.
fn truncate(s: &str, max: usize) -> String {
    if s.len() > max && max > 3 {
        format!("{}...", &s[..max - 3])
    } else {
        s.to_string()
    }
}

fn list_standard(
    kind: EntityKind,
    cfg: &ResolvedConfig,
    status_filter: &Option<String>,
    tag_filter: &Option<String>,
) -> McResult<()> {
    let entries =
        data::collect_filtered(kind, cfg, status_filter.as_deref(), tag_filter.as_deref())?;

    let has_filters = status_filter.is_some() || tag_filter.is_some();

    if entries.is_empty() {
        if has_filters {
            let mut filter_desc = Vec::new();
            if let Some(s) = status_filter {
                filter_desc.push(format!("status '{}'", s));
            }
            if let Some(t) = tag_filter {
                filter_desc.push(format!("tag '{}'", t));
            }
            let valid = kind.statuses(cfg);
            println!(
                "{} No {} found with {} (valid statuses: {})",
                "i".blue(),
                kind.label_plural(),
                filter_desc.join(" and "),
                valid.join(", "),
            );
        } else {
            println!("{} No {} found.", "i".blue(), kind.label_plural());
        }
        return Ok(());
    }

    // Print filter banner
    if has_filters {
        let mut parts = Vec::new();
        if let Some(s) = status_filter {
            parts.push(format!("status = {}", s));
        }
        if let Some(t) = tag_filter {
            parts.push(format!("tag = {}", t));
        }
        println!(
            "{} Showing {} with {}\n",
            "i".blue(),
            kind.label_plural(),
            parts.join(", "),
        );
    }

    // Print header
    match kind {
        EntityKind::Customer => {
            println!(
                "  {:<10} {:<28} {:<12} {:<12}",
                "ID".bold(),
                "Name".bold(),
                "Status".bold(),
                "Owner".bold()
            );
            println!("  {}", "─".repeat(62).dimmed());
            for e in &entries {
                let id = frontmatter::get_str_or(&e.frontmatter, "id", "");
                let name = frontmatter::get_str_or(&e.frontmatter, "name", "");
                let status = frontmatter::get_str_or(&e.frontmatter, "status", "");
                let owner = frontmatter::get_str_or(&e.frontmatter, "owner", "");
                println!(
                    "  {:<10} {:<28} {:<12} {}",
                    id.cyan(),
                    truncate(name, 27),
                    format_status(status),
                    owner.dimmed()
                );
            }
        }
        EntityKind::Project => {
            println!(
                "  {:<10} {:<28} {:<12} {:<12}",
                "ID".bold(),
                "Name".bold(),
                "Status".bold(),
                "Owner".bold()
            );
            println!("  {}", "─".repeat(62).dimmed());
            for e in &entries {
                let id = frontmatter::get_str_or(&e.frontmatter, "id", "");
                let name = frontmatter::get_str_or(&e.frontmatter, "name", "");
                let status = frontmatter::get_str_or(&e.frontmatter, "status", "");
                let owner = frontmatter::get_str_or(&e.frontmatter, "owner", "");
                println!(
                    "  {:<10} {:<28} {:<12} {}",
                    id.cyan(),
                    truncate(name, 27),
                    format_status(status),
                    owner.dimmed()
                );
            }
        }
        EntityKind::Meeting => {
            println!(
                "  {:<10} {:<30} {:<12} {:<6} {:<12}",
                "ID".bold(),
                "Title".bold(),
                "Date".bold(),
                "Time".bold(),
                "Status".bold()
            );
            println!("  {}", "─".repeat(70).dimmed());
            for e in &entries {
                let id = frontmatter::get_str_or(&e.frontmatter, "id", "");
                let title = frontmatter::get_str_or(&e.frontmatter, "title", "");
                let date = frontmatter::get_str_or(&e.frontmatter, "date", "");
                let time = frontmatter::get_str_or(&e.frontmatter, "time", "");
                let status = frontmatter::get_str_or(&e.frontmatter, "status", "");
                println!(
                    "  {:<10} {:<30} {:<12} {:<6} {}",
                    id.cyan(),
                    truncate(title, 29),
                    date,
                    time,
                    format_status(status)
                );
            }
        }
        EntityKind::Research => {
            println!(
                "  {:<10} {:<30} {:<12} {:<12}",
                "ID".bold(),
                "Title".bold(),
                "Status".bold(),
                "Owner".bold()
            );
            println!("  {}", "─".repeat(64).dimmed());
            for e in &entries {
                let id = frontmatter::get_str_or(&e.frontmatter, "id", "");
                let title = frontmatter::get_str_or(&e.frontmatter, "title", "");
                let status = frontmatter::get_str_or(&e.frontmatter, "status", "");
                let owner = frontmatter::get_str_or(&e.frontmatter, "owner", "");
                println!(
                    "  {:<10} {:<30} {:<12} {}",
                    id.cyan(),
                    truncate(title, 29),
                    format_status(status),
                    owner.dimmed()
                );
            }
        }
        EntityKind::Task => unreachable!(),
    }

    println!(
        "\n  {} {} total",
        entries.len().to_string().bold(),
        kind.label_plural()
    );

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn list_tasks(
    cfg: &ResolvedConfig,
    status: &Option<String>,
    tag: &Option<String>,
    project: &Option<String>,
    customer: &Option<String>,
    priority: &Option<u32>,
    sprint: &Option<String>,
    owner: &Option<String>,
) -> McResult<()> {
    let filter = TaskFilter {
        status: status.as_deref(),
        tag: tag.as_deref(),
        project: project.as_deref(),
        customer: customer.as_deref(),
        priority: *priority,
        sprint: sprint.as_deref(),
        owner: owner.as_deref(),
    };

    let entries = data::collect_tasks_filtered(cfg, &filter)?;

    let has_filters = status.is_some()
        || tag.is_some()
        || project.is_some()
        || customer.is_some()
        || priority.is_some()
        || sprint.is_some()
        || owner.is_some();

    if entries.is_empty() {
        if has_filters {
            let mut filter_desc = Vec::new();
            if let Some(s) = status {
                filter_desc.push(format!("status '{}'", s));
            }
            if let Some(p) = project {
                filter_desc.push(format!("project '{}'", p));
            }
            if let Some(c) = customer {
                filter_desc.push(format!("customer '{}'", c));
            }
            if let Some(pr) = priority {
                filter_desc.push(format!("priority {}", pr));
            }
            if let Some(sp) = sprint {
                filter_desc.push(format!("sprint '{}'", sp));
            }
            if let Some(o) = owner {
                filter_desc.push(format!("owner '{}'", o));
            }
            if let Some(t) = tag {
                filter_desc.push(format!("tag '{}'", t));
            }
            println!(
                "{} No tasks found with {}",
                "i".blue(),
                filter_desc.join(" and "),
            );
        } else {
            println!("{} No tasks found.", "i".blue());
        }
        return Ok(());
    }

    if has_filters {
        let mut parts = Vec::new();
        if let Some(s) = status {
            parts.push(format!("status = {}", s));
        }
        if let Some(p) = project {
            parts.push(format!("project = {}", p));
        }
        if let Some(c) = customer {
            parts.push(format!("customer = {}", c));
        }
        if let Some(pr) = priority {
            parts.push(format!("priority = {}", pr));
        }
        if let Some(sp) = sprint {
            parts.push(format!("sprint = {}", sp));
        }
        if let Some(o) = owner {
            parts.push(format!("owner = {}", o));
        }
        if let Some(t) = tag {
            parts.push(format!("tag = {}", t));
        }
        println!("{} Showing tasks with {}\n", "i".blue(), parts.join(", "),);
    }

    println!(
        "  {:<10} {:<26} {:<13} {:<4} {:<10} {:<10} {:<8}",
        "ID".bold(),
        "Title".bold(),
        "Status".bold(),
        "Pri".bold(),
        "Owner".bold(),
        "Project".bold(),
        "Sprint".bold()
    );
    println!("  {}", "─".repeat(81).dimmed());
    for e in &entries {
        let id = frontmatter::get_str_or(&e.frontmatter, "id", "");
        let title = frontmatter::get_str_or(&e.frontmatter, "title", "");
        let status = frontmatter::get_str_or(&e.frontmatter, "status", "");
        let owner = frontmatter::get_str_or(&e.frontmatter, "owner", "");
        let sprint = frontmatter::get_str_or(&e.frontmatter, "sprint", "");
        let priority = data::get_number(&e.frontmatter, "priority").unwrap_or(3);
        let projects = frontmatter::get_string_list(&e.frontmatter, "projects");
        let proj_display = projects.first().map(|s| s.as_str()).unwrap_or("");

        let pri_display = match priority {
            1 => "C".red().bold().to_string(),
            2 => "H".yellow().bold().to_string(),
            3 => "M".normal().to_string(),
            4 => "L".dimmed().to_string(),
            _ => priority.to_string(),
        };

        let title_trunc = truncate(title, 25);

        println!(
            "  {:<10} {:<26} {:<13} {:<4} {:<10} {:<10} {}",
            id.cyan(),
            title_trunc,
            format_status(status),
            pri_display,
            if owner.is_empty() {
                "-".dimmed().to_string()
            } else {
                truncate(owner, 9).dimmed().to_string()
            },
            if proj_display.is_empty() {
                "-".dimmed().to_string()
            } else {
                truncate(proj_display, 9)
            },
            if sprint.is_empty() {
                "-".dimmed().to_string()
            } else {
                truncate(sprint, 7)
            },
        );
    }

    println!("\n  {} tasks total", entries.len().to_string().bold());

    Ok(())
}

pub fn format_status(status: &str) -> colored::ColoredString {
    match status {
        "active" | "completed" | "final" | "done" => status.green(),
        "inactive" | "cancelled" | "churned" | "outdated" => status.red(),
        "on-hold" | "draft" | "in-progress" | "review" => status.yellow(),
        "prospect" | "scheduled" | "todo" => status.blue(),
        "backlog" => status.dimmed(),
        _ => status.normal(),
    }
}
