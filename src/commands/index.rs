use crate::config::ResolvedConfig;
use crate::data;
use crate::entity::EntityKind;
use crate::error::McResult;
use crate::util;
use colored::*;
use serde_json::Value as JsonValue;

pub fn run(cfg: &ResolvedConfig) -> McResult<()> {
    println!("{} Building indexes...", "⟳".blue());

    let result = run_quiet(cfg)?;

    println!(
        "{} Index built: {} customers, {} projects, {} meetings, {} research, {} tasks",
        "✓".green().bold(),
        result.customers.to_string().cyan(),
        result.projects.to_string().cyan(),
        result.meetings.to_string().cyan(),
        result.research.to_string().cyan(),
        result.tasks.to_string().cyan(),
    );

    Ok(())
}

pub struct IndexResult {
    pub customers: usize,
    pub projects: usize,
    pub meetings: usize,
    pub research: usize,
    pub tasks: usize,
}

/// Build indexes without printing to stdout.
pub fn run_quiet(cfg: &ResolvedConfig) -> McResult<IndexResult> {
    let customers = collect_json(EntityKind::Customer, cfg)?;
    let projects = collect_json(EntityKind::Project, cfg)?;
    let meetings = collect_json(EntityKind::Meeting, cfg)?;
    let research = collect_json(EntityKind::Research, cfg)?;
    let tasks = collect_json(EntityKind::Task, cfg)?;

    std::fs::create_dir_all(&cfg.data_dir)?;

    // Build combined index
    let index = serde_json::json!({
        "customers": customers,
        "projects": projects,
        "meetings": meetings,
        "research": research,
        "tasks": tasks,
    });

    let index_path = cfg.data_dir.join("index.json");
    let data = serde_json::to_string_pretty(&index)? + "\n";
    util::atomic_write(&index_path, data.as_bytes())?;

    // Individual files
    let customers_data = serde_json::to_string_pretty(&customers)? + "\n";
    util::atomic_write(
        &cfg.data_dir.join("customers.json"),
        customers_data.as_bytes(),
    )?;

    let projects_data = serde_json::to_string_pretty(&projects)? + "\n";
    util::atomic_write(
        &cfg.data_dir.join("projects.json"),
        projects_data.as_bytes(),
    )?;

    let research_data = serde_json::to_string_pretty(&research)? + "\n";
    util::atomic_write(
        &cfg.data_dir.join("research.json"),
        research_data.as_bytes(),
    )?;

    let tasks_data = serde_json::to_string_pretty(&tasks)? + "\n";
    util::atomic_write(
        &cfg.data_dir.join("tasks.json"),
        tasks_data.as_bytes(),
    )?;

    Ok(IndexResult {
        customers: customers.len(),
        projects: projects.len(),
        meetings: meetings.len(),
        research: research.len(),
        tasks: tasks.len(),
    })
}

fn collect_json(kind: EntityKind, cfg: &ResolvedConfig) -> McResult<Vec<JsonValue>> {
    let entities = data::collect_entities(kind, cfg)?;
    let mut json_entries: Vec<JsonValue> = Vec::new();

    for entity in &entities {
        let mut json_val = data::yaml_to_json(&entity.frontmatter);
        if let Some(obj) = json_val.as_object_mut() {
            let rel = entity
                .source_path
                .strip_prefix(&cfg.root)
                .unwrap_or(&entity.source_path)
                .to_string_lossy()
                .to_string();
            obj.insert("_source".into(), JsonValue::String(rel));
        }
        json_entries.push(json_val);
    }

    // Sort by ID
    json_entries.sort_by(|a, b| {
        let aid = a.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let bid = b.get("id").and_then(|v| v.as_str()).unwrap_or("");
        aid.cmp(bid)
    });

    Ok(json_entries)
}
