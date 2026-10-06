//! HTML rendering for the `mc serve` web dashboard.
//!
//! Pages are rendered server-side as plain strings. A [`Catalog`] of every
//! entity in the repo is loaded once per request so pages can show sidebar
//! counts, resolve `[[ID|alias]]` references to display names, and compute
//! backlinks without re-walking the filesystem.
//!
//! Module layout:
//! - `catalog`: the entity catalog and reference resolution
//! - `format`: escaping, dates, URLs and other string helpers
//! - `components`: small reusable HTML pieces (badges, buttons, tables, ...)
//! - `layout`: the app shell around every page
//! - `edit`: edit UI (quick-create dialog, task edit form, move menu)
//! - `brand`: brand colours, fonts and base-path rewriting
//! - `markdown`: Markdown rendering with wikilinks and sanitised raw HTML
//! - `notes`: detail-page body with tickable checklists, and comments
//! - `search`: full-text entity search
//! - `pages`: one module per page (including the meeting calendar), plus the entity preview card

mod brand;
mod catalog;
mod components;
mod edit;
mod format;
mod layout;
mod markdown;
mod notes;
mod pages;
mod search;

pub use brand::{brand_css, font_face_css, prefix_base_path, rewrite_css_urls};
pub use catalog::{display_name, Catalog};
pub use components::{status_badge, status_tone};
pub use edit::PRIORITIES;
pub use markdown::render_markdown;
pub use notes::comment_html;
pub use pages::calendar::{calendar_page, CalendarQuery};
pub use pages::dashboard::dashboard_page;
pub use pages::detail::{detail_fragments, detail_page};
pub use pages::errors::{error_page, not_found_page};
pub use pages::files::file_page;
pub use pages::lists::{list_page, sort_entities, ListQuery};
pub use pages::preview::{preview_card, preview_not_found};
pub use pages::search::{index_json, palette_json, search_page};
pub use pages::tasks::{board_page, task_card, tasks_list_page, TaskFilterOptions, TaskQuery};
pub use search::{search, SearchHit};

use crate::config::ResolvedConfig;
use crate::entity::EntityKind;
use chrono::{Local, NaiveDate};

/// The self-hosted UI font (Archivo, SIL OFL 1.1), served at `/assets/archivo.woff2`.
pub static ARCHIVO_WOFF2: &[u8] = include_bytes!("assets/fonts/archivo.woff2");

/// Everything a page needs besides its own data.
pub struct Page<'a> {
    pub cfg: &'a ResolvedConfig,
    pub catalog: &'a Catalog,
    pub custom_css: &'a str,
    pub today: NaiveDate,
    /// Whether the server accepts edits; controls the edit UI.
    pub editable: bool,
}

impl<'a> Page<'a> {
    pub fn new(cfg: &'a ResolvedConfig, catalog: &'a Catalog, custom_css: &'a str) -> Self {
        Self {
            cfg,
            catalog,
            custom_css,
            today: Local::now().date_naive(),
            editable: false,
        }
    }

    /// Show or hide the edit UI (task moves, quick create, edit forms).
    pub fn with_editable(mut self, editable: bool) -> Self {
        self.editable = editable;
        self
    }
}

/// Sidebar navigation groups, in display order.
const NAV_GROUPS: &[(&str, &[EntityKind])] = &[
    (
        "Work",
        &[EntityKind::Task, EntityKind::Sprint, EntityKind::Meeting],
    ),
    (
        "Clients",
        &[
            EntityKind::Customer,
            EntityKind::Contact,
            EntityKind::Project,
        ],
    ),
    ("Knowledge", &[EntityKind::Research, EntityKind::Proposal]),
];

/// Key that follows `g` to jump to a kind's list (`g m` for meetings).
fn go_key(kind: EntityKind) -> Option<&'static str> {
    Some(match kind {
        EntityKind::Task => "t",
        EntityKind::Meeting => "m",
        EntityKind::Customer => "c",
        EntityKind::Project => "p",
        EntityKind::Research => "r",
        EntityKind::Sprint => "s",
        EntityKind::Contact | EntityKind::Proposal => return None,
    })
}

fn all_kinds() -> impl Iterator<Item = EntityKind> {
    [
        EntityKind::Customer,
        EntityKind::Project,
        EntityKind::Meeting,
        EntityKind::Research,
        EntityKind::Task,
        EntityKind::Sprint,
        EntityKind::Proposal,
        EntityKind::Contact,
    ]
    .into_iter()
}

/// Statuses that mean a task needs no more work.
fn is_closed(status: &str) -> bool {
    matches!(
        status,
        "done" | "completed" | "cancelled" | "canceled" | "archived"
    )
}

fn is_cancelled(status: &str) -> bool {
    matches!(status, "cancelled" | "canceled")
}
