use crate::error::{McError, McResult};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct RawConfig {
    #[allow(dead_code)]
    pub site: Option<SiteConfig>,
    pub paths: Option<HashMap<String, String>>,
    pub id_prefixes: Option<HashMap<String, String>>,
    pub statuses: Option<HashMap<String, Vec<String>>>,
    pub brand: Option<BrandConfig>,
}

#[derive(Debug, Deserialize)]
pub struct BrandConfig {
    pub name: Option<String>,
    pub tagline: Option<String>,
    pub logo: Option<String>,
    pub fonts_dir: Option<String>,
    pub font_name: Option<String>,
    pub primary_color: Option<Vec<u8>>,
    pub accent_color: Option<Vec<u8>>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct SiteConfig {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// Resolved configuration with absolute paths.
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub root: PathBuf,
    pub customers_dir: PathBuf,
    pub projects_dir: PathBuf,
    pub meetings_dir: PathBuf,
    pub research_dir: PathBuf,
    pub tasks_dir: PathBuf,
    #[allow(dead_code)]
    pub notes_dir: PathBuf,
    pub data_dir: PathBuf,
    pub templates_dir: PathBuf,
    pub archive_dir: PathBuf,
    pub id_prefixes: IdPrefixes,
    pub statuses: StatusConfig,
    pub brand: ResolvedBrand,
}

/// Resolved brand configuration with absolute paths and defaults applied.
#[derive(Debug, Clone)]
pub struct ResolvedBrand {
    pub name: String,
    pub tagline: String,
    #[allow(dead_code)]
    pub logo: Option<PathBuf>,
    pub fonts_dir: Option<PathBuf>,
    pub font_name: String,
    pub primary_color: [u8; 3],
    pub accent_color: [u8; 3],
}

#[derive(Debug, Clone)]
pub struct IdPrefixes {
    pub customer: String,
    pub project: String,
    pub meeting: String,
    pub research: String,
    pub task: String,
}

#[derive(Debug, Clone)]
pub struct StatusConfig {
    pub customer: Vec<String>,
    pub project: Vec<String>,
    pub meeting: Vec<String>,
    pub research: Vec<String>,
    pub task: Vec<String>,
}

/// Walk up from `start` looking for a directory that contains `config/config.yml`.
pub fn find_repo_root(start: &Path) -> McResult<PathBuf> {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join("config").join("config.yml").is_file() {
            return Ok(dir);
        }
        if !dir.pop() {
            return Err(McError::RepoRootNotFound);
        }
    }
}

/// Load and resolve configuration.
pub fn load_config(root: &Path) -> McResult<ResolvedConfig> {
    let config_path = root.join("config").join("config.yml");
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
        root.join(paths.get(key).map(|s| s.as_str()).unwrap_or(default))
    };

    Ok(ResolvedConfig {
        root: root.to_path_buf(),
        customers_dir: resolve("customers", "customers/"),
        projects_dir: resolve("projects", "projects/"),
        meetings_dir: resolve("meetings", "meetings/"),
        research_dir: resolve("research", "research/"),
        tasks_dir: resolve("tasks", "tasks/"),
        notes_dir: resolve("notes", "notes/"),
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
        },
        brand: resolve_brand(root, raw_brand),
    })
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
            let logo = b.logo.map(|p| root.join(p)).filter(|p| p.is_file());
            let fonts_dir = b.fonts_dir.map(|p| root.join(p)).filter(|p| p.is_dir());
            ResolvedBrand {
                name: b.name.unwrap_or_else(|| "MissionControl".into()),
                tagline: b.tagline.unwrap_or_default(),
                logo,
                fonts_dir,
                font_name: b.font_name.unwrap_or_else(|| "LiberationSans".into()),
                primary_color: b
                    .primary_color
                    .as_deref()
                    .map(|v| color_from_vec(v, [0, 82, 155]))
                    .unwrap_or([0, 82, 155]),
                accent_color: b
                    .accent_color
                    .as_deref()
                    .map(|v| color_from_vec(v, [102, 102, 102]))
                    .unwrap_or([102, 102, 102]),
            }
        }
        None => ResolvedBrand {
            name: "MissionControl".into(),
            tagline: String::new(),
            logo: None,
            fonts_dir: None,
            font_name: "LiberationSans".into(),
            primary_color: [0, 82, 155],
            accent_color: [102, 102, 102],
        },
    }
}
