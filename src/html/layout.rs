//! The app shell: document head, sidebar navigation and page-level mounts.

use super::brand::{brand_css, font_face_css};
use super::components::{icon, kbd, lamp, ICON_SPRITE};
use super::edit::new_task_dialog;
use super::format::{capitalize, escape_html, parse_date};
use super::{go_key, is_closed, Page, NAV_GROUPS};
use crate::config::RepoMode;
use crate::entity::EntityKind;
use crate::frontmatter;

static APP_CSS: &str = include_str!("../assets/app.css");
static APP_JS: &str = include_str!("../assets/app.js");

/// Applies the saved theme and density before first paint.
const BOOT_SCRIPT: &str = r#"<script>try{var d=document.documentElement,t=localStorage.getItem("mc-theme");if(t==="light"||t==="dark")d.dataset.theme=t;if(localStorage.getItem("mc-density")==="compact")d.classList.add("density-compact")}catch(e){}</script>"#;

const FAVICON: &str = r#"<link rel="icon" href="data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><rect width='32' height='32' rx='7' fill='%23121C27'/><path d='M8 22V10l8 7 8-7v12' fill='none' stroke='%235FB0E8' stroke-width='3' stroke-linejoin='round'/></svg>">"#;

/// Wrap body HTML in the full app shell (sidebar + main).
///
/// `active_nav` is the href of the highlighted nav item; `search_query`
/// pre-fills the sidebar search.
pub(crate) fn layout(
    page: &Page,
    title: &str,
    active_nav: &str,
    search_query: &str,
    body: &str,
) -> String {
    let cfg = page.cfg;
    let brand = &cfg.brand;
    let brand_name = escape_html(&brand.name);

    let logo_html = if brand.logo.is_some() {
        r#"<img src="/brand/logo" alt="" class="brand-logo">"#
    } else {
        r#"<span class="brand-mark" aria-hidden="true"></span>"#
    };
    let custom_css_block = if page.custom_css.is_empty() {
        String::new()
    } else {
        format!("<style>{}</style>", page.custom_css)
    };
    let mode_note = if cfg.mode == RepoMode::Embedded {
        "embedded"
    } else {
        "standalone"
    };
    let (edit_attr, access_note, dialog) = if page.editable {
        (" data-edit", "", new_task_dialog(page))
    } else {
        ("", ", read-only", String::new())
    };

    format!(
        r##"<!DOCTYPE html>
<html lang="en"{edit_attr}>
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta name="color-scheme" content="light dark">
  {FAVICON}
  <title>{title} – {brand_name}</title>
  {BOOT_SCRIPT}
  <style>{APP_CSS}</style>
  {brand_css}
  {font_css}
  {custom_css_block}
</head>
<body>
{ICON_SPRITE}
<a class="skip-link" href="#main">Skip to content</a>
<div class="app">
  <aside class="sidebar">
    <input type="checkbox" id="nav-toggle" class="nav-toggle">
    <div class="sidebar-top">
      <a href="/" class="brand">{logo_html}<span class="brand-name">{brand_name}</span></a>
      <a href="/search" class="icon-btn topbar-search" aria-label="Search" title="Search">{search_icon}</a>
      <label for="nav-toggle" class="icon-btn nav-toggle-label" aria-label="Toggle navigation" title="Menu">{menu_icon}</label>
    </div>
    <div class="sidebar-body">
      <form class="site-search" method="get" action="/search" role="search">
        {search_icon}<input type="search" id="site-search" name="q" value="{search_query}" placeholder="Search or jump to" aria-label="Search everything" autocomplete="off"><kbd class="site-search-kbd" data-mod-key>⌘K</kbd>
      </form>
      <nav class="main-nav" aria-label="Main">{nav}</nav>
      <div class="sidebar-foot">
        {theme_switch}
        <p class="app-version">mc {version}, {mode_note}{access_note}</p>
      </div>
    </div>
  </aside>
  <main class="content" id="main" tabindex="-1">
{body}
  </main>
</div>
<div class="toast-region" role="status" aria-live="polite"></div>
{palette}
{sheet}
{dialog}
<script>{APP_JS}</script>
</body>
</html>"##,
        title = escape_html(title),
        brand_css = brand_css(brand),
        font_css = font_face_css(brand),
        search_query = escape_html(search_query),
        search_icon = icon("search"),
        menu_icon = icon("menu"),
        nav = nav_html(page, active_nav),
        theme_switch = theme_switch(),
        version = env!("CARGO_PKG_VERSION"),
        palette = palette(),
        sheet = shortcut_sheet(page),
    )
}

fn nav_html(page: &Page, active_nav: &str) -> String {
    let cfg = page.cfg;
    let nav_link = |href: &str, label: &str, extra: &str, go: Option<&str>| -> String {
        let active = if href == active_nav {
            r#" class="nav-link active" aria-current="page""#
        } else {
            r#" class="nav-link""#
        };
        let go = go.map(|k| format!(r#" data-go="{k}""#)).unwrap_or_default();
        format!(r#"<a href="{href}"{active}{go}><span class="nav-label">{label}</span>{extra}</a>"#)
    };

    let mut nav = nav_link("/", "Overview", "", Some("d"));
    for (group, kinds) in NAV_GROUPS {
        let links: String = kinds
            .iter()
            .filter(|k| cfg.entity_available(k))
            .map(|k| {
                let plural = k.label_plural();
                let mut extra = String::new();
                if *k == EntityKind::Task {
                    let overdue = overdue_tasks(page);
                    if overdue > 0 {
                        extra.push_str(&format!(
                            r#"<span class="nav-alert" title="{overdue} overdue">{}{overdue}<span class="sr-only"> overdue</span></span>"#,
                            lamp("negative", " lamp-solid")
                        ));
                    }
                }
                if let Some(c) = page.catalog.count_for(*k) {
                    extra.push_str(&format!(r#"<span class="nav-count">{}</span>"#, c.total));
                }
                nav_link(
                    &format!("/{plural}"),
                    &capitalize(plural),
                    &extra,
                    go_key(*k),
                )
            })
            .collect();
        if !links.is_empty() {
            nav.push_str(&format!(
                r#"<div class="nav-group"><div class="nav-group-label">{group}</div>{links}</div>"#
            ));
        }
    }
    nav
}

/// Open tasks whose due date has passed.
fn overdue_tasks(page: &Page) -> usize {
    page.catalog
        .of_kind(EntityKind::Task)
        .filter(|t| !is_closed(frontmatter::get_str_or(&t.frontmatter, "status", "")))
        .filter_map(|t| parse_date(frontmatter::get_str_or(&t.frontmatter, "due_date", "")))
        .filter(|d| *d < page.today)
        .count()
}

/// System / light / dark switch. Hidden until the script wires it up.
fn theme_switch() -> String {
    let item = |value: &str, icon_name: &str, label: &str| {
        format!(
            r#"<button type="button" role="radio" aria-checked="false" data-theme-value="{value}" aria-label="{label}" title="{label}">{}</button>"#,
            icon(icon_name)
        )
    };
    format!(
        r#"<div class="theme-switch" role="radiogroup" aria-label="Theme" hidden>{}{}{}</div>"#,
        item("system", "system", "System theme"),
        item("light", "sun", "Light theme"),
        item("dark", "moon", "Dark theme"),
    )
}

/// Empty command palette; the script fills it.
fn palette() -> String {
    format!(
        r#"<dialog class="palette" aria-label="Search or jump to">
  <div class="pal-input">{}<input type="text" role="combobox" aria-expanded="true" aria-controls="pal-list" aria-autocomplete="list" placeholder="Search or jump to" autocomplete="off" spellcheck="false"></div>
  <ul class="pal-list" id="pal-list" role="listbox"></ul>
  <div class="pal-foot"><span>{}{} to move</span><span>{} to open</span><span>{} to close</span></div>
</dialog>"#,
        icon("search"),
        kbd("↑"),
        kbd("↓"),
        kbd("↵"),
        kbd("esc"),
    )
}

/// Keyboard shortcuts for this repo, as (action, keys) pairs.
fn shortcuts(page: &Page) -> Vec<(String, Vec<&'static str>)> {
    let cfg = page.cfg;
    let row = |action: &str, keys: &[&'static str]| (action.to_string(), keys.to_vec());
    let mut rows = vec![row("Search or jump to", &["⌘K", "/"])];
    let tasks = cfg.entity_available(&EntityKind::Task);
    if page.editable && tasks {
        rows.push(row("New task", &["c"]));
    }
    rows.push(row("Go to overview", &["g", "d"]));
    for kind in NAV_GROUPS.iter().flat_map(|(_, k)| k.iter()) {
        let Some(key) = go_key(*kind).filter(|_| cfg.entity_available(kind)) else {
            continue;
        };
        if *kind == EntityKind::Task {
            rows.push(row("Go to task board", &["g", key]));
            rows.push(row("Go to task list", &["g", "l"]));
        } else {
            rows.push((format!("Go to {}", kind.label_plural()), vec!["g", key]));
        }
    }
    if tasks {
        rows.push(row("Board or list view", &["b", "l"]));
    }
    if cfg.entity_available(&EntityKind::Meeting) {
        rows.push(row("Calendar: previous or next month", &["←", "→"]));
        rows.push(row("Calendar: this month", &["."]));
    }
    rows.push(row("Move through rows", &["j", "k"]));
    rows.push(row("Open the selected row", &["↵"]));
    if page.editable && tasks {
        rows.push(row("Move the focused card", &["m"]));
        rows.push(row("Edit this task", &["e"]));
    }
    rows.push(row("Switch light and dark", &["t"]));
    rows.push(row("Show shortcuts", &["?"]));
    rows.push(row("Close or clear", &["esc"]));
    rows
}

fn shortcut_sheet(page: &Page) -> String {
    let rows: String = shortcuts(page)
        .iter()
        .map(|(action, keys)| {
            let keys: String = keys.iter().map(|k| kbd(k)).collect::<Vec<_>>().join(" ");
            format!(r#"<tr><th scope="row">{action}</th><td>{keys}</td></tr>"#)
        })
        .collect();
    format!(
        r#"<dialog class="sheet shortcut-sheet" aria-labelledby="sheet-title">
  <div class="sheet-head"><h2 class="sheet-title" id="sheet-title">Keyboard shortcuts</h2><form method="dialog"><button class="icon-btn" aria-label="Close" title="Close">{}</button></form></div>
  <div class="sheet-body"><table class="shortcut-table"><tbody>{rows}</tbody></table></div>
</dialog>"#,
        icon("close")
    )
}
