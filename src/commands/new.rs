use crate::cli::NewEntity;
use crate::config::ResolvedConfig;
use crate::entity::{self, EntityKind};
use crate::error::{McError, McResult};
use crate::frontmatter;
use crate::template;
use crate::util;
use colored::*;
use serde_json::Value as JsonValue;
use serde_yaml::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub fn run(entity: &NewEntity, cfg: &ResolvedConfig, yes: bool) -> McResult<()> {
    match entity {
        NewEntity::Customer {
            name,
            owner,
            status,
            tags,
        } => new_customer(
            cfg,
            name,
            owner.as_deref(),
            status.as_deref(),
            tags.as_deref(),
            yes,
        ),
        NewEntity::Project {
            name,
            owner,
            status,
            customers,
            tags,
        } => new_project(
            cfg,
            name,
            owner.as_deref(),
            status.as_deref(),
            customers.as_deref(),
            tags.as_deref(),
            yes,
        ),
        NewEntity::Meeting {
            title,
            date,
            time,
            duration,
            status,
            tags,
            customers,
            projects,
        } => new_meeting(
            cfg,
            title,
            date.as_deref(),
            time.as_deref(),
            duration.as_deref(),
            status.as_deref(),
            tags.as_deref(),
            customers.as_deref(),
            projects.as_deref(),
            yes,
        ),
        NewEntity::Research {
            title,
            owner,
            agents,
            tags,
        } => new_research(
            cfg,
            title,
            owner.as_deref(),
            agents.as_deref(),
            tags.as_deref(),
            yes,
        ),
        NewEntity::Task {
            title,
            project,
            customer,
            owner,
            status,
            priority,
            tags,
            sprint,
            depends_on,
            due_date,
        } => new_task(
            cfg,
            title,
            project.as_deref(),
            customer.as_deref(),
            owner.as_deref(),
            status.as_deref(),
            *priority,
            tags.as_deref(),
            sprint.as_deref(),
            depends_on.as_deref(),
            due_date.as_deref(),
            yes,
        ),
    }
}

fn prompt_select(label: &str, options: &[String], default_idx: usize, yes: bool) -> String {
    if yes || options.is_empty() {
        return options.get(default_idx).cloned().unwrap_or_default();
    }
    let selection = dialoguer::Select::new()
        .with_prompt(label)
        .items(options)
        .default(default_idx)
        .interact_opt();
    match selection {
        Ok(Some(idx)) => options[idx].clone(),
        _ => options.get(default_idx).cloned().unwrap_or_default(),
    }
}

fn prompt_input(label: &str, default: &str, yes: bool) -> String {
    if yes {
        return default.to_string();
    }
    let result = dialoguer::Input::<String>::new()
        .with_prompt(label)
        .default(default.to_string())
        .interact_text();
    match result {
        Ok(val) => val,
        _ => default.to_string(),
    }
}

fn prompt_input_optional(label: &str, yes: bool) -> String {
    if yes {
        return String::new();
    }
    let result = dialoguer::Input::<String>::new()
        .with_prompt(format!("{} (blank to skip)", label))
        .allow_empty(true)
        .interact_text();
    match result {
        Ok(val) => val,
        _ => String::new(),
    }
}

fn print_summary(kind: &str, fields: &[(&str, &str)]) {
    println!();
    println!("  {} {}", "New".bold(), kind.bold());
    println!("  {}", "────────────────────────────────────".dimmed());
    for (key, value) in fields {
        let display = if value.is_empty() {
            "(none)".dimmed().to_string()
        } else {
            value.to_string()
        };
        println!("  {:<14} {}", format!("{}:", key).dimmed(), display);
    }
    println!("  {}", "────────────────────────────────────".dimmed());
}

fn confirm_creation(yes: bool) -> bool {
    if yes {
        return true;
    }
    let result = dialoguer::Confirm::new()
        .with_prompt("Create this entity?")
        .default(true)
        .interact();
    match result {
        Ok(v) => v,
        _ => false,
    }
}

/// Create a directory with a .gitkeep file so git tracks it.
fn mkdir_with_gitkeep(path: &Path) -> McResult<()> {
    fs::create_dir_all(path)?;
    fs::write(path.join(".gitkeep"), "")?;
    Ok(())
}

fn new_customer(
    cfg: &ResolvedConfig,
    name: &str,
    owner: Option<&str>,
    status: Option<&str>,
    tags: Option<&str>,
    yes: bool,
) -> McResult<()> {
    let id = entity::next_id(EntityKind::Customer, cfg)?;
    let slug = util::slugify(name);
    let today = util::today_str();

    let owner = match owner {
        Some(o) => o.to_string(),
        None => prompt_input("Owner", "", yes),
    };
    let status = match status {
        Some(s) => s.to_string(),
        None => prompt_select("Status", &cfg.statuses.customer, 0, yes),
    };
    let tags: Vec<String> = match tags {
        Some(t) => util::parse_comma_list(t),
        None => {
            let input = prompt_input_optional("Tags (comma-separated)", yes);
            if input.is_empty() {
                vec![]
            } else {
                util::parse_comma_list(&input)
            }
        }
    };

    let tags_display = tags.join(", ");
    print_summary(
        "customer",
        &[
            ("ID", &id.to_string()),
            ("Name", name),
            ("Owner", &owner),
            ("Status", &status),
            ("Tags", &tags_display),
        ],
    );

    if !confirm_creation(yes) {
        println!("{}", "Cancelled.".dimmed());
        return Ok(());
    }

    // Load template
    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "customer")?;

    // Build fields
    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert("name".into(), Value::String(name.to_string()));
    fields.insert("slug".into(), Value::String(slug.clone()));
    fields.insert("status".into(), Value::String(status));
    fields.insert("owner".into(), Value::String(owner));
    fields.insert(
        "tags".into(),
        Value::Sequence(tags.iter().map(|t| Value::String(t.clone())).collect()),
    );
    fields.insert("projects".into(), Value::Sequence(vec![]));
    fields.insert("contracts".into(), Value::Sequence(vec![]));
    fields.insert("notes".into(), Value::String(String::new()));
    fields.insert("created".into(), Value::String(today.clone()));
    fields.insert("updated".into(), Value::String(today));

    let mut placeholders = HashMap::new();
    placeholders.insert("name".into(), name.to_string());

    let (fm, body) = template::render_template(tmpl_fm, &tmpl_body, &fields, &placeholders);
    let doc = frontmatter::serialize_document(&fm, &body);

    // Create directory structure
    let dir_name = format!("{}-{}", id, slug);
    let dir_path = cfg.customers_dir.join(&dir_name);
    fs::create_dir_all(&dir_path)?;
    fs::write(dir_path.join("_index.md"), &doc)?;
    fs::write(
        dir_path.join("contacts.md"),
        format!("# {} -- Contacts\n", name),
    )?;
    mkdir_with_gitkeep(&dir_path.join("contracts"))?;
    mkdir_with_gitkeep(&dir_path.join("meetings"))?;
    mkdir_with_gitkeep(&dir_path.join("projects"))?;
    mkdir_with_gitkeep(&dir_path.join("assets"))?;

    println!(
        "{} Created customer {} ({}) at {}",
        "✓".green().bold(),
        id.to_string().cyan().bold(),
        name.bold(),
        dir_path.display().to_string().dimmed()
    );

    Ok(())
}

fn new_project(
    cfg: &ResolvedConfig,
    name: &str,
    owner: Option<&str>,
    status: Option<&str>,
    customers: Option<&str>,
    tags: Option<&str>,
    yes: bool,
) -> McResult<()> {
    let id = entity::next_id(EntityKind::Project, cfg)?;
    let slug = util::slugify(name);
    let today = util::today_str();

    let owner = match owner {
        Some(o) => o.to_string(),
        None => prompt_input("Owner", "", yes),
    };
    let status = match status {
        Some(s) => s.to_string(),
        None => prompt_select("Status", &cfg.statuses.project, 0, yes),
    };
    let tags: Vec<String> = match tags {
        Some(t) => util::parse_comma_list(t),
        None => {
            let input = prompt_input_optional("Tags (comma-separated)", yes);
            if input.is_empty() {
                vec![]
            } else {
                util::parse_comma_list(&input)
            }
        }
    };
    let customers: Vec<String> = match customers {
        Some(c) => util::parse_comma_list(c),
        None => {
            let input = prompt_input_optional("Link customers (comma-separated IDs)", yes);
            if input.is_empty() {
                vec![]
            } else {
                util::parse_comma_list(&input)
            }
        }
    };

    let tags_display = tags.join(", ");
    let customers_display = customers.join(", ");
    print_summary(
        "project",
        &[
            ("ID", &id.to_string()),
            ("Name", name),
            ("Owner", &owner),
            ("Status", &status),
            ("Tags", &tags_display),
            ("Customers", &customers_display),
        ],
    );

    if !confirm_creation(yes) {
        println!("{}", "Cancelled.".dimmed());
        return Ok(());
    }

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "project")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert("name".into(), Value::String(name.to_string()));
    fields.insert("slug".into(), Value::String(slug.clone()));
    fields.insert("status".into(), Value::String(status));
    fields.insert("owner".into(), Value::String(owner));
    fields.insert(
        "customers".into(),
        Value::Sequence(customers.iter().map(|c| Value::String(c.clone())).collect()),
    );
    fields.insert(
        "tags".into(),
        Value::Sequence(tags.iter().map(|t| Value::String(t.clone())).collect()),
    );
    fields.insert("start_date".into(), Value::String(today.clone()));
    fields.insert("target_date".into(), Value::String(String::new()));
    fields.insert("created".into(), Value::String(today.clone()));
    fields.insert("updated".into(), Value::String(today));

    let mut placeholders = HashMap::new();
    placeholders.insert("name".into(), name.to_string());

    let (fm, body) = template::render_template(tmpl_fm, &tmpl_body, &fields, &placeholders);
    let doc = frontmatter::serialize_document(&fm, &body);

    let dir_name = format!("{}-{}", id, slug);
    let dir_path = cfg.projects_dir.join(&dir_name);
    fs::create_dir_all(&dir_path)?;
    fs::write(dir_path.join("overview.md"), &doc)?;
    fs::write(
        dir_path.join("roadmap.md"),
        format!("# {} -- Roadmap\n", name),
    )?;
    fs::write(
        dir_path.join("backlog.md"),
        format!("# {} -- Backlog\n", name),
    )?;
    mkdir_with_gitkeep(&dir_path.join("specs"))?;
    mkdir_with_gitkeep(&dir_path.join("releases"))?;
    mkdir_with_gitkeep(&dir_path.join("infra"))?;

    println!(
        "{} Created project {} ({}) at {}",
        "✓".green().bold(),
        id.to_string().cyan().bold(),
        name.bold(),
        dir_path.display().to_string().dimmed()
    );

    Ok(())
}

fn new_meeting(
    cfg: &ResolvedConfig,
    title: &str,
    date: Option<&str>,
    time: Option<&str>,
    duration: Option<&str>,
    status: Option<&str>,
    tags: Option<&str>,
    customers: Option<&str>,
    projects: Option<&str>,
    yes: bool,
) -> McResult<()> {
    let id = entity::next_id(EntityKind::Meeting, cfg)?;
    let date = date.unwrap_or(&util::today_str()).to_string();
    let slug = util::slugify(title);

    let time = match time {
        Some(t) => t.to_string(),
        None => prompt_input("Time (HH:MM)", "10:00", yes),
    };
    let duration = match duration {
        Some(d) => d.to_string(),
        None => prompt_input("Duration", "30m", yes),
    };
    let status = match status {
        Some(s) => s.to_string(),
        None => prompt_select("Status", &cfg.statuses.meeting, 0, yes),
    };
    let tags: Vec<String> = match tags {
        Some(t) => util::parse_comma_list(t),
        None => {
            let input = prompt_input_optional("Tags (comma-separated)", yes);
            if input.is_empty() {
                vec![]
            } else {
                util::parse_comma_list(&input)
            }
        }
    };
    let customers: Vec<String> = match customers {
        Some(c) => util::parse_comma_list(c),
        None => {
            let input = prompt_input_optional("Link customers (comma-separated IDs)", yes);
            if input.is_empty() {
                vec![]
            } else {
                util::parse_comma_list(&input)
            }
        }
    };
    let projects: Vec<String> = match projects {
        Some(p) => util::parse_comma_list(p),
        None => {
            let input = prompt_input_optional("Link projects (comma-separated IDs)", yes);
            if input.is_empty() {
                vec![]
            } else {
                util::parse_comma_list(&input)
            }
        }
    };

    let tags_display = tags.join(", ");
    let customers_display = customers.join(", ");
    let projects_display = projects.join(", ");
    print_summary(
        "meeting",
        &[
            ("ID", &id.to_string()),
            ("Title", title),
            ("Date", &date),
            ("Time", &time),
            ("Duration", &duration),
            ("Status", &status),
            ("Tags", &tags_display),
            ("Customers", &customers_display),
            ("Projects", &projects_display),
        ],
    );

    if !confirm_creation(yes) {
        println!("{}", "Cancelled.".dimmed());
        return Ok(());
    }

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "meeting")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert("title".into(), Value::String(title.to_string()));
    fields.insert("date".into(), Value::String(date.clone()));
    fields.insert("time".into(), Value::String(time));
    fields.insert("duration".into(), Value::String(duration));
    fields.insert(
        "tags".into(),
        Value::Sequence(tags.iter().map(|t| Value::String(t.clone())).collect()),
    );
    fields.insert(
        "customers".into(),
        Value::Sequence(customers.iter().map(|c| Value::String(c.clone())).collect()),
    );
    fields.insert(
        "projects".into(),
        Value::Sequence(projects.iter().map(|p| Value::String(p.clone())).collect()),
    );
    fields.insert("status".into(), Value::String(status));

    let mut placeholders = HashMap::new();
    placeholders.insert("title".into(), title.to_string());

    let (fm, body) = template::render_template(tmpl_fm, &tmpl_body, &fields, &placeholders);
    let doc = frontmatter::serialize_document(&fm, &body);

    let filename = format!("{}-{}.md", date, slug);
    let file_path = cfg.meetings_dir.join(&filename);
    fs::create_dir_all(&cfg.meetings_dir)?;
    fs::write(&file_path, &doc)?;

    println!(
        "{} Created meeting {} ({}) at {}",
        "✓".green().bold(),
        id.to_string().cyan().bold(),
        title.bold(),
        file_path.display().to_string().dimmed()
    );

    Ok(())
}

fn new_research(
    cfg: &ResolvedConfig,
    title: &str,
    owner: Option<&str>,
    agents: Option<&str>,
    tags: Option<&str>,
    yes: bool,
) -> McResult<()> {
    let id = entity::next_id(EntityKind::Research, cfg)?;
    let slug = util::slugify(title);
    let today = util::today_str();

    let owner = match owner {
        Some(o) => o.to_string(),
        None => prompt_input("Owner", "", yes),
    };
    let agents: Vec<String> = agents
        .map(|a| util::parse_comma_list(a))
        .unwrap_or_else(|| {
            vec![
                "claude".into(),
                "gemini".into(),
                "chatgpt".into(),
                "perplexity".into(),
            ]
        });
    let tags: Vec<String> = match tags {
        Some(t) => util::parse_comma_list(t),
        None => {
            let input = prompt_input_optional("Tags (comma-separated)", yes);
            if input.is_empty() {
                vec![]
            } else {
                util::parse_comma_list(&input)
            }
        }
    };

    let tags_display = tags.join(", ");
    let agents_display = agents.join(", ");
    print_summary(
        "research",
        &[
            ("ID", &id.to_string()),
            ("Title", title),
            ("Owner", &owner),
            ("Agents", &agents_display),
            ("Tags", &tags_display),
        ],
    );

    if !confirm_creation(yes) {
        println!("{}", "Cancelled.".dimmed());
        return Ok(());
    }

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "research")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert("title".into(), Value::String(title.to_string()));
    fields.insert("slug".into(), Value::String(slug.clone()));
    fields.insert("status".into(), Value::String("draft".into()));
    fields.insert("owner".into(), Value::String(owner));
    fields.insert("customers".into(), Value::Sequence(vec![]));
    fields.insert("projects".into(), Value::Sequence(vec![]));
    fields.insert(
        "tags".into(),
        Value::Sequence(tags.iter().map(|t| Value::String(t.clone())).collect()),
    );
    fields.insert("created".into(), Value::String(today.clone()));
    fields.insert("updated".into(), Value::String(today));
    fields.insert(
        "agents".into(),
        Value::Sequence(agents.iter().map(|a| Value::String(a.clone())).collect()),
    );
    fields.insert("summary".into(), Value::String(String::new()));

    let mut placeholders = HashMap::new();
    placeholders.insert("title".into(), title.to_string());

    let (fm, body) = template::render_template(tmpl_fm, &tmpl_body, &fields, &placeholders);
    let doc = frontmatter::serialize_document(&fm, &body);

    let dir_name = format!("{}-{}", id, slug);
    let dir_path = cfg.research_dir.join(&dir_name);
    fs::create_dir_all(&dir_path)?;
    fs::write(dir_path.join("_index.md"), &doc)?;

    // Create agent subdirectories
    for agent in &agents {
        mkdir_with_gitkeep(&dir_path.join(agent))?;
    }
    mkdir_with_gitkeep(&dir_path.join("final"))?;

    println!(
        "{} Created research {} ({}) at {}",
        "✓".green().bold(),
        id.to_string().cyan().bold(),
        title.bold(),
        dir_path.display().to_string().dimmed()
    );

    Ok(())
}

fn new_task(
    cfg: &ResolvedConfig,
    title: &str,
    project: Option<&str>,
    customer: Option<&str>,
    owner: Option<&str>,
    status: Option<&str>,
    priority: Option<u32>,
    tags: Option<&str>,
    sprint: Option<&str>,
    depends_on: Option<&str>,
    due_date: Option<&str>,
    yes: bool,
) -> McResult<()> {
    let id = entity::next_id(EntityKind::Task, cfg)?;
    let slug = util::slugify(title);
    let today = util::today_str();

    let owner = match owner {
        Some(o) => o.to_string(),
        None => prompt_input("Owner", "", yes),
    };
    let status = match status {
        Some(s) => s.to_string(),
        None => prompt_select("Status", &cfg.statuses.task, 0, yes),
    };
    let priority = priority.unwrap_or(3);
    let tags: Vec<String> = match tags {
        Some(t) => util::parse_comma_list(t),
        None => {
            let input = prompt_input_optional("Tags (comma-separated)", yes);
            if input.is_empty() {
                vec![]
            } else {
                util::parse_comma_list(&input)
            }
        }
    };
    let sprint = sprint.unwrap_or("").to_string();
    let depends_on: Vec<String> = depends_on
        .map(|d| util::parse_comma_list(d))
        .unwrap_or_default();
    let due_date = due_date.unwrap_or("").to_string();

    // Determine project/customer lists
    let projects: Vec<String> = project.map(|p| vec![p.to_string()]).unwrap_or_default();
    let customers: Vec<String> = customer.map(|c| vec![c.to_string()]).unwrap_or_default();

    let tags_display = tags.join(", ");
    let projects_display = projects.join(", ");
    let customers_display = customers.join(", ");
    let depends_display = depends_on.join(", ");
    let priority_label = match priority {
        1 => "1 (critical)",
        2 => "2 (high)",
        3 => "3 (medium)",
        4 => "4 (low)",
        _ => "3 (medium)",
    };
    print_summary(
        "task",
        &[
            ("ID", &id.to_string()),
            ("Title", title),
            ("Status", &status),
            ("Priority", priority_label),
            ("Owner", &owner),
            ("Projects", &projects_display),
            ("Customers", &customers_display),
            ("Sprint", &sprint),
            ("Tags", &tags_display),
            ("Depends on", &depends_display),
            ("Due date", &due_date),
        ],
    );

    if !confirm_creation(yes) {
        println!("{}", "Cancelled.".dimmed());
        return Ok(());
    }

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "task")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert("title".into(), Value::String(title.to_string()));
    fields.insert("slug".into(), Value::String(slug.clone()));
    fields.insert("status".into(), Value::String(status));
    fields.insert(
        "priority".into(),
        Value::Number(serde_yaml::Number::from(priority as u64)),
    );
    fields.insert("owner".into(), Value::String(owner));
    fields.insert(
        "projects".into(),
        Value::Sequence(projects.iter().map(|p| Value::String(p.clone())).collect()),
    );
    fields.insert(
        "customers".into(),
        Value::Sequence(customers.iter().map(|c| Value::String(c.clone())).collect()),
    );
    fields.insert(
        "tags".into(),
        Value::Sequence(tags.iter().map(|t| Value::String(t.clone())).collect()),
    );
    fields.insert("sprint".into(), Value::String(sprint));
    fields.insert(
        "depends_on".into(),
        Value::Sequence(
            depends_on
                .iter()
                .map(|d| Value::String(d.clone()))
                .collect(),
        ),
    );
    fields.insert("due_date".into(), Value::String(due_date));
    fields.insert("created".into(), Value::String(today.clone()));
    fields.insert("updated".into(), Value::String(today));

    let mut placeholders = HashMap::new();
    placeholders.insert("title".into(), title.to_string());

    let (fm, body) = template::render_template(tmpl_fm, &tmpl_body, &fields, &placeholders);
    let doc = frontmatter::serialize_document(&fm, &body);

    // Determine location based on --project or --customer flag
    let tasks_base = if let Some(proj_id) = project {
        // Find the project directory
        find_project_dir(cfg, proj_id)?.join("tasks")
    } else if let Some(cust_id) = customer {
        find_customer_dir(cfg, cust_id)?.join("tasks")
    } else {
        cfg.tasks_dir.clone()
    };

    // Create todo/ and done/ subfolders
    let todo_dir = tasks_base.join("todo");
    let done_dir = tasks_base.join("done");
    fs::create_dir_all(&todo_dir)?;
    if !done_dir.exists() {
        mkdir_with_gitkeep(&done_dir)?;
    }

    let filename = format!("{}-{}.md", id, slug);
    let file_path = todo_dir.join(&filename);
    fs::write(&file_path, &doc)?;

    println!(
        "{} Created task {} ({}) at {}",
        "✓".green().bold(),
        id.to_string().cyan().bold(),
        title.bold(),
        file_path.display().to_string().dimmed()
    );

    Ok(())
}

/// Find a project directory by its ID prefix (e.g. "PROJ-001").
fn find_project_dir(cfg: &ResolvedConfig, proj_id: &str) -> McResult<std::path::PathBuf> {
    if cfg.projects_dir.is_dir() {
        for entry in std::fs::read_dir(&cfg.projects_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(&format!("{}-", proj_id)) {
                    return Ok(entry.path());
                }
            }
        }
    }
    Err(McError::EntityNotFound(proj_id.to_string()))
}

/// Find a customer directory by its ID prefix (e.g. "CUST-001").
fn find_customer_dir(cfg: &ResolvedConfig, cust_id: &str) -> McResult<std::path::PathBuf> {
    if cfg.customers_dir.is_dir() {
        for entry in std::fs::read_dir(&cfg.customers_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(&format!("{}-", cust_id)) {
                    return Ok(entry.path());
                }
            }
        }
    }
    Err(McError::EntityNotFound(cust_id.to_string()))
}

// ---------------------------------------------------------------------------
// Programmatic creation functions (no prompts, no printing, return JSON)
// ---------------------------------------------------------------------------

pub fn create_customer_programmatic(
    cfg: &ResolvedConfig,
    name: &str,
    owner: Option<&str>,
    status: Option<&str>,
    tags: Option<&str>,
) -> McResult<JsonValue> {
    let id = entity::next_id(EntityKind::Customer, cfg)?;
    let slug = util::slugify(name);
    let today = util::today_str();

    let owner = owner.unwrap_or("").to_string();
    let status = status
        .map(|s| s.to_string())
        .unwrap_or_else(|| cfg.statuses.customer.first().cloned().unwrap_or_default());
    let tags: Vec<String> = tags.map(|t| util::parse_comma_list(t)).unwrap_or_default();

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "customer")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert("name".into(), Value::String(name.to_string()));
    fields.insert("slug".into(), Value::String(slug.clone()));
    fields.insert("status".into(), Value::String(status));
    fields.insert("owner".into(), Value::String(owner));
    fields.insert(
        "tags".into(),
        Value::Sequence(tags.iter().map(|t| Value::String(t.clone())).collect()),
    );
    fields.insert("projects".into(), Value::Sequence(vec![]));
    fields.insert("contracts".into(), Value::Sequence(vec![]));
    fields.insert("notes".into(), Value::String(String::new()));
    fields.insert("created".into(), Value::String(today.clone()));
    fields.insert("updated".into(), Value::String(today));

    let mut placeholders = HashMap::new();
    placeholders.insert("name".into(), name.to_string());

    let (fm, body) = template::render_template(tmpl_fm, &tmpl_body, &fields, &placeholders);
    let doc = frontmatter::serialize_document(&fm, &body);

    let dir_name = format!("{}-{}", id, slug);
    let dir_path = cfg.customers_dir.join(&dir_name);
    fs::create_dir_all(&dir_path)?;
    fs::write(dir_path.join("_index.md"), &doc)?;
    fs::write(
        dir_path.join("contacts.md"),
        format!("# {} -- Contacts\n", name),
    )?;
    mkdir_with_gitkeep(&dir_path.join("contracts"))?;
    mkdir_with_gitkeep(&dir_path.join("meetings"))?;
    mkdir_with_gitkeep(&dir_path.join("projects"))?;
    mkdir_with_gitkeep(&dir_path.join("assets"))?;

    Ok(serde_json::json!({
        "id": id.to_string(),
        "name": name,
        "path": dir_path.display().to_string(),
    }))
}

pub fn create_project_programmatic(
    cfg: &ResolvedConfig,
    name: &str,
    owner: Option<&str>,
    status: Option<&str>,
    customers: Option<&str>,
    tags: Option<&str>,
) -> McResult<JsonValue> {
    let id = entity::next_id(EntityKind::Project, cfg)?;
    let slug = util::slugify(name);
    let today = util::today_str();

    let owner = owner.unwrap_or("").to_string();
    let status = status
        .map(|s| s.to_string())
        .unwrap_or_else(|| cfg.statuses.project.first().cloned().unwrap_or_default());
    let tags: Vec<String> = tags.map(|t| util::parse_comma_list(t)).unwrap_or_default();
    let customers: Vec<String> = customers
        .map(|c| util::parse_comma_list(c))
        .unwrap_or_default();

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "project")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert("name".into(), Value::String(name.to_string()));
    fields.insert("slug".into(), Value::String(slug.clone()));
    fields.insert("status".into(), Value::String(status));
    fields.insert("owner".into(), Value::String(owner));
    fields.insert(
        "customers".into(),
        Value::Sequence(customers.iter().map(|c| Value::String(c.clone())).collect()),
    );
    fields.insert(
        "tags".into(),
        Value::Sequence(tags.iter().map(|t| Value::String(t.clone())).collect()),
    );
    fields.insert("start_date".into(), Value::String(today.clone()));
    fields.insert("target_date".into(), Value::String(String::new()));
    fields.insert("created".into(), Value::String(today.clone()));
    fields.insert("updated".into(), Value::String(today));

    let mut placeholders = HashMap::new();
    placeholders.insert("name".into(), name.to_string());

    let (fm, body) = template::render_template(tmpl_fm, &tmpl_body, &fields, &placeholders);
    let doc = frontmatter::serialize_document(&fm, &body);

    let dir_name = format!("{}-{}", id, slug);
    let dir_path = cfg.projects_dir.join(&dir_name);
    fs::create_dir_all(&dir_path)?;
    fs::write(dir_path.join("overview.md"), &doc)?;
    fs::write(
        dir_path.join("roadmap.md"),
        format!("# {} -- Roadmap\n", name),
    )?;
    fs::write(
        dir_path.join("backlog.md"),
        format!("# {} -- Backlog\n", name),
    )?;
    mkdir_with_gitkeep(&dir_path.join("specs"))?;
    mkdir_with_gitkeep(&dir_path.join("releases"))?;
    mkdir_with_gitkeep(&dir_path.join("infra"))?;

    Ok(serde_json::json!({
        "id": id.to_string(),
        "name": name,
        "path": dir_path.display().to_string(),
    }))
}

pub fn create_meeting_programmatic(
    cfg: &ResolvedConfig,
    title: &str,
    date: Option<&str>,
    time: Option<&str>,
    duration: Option<&str>,
    status: Option<&str>,
    tags: Option<&str>,
    customers: Option<&str>,
    projects: Option<&str>,
) -> McResult<JsonValue> {
    let id = entity::next_id(EntityKind::Meeting, cfg)?;
    let today = util::today_str();
    let date = date.unwrap_or(&today).to_string();
    let slug = util::slugify(title);

    let time = time.unwrap_or("10:00").to_string();
    let duration = duration.unwrap_or("30m").to_string();
    let status = status
        .map(|s| s.to_string())
        .unwrap_or_else(|| cfg.statuses.meeting.first().cloned().unwrap_or_default());
    let tags: Vec<String> = tags.map(|t| util::parse_comma_list(t)).unwrap_or_default();
    let customers: Vec<String> = customers
        .map(|c| util::parse_comma_list(c))
        .unwrap_or_default();
    let projects: Vec<String> = projects
        .map(|p| util::parse_comma_list(p))
        .unwrap_or_default();

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "meeting")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert("title".into(), Value::String(title.to_string()));
    fields.insert("date".into(), Value::String(date.clone()));
    fields.insert("time".into(), Value::String(time));
    fields.insert("duration".into(), Value::String(duration));
    fields.insert(
        "tags".into(),
        Value::Sequence(tags.iter().map(|t| Value::String(t.clone())).collect()),
    );
    fields.insert(
        "customers".into(),
        Value::Sequence(customers.iter().map(|c| Value::String(c.clone())).collect()),
    );
    fields.insert(
        "projects".into(),
        Value::Sequence(projects.iter().map(|p| Value::String(p.clone())).collect()),
    );
    fields.insert("status".into(), Value::String(status));

    let mut placeholders = HashMap::new();
    placeholders.insert("title".into(), title.to_string());

    let (fm, body) = template::render_template(tmpl_fm, &tmpl_body, &fields, &placeholders);
    let doc = frontmatter::serialize_document(&fm, &body);

    let filename = format!("{}-{}.md", date, slug);
    let file_path = cfg.meetings_dir.join(&filename);
    fs::create_dir_all(&cfg.meetings_dir)?;
    fs::write(&file_path, &doc)?;

    Ok(serde_json::json!({
        "id": id.to_string(),
        "title": title,
        "path": file_path.display().to_string(),
    }))
}

pub fn create_research_programmatic(
    cfg: &ResolvedConfig,
    title: &str,
    owner: Option<&str>,
    agents: Option<&str>,
    tags: Option<&str>,
) -> McResult<JsonValue> {
    let id = entity::next_id(EntityKind::Research, cfg)?;
    let slug = util::slugify(title);
    let today = util::today_str();

    let owner = owner.unwrap_or("").to_string();
    let agents: Vec<String> = agents
        .map(|a| util::parse_comma_list(a))
        .unwrap_or_else(|| {
            vec![
                "claude".into(),
                "gemini".into(),
                "chatgpt".into(),
                "perplexity".into(),
            ]
        });
    let tags: Vec<String> = tags.map(|t| util::parse_comma_list(t)).unwrap_or_default();

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "research")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert("title".into(), Value::String(title.to_string()));
    fields.insert("slug".into(), Value::String(slug.clone()));
    fields.insert("status".into(), Value::String("draft".into()));
    fields.insert("owner".into(), Value::String(owner));
    fields.insert("customers".into(), Value::Sequence(vec![]));
    fields.insert("projects".into(), Value::Sequence(vec![]));
    fields.insert(
        "tags".into(),
        Value::Sequence(tags.iter().map(|t| Value::String(t.clone())).collect()),
    );
    fields.insert("created".into(), Value::String(today.clone()));
    fields.insert("updated".into(), Value::String(today));
    fields.insert(
        "agents".into(),
        Value::Sequence(agents.iter().map(|a| Value::String(a.clone())).collect()),
    );
    fields.insert("summary".into(), Value::String(String::new()));

    let mut placeholders = HashMap::new();
    placeholders.insert("title".into(), title.to_string());

    let (fm, body) = template::render_template(tmpl_fm, &tmpl_body, &fields, &placeholders);
    let doc = frontmatter::serialize_document(&fm, &body);

    let dir_name = format!("{}-{}", id, slug);
    let dir_path = cfg.research_dir.join(&dir_name);
    fs::create_dir_all(&dir_path)?;
    fs::write(dir_path.join("_index.md"), &doc)?;

    for agent in &agents {
        mkdir_with_gitkeep(&dir_path.join(agent))?;
    }
    mkdir_with_gitkeep(&dir_path.join("final"))?;

    Ok(serde_json::json!({
        "id": id.to_string(),
        "title": title,
        "path": dir_path.display().to_string(),
    }))
}

pub fn create_task_programmatic(
    cfg: &ResolvedConfig,
    title: &str,
    project: Option<&str>,
    customer: Option<&str>,
    owner: Option<&str>,
    status: Option<&str>,
    priority: Option<u32>,
    tags: Option<&str>,
    sprint: Option<&str>,
    depends_on: Option<&str>,
    due_date: Option<&str>,
) -> McResult<JsonValue> {
    let id = entity::next_id(EntityKind::Task, cfg)?;
    let slug = util::slugify(title);
    let today = util::today_str();

    let owner = owner.unwrap_or("").to_string();
    let status = status
        .map(|s| s.to_string())
        .unwrap_or_else(|| cfg.statuses.task.first().cloned().unwrap_or_default());
    let priority = priority.unwrap_or(3);
    let tags: Vec<String> = tags.map(|t| util::parse_comma_list(t)).unwrap_or_default();
    let sprint = sprint.unwrap_or("").to_string();
    let depends_on: Vec<String> = depends_on
        .map(|d| util::parse_comma_list(d))
        .unwrap_or_default();
    let due_date = due_date.unwrap_or("").to_string();

    let projects: Vec<String> = project.map(|p| vec![p.to_string()]).unwrap_or_default();
    let customers: Vec<String> = customer.map(|c| vec![c.to_string()]).unwrap_or_default();

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "task")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert("title".into(), Value::String(title.to_string()));
    fields.insert("slug".into(), Value::String(slug.clone()));
    fields.insert("status".into(), Value::String(status));
    fields.insert(
        "priority".into(),
        Value::Number(serde_yaml::Number::from(priority as u64)),
    );
    fields.insert("owner".into(), Value::String(owner));
    fields.insert(
        "projects".into(),
        Value::Sequence(projects.iter().map(|p| Value::String(p.clone())).collect()),
    );
    fields.insert(
        "customers".into(),
        Value::Sequence(customers.iter().map(|c| Value::String(c.clone())).collect()),
    );
    fields.insert(
        "tags".into(),
        Value::Sequence(tags.iter().map(|t| Value::String(t.clone())).collect()),
    );
    fields.insert("sprint".into(), Value::String(sprint));
    fields.insert(
        "depends_on".into(),
        Value::Sequence(
            depends_on
                .iter()
                .map(|d| Value::String(d.clone()))
                .collect(),
        ),
    );
    fields.insert("due_date".into(), Value::String(due_date));
    fields.insert("created".into(), Value::String(today.clone()));
    fields.insert("updated".into(), Value::String(today));

    let mut placeholders = HashMap::new();
    placeholders.insert("title".into(), title.to_string());

    let (fm, body) = template::render_template(tmpl_fm, &tmpl_body, &fields, &placeholders);
    let doc = frontmatter::serialize_document(&fm, &body);

    // Determine location based on project or customer scope
    let tasks_base = if let Some(proj_id) = project {
        find_project_dir(cfg, proj_id)?.join("tasks")
    } else if let Some(cust_id) = customer {
        find_customer_dir(cfg, cust_id)?.join("tasks")
    } else {
        cfg.tasks_dir.clone()
    };

    // Create todo/ and done/ subfolders
    let todo_dir = tasks_base.join("todo");
    let done_dir = tasks_base.join("done");
    fs::create_dir_all(&todo_dir)?;
    if !done_dir.exists() {
        mkdir_with_gitkeep(&done_dir)?;
    }

    let filename = format!("{}-{}.md", id, slug);
    let file_path = todo_dir.join(&filename);
    fs::write(&file_path, &doc)?;

    Ok(serde_json::json!({
        "id": id.to_string(),
        "title": title,
        "path": file_path.display().to_string(),
    }))
}
