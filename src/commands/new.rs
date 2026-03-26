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

fn check_mode(kind: EntityKind, cfg: &ResolvedConfig) -> McResult<()> {
    if !cfg.entity_available(&kind) {
        return Err(McError::NotAvailableInMode {
            kind: kind.label().to_string(),
        });
    }
    Ok(())
}

fn validate_status(status: &str, kind: EntityKind, cfg: &ResolvedConfig) -> McResult<()> {
    let valid = kind.statuses(cfg);
    if !valid.iter().any(|s| s == status) {
        return Err(McError::Other(format!(
            "Invalid {} status '{}'. Valid statuses: {}",
            kind.label(),
            status,
            valid.join(", ")
        )));
    }
    Ok(())
}

fn validate_name_not_empty(name: &str, kind: EntityKind) -> McResult<()> {
    if name.trim().is_empty() {
        return Err(McError::Other(format!(
            "{} name/title cannot be empty",
            kind.label()
        )));
    }
    Ok(())
}

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
            attendees,
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
            attendees.as_deref(),
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
        NewEntity::Sprint {
            title,
            owner,
            status,
            goal,
            start_date,
            end_date,
            projects,
            tags,
        } => new_sprint(
            cfg,
            title,
            owner.as_deref(),
            status.as_deref(),
            goal.as_deref(),
            start_date.as_deref(),
            end_date.as_deref(),
            projects.as_deref(),
            tags.as_deref(),
            yes,
        ),
        NewEntity::Proposal {
            title,
            author,
            status,
            proposal_type,
            tags,
            supersedes,
        } => new_proposal(
            cfg,
            title,
            author.as_deref(),
            status.as_deref(),
            proposal_type.as_deref(),
            tags.as_deref(),
            supersedes.as_deref(),
            yes,
        ),
        NewEntity::Contact {
            name,
            customer,
            role,
            email,
            phone,
            status,
            tags,
        } => new_contact(
            cfg,
            name,
            customer,
            role.as_deref(),
            email.as_deref(),
            phone.as_deref(),
            status.as_deref(),
            tags.as_deref(),
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
    result.unwrap_or_default()
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
    // Auto-confirm in non-interactive environments (pipes, agent shells, CI)
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return true;
    }
    let result = dialoguer::Confirm::new()
        .with_prompt("Create this entity?")
        .default(true)
        .interact();
    result.unwrap_or_default()
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
    check_mode(EntityKind::Customer, cfg)?;
    validate_name_not_empty(name, EntityKind::Customer)?;
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
    validate_status(&status, EntityKind::Customer, cfg)?;
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
    fields.insert(
        "aliases".into(),
        Value::Sequence(vec![Value::String(id.to_string())]),
    );
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
    util::atomic_write(&dir_path.join(format!("{}.md", id)), doc.as_bytes())?;
    mkdir_with_gitkeep(&dir_path.join("contacts"))?;
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
    check_mode(EntityKind::Project, cfg)?;
    validate_name_not_empty(name, EntityKind::Project)?;
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
    validate_status(&status, EntityKind::Project, cfg)?;
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
    fields.insert(
        "aliases".into(),
        Value::Sequence(vec![Value::String(id.to_string())]),
    );
    fields.insert("name".into(), Value::String(name.to_string()));
    fields.insert("slug".into(), Value::String(slug.clone()));
    fields.insert("status".into(), Value::String(status));
    fields.insert("owner".into(), Value::String(owner));
    fields.insert(
        "customers".into(),
        Value::Sequence(
            customers
                .iter()
                .map(|c| Value::String(frontmatter::wrap_wikilink(c)))
                .collect(),
        ),
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
    util::atomic_write(&dir_path.join(format!("{}.md", id)), doc.as_bytes())?;
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

#[allow(clippy::too_many_arguments)]
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
    attendees: Option<&str>,
    yes: bool,
) -> McResult<()> {
    validate_name_not_empty(title, EntityKind::Meeting)?;
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
    validate_status(&status, EntityKind::Meeting, cfg)?;
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

    let attendees: Vec<String> = attendees.map(util::parse_comma_list).unwrap_or_default();

    let tags_display = tags.join(", ");
    let customers_display = customers.join(", ");
    let projects_display = projects.join(", ");
    let attendees_display = attendees.join(", ");
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
            ("Attendees", &attendees_display),
        ],
    );

    if !confirm_creation(yes) {
        println!("{}", "Cancelled.".dimmed());
        return Ok(());
    }

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "meeting")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert(
        "aliases".into(),
        Value::Sequence(vec![Value::String(id.to_string())]),
    );
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
        Value::Sequence(
            customers
                .iter()
                .map(|c| Value::String(frontmatter::wrap_wikilink(c)))
                .collect(),
        ),
    );
    fields.insert(
        "projects".into(),
        Value::Sequence(
            projects
                .iter()
                .map(|p| Value::String(frontmatter::wrap_wikilink(p)))
                .collect(),
        ),
    );
    fields.insert(
        "attendees".into(),
        Value::Sequence(attendees.iter().map(|a| Value::String(a.clone())).collect()),
    );
    fields.insert("status".into(), Value::String(status));

    let mut placeholders = HashMap::new();
    placeholders.insert("title".into(), title.to_string());

    let (fm, body) = template::render_template(tmpl_fm, &tmpl_body, &fields, &placeholders);
    let doc = frontmatter::serialize_document(&fm, &body);

    let filename = format!("{}-{}.md", date, slug);
    let file_path = cfg.meetings_dir.join(&filename);
    fs::create_dir_all(&cfg.meetings_dir)?;
    util::atomic_write(&file_path, doc.as_bytes())?;

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
    validate_name_not_empty(title, EntityKind::Research)?;
    let id = entity::next_id(EntityKind::Research, cfg)?;
    let slug = util::slugify(title);
    let today = util::today_str();

    let owner = match owner {
        Some(o) => o.to_string(),
        None => prompt_input("Owner", "", yes),
    };
    let agents: Vec<String> = agents.map(util::parse_comma_list).unwrap_or_else(|| {
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
    fields.insert(
        "aliases".into(),
        Value::Sequence(vec![Value::String(id.to_string())]),
    );
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
    util::atomic_write(&dir_path.join(format!("{}.md", id)), doc.as_bytes())?;

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

#[allow(clippy::too_many_arguments)]
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
    validate_name_not_empty(title, EntityKind::Task)?;
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
    validate_status(&status, EntityKind::Task, cfg)?;
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
    let depends_on: Vec<String> = depends_on.map(util::parse_comma_list).unwrap_or_default();
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
    fields.insert(
        "aliases".into(),
        Value::Sequence(vec![Value::String(id.to_string())]),
    );
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
        Value::Sequence(
            projects
                .iter()
                .map(|p| Value::String(frontmatter::wrap_wikilink(p)))
                .collect(),
        ),
    );
    fields.insert(
        "customers".into(),
        Value::Sequence(
            customers
                .iter()
                .map(|c| Value::String(frontmatter::wrap_wikilink(c)))
                .collect(),
        ),
    );
    fields.insert(
        "tags".into(),
        Value::Sequence(tags.iter().map(|t| Value::String(t.clone())).collect()),
    );
    fields.insert(
        "sprint".into(),
        Value::String(frontmatter::wrap_wikilink(&sprint)),
    );
    fields.insert(
        "depends_on".into(),
        Value::Sequence(
            depends_on
                .iter()
                .map(|d| Value::String(frontmatter::wrap_wikilink(d)))
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
    util::atomic_write(&file_path, doc.as_bytes())?;

    println!(
        "{} Created task {} ({}) at {}",
        "✓".green().bold(),
        id.to_string().cyan().bold(),
        title.bold(),
        file_path.display().to_string().dimmed()
    );

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn new_sprint(
    cfg: &ResolvedConfig,
    title: &str,
    owner: Option<&str>,
    status: Option<&str>,
    goal: Option<&str>,
    start_date: Option<&str>,
    end_date: Option<&str>,
    projects: Option<&str>,
    tags: Option<&str>,
    yes: bool,
) -> McResult<()> {
    validate_name_not_empty(title, EntityKind::Sprint)?;
    let id = entity::next_id(EntityKind::Sprint, cfg)?;
    let slug = util::slugify(title);
    let today = util::today_str();

    let owner = match owner {
        Some(o) => o.to_string(),
        None => prompt_input("Owner", "", yes),
    };
    let status = match status {
        Some(s) => s.to_string(),
        None => prompt_select("Status", &cfg.statuses.sprint, 0, yes),
    };
    validate_status(&status, EntityKind::Sprint, cfg)?;
    let goal = match goal {
        Some(g) => g.to_string(),
        None => prompt_input_optional("Sprint goal", yes),
    };
    let start_date = start_date.unwrap_or(&today).to_string();
    let end_date = end_date.unwrap_or("").to_string();
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
    let projects_display = projects.join(", ");
    print_summary(
        "sprint",
        &[
            ("ID", &id.to_string()),
            ("Title", title),
            ("Status", &status),
            ("Goal", &goal),
            ("Start", &start_date),
            ("End", &end_date),
            ("Owner", &owner),
            ("Projects", &projects_display),
            ("Tags", &tags_display),
        ],
    );

    if !confirm_creation(yes) {
        println!("{}", "Cancelled.".dimmed());
        return Ok(());
    }

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "sprint")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert(
        "aliases".into(),
        Value::Sequence(vec![Value::String(id.to_string())]),
    );
    fields.insert("title".into(), Value::String(title.to_string()));
    fields.insert("status".into(), Value::String(status));
    fields.insert("goal".into(), Value::String(goal));
    fields.insert("start_date".into(), Value::String(start_date));
    fields.insert("end_date".into(), Value::String(end_date));
    fields.insert("owner".into(), Value::String(owner));
    fields.insert(
        "projects".into(),
        Value::Sequence(
            projects
                .iter()
                .map(|p| Value::String(frontmatter::wrap_wikilink(p)))
                .collect(),
        ),
    );
    fields.insert(
        "tags".into(),
        Value::Sequence(tags.iter().map(|t| Value::String(t.clone())).collect()),
    );
    fields.insert("created".into(), Value::String(today.clone()));
    fields.insert("updated".into(), Value::String(today));

    let mut placeholders = HashMap::new();
    placeholders.insert("title".into(), title.to_string());

    let (fm, body) = template::render_template(tmpl_fm, &tmpl_body, &fields, &placeholders);
    let doc = frontmatter::serialize_document(&fm, &body);

    let dir_name = format!("{}-{}", id, slug);
    let dir_path = cfg.sprints_dir.join(&dir_name);
    fs::create_dir_all(&dir_path)?;
    util::atomic_write(&dir_path.join(format!("{}.md", id)), doc.as_bytes())?;

    // Create ceremony stub files
    fs::write(
        dir_path.join("planning.md"),
        format!(
            "# {} -- Sprint Planning\n\n## Capacity\n\n## Selected Items\n\n## Notes\n",
            title
        ),
    )?;
    fs::write(
        dir_path.join("review.md"),
        format!(
            "# {} -- Sprint Review\n\n## Demo Outcomes\n\n## Feedback\n\n## Notes\n",
            title
        ),
    )?;
    fs::write(
        dir_path.join("retrospective.md"),
        format!(
            "# {} -- Retrospective\n\n## What Went Well\n\n## What Could Improve\n\n## Action Items\n",
            title
        ),
    )?;

    println!(
        "{} Created sprint {} ({}) at {}",
        "✓".green().bold(),
        id.to_string().cyan().bold(),
        title.bold(),
        dir_path.display().to_string().dimmed()
    );

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn new_proposal(
    cfg: &ResolvedConfig,
    title: &str,
    author: Option<&str>,
    status: Option<&str>,
    proposal_type: Option<&str>,
    tags: Option<&str>,
    supersedes: Option<&str>,
    yes: bool,
) -> McResult<()> {
    check_mode(EntityKind::Proposal, cfg)?;
    validate_name_not_empty(title, EntityKind::Proposal)?;
    let id = entity::next_id(EntityKind::Proposal, cfg)?;
    let slug = util::slugify(title);
    let today = util::today_str();

    let author = match author {
        Some(a) => a.to_string(),
        None => prompt_input("Author", "", yes),
    };
    let status = match status {
        Some(s) => s.to_string(),
        None => prompt_select("Status", &cfg.statuses.proposal, 0, yes),
    };
    validate_status(&status, EntityKind::Proposal, cfg)?;
    let proposal_type = match proposal_type {
        Some(t) => t.to_string(),
        None => {
            let types = vec![
                "architecture".to_string(),
                "feature".to_string(),
                "process".to_string(),
            ];
            prompt_select("Type", &types, 0, yes)
        }
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
    let supersedes = supersedes.unwrap_or("").to_string();

    let tags_display = tags.join(", ");
    print_summary(
        "proposal",
        &[
            ("ID", &id.to_string()),
            ("Title", title),
            ("Author", &author),
            ("Status", &status),
            ("Type", &proposal_type),
            ("Tags", &tags_display),
            ("Supersedes", &supersedes),
        ],
    );

    if !confirm_creation(yes) {
        println!("{}", "Cancelled.".dimmed());
        return Ok(());
    }

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "proposal")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert(
        "aliases".into(),
        Value::Sequence(vec![Value::String(id.to_string())]),
    );
    fields.insert("title".into(), Value::String(title.to_string()));
    fields.insert("status".into(), Value::String(status));
    fields.insert("type".into(), Value::String(proposal_type));
    fields.insert("author".into(), Value::String(author));
    fields.insert(
        "supersedes".into(),
        Value::String(frontmatter::wrap_wikilink(&supersedes)),
    );
    fields.insert("superseded_by".into(), Value::String(String::new()));
    fields.insert(
        "tags".into(),
        Value::Sequence(tags.iter().map(|t| Value::String(t.clone())).collect()),
    );
    fields.insert("created".into(), Value::String(today.clone()));
    fields.insert("updated".into(), Value::String(today));

    let mut placeholders = HashMap::new();
    placeholders.insert("title".into(), title.to_string());

    let (fm, body) = template::render_template(tmpl_fm, &tmpl_body, &fields, &placeholders);
    let doc = frontmatter::serialize_document(&fm, &body);

    let filename = format!("{}-{}.md", id, slug);
    let file_path = cfg.proposals_dir.join(&filename);
    fs::create_dir_all(&cfg.proposals_dir)?;
    util::atomic_write(&file_path, doc.as_bytes())?;

    println!(
        "{} Created proposal {} ({}) at {}",
        "✓".green().bold(),
        id.to_string().cyan().bold(),
        title.bold(),
        file_path.display().to_string().dimmed()
    );

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn new_contact(
    cfg: &ResolvedConfig,
    name: &str,
    customer: &str,
    role: Option<&str>,
    email: Option<&str>,
    phone: Option<&str>,
    status: Option<&str>,
    tags: Option<&str>,
    yes: bool,
) -> McResult<()> {
    check_mode(EntityKind::Contact, cfg)?;
    validate_name_not_empty(name, EntityKind::Contact)?;

    // Validate customer exists
    let cust_dir = find_customer_dir(cfg, customer)?;

    let id = entity::next_id(EntityKind::Contact, cfg)?;
    let today = util::today_str();

    let role = role.unwrap_or("").to_string();
    let email = email.unwrap_or("").to_string();
    let phone = phone.unwrap_or("").to_string();
    let status = match status {
        Some(s) => s.to_string(),
        None => prompt_select("Status", &cfg.statuses.contact, 0, yes),
    };
    validate_status(&status, EntityKind::Contact, cfg)?;
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
        "contact",
        &[
            ("ID", &id.to_string()),
            ("Name", name),
            ("Customer", customer),
            ("Role", &role),
            ("Email", &email),
            ("Phone", &phone),
            ("Status", &status),
            ("Tags", &tags_display),
        ],
    );

    if !confirm_creation(yes) {
        println!("{}", "Cancelled.".dimmed());
        return Ok(());
    }

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "contact")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert(
        "aliases".into(),
        Value::Sequence(vec![Value::String(id.to_string())]),
    );
    fields.insert("name".into(), Value::String(name.to_string()));
    fields.insert("role".into(), Value::String(role));
    fields.insert("email".into(), Value::String(email));
    fields.insert("phone".into(), Value::String(phone));
    fields.insert(
        "customer".into(),
        Value::String(frontmatter::wrap_wikilink(customer)),
    );
    fields.insert("status".into(), Value::String(status));
    fields.insert(
        "tags".into(),
        Value::Sequence(tags.iter().map(|t| Value::String(t.clone())).collect()),
    );
    fields.insert("created".into(), Value::String(today.clone()));
    fields.insert("updated".into(), Value::String(today));

    let mut placeholders = HashMap::new();
    placeholders.insert("name".into(), name.to_string());

    let (fm, body) = template::render_template(tmpl_fm, &tmpl_body, &fields, &placeholders);
    let doc = frontmatter::serialize_document(&fm, &body);

    let slug = util::slugify(name);
    let filename = format!("{}-{}.md", id, slug);
    let contacts_dir = cust_dir.join("contacts");
    fs::create_dir_all(&contacts_dir)?;
    let file_path = contacts_dir.join(&filename);
    util::atomic_write(&file_path, doc.as_bytes())?;

    println!(
        "{} Created contact {} ({}) at {}",
        "✓".green().bold(),
        id.to_string().cyan().bold(),
        name.bold(),
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
pub fn find_customer_dir(cfg: &ResolvedConfig, cust_id: &str) -> McResult<std::path::PathBuf> {
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
    check_mode(EntityKind::Customer, cfg)?;
    validate_name_not_empty(name, EntityKind::Customer)?;
    let id = entity::next_id(EntityKind::Customer, cfg)?;
    let slug = util::slugify(name);
    let today = util::today_str();

    let owner = owner.unwrap_or("").to_string();
    let status = status
        .map(|s| s.to_string())
        .unwrap_or_else(|| cfg.statuses.customer.first().cloned().unwrap_or_default());
    validate_status(&status, EntityKind::Customer, cfg)?;
    let tags: Vec<String> = tags.map(util::parse_comma_list).unwrap_or_default();

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "customer")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert(
        "aliases".into(),
        Value::Sequence(vec![Value::String(id.to_string())]),
    );
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
    util::atomic_write(&dir_path.join(format!("{}.md", id)), doc.as_bytes())?;
    mkdir_with_gitkeep(&dir_path.join("contacts"))?;
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
    check_mode(EntityKind::Project, cfg)?;
    validate_name_not_empty(name, EntityKind::Project)?;
    let id = entity::next_id(EntityKind::Project, cfg)?;
    let slug = util::slugify(name);
    let today = util::today_str();

    let owner = owner.unwrap_or("").to_string();
    let status = status
        .map(|s| s.to_string())
        .unwrap_or_else(|| cfg.statuses.project.first().cloned().unwrap_or_default());
    validate_status(&status, EntityKind::Project, cfg)?;
    let tags: Vec<String> = tags.map(util::parse_comma_list).unwrap_or_default();
    let customers: Vec<String> = customers.map(util::parse_comma_list).unwrap_or_default();

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "project")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert(
        "aliases".into(),
        Value::Sequence(vec![Value::String(id.to_string())]),
    );
    fields.insert("name".into(), Value::String(name.to_string()));
    fields.insert("slug".into(), Value::String(slug.clone()));
    fields.insert("status".into(), Value::String(status));
    fields.insert("owner".into(), Value::String(owner));
    fields.insert(
        "customers".into(),
        Value::Sequence(
            customers
                .iter()
                .map(|c| Value::String(frontmatter::wrap_wikilink(c)))
                .collect(),
        ),
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
    util::atomic_write(&dir_path.join(format!("{}.md", id)), doc.as_bytes())?;
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

#[allow(clippy::too_many_arguments)]
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
    attendees: Option<&str>,
) -> McResult<JsonValue> {
    validate_name_not_empty(title, EntityKind::Meeting)?;
    let id = entity::next_id(EntityKind::Meeting, cfg)?;
    let today = util::today_str();
    let date = date.unwrap_or(&today).to_string();
    let slug = util::slugify(title);

    let time = time.unwrap_or("10:00").to_string();
    let duration = duration.unwrap_or("30m").to_string();
    let status = status
        .map(|s| s.to_string())
        .unwrap_or_else(|| cfg.statuses.meeting.first().cloned().unwrap_or_default());
    validate_status(&status, EntityKind::Meeting, cfg)?;
    let tags: Vec<String> = tags.map(util::parse_comma_list).unwrap_or_default();
    let customers: Vec<String> = customers.map(util::parse_comma_list).unwrap_or_default();
    let projects: Vec<String> = projects.map(util::parse_comma_list).unwrap_or_default();
    let attendees: Vec<String> = attendees.map(util::parse_comma_list).unwrap_or_default();

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "meeting")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert(
        "aliases".into(),
        Value::Sequence(vec![Value::String(id.to_string())]),
    );
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
        Value::Sequence(
            customers
                .iter()
                .map(|c| Value::String(frontmatter::wrap_wikilink(c)))
                .collect(),
        ),
    );
    fields.insert(
        "projects".into(),
        Value::Sequence(
            projects
                .iter()
                .map(|p| Value::String(frontmatter::wrap_wikilink(p)))
                .collect(),
        ),
    );
    fields.insert(
        "attendees".into(),
        Value::Sequence(attendees.iter().map(|a| Value::String(a.clone())).collect()),
    );
    fields.insert("status".into(), Value::String(status));

    let mut placeholders = HashMap::new();
    placeholders.insert("title".into(), title.to_string());

    let (fm, body) = template::render_template(tmpl_fm, &tmpl_body, &fields, &placeholders);
    let doc = frontmatter::serialize_document(&fm, &body);

    let filename = format!("{}-{}.md", date, slug);
    let file_path = cfg.meetings_dir.join(&filename);
    fs::create_dir_all(&cfg.meetings_dir)?;
    util::atomic_write(&file_path, doc.as_bytes())?;

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
    validate_name_not_empty(title, EntityKind::Research)?;
    let id = entity::next_id(EntityKind::Research, cfg)?;
    let slug = util::slugify(title);
    let today = util::today_str();

    let owner = owner.unwrap_or("").to_string();
    let agents: Vec<String> = agents.map(util::parse_comma_list).unwrap_or_else(|| {
        vec![
            "claude".into(),
            "gemini".into(),
            "chatgpt".into(),
            "perplexity".into(),
        ]
    });
    let tags: Vec<String> = tags.map(util::parse_comma_list).unwrap_or_default();

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "research")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert(
        "aliases".into(),
        Value::Sequence(vec![Value::String(id.to_string())]),
    );
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
    util::atomic_write(&dir_path.join(format!("{}.md", id)), doc.as_bytes())?;

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

#[allow(clippy::too_many_arguments)]
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
    validate_name_not_empty(title, EntityKind::Task)?;
    let id = entity::next_id(EntityKind::Task, cfg)?;
    let slug = util::slugify(title);
    let today = util::today_str();

    let owner = owner.unwrap_or("").to_string();
    let status = status
        .map(|s| s.to_string())
        .unwrap_or_else(|| cfg.statuses.task.first().cloned().unwrap_or_default());
    validate_status(&status, EntityKind::Task, cfg)?;
    let priority = priority.unwrap_or(3);
    let tags: Vec<String> = tags.map(util::parse_comma_list).unwrap_or_default();
    let sprint = sprint.unwrap_or("").to_string();
    let depends_on: Vec<String> = depends_on.map(util::parse_comma_list).unwrap_or_default();
    let due_date = due_date.unwrap_or("").to_string();

    let projects: Vec<String> = project.map(|p| vec![p.to_string()]).unwrap_or_default();
    let customers: Vec<String> = customer.map(|c| vec![c.to_string()]).unwrap_or_default();

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "task")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert(
        "aliases".into(),
        Value::Sequence(vec![Value::String(id.to_string())]),
    );
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
        Value::Sequence(
            projects
                .iter()
                .map(|p| Value::String(frontmatter::wrap_wikilink(p)))
                .collect(),
        ),
    );
    fields.insert(
        "customers".into(),
        Value::Sequence(
            customers
                .iter()
                .map(|c| Value::String(frontmatter::wrap_wikilink(c)))
                .collect(),
        ),
    );
    fields.insert(
        "tags".into(),
        Value::Sequence(tags.iter().map(|t| Value::String(t.clone())).collect()),
    );
    fields.insert(
        "sprint".into(),
        Value::String(frontmatter::wrap_wikilink(&sprint)),
    );
    fields.insert(
        "depends_on".into(),
        Value::Sequence(
            depends_on
                .iter()
                .map(|d| Value::String(frontmatter::wrap_wikilink(d)))
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
    util::atomic_write(&file_path, doc.as_bytes())?;

    Ok(serde_json::json!({
        "id": id.to_string(),
        "title": title,
        "path": file_path.display().to_string(),
    }))
}

#[allow(clippy::too_many_arguments)]
pub fn create_sprint_programmatic(
    cfg: &ResolvedConfig,
    title: &str,
    owner: Option<&str>,
    status: Option<&str>,
    goal: Option<&str>,
    start_date: Option<&str>,
    end_date: Option<&str>,
    projects: Option<&str>,
    tags: Option<&str>,
) -> McResult<JsonValue> {
    validate_name_not_empty(title, EntityKind::Sprint)?;
    let id = entity::next_id(EntityKind::Sprint, cfg)?;
    let slug = util::slugify(title);
    let today = util::today_str();

    let owner = owner.unwrap_or("").to_string();
    let status = status
        .map(|s| s.to_string())
        .unwrap_or_else(|| cfg.statuses.sprint.first().cloned().unwrap_or_default());
    validate_status(&status, EntityKind::Sprint, cfg)?;
    let goal = goal.unwrap_or("").to_string();
    let start_date = start_date.unwrap_or(&today).to_string();
    let end_date = end_date.unwrap_or("").to_string();
    let tags: Vec<String> = tags.map(util::parse_comma_list).unwrap_or_default();
    let projects: Vec<String> = projects.map(util::parse_comma_list).unwrap_or_default();

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "sprint")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert(
        "aliases".into(),
        Value::Sequence(vec![Value::String(id.to_string())]),
    );
    fields.insert("title".into(), Value::String(title.to_string()));
    fields.insert("status".into(), Value::String(status));
    fields.insert("goal".into(), Value::String(goal));
    fields.insert("start_date".into(), Value::String(start_date));
    fields.insert("end_date".into(), Value::String(end_date));
    fields.insert("owner".into(), Value::String(owner));
    fields.insert(
        "projects".into(),
        Value::Sequence(
            projects
                .iter()
                .map(|p| Value::String(frontmatter::wrap_wikilink(p)))
                .collect(),
        ),
    );
    fields.insert(
        "tags".into(),
        Value::Sequence(tags.iter().map(|t| Value::String(t.clone())).collect()),
    );
    fields.insert("created".into(), Value::String(today.clone()));
    fields.insert("updated".into(), Value::String(today));

    let mut placeholders = HashMap::new();
    placeholders.insert("title".into(), title.to_string());

    let (fm, body) = template::render_template(tmpl_fm, &tmpl_body, &fields, &placeholders);
    let doc = frontmatter::serialize_document(&fm, &body);

    let dir_name = format!("{}-{}", id, slug);
    let dir_path = cfg.sprints_dir.join(&dir_name);
    fs::create_dir_all(&dir_path)?;
    util::atomic_write(&dir_path.join(format!("{}.md", id)), doc.as_bytes())?;

    // Create ceremony stub files
    fs::write(
        dir_path.join("planning.md"),
        format!(
            "# {} -- Sprint Planning\n\n## Capacity\n\n## Selected Items\n\n## Notes\n",
            title
        ),
    )?;
    fs::write(
        dir_path.join("review.md"),
        format!(
            "# {} -- Sprint Review\n\n## Demo Outcomes\n\n## Feedback\n\n## Notes\n",
            title
        ),
    )?;
    fs::write(
        dir_path.join("retrospective.md"),
        format!(
            "# {} -- Retrospective\n\n## What Went Well\n\n## What Could Improve\n\n## Action Items\n",
            title
        ),
    )?;

    Ok(serde_json::json!({
        "id": id.to_string(),
        "title": title,
        "path": dir_path.display().to_string(),
    }))
}

#[allow(clippy::too_many_arguments)]
pub fn create_proposal_programmatic(
    cfg: &ResolvedConfig,
    title: &str,
    author: Option<&str>,
    status: Option<&str>,
    proposal_type: Option<&str>,
    tags: Option<&str>,
    supersedes: Option<&str>,
) -> McResult<JsonValue> {
    check_mode(EntityKind::Proposal, cfg)?;
    validate_name_not_empty(title, EntityKind::Proposal)?;
    let id = entity::next_id(EntityKind::Proposal, cfg)?;
    let slug = util::slugify(title);
    let today = util::today_str();

    let author = author.unwrap_or("").to_string();
    let status = status
        .map(|s| s.to_string())
        .unwrap_or_else(|| cfg.statuses.proposal.first().cloned().unwrap_or_default());
    validate_status(&status, EntityKind::Proposal, cfg)?;
    let proposal_type = proposal_type.unwrap_or("architecture").to_string();
    let tags: Vec<String> = tags.map(util::parse_comma_list).unwrap_or_default();
    let supersedes = supersedes.unwrap_or("").to_string();

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "proposal")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert(
        "aliases".into(),
        Value::Sequence(vec![Value::String(id.to_string())]),
    );
    fields.insert("title".into(), Value::String(title.to_string()));
    fields.insert("status".into(), Value::String(status));
    fields.insert("type".into(), Value::String(proposal_type));
    fields.insert("author".into(), Value::String(author));
    fields.insert(
        "supersedes".into(),
        Value::String(frontmatter::wrap_wikilink(&supersedes)),
    );
    fields.insert("superseded_by".into(), Value::String(String::new()));
    fields.insert(
        "tags".into(),
        Value::Sequence(tags.iter().map(|t| Value::String(t.clone())).collect()),
    );
    fields.insert("created".into(), Value::String(today.clone()));
    fields.insert("updated".into(), Value::String(today));

    let mut placeholders = HashMap::new();
    placeholders.insert("title".into(), title.to_string());

    let (fm, body) = template::render_template(tmpl_fm, &tmpl_body, &fields, &placeholders);
    let doc = frontmatter::serialize_document(&fm, &body);

    let filename = format!("{}-{}.md", id, slug);
    let file_path = cfg.proposals_dir.join(&filename);
    fs::create_dir_all(&cfg.proposals_dir)?;
    util::atomic_write(&file_path, doc.as_bytes())?;

    Ok(serde_json::json!({
        "id": id.to_string(),
        "title": title,
        "path": file_path.display().to_string(),
    }))
}

#[allow(clippy::too_many_arguments)]
pub fn create_contact_programmatic(
    cfg: &ResolvedConfig,
    name: &str,
    customer: &str,
    role: Option<&str>,
    email: Option<&str>,
    phone: Option<&str>,
    status: Option<&str>,
    tags: Option<&str>,
) -> McResult<JsonValue> {
    check_mode(EntityKind::Contact, cfg)?;
    validate_name_not_empty(name, EntityKind::Contact)?;

    // Validate customer exists
    let cust_dir = find_customer_dir(cfg, customer)?;

    let id = entity::next_id(EntityKind::Contact, cfg)?;
    let today = util::today_str();

    let role = role.unwrap_or("").to_string();
    let email_val = email.unwrap_or("").to_string();
    let phone = phone.unwrap_or("").to_string();
    let status = status
        .map(|s| s.to_string())
        .unwrap_or_else(|| cfg.statuses.contact.first().cloned().unwrap_or_default());
    validate_status(&status, EntityKind::Contact, cfg)?;
    let tags: Vec<String> = tags.map(util::parse_comma_list).unwrap_or_default();

    let (tmpl_fm, tmpl_body) = template::load_template(&cfg.templates_dir, "contact")?;

    let mut fields = HashMap::new();
    fields.insert("id".into(), Value::String(id.to_string()));
    fields.insert(
        "aliases".into(),
        Value::Sequence(vec![Value::String(id.to_string())]),
    );
    fields.insert("name".into(), Value::String(name.to_string()));
    fields.insert("role".into(), Value::String(role));
    fields.insert("email".into(), Value::String(email_val));
    fields.insert("phone".into(), Value::String(phone));
    fields.insert(
        "customer".into(),
        Value::String(frontmatter::wrap_wikilink(customer)),
    );
    fields.insert("status".into(), Value::String(status));
    fields.insert(
        "tags".into(),
        Value::Sequence(tags.iter().map(|t| Value::String(t.clone())).collect()),
    );
    fields.insert("created".into(), Value::String(today.clone()));
    fields.insert("updated".into(), Value::String(today));

    let mut placeholders = HashMap::new();
    placeholders.insert("name".into(), name.to_string());

    let (fm, body) = template::render_template(tmpl_fm, &tmpl_body, &fields, &placeholders);
    let doc = frontmatter::serialize_document(&fm, &body);

    let slug = util::slugify(name);
    let filename = format!("{}-{}.md", id, slug);
    let contacts_dir = cust_dir.join("contacts");
    fs::create_dir_all(&contacts_dir)?;
    let file_path = contacts_dir.join(&filename);
    util::atomic_write(&file_path, doc.as_bytes())?;

    Ok(serde_json::json!({
        "id": id.to_string(),
        "name": name,
        "path": file_path.display().to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::init;
    use crate::config;
    use tempfile::TempDir;

    fn setup_repo() -> (TempDir, ResolvedConfig) {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        init::run(root, false, false, Some("TestRepo"), false, true).unwrap();
        let cfg = config::load_config(root, config::RepoMode::Standalone).unwrap();
        (tmp, cfg)
    }

    #[test]
    fn test_create_customer_and_verify_files() {
        let (_tmp, cfg) = setup_repo();

        let result =
            create_customer_programmatic(&cfg, "Acme Inc", Some("alice"), Some("active"), None)
                .unwrap();
        assert_eq!(result["id"], "CUST-001");
        assert_eq!(result["name"], "Acme Inc");

        // Verify directory and entity file exist
        let dir = cfg.customers_dir.join("CUST-001-acme-inc");
        assert!(dir.is_dir());
        assert!(dir.join("CUST-001.md").is_file());
        assert!(dir.join("contacts").is_dir());
        assert!(dir.join("contracts").is_dir());

        // Verify frontmatter
        let content = std::fs::read_to_string(dir.join("CUST-001.md")).unwrap();
        let (fm_str, _) = frontmatter::split_frontmatter(&content).unwrap();
        let fm = frontmatter::parse_raw(&fm_str, &dir.join("CUST-001.md")).unwrap();
        assert_eq!(frontmatter::get_str(&fm, "id").unwrap(), "CUST-001");
        assert_eq!(frontmatter::get_str(&fm, "name").unwrap(), "Acme Inc");
        assert_eq!(frontmatter::get_str(&fm, "status").unwrap(), "active");
        assert_eq!(frontmatter::get_str(&fm, "owner").unwrap(), "alice");
    }

    #[test]
    fn test_create_task_in_todo() {
        let (_tmp, cfg) = setup_repo();

        let result = create_task_programmatic(
            &cfg,
            "Fix login bug",
            None,
            None,
            Some("bob"),
            Some("todo"),
            Some(2),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(result["id"], "TASK-001");

        // Task should be in todo/
        let task_path = cfg.tasks_dir.join("todo").join("TASK-001-fix-login-bug.md");
        assert!(task_path.is_file());

        // Verify frontmatter
        let content = std::fs::read_to_string(&task_path).unwrap();
        let (fm_str, _) = frontmatter::split_frontmatter(&content).unwrap();
        let fm = frontmatter::parse_raw(&fm_str, &task_path).unwrap();
        assert_eq!(frontmatter::get_str(&fm, "status").unwrap(), "todo");
        assert_eq!(frontmatter::get_str(&fm, "owner").unwrap(), "bob");
    }

    #[test]
    fn test_invalid_status_rejected() {
        let (_tmp, cfg) = setup_repo();

        let result = create_customer_programmatic(&cfg, "Acme", Some("alice"), Some("bogus"), None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid customer status 'bogus'"));
        assert!(err.contains("active"));
    }

    #[test]
    fn test_empty_name_rejected() {
        let (_tmp, cfg) = setup_repo();

        let result = create_customer_programmatic(&cfg, "", None, Some("active"), None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cannot be empty"));

        // Also test whitespace-only
        let result = create_customer_programmatic(&cfg, "   ", None, Some("active"), None);
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_task_title_rejected() {
        let (_tmp, cfg) = setup_repo();

        let result = create_task_programmatic(
            &cfg,
            "",
            None,
            None,
            None,
            Some("todo"),
            None,
            None,
            None,
            None,
            None,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cannot be empty"));
    }

    #[test]
    fn test_invalid_task_status_rejected() {
        let (_tmp, cfg) = setup_repo();

        let result = create_task_programmatic(
            &cfg,
            "Some task",
            None,
            None,
            None,
            Some("invalid-status"),
            None,
            None,
            None,
            None,
            None,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid task status"));
    }

    #[test]
    fn test_sequential_id_generation() {
        let (_tmp, cfg) = setup_repo();

        let r1 = create_customer_programmatic(&cfg, "First", None, Some("active"), None).unwrap();
        let r2 = create_customer_programmatic(&cfg, "Second", None, Some("active"), None).unwrap();
        assert_eq!(r1["id"], "CUST-001");
        assert_eq!(r2["id"], "CUST-002");
    }

    #[test]
    fn test_create_meeting() {
        let (_tmp, cfg) = setup_repo();

        let result = create_meeting_programmatic(
            &cfg,
            "Sprint Planning",
            Some("2025-01-15"),
            Some("09:00"),
            Some("60m"),
            Some("scheduled"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(result["id"], "MTG-001");

        let path_str = result["path"].as_str().unwrap();
        assert!(std::path::Path::new(path_str).is_file());
    }

    #[test]
    fn test_create_contact() {
        let (_tmp, cfg) = setup_repo();

        // First create a customer to attach the contact to
        create_customer_programmatic(&cfg, "Acme Inc", Some("alice"), Some("active"), None)
            .unwrap();

        let result = create_contact_programmatic(
            &cfg,
            "Alice Smith",
            "CUST-001",
            Some("VP Engineering"),
            Some("alice@acme.com"),
            Some("+1-555-0101"),
            Some("active"),
            None,
        )
        .unwrap();
        assert_eq!(result["id"], "CONT-001");
        assert_eq!(result["name"], "Alice Smith");

        // Verify file location
        let cust_dir = cfg.customers_dir.join("CUST-001-acme-inc");
        let contact_path = cust_dir.join("contacts").join("CONT-001-alice-smith.md");
        assert!(contact_path.is_file());

        // Verify frontmatter
        let content = std::fs::read_to_string(&contact_path).unwrap();
        let (fm_str, _) = frontmatter::split_frontmatter(&content).unwrap();
        let fm = frontmatter::parse_raw(&fm_str, &contact_path).unwrap();
        assert_eq!(frontmatter::get_str(&fm, "id").unwrap(), "CONT-001");
        assert_eq!(frontmatter::get_str(&fm, "name").unwrap(), "Alice Smith");
        assert_eq!(frontmatter::get_str(&fm, "role").unwrap(), "VP Engineering");
        assert_eq!(
            frontmatter::get_str(&fm, "email").unwrap(),
            "alice@acme.com"
        );
        assert_eq!(
            frontmatter::get_str(&fm, "customer").unwrap(),
            "[[CUST-001]]"
        );
        assert_eq!(frontmatter::get_str(&fm, "status").unwrap(), "active");
    }

    #[test]
    fn test_create_contact_invalid_customer() {
        let (_tmp, cfg) = setup_repo();

        let result = create_contact_programmatic(
            &cfg,
            "Alice Smith",
            "CUST-999",
            None,
            None,
            None,
            None,
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_create_contact_empty_name_rejected() {
        let (_tmp, cfg) = setup_repo();

        create_customer_programmatic(&cfg, "Acme", None, Some("active"), None).unwrap();

        let result =
            create_contact_programmatic(&cfg, "", "CUST-001", None, None, None, None, None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cannot be empty"));
    }

    #[test]
    fn test_create_contact_invalid_status_rejected() {
        let (_tmp, cfg) = setup_repo();

        create_customer_programmatic(&cfg, "Acme", None, Some("active"), None).unwrap();

        let result = create_contact_programmatic(
            &cfg,
            "Alice",
            "CUST-001",
            None,
            None,
            None,
            Some("bogus"),
            None,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Invalid contact status"));
    }

    #[test]
    fn test_contact_sequential_ids_across_customers() {
        let (_tmp, cfg) = setup_repo();

        create_customer_programmatic(&cfg, "Acme", None, Some("active"), None).unwrap();
        create_customer_programmatic(&cfg, "Beta", None, Some("active"), None).unwrap();

        let r1 =
            create_contact_programmatic(&cfg, "Alice", "CUST-001", None, None, None, None, None)
                .unwrap();
        let r2 = create_contact_programmatic(&cfg, "Bob", "CUST-002", None, None, None, None, None)
            .unwrap();

        assert_eq!(r1["id"], "CONT-001");
        assert_eq!(r2["id"], "CONT-002");
    }

    #[test]
    fn test_create_sprint() {
        let (_tmp, cfg) = setup_repo();

        let result = create_sprint_programmatic(
            &cfg,
            "Sprint 1",
            Some("alice"),
            Some("planning"),
            Some("Deliver MVP"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(result["id"], "SPR-001");

        let dir = cfg.sprints_dir.join("SPR-001-sprint-1");
        assert!(dir.join("SPR-001.md").is_file());
        assert!(dir.join("planning.md").is_file());
        assert!(dir.join("review.md").is_file());
        assert!(dir.join("retrospective.md").is_file());
    }
}
