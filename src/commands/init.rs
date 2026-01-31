use crate::error::{McError, McResult};
use colored::*;
use std::io::Write;
use std::path::Path;
use std::process::Command;

// ---------------------------------------------------------------------------
// Embedded templates (single source of truth: ../../templates/)
// ---------------------------------------------------------------------------

const TEMPLATE_CUSTOMER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../templates/customer.md"
));
const TEMPLATE_PROJECT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../templates/project.md"
));
const TEMPLATE_MEETING: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../templates/meeting.md"
));
const TEMPLATE_RESEARCH: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../templates/research.md"
));
const TEMPLATE_TASK: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../templates/task.md"
));

// ---------------------------------------------------------------------------
// Embedded repo files
// ---------------------------------------------------------------------------

const GITIGNORE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.gitignore"));
const GITATTRIBUTES: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.gitattributes"));

// ---------------------------------------------------------------------------
// Config YAML templates
// ---------------------------------------------------------------------------

const FULL_CONFIG: &str = r#"# MissionControl Configuration
# Used by the mc CLI for repo structure, IDs, and statuses.

site:
  name: {name}
  description: Customer and project knowledge base

paths:
  customers: customers/
  projects: projects/
  meetings: meetings/
  research: research/
  tasks: tasks/
  notes: notes/
  data: data/
  templates: templates/
  assets: assets/
  archive: archive/

id_prefixes:
  customer: CUST
  project: PROJ
  meeting: MTG
  research: RES
  task: TASK

statuses:
  customer:
    - active
    - inactive
    - prospect
    - churned
  project:
    - active
    - on-hold
    - completed
    - cancelled
  meeting:
    - scheduled
    - completed
    - cancelled
  research:
    - draft
    - in-progress
    - final
    - outdated
  task:
    - backlog
    - todo
    - in-progress
    - review
    - done
    - cancelled

priorities:
  1: critical
  2: high
  3: medium
  4: low
"#;

const PROJECT_CONFIG: &str = r#"# MissionControl Configuration (project mode)
# Lightweight setup for a single project.

site:
  name: {name}
  description: Project knowledge base

paths:
  meetings: meetings/
  research: research/
  tasks: tasks/
  data: data/
  templates: templates/
  archive: archive/

id_prefixes:
  meeting: MTG
  research: RES
  task: TASK

statuses:
  meeting:
    - scheduled
    - completed
    - cancelled
  research:
    - draft
    - in-progress
    - final
    - outdated
  task:
    - backlog
    - todo
    - in-progress
    - review
    - done
    - cancelled

priorities:
  1: critical
  2: high
  3: medium
  4: low
"#;

// ---------------------------------------------------------------------------
// Directory lists
// ---------------------------------------------------------------------------

const FULL_DIRS: &[&str] = &[
    "config",
    "customers",
    "projects",
    "meetings",
    "research",
    "tasks/todo",
    "tasks/done",
    "notes/how-tos",
    "notes/playbooks",
    "data",
    "templates",
    "assets",
    "archive",
];

const PROJECT_DIRS: &[&str] = &[
    "config",
    "tasks/todo",
    "tasks/done",
    "meetings",
    "research",
    "templates",
    "data",
    "archive",
];

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn run(
    target: &Path,
    project_mode: bool,
    name: Option<&str>,
    force: bool,
    yes: bool,
) -> McResult<()> {
    let config_path = target.join("config").join("config.yml");

    // Guard: error if already initialized
    if config_path.is_file() && !force {
        return Err(McError::AlreadyInitialized(config_path));
    }

    let mode_label = if project_mode { "project" } else { "full" };
    let default_name = if project_mode {
        "Project"
    } else {
        "MissionControl"
    };

    // Determine name
    let repo_name = match name {
        Some(n) => n.to_string(),
        None => {
            if yes {
                default_name.to_string()
            } else {
                prompt_name(default_name)?
            }
        }
    };

    // Print summary
    println!();
    println!("  {} {}", "Mode:".bold(), mode_label);
    println!("  {} {}", "Name:".bold(), repo_name);
    println!("  {} {}", "Location:".bold(), target.display());
    if force && config_path.is_file() {
        println!("  {} reinitializing (--force)", "Note:".yellow().bold());
    }
    println!();

    // Confirm
    if !yes && !confirm("Initialize repository?")? {
        println!("Aborted.");
        return Ok(());
    }

    // Create directories
    let dirs = if project_mode {
        PROJECT_DIRS
    } else {
        FULL_DIRS
    };
    for dir in dirs {
        let path = target.join(dir);
        std::fs::create_dir_all(&path)?;
        // Add .gitkeep to leaf directories that are empty
        let gitkeep = path.join(".gitkeep");
        if !gitkeep.exists() && is_empty_dir(&path)? {
            std::fs::File::create(gitkeep)?;
        }
    }

    // Write config
    let config_content = if project_mode {
        PROJECT_CONFIG.replace("{name}", &repo_name)
    } else {
        FULL_CONFIG.replace("{name}", &repo_name)
    };
    std::fs::create_dir_all(target.join("config"))?;
    std::fs::write(&config_path, config_content)?;

    // Write templates
    let templates_dir = target.join("templates");
    std::fs::create_dir_all(&templates_dir)?;

    if project_mode {
        // Project-only: meeting, research, task
        write_if_missing_or_force(&templates_dir.join("meeting.md"), TEMPLATE_MEETING, force)?;
        write_if_missing_or_force(&templates_dir.join("research.md"), TEMPLATE_RESEARCH, force)?;
        write_if_missing_or_force(&templates_dir.join("task.md"), TEMPLATE_TASK, force)?;
    } else {
        // Full: all 5 templates
        write_if_missing_or_force(&templates_dir.join("customer.md"), TEMPLATE_CUSTOMER, force)?;
        write_if_missing_or_force(&templates_dir.join("project.md"), TEMPLATE_PROJECT, force)?;
        write_if_missing_or_force(&templates_dir.join("meeting.md"), TEMPLATE_MEETING, force)?;
        write_if_missing_or_force(&templates_dir.join("research.md"), TEMPLATE_RESEARCH, force)?;
        write_if_missing_or_force(&templates_dir.join("task.md"), TEMPLATE_TASK, force)?;
    }

    // Write .gitignore and .gitattributes
    write_if_missing_or_force(&target.join(".gitignore"), GITIGNORE, force)?;
    write_if_missing_or_force(&target.join(".gitattributes"), GITATTRIBUTES, force)?;

    // Remove .gitkeep from directories that now have content
    remove_gitkeep_if_nonempty(&target.join("config"))?;
    remove_gitkeep_if_nonempty(&target.join("templates"))?;

    // Git init
    let git_dir = target.join(".git");
    if !git_dir.exists() {
        let should_init = if yes {
            true
        } else {
            confirm("Run 'git init'?")?
        };
        if should_init {
            let status = Command::new("git").arg("init").current_dir(target).status();
            match status {
                Ok(s) if s.success() => {
                    println!("  {} git repository initialized", "✓".green());
                }
                Ok(s) => {
                    eprintln!("  {} git init exited with {}", "⚠".yellow(), s);
                }
                Err(e) => {
                    eprintln!(
                        "  {} git init failed: {} (is git installed?)",
                        "⚠".yellow(),
                        e
                    );
                }
            }
        }
    } else {
        println!("  {} .git already exists, skipping git init", "·".dimmed());
    }

    // Success
    println!();
    println!(
        "{} MissionControl repo initialized at {}",
        "✓".green().bold(),
        target.display()
    );
    println!();
    println!("Next steps:");
    println!("  mc status              Show repo dashboard");
    println!("  mc validate            Verify repo structure");
    if project_mode {
        println!("  mc new task \"...\"      Create your first task");
    } else {
        println!("  mc new customer \"...\"  Create your first customer");
        println!("  mc new project \"...\"   Create your first project");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn prompt_name(default: &str) -> McResult<String> {
    print!("  Repository name [{}]: ", default);
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

fn confirm(question: &str) -> McResult<bool> {
    print!("  {} [Y/n] ", question);
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_lowercase();
    Ok(trimmed.is_empty() || trimmed == "y" || trimmed == "yes")
}

fn write_if_missing_or_force(path: &Path, content: &str, force: bool) -> McResult<()> {
    if !path.exists() || force {
        std::fs::write(path, content)?;
    }
    Ok(())
}

fn is_empty_dir(path: &Path) -> McResult<bool> {
    Ok(std::fs::read_dir(path)?.next().is_none())
}

fn remove_gitkeep_if_nonempty(dir: &Path) -> McResult<()> {
    let gitkeep = dir.join(".gitkeep");
    if gitkeep.exists() {
        // Count entries other than .gitkeep
        let has_other = std::fs::read_dir(dir)?
            .filter_map(|e| e.ok())
            .any(|e| e.file_name() != ".gitkeep");
        if has_other {
            std::fs::remove_file(gitkeep)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn run_init(dir: &Path, project_mode: bool, name: Option<&str>, force: bool) -> McResult<()> {
        run(dir, project_mode, name, force, true)
    }

    #[test]
    fn test_full_init_creates_structure() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        run_init(root, false, Some("TestRepo"), false).unwrap();

        // Config exists
        assert!(root.join("config/config.yml").is_file());

        // Key directories exist
        assert!(root.join("customers").is_dir());
        assert!(root.join("projects").is_dir());
        assert!(root.join("meetings").is_dir());
        assert!(root.join("research").is_dir());
        assert!(root.join("tasks/todo").is_dir());
        assert!(root.join("tasks/done").is_dir());
        assert!(root.join("notes/how-tos").is_dir());
        assert!(root.join("notes/playbooks").is_dir());
        assert!(root.join("data").is_dir());
        assert!(root.join("templates").is_dir());
        assert!(root.join("assets").is_dir());
        assert!(root.join("archive").is_dir());

        // All 5 templates
        assert!(root.join("templates/customer.md").is_file());
        assert!(root.join("templates/project.md").is_file());
        assert!(root.join("templates/meeting.md").is_file());
        assert!(root.join("templates/research.md").is_file());
        assert!(root.join("templates/task.md").is_file());

        // Repo files
        assert!(root.join(".gitignore").is_file());
        assert!(root.join(".gitattributes").is_file());

        // Config contains the name
        let config = std::fs::read_to_string(root.join("config/config.yml")).unwrap();
        assert!(config.contains("name: TestRepo"));
    }

    #[test]
    fn test_project_init_creates_structure() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        run_init(root, true, Some("MyProject"), false).unwrap();

        // Config exists
        assert!(root.join("config/config.yml").is_file());

        // Project-only directories
        assert!(root.join("tasks/todo").is_dir());
        assert!(root.join("tasks/done").is_dir());
        assert!(root.join("meetings").is_dir());
        assert!(root.join("research").is_dir());
        assert!(root.join("templates").is_dir());
        assert!(root.join("data").is_dir());
        assert!(root.join("archive").is_dir());

        // No full-mode directories
        assert!(!root.join("customers").exists());
        assert!(!root.join("projects").exists());
        assert!(!root.join("notes").exists());
        assert!(!root.join("assets").exists());

        // Only 3 templates
        assert!(!root.join("templates/customer.md").exists());
        assert!(!root.join("templates/project.md").exists());
        assert!(root.join("templates/meeting.md").is_file());
        assert!(root.join("templates/research.md").is_file());
        assert!(root.join("templates/task.md").is_file());

        // Config is project mode (no customer/project sections)
        let config = std::fs::read_to_string(root.join("config/config.yml")).unwrap();
        assert!(config.contains("name: MyProject"));
        assert!(config.contains("project mode"));
        assert!(!config.contains("customer:"));
    }

    #[test]
    fn test_already_initialized_error() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        run_init(root, false, Some("First"), false).unwrap();

        // Second init should fail
        let result = run_init(root, false, Some("Second"), false);
        assert!(result.is_err());
        match result.unwrap_err() {
            McError::AlreadyInitialized(_) => {} // expected
            other => panic!("Expected AlreadyInitialized, got: {other}"),
        }
    }

    #[test]
    fn test_force_reinitialize() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        run_init(root, false, Some("First"), false).unwrap();

        // Force reinit should succeed
        run_init(root, false, Some("Second"), true).unwrap();

        let config = std::fs::read_to_string(root.join("config/config.yml")).unwrap();
        assert!(config.contains("name: Second"));
    }

    #[test]
    fn test_config_is_valid_yaml() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        run_init(root, false, Some("YamlTest"), false).unwrap();

        let config = std::fs::read_to_string(root.join("config/config.yml")).unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&config).unwrap();
        assert!(parsed.get("site").is_some());
        assert!(parsed.get("paths").is_some());
        assert!(parsed.get("id_prefixes").is_some());
        assert!(parsed.get("statuses").is_some());
    }

    #[test]
    fn test_gitkeep_in_empty_dirs() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        run_init(root, false, Some("Test"), false).unwrap();

        // Empty dirs should have .gitkeep
        assert!(root.join("customers/.gitkeep").is_file());
        assert!(root.join("assets/.gitkeep").is_file());

        // Dirs with content should NOT have .gitkeep
        assert!(!root.join("config/.gitkeep").exists());
        assert!(!root.join("templates/.gitkeep").exists());
    }

    #[test]
    fn test_git_init_creates_git_dir() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        run_init(root, false, Some("GitTest"), false).unwrap();

        // With yes=true, git init should have run
        assert!(root.join(".git").is_dir());
    }

    #[test]
    fn test_idempotent_with_force() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        // Init, then force-init again — should not panic or error
        run_init(root, false, Some("One"), false).unwrap();
        run_init(root, false, Some("Two"), true).unwrap();
        run_init(root, false, Some("Three"), true).unwrap();

        let config = std::fs::read_to_string(root.join("config/config.yml")).unwrap();
        assert!(config.contains("name: Three"));
    }

    fn resolve_config_after_init(root: &Path) -> crate::config::ResolvedConfig {
        crate::config::load_config(root).unwrap()
    }

    #[test]
    fn test_full_init_config_loads() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        run_init(root, false, Some("LoadTest"), false).unwrap();

        let cfg = resolve_config_after_init(root);
        assert_eq!(cfg.id_prefixes.customer, "CUST");
        assert_eq!(cfg.id_prefixes.task, "TASK");
        assert_eq!(cfg.statuses.customer.len(), 4);
        assert_eq!(cfg.statuses.task.len(), 6);
    }

    #[test]
    fn test_project_init_config_loads() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        run_init(root, true, Some("ProjLoad"), false).unwrap();

        let cfg = resolve_config_after_init(root);
        assert_eq!(cfg.id_prefixes.task, "TASK");
        assert_eq!(cfg.statuses.task.len(), 6);
        assert_eq!(cfg.statuses.meeting.len(), 3);
    }
}
