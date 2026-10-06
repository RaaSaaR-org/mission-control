use crate::config::{RepoMode, ResolvedConfig};
use crate::entity::EntityKind;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum McError {
    #[error(
        "Could not find MissionControl repo root (looked for .mc/config.yml and config/config.yml)"
    )]
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

    #[error("Already initialized: config exists at {0}")]
    AlreadyInitialized(PathBuf),

    /// The repo does not enable this kind: embedded repos never have
    /// customers, contacts or projects; standalone repos with a `paths:`
    /// section only have the kinds listed there. Exits with code 2.
    #[error("{}", not_available_message(*.kind, *.embedded))]
    NotAvailableInMode { kind: EntityKind, embedded: bool },

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

    /// Invalid user input (bad value, unknown option). Exits with code 2.
    #[error("{message}")]
    Usage {
        message: String,
        hint: Option<String>,
    },

    /// A specific thing the user asked for does not exist.
    #[error("{message}")]
    NotFound {
        message: String,
        hint: Option<String>,
    },

    /// The file changed since the caller read it (e.g. a checklist item was
    /// edited elsewhere), so the requested change was not applied.
    #[error("{message}")]
    Conflict {
        message: String,
        hint: Option<String>,
    },
}

impl McError {
    /// Return an actionable hint for the user, if applicable.
    pub fn hint(&self) -> Option<String> {
        match self {
            McError::InvalidId(_) => Some(
                "IDs look like TASK-001, PROJ-002 or MTG-003 (prefix, dash, number).".into(),
            ),
            McError::EntityNotFound(_) => Some(
                "Run 'mc list <kind>' (e.g. 'mc list tasks') to see available IDs.".into(),
            ),
            McError::Usage { hint, .. }
            | McError::NotFound { hint, .. }
            | McError::Conflict { hint, .. } => hint.clone(),
            McError::ValidationFailed(_) => Some(
                "Fix the files listed above, then re-run 'mc validate'.".into(),
            ),
            McError::RepoRootNotFound => Some(
                "Run mc from inside a MissionControl repo, pass --root <path>, or run 'mc init' (or 'mc init --embedded') to create one.".into(),
            ),
            McError::AlreadyInitialized(_) => Some(
                "Use --force to reinitialize, or run mc init in a different directory.".into(),
            ),
            McError::TemplateNotFound(_) => Some(
                "Check that your templates/ directory contains the required .md templates.".into(),
            ),
            McError::NotAvailableInMode { embedded: true, .. } => Some(
                "Embedded mode (.mc/) only supports tasks, meetings, research, sprints, and proposals. Use a standalone repo for customers and projects.".into(),
            ),
            McError::NotAvailableInMode { kind, .. } => {
                // Contacts live under customers and have no `paths:` key of their own.
                let key = match kind {
                    EntityKind::Contact => EntityKind::Customer.label_plural(),
                    other => other.label_plural(),
                };
                Some(format!(
                    "Add `{key}: {key}/` under `paths:` in config/config.yml to enable them."
                ))
            }
            _ => None,
        }
    }

    /// Process exit code: 2 for invalid input (matching clap's usage errors), 1 otherwise.
    pub fn exit_code(&self) -> i32 {
        match self {
            McError::Usage { .. } | McError::InvalidId(_) | McError::NotAvailableInMode { .. } => 2,
            _ => 1,
        }
    }

    /// Error for a kind that `cfg` does not enable, worded for the repo's mode.
    pub fn not_available(kind: EntityKind, cfg: &ResolvedConfig) -> Self {
        McError::NotAvailableInMode {
            kind,
            embedded: cfg.mode == RepoMode::Embedded,
        }
    }

    pub fn usage(message: impl Into<String>, hint: Option<String>) -> Self {
        McError::Usage {
            message: message.into(),
            hint,
        }
    }

    pub fn not_found(message: impl Into<String>, hint: Option<String>) -> Self {
        McError::NotFound {
            message: message.into(),
            hint,
        }
    }

    pub fn conflict(message: impl Into<String>, hint: Option<String>) -> Self {
        McError::Conflict {
            message: message.into(),
            hint,
        }
    }
}

fn not_available_message(kind: EntityKind, embedded: bool) -> String {
    if embedded {
        format!(
            "{} entities are not available in embedded mode",
            kind.label()
        )
    } else {
        format!(
            "{} are not enabled in this repo (not listed under `paths:` in the config)",
            kind.label_plural()
        )
    }
}

pub type McResult<T> = Result<T, McError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_available_message_matches_the_mode() {
        let embedded = McError::NotAvailableInMode {
            kind: EntityKind::Customer,
            embedded: true,
        };
        assert_eq!(
            embedded.to_string(),
            "customer entities are not available in embedded mode"
        );
        assert!(embedded.hint().unwrap().contains("Embedded mode"));
        assert_eq!(embedded.exit_code(), 2);

        let standalone = McError::NotAvailableInMode {
            kind: EntityKind::Contact,
            embedded: false,
        };
        let msg = standalone.to_string();
        assert!(
            msg.starts_with("contacts are not enabled in this repo"),
            "{msg}"
        );
        assert!(!msg.contains("embedded"), "{msg}");
        let hint = standalone.hint().unwrap();
        assert!(hint.contains("`customers: customers/`"), "{hint}");
        assert!(!hint.contains("Embedded"), "{hint}");
        assert_eq!(standalone.exit_code(), 2);
    }
}
