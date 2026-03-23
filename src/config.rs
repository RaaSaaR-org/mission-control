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

#[derive(Debug, Deserialize)]
pub struct RawConfig {
    pub paths: Option<HashMap<String, String>>,
    pub id_prefixes: Option<HashMap<String, String>>,
    pub statuses: Option<HashMap<String, Vec<String>>>,
    pub brand: Option<BrandConfig>,
}

#[derive(Debug, Deserialize)]
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
    pub proposals_dir: PathBuf,
    pub data_dir: PathBuf,
    pub templates_dir: PathBuf,
    pub archive_dir: PathBuf,
    pub id_prefixes: IdPrefixes,
    pub statuses: StatusConfig,
    pub brand: ResolvedBrand,
}

/// Default primary color (blue).
pub const DEFAULT_PRIMARY: [u8; 3] = [0, 82, 155];
/// Default accent color (gray).
pub const DEFAULT_ACCENT: [u8; 3] = [102, 102, 102];

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
    pub proposal: Vec<String>,
    pub contact: Vec<String>,
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
    let raw: RawConfig =
        serde_yaml::from_str(&content).map_err(|e| McError::ConfigParse(e.to_string()))?;

    let paths = raw.paths.unwrap_or_default();
    let prefixes = raw.id_prefixes.unwrap_or_default();
    let statuses = raw.statuses.unwrap_or_default();
    let raw_brand = raw.brand;

    let resolve = |key: &str, default: &str| -> PathBuf {
        base_dir.join(paths.get(key).map(|s| s.as_str()).unwrap_or(default))
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
        proposals_dir: resolve("proposals", "proposals/"),
        data_dir: resolve("data", "data/"),
        templates_dir: resolve("templates", "templates/"),
        archive_dir: resolve("archive", "archive/"),
        id_prefixes: IdPrefixes {
            customer: prefixes
                .get("customer")
                .cloned()
                .unwrap_or_else(|| "CUST".into()),
            project: prefixes
                .get("project")
                .cloned()
                .unwrap_or_else(|| "PROJ".into()),
            meeting: prefixes
                .get("meeting")
                .cloned()
                .unwrap_or_else(|| "MTG".into()),
            research: prefixes
                .get("research")
                .cloned()
                .unwrap_or_else(|| "RES".into()),
            task: prefixes
                .get("task")
                .cloned()
                .unwrap_or_else(|| "TASK".into()),
            sprint: prefixes
                .get("sprint")
                .cloned()
                .unwrap_or_else(|| "SPR".into()),
            proposal: prefixes
                .get("proposal")
                .cloned()
                .unwrap_or_else(|| "PROP".into()),
            contact: prefixes
                .get("contact")
                .cloned()
                .unwrap_or_else(|| "CONT".into()),
        },
        statuses: StatusConfig {
            customer: statuses
                .get("customer")
                .cloned()
                .unwrap_or_else(|| vec!["active".into(), "inactive".into()]),
            project: statuses
                .get("project")
                .cloned()
                .unwrap_or_else(|| vec!["active".into(), "on-hold".into(), "completed".into()]),
            meeting: statuses
                .get("meeting")
                .cloned()
                .unwrap_or_else(|| vec!["scheduled".into(), "completed".into()]),
            research: statuses
                .get("research")
                .cloned()
                .unwrap_or_else(|| vec!["draft".into(), "final".into()]),
            task: statuses.get("task").cloned().unwrap_or_else(|| {
                vec![
                    "backlog".into(),
                    "todo".into(),
                    "in-progress".into(),
                    "review".into(),
                    "done".into(),
                    "cancelled".into(),
                ]
            }),
            sprint: statuses.get("sprint").cloned().unwrap_or_else(|| {
                vec![
                    "planning".into(),
                    "active".into(),
                    "review".into(),
                    "completed".into(),
                    "cancelled".into(),
                ]
            }),
            proposal: statuses.get("proposal").cloned().unwrap_or_else(|| {
                vec![
                    "draft".into(),
                    "proposed".into(),
                    "accepted".into(),
                    "rejected".into(),
                    "superseded".into(),
                    "withdrawn".into(),
                ]
            }),
            contact: statuses
                .get("contact")
                .cloned()
                .unwrap_or_else(|| vec!["active".into(), "inactive".into()]),
        },
        brand: resolve_brand(&base_dir, raw_brand),
    };

    validate_status_config(&resolved.statuses)?;

    Ok(resolved)
}

fn validate_status_config(statuses: &StatusConfig) -> McResult<()> {
    let checks = [
        ("customer", &statuses.customer),
        ("project", &statuses.project),
        ("meeting", &statuses.meeting),
        ("research", &statuses.research),
        ("task", &statuses.task),
        ("sprint", &statuses.sprint),
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
}

fn resolve_brand(root: &Path, raw: Option<BrandConfig>) -> ResolvedBrand {
    let color_from_vec = |v: &[u8], default: [u8; 3]| -> [u8; 3] {
        if v.len() >= 3 {
            [v[0], v[1], v[2]]
        } else {
            default
        }
    };

    match raw {
        Some(b) => {
            let fonts_dir = b.fonts_dir.map(|p| root.join(p)).filter(|p| p.is_dir());
            let logo = b.logo.map(|p| root.join(p)).filter(|p| p.is_file());
            let custom_css = b.custom_css.map(|p| root.join(p)).filter(|p| p.is_file());
            ResolvedBrand {
                name: b.name.unwrap_or_else(|| "MissionControl".into()),
                tagline: b.tagline.unwrap_or_default(),
                fonts_dir,
                font_name: b.font_name.unwrap_or_else(|| "LiberationSans".into()),
                primary_color: b
                    .primary_color
                    .as_deref()
                    .map(|v| color_from_vec(v, DEFAULT_PRIMARY))
                    .unwrap_or(DEFAULT_PRIMARY),
                accent_color: b
                    .accent_color
                    .as_deref()
                    .map(|v| color_from_vec(v, DEFAULT_ACCENT))
                    .unwrap_or(DEFAULT_ACCENT),
                logo,
                custom_css,
            }
        }
        None => ResolvedBrand {
            name: "MissionControl".into(),
            tagline: String::new(),
            fonts_dir: None,
            font_name: "LiberationSans".into(),
            primary_color: DEFAULT_PRIMARY,
            accent_color: DEFAULT_ACCENT,
            logo: None,
            custom_css: None,
        },
    }
}
