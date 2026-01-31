use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum McError {
    #[error("Could not find MissionControl repo root (looked for config/config.yml)")]
    RepoRootNotFound,

    #[error("Config file not found: {0}")]
    ConfigNotFound(PathBuf),

    #[error("Failed to parse config: {0}")]
    ConfigParse(String),

    #[error("Template not found: {0}")]
    TemplateNotFound(PathBuf),

    #[error("Entity not found: {0}")]
    EntityNotFound(String),

    #[error("Invalid ID format: {0}")]
    InvalidId(String),

    #[error("Frontmatter error in {path}: {message}")]
    Frontmatter { path: PathBuf, message: String },

    #[error("Validation failed: {0} issue(s) found")]
    ValidationFailed(usize),

    #[error("Already initialized: config/config.yml exists at {0}")]
    AlreadyInitialized(PathBuf),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("YAML error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Zip error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("PDF generation error: {0}")]
    Pdf(String),

    #[error("{0}")]
    Other(String),
}

impl McError {
    /// Return an actionable hint for the user, if applicable.
    pub fn hint(&self) -> Option<String> {
        match self {
            McError::InvalidId(_) => Some(
                "Valid IDs look like CUST-001, PROJ-002, MTG-003, or RES-001.".into(),
            ),
            McError::EntityNotFound(_) => Some(
                "Try 'mc list customers' (or projects, meetings, research) to see available entities.".into(),
            ),
            McError::RepoRootNotFound => Some(
                "Run mc from inside a MissionControl repo, pass --root <path>, or run 'mc init' to create a new repo.".into(),
            ),
            McError::AlreadyInitialized(_) => Some(
                "Use --force to reinitialize, or run mc init in a different directory.".into(),
            ),
            McError::TemplateNotFound(_) => Some(
                "Check that your templates/ directory contains the required .md templates.".into(),
            ),
            _ => None,
        }
    }
}

pub type McResult<T> = Result<T, McError>;
