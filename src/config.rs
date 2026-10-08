use crate::error::{McError, McResult};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Operating mode for a MissionControl repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoMode {
    /// Standalone repo where the entire directory is managed by mc.
    Standalone,
    /// Embedded `.mc/` folder inside an existing project.
    Embedded,
}

#[derive(Debug, Default, Deserialize)]
pub struct RawConfig {
    pub paths: Option<HashMap<String, String>>,
    pub id_prefixes: Option<HashMap<String, String>>,
    pub statuses: Option<HashMap<String, Vec<String>>>,
    pub brand: Option<BrandConfig>,
    pub site: Option<SiteConfig>,
}

/// The `site:` section written by `mc init` (`name`, `description`).
#[derive(Debug, Default, Deserialize)]
pub struct SiteConfig {
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct BrandConfig {
    pub name: Option<String>,
    pub tagline: Option<String>,
    pub fonts_dir: Option<String>,
    pub font_name: Option<String>,
    pub primary_color: Option<Vec<u8>>,
    pub accent_color: Option<Vec<u8>>,
    pub logo: Option<String>,
    pub custom_css: Option<String>,
}

/// Resolved configuration with absolute paths.
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub root: PathBuf,
    pub mode: RepoMode,
    pub customers_dir: PathBuf,
    pub projects_dir: PathBuf,
    pub meetings_dir: PathBuf,
    pub research_dir: PathBuf,
    pub tasks_dir: PathBuf,
    pub sprints_dir: PathBuf,
    pub milestones_dir: PathBuf,
    pub proposals_dir: PathBuf,
    pub data_dir: PathBuf,
    pub templates_dir: PathBuf,
    pub archive_dir: PathBuf,
    pub id_prefixes: IdPrefixes,
    pub statuses: StatusConfig,
    pub brand: ResolvedBrand,
    /// Entity path keys explicitly set in config (e.g. "tasks", "research").
    /// If empty, all defaults apply (backwards compatible).
    pub configured_entities: std::collections::HashSet<String>,
}

/// Default primary color (blue).
pub const DEFAULT_PRIMARY: [u8; 3] = [0, 82, 155];
/// Default accent color (gray).
pub const DEFAULT_ACCENT: [u8; 3] = [102, 102, 102];
/// Display name used when neither `brand.name` nor `site.name` is configured.
pub const DEFAULT_BRAND_NAME: &str = "MissionControl";

/// Resolved brand configuration with absolute paths and defaults applied.
#[derive(Debug, Clone)]
pub struct ResolvedBrand {
    pub name: String,
    pub tagline: String,
    pub fonts_dir: Option<PathBuf>,
    pub font_name: String,
    pub primary_color: [u8; 3],
    pub accent_color: [u8; 3],
    pub logo: Option<PathBuf>,
    pub custom_css: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct IdPrefixes {
    pub customer: String,
    pub project: String,
    pub meeting: String,
    pub research: String,
    pub task: String,
    pub sprint: String,
    pub milestone: String,
    pub proposal: String,
    pub contact: String,
}

#[derive(Debug, Clone)]
pub struct StatusConfig {
    pub customer: Vec<String>,
    pub project: Vec<String>,
    pub meeting: Vec<String>,
    pub research: Vec<String>,
    pub task: Vec<String>,
    pub sprint: Vec<String>,
    pub milestone: Vec<String>,
    pub proposal: Vec<String>,
    pub contact: Vec<String>,
}

impl ResolvedConfig {
    /// Check if an entity kind is available in this config.
    /// In embedded mode, only task/milestone/meeting/research/sprint/proposal are available.
    /// In standalone mode, if `paths` is configured, only explicitly listed entity types
    /// are shown (plus their singular/plural variants); milestones also come with
    /// `tasks`, so configs from before milestones need no migration. If no paths
    /// are configured, all entity types are available (backwards compatible).
    pub fn entity_available(&self, kind: &crate::entity::EntityKind) -> bool {
        use crate::entity::EntityKind;
        // Embedded mode filter
        if self.mode == RepoMode::Embedded {
            return matches!(
                kind,
                EntityKind::Task
                    | EntityKind::Meeting
                    | EntityKind::Research
                    | EntityKind::Sprint
                    | EntityKind::Milestone
                    | EntityKind::Proposal
            );
        }
        // Standalone: if no paths configured, show all (backwards compatible)
        if self.configured_entities.is_empty() {
            return true;
        }
        // Check if this entity's path key is in the configured set
        // Config uses plural keys (tasks, customers, etc.) or singular (task, customer)
        let plural = kind.label_plural();
        let singular = kind.label();
        if self.configured_entities.contains(plural) || self.configured_entities.contains(singular)
        {
            return true;
        }
        if *kind == EntityKind::Milestone {
            return self.configured_entities.contains("tasks")
                || self.configured_entities.contains("task");
        }
        // Contacts are a sub-entity of customers — they don't have their own path key
        // but are available whenever customers are configured.
        if matches!(kind, EntityKind::Contact) {
            return self.configured_entities.contains("customers")
                || self.configured_entities.contains("customer");
        }
        false
    }
}

/// Walk up from `start` looking for a MissionControl config.
/// Checks `.mc/config.yml` (embedded) first, then `config/config.yml` (standalone).
pub fn find_repo_root(start: &Path) -> McResult<(PathBuf, RepoMode)> {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join(".mc").join("config.yml").is_file() {
            return Ok((dir, RepoMode::Embedded));
        }
        if dir.join("config").join("config.yml").is_file() {
            return Ok((dir, RepoMode::Standalone));
        }
        if !dir.pop() {
            return Err(McError::RepoRootNotFound);
        }
    }
}

/// Detect the repo mode for an explicit root path.
pub fn detect_mode(root: &Path) -> RepoMode {
    if root.join(".mc").join("config.yml").is_file() {
        RepoMode::Embedded
    } else {
        RepoMode::Standalone
    }
}

/// Load and resolve configuration.
pub fn load_config(root: &Path, mode: RepoMode) -> McResult<ResolvedConfig> {
    let (config_path, base_dir) = match mode {
        RepoMode::Standalone => (root.join("config").join("config.yml"), root.to_path_buf()),
        RepoMode::Embedded => (root.join(".mc").join("config.yml"), root.join(".mc")),
    };

    if !config_path.is_file() {
        return Err(McError::ConfigNotFound(config_path));
    }

    let content = std::fs::read_to_string(&config_path)?;
    let raw = parse_raw_config(&content)
        .map_err(|e| McError::ConfigParse(format!("{}: {}", config_path.display(), e)))?;

    let raw_paths = raw.paths.unwrap_or_default();
    let configured_entities: std::collections::HashSet<String> =
        raw_paths.keys().cloned().collect();
    let paths = raw_paths;
    let prefixes = raw.id_prefixes.unwrap_or_default();
    let statuses = raw.statuses.unwrap_or_default();
    let raw_brand = raw.brand;
    let site_name = raw
        .site
        .and_then(|s| s.name)
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());

    let resolve = |key: &str, default: &str| -> PathBuf {
        base_dir.join(paths.get(key).map(|s| s.as_str()).unwrap_or(default))
    };
    let prefix = |key: &str, default: &str| -> String {
        prefixes
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.to_string())
    };
    let status_list = |key: &str, default: &[&str]| -> Vec<String> {
        statuses
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.iter().map(|s| s.to_string()).collect())
    };

    let resolved = ResolvedConfig {
        root: root.to_path_buf(),
        mode,
        customers_dir: resolve("customers", "customers/"),
        projects_dir: resolve("projects", "projects/"),
        meetings_dir: resolve("meetings", "meetings/"),
        research_dir: resolve("research", "research/"),
        tasks_dir: resolve("tasks", "tasks/"),
        sprints_dir: resolve("sprints", "sprints/"),
        milestones_dir: resolve("milestones", "milestones/"),
        proposals_dir: resolve("proposals", "proposals/"),
        data_dir: resolve("data", "data/"),
        templates_dir: resolve("templates", "templates/"),
        archive_dir: resolve("archive", "archive/"),
        id_prefixes: IdPrefixes {
            customer: prefix("customer", "CUST"),
            project: prefix("project", "PROJ"),
            meeting: prefix("meeting", "MTG"),
            research: prefix("research", "RES"),
            task: prefix("task", "TASK"),
            sprint: prefix("sprint", "SPR"),
            milestone: prefix("milestone", "MS"),
            proposal: prefix("proposal", "PROP"),
            contact: prefix("contact", "CONT"),
        },
        statuses: StatusConfig {
            customer: status_list("customer", &["active", "inactive"]),
            project: status_list("project", &["active", "on-hold", "completed"]),
            meeting: status_list("meeting", &["scheduled", "completed"]),
            research: status_list("research", &["draft", "final"]),
            task: status_list(
                "task",
                &[
                    "backlog",
                    "todo",
                    "in-progress",
                    "review",
                    "done",
                    "cancelled",
                ],
            ),
            milestone: status_list(
                "milestone",
                &["planned", "active", "completed", "cancelled"],
            ),
            sprint: status_list(
                "sprint",
                &["planning", "active", "review", "completed", "cancelled"],
            ),
            proposal: status_list(
                "proposal",
                &[
                    "draft",
                    "proposed",
                    "accepted",
                    "rejected",
                    "superseded",
                    "withdrawn",
                ],
            ),
            contact: status_list("contact", &["active", "inactive"]),
        },
        brand: resolve_brand(&base_dir, raw_brand, site_name),
        configured_entities,
    };

    validate_status_config(&resolved.statuses)?;

    Ok(resolved)
}

/// Parse config YAML. A file that is empty or only comments yields all defaults.
fn parse_raw_config(content: &str) -> Result<RawConfig, serde_yaml::Error> {
    Ok(serde_yaml::from_str::<Option<RawConfig>>(content)?.unwrap_or_default())
}

fn validate_status_config(statuses: &StatusConfig) -> McResult<()> {
    let checks = [
        ("customer", &statuses.customer),
        ("project", &statuses.project),
        ("meeting", &statuses.meeting),
        ("research", &statuses.research),
        ("task", &statuses.task),
        ("sprint", &statuses.sprint),
        ("milestone", &statuses.milestone),
        ("proposal", &statuses.proposal),
        ("contact", &statuses.contact),
    ];
    for (name, list) in checks {
        if list.is_empty() {
            return Err(McError::ConfigParse(format!(
                "statuses.{} must not be empty",
                name
            )));
        }
    }
    Ok(())
}

/// Resolve the brand section. The display name falls back from `brand.name`
/// to `site.name` (what `mc init` writes) and finally to [`DEFAULT_BRAND_NAME`].
fn resolve_brand(
    root: &Path,
    raw: Option<BrandConfig>,
    site_name: Option<String>,
) -> ResolvedBrand {
    let color = |v: Option<Vec<u8>>, default: [u8; 3]| -> [u8; 3] {
        match v.as_deref() {
            Some([r, g, b, ..]) => [*r, *g, *b],
            _ => default,
        }
    };

    let b = raw.unwrap_or_default();
    let name = b
        .name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .or(site_name)
        .unwrap_or_else(|| DEFAULT_BRAND_NAME.into());

    ResolvedBrand {
        name,
        tagline: b.tagline.unwrap_or_default(),
        fonts_dir: b.fonts_dir.map(|p| root.join(p)).filter(|p| p.is_dir()),
        font_name: b.font_name.unwrap_or_else(|| "LiberationSans".into()),
        primary_color: color(b.primary_color, DEFAULT_PRIMARY),
        accent_color: color(b.accent_color, DEFAULT_ACCENT),
        logo: b.logo.map(|p| root.join(p)).filter(|p| p.is_file()),
        custom_css: b.custom_css.map(|p| root.join(p)).filter(|p| p.is_file()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_statuses() -> StatusConfig {
        StatusConfig {
            customer: vec!["active".into()],
            project: vec!["active".into()],
            meeting: vec!["scheduled".into()],
            research: vec!["draft".into()],
            task: vec!["todo".into()],
            sprint: vec!["planning".into()],
            milestone: vec!["planned".into()],
            proposal: vec!["draft".into()],
            contact: vec!["active".into()],
        }
    }

    #[test]
    fn test_valid_statuses_pass() {
        assert!(validate_status_config(&default_statuses()).is_ok());
    }

    #[test]
    fn test_empty_customer_statuses_rejected() {
        let mut s = default_statuses();
        s.customer = vec![];
        let err = validate_status_config(&s).unwrap_err();
        assert!(err
            .to_string()
            .contains("statuses.customer must not be empty"));
    }

    #[test]
    fn test_empty_task_statuses_rejected() {
        let mut s = default_statuses();
        s.task = vec![];
        let err = validate_status_config(&s).unwrap_err();
        assert!(err.to_string().contains("statuses.task must not be empty"));
    }

    fn write_config(yaml: &str) -> tempfile::TempDir {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("config")).unwrap();
        std::fs::write(tmp.path().join("config/config.yml"), yaml).unwrap();
        tmp
    }

    fn load(yaml: &str) -> ResolvedConfig {
        let tmp = write_config(yaml);
        load_config(tmp.path(), RepoMode::Standalone).unwrap()
    }

    #[test]
    fn test_brand_name_falls_back_to_site_name() {
        let cfg = load("site:\n  name: headquarter\n  description: KB\n");
        assert_eq!(cfg.brand.name, "headquarter");
    }

    #[test]
    fn test_brand_name_wins_over_site_name() {
        let cfg = load("site:\n  name: headquarter\nbrand:\n  name: EmAI\n");
        assert_eq!(cfg.brand.name, "EmAI");
    }

    #[test]
    fn test_brand_without_name_uses_site_name() {
        let cfg = load("site:\n  name: Acme KB\nbrand:\n  primary_color: [1, 2, 3]\n");
        assert_eq!(cfg.brand.name, "Acme KB");
        assert_eq!(cfg.brand.primary_color, [1, 2, 3]);
    }

    #[test]
    fn test_blank_names_fall_through_to_default() {
        let cfg = load("site:\n  name: '  '\nbrand:\n  name: ''\n");
        assert_eq!(cfg.brand.name, DEFAULT_BRAND_NAME);
    }

    #[test]
    fn test_no_brand_no_site_uses_default() {
        let cfg = load("paths:\n  tasks: tasks/\n");
        assert_eq!(cfg.brand.name, DEFAULT_BRAND_NAME);
        assert_eq!(cfg.brand.primary_color, DEFAULT_PRIMARY);
        assert_eq!(cfg.brand.accent_color, DEFAULT_ACCENT);
    }

    #[test]
    fn test_short_color_falls_back_to_default() {
        let cfg = load("brand:\n  accent_color: [9, 9]\n");
        assert_eq!(cfg.brand.accent_color, DEFAULT_ACCENT);
    }

    #[test]
    fn test_empty_config_file_uses_defaults() {
        let cfg = load("# only a comment\n");
        assert_eq!(cfg.id_prefixes.task, "TASK");
        assert_eq!(
            cfg.statuses.task.first().map(String::as_str),
            Some("backlog")
        );
        assert!(cfg.configured_entities.is_empty());
    }

    #[test]
    fn test_custom_prefixes_and_statuses() {
        let cfg = load("id_prefixes:\n  task: T\nstatuses:\n  task: [open, closed]\n");
        assert_eq!(cfg.id_prefixes.task, "T");
        assert_eq!(cfg.id_prefixes.customer, "CUST");
        assert_eq!(cfg.statuses.task, vec!["open", "closed"]);
        assert_eq!(cfg.statuses.contact, vec!["active", "inactive"]);
    }

    #[test]
    fn test_parse_error_names_config_file() {
        let tmp = write_config("paths: [unclosed\n");
        let err = load_config(tmp.path(), RepoMode::Standalone).unwrap_err();
        assert!(err.to_string().contains("config.yml"), "{err}");
    }
}
