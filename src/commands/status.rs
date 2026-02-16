use crate::config::{RepoMode, ResolvedConfig};
use crate::data;
use crate::entity::EntityKind;
use crate::error::McResult;
use colored::*;

pub fn run(cfg: &ResolvedConfig) -> McResult<()> {
    let meetings = data::count_by_status(EntityKind::Meeting, cfg)?;
    let research = data::count_by_status(EntityKind::Research, cfg)?;
    let tasks = data::count_by_status(EntityKind::Task, cfg)?;
    let sprints = data::count_by_status(EntityKind::Sprint, cfg)?;
    let proposals = data::count_by_status(EntityKind::Proposal, cfg)?;

    println!();
    let header = match cfg.mode {
        RepoMode::Embedded => format!("{} {}", "MissionControl", "(embedded)".dimmed()),
        RepoMode::Standalone => "MissionControl".to_string(),
    };
    println!("  {}", header.bold());
    println!("  {}", "────────────────────────────────────────".dimmed());
    println!();

    if cfg.mode == RepoMode::Standalone {
        let customers = data::count_by_status(EntityKind::Customer, cfg)?;
        let projects = data::count_by_status(EntityKind::Project, cfg)?;
        let contacts = data::count_by_status(EntityKind::Contact, cfg)?;
        print_section("Customers", &customers.by_status);
        print_section("Projects", &projects.by_status);
        print_section("Contacts", &contacts.by_status);
    }
    print_section("Meetings", &meetings.by_status);
    print_section("Research", &research.by_status);
    print_section("Tasks", &tasks.by_status);
    print_section("Sprints", &sprints.by_status);
    print_section("Proposals", &proposals.by_status);

    // Recent activity
    println!("  {}", "Recent Activity".bold());
    println!("  {}", "────────────────────────────────────────".dimmed());

    let recent = data::recent_activity(cfg, 5)?;
    if recent.is_empty() {
        println!("    {}", "(no files found)".dimmed());
    } else {
        for f in &recent {
            println!("    {} {}", f.id.cyan(), f.name.dimmed());
        }
    }
    println!();

    Ok(())
}

fn print_section(label: &str, counts: &[(String, usize)]) {
    let total: usize = counts.iter().map(|(_, c)| c).sum();
    println!("  {} {}", label.bold(), format!("({})", total).dimmed());

    if counts.is_empty() {
        println!("    {}", "(none)".dimmed());
        println!();
        return;
    }

    // Find max count for proportional bars
    let max_count = counts.iter().map(|(_, c)| *c).max().unwrap_or(1).max(1);
    let bar_max_width: usize = 20;

    for (status, count) in counts {
        let bar_len = if max_count > 0 {
            (*count as f64 / max_count as f64 * bar_max_width as f64).ceil() as usize
        } else {
            0
        };
        let bar_len = bar_len.max(if *count > 0 { 1 } else { 0 });
        let bar = "█".repeat(bar_len);
        let pct = if total > 0 {
            *count as f64 / total as f64 * 100.0
        } else {
            0.0
        };

        let color_status = match status.as_str() {
            "active" | "completed" | "final" | "done" | "accepted" => status.green(),
            "inactive" | "cancelled" | "churned" | "outdated" | "rejected" | "withdrawn" => {
                status.red()
            }
            "on-hold" | "draft" | "in-progress" | "review" | "planning" | "proposed" => {
                status.yellow()
            }
            "prospect" | "scheduled" | "todo" => status.blue(),
            "backlog" | "superseded" => status.dimmed(),
            _ => status.normal(),
        };
        let color_bar = match status.as_str() {
            "active" | "completed" | "final" | "done" | "accepted" => bar.green(),
            "inactive" | "cancelled" | "churned" | "outdated" | "rejected" | "withdrawn" => {
                bar.red()
            }
            "on-hold" | "draft" | "in-progress" | "review" | "planning" | "proposed" => {
                bar.yellow()
            }
            "prospect" | "scheduled" | "todo" => bar.blue(),
            "backlog" | "superseded" => bar.dimmed(),
            _ => bar.normal(),
        };
        println!(
            "    {:<14} {:>3}  {}  {}",
            color_status,
            count,
            color_bar,
            format!("{:.0}%", pct).dimmed()
        );
    }
    println!();
}
