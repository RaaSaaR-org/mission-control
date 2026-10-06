//! Search results page and the JSON index used by the command palette.

use crate::entity::EntityKind;
use crate::frontmatter;
use crate::html::catalog::{display_name, Catalog};
use crate::html::components::{
    empty_state, entity_name_link, id_chip, kbd, page_header, status_badge, status_tone,
    submit_button, Btn,
};
use crate::html::format::{capitalize, escape_html};
use crate::html::layout::layout;
use crate::html::search::search;
use crate::html::{go_key, Page, NAV_GROUPS};
use serde_json::json;

const MAX_RESULTS: usize = 100;

/// Render the search results page.
pub fn search_page(page: &Page, query: &str) -> String {
    let q = query.trim();
    let hits = search(page.catalog, q);
    let meta = if q.is_empty() {
        String::new()
    } else {
        format!(
            "{} result{} for “{}”",
            hits.len(),
            if hits.len() == 1 { "" } else { "s" },
            escape_html(q)
        )
    };
    let mut body = page_header("Search", &meta, "");
    body.push_str(&format!(
        r#"<form class="search-large" method="get" action="/search" role="search"><input type="search" name="q" value="{}" placeholder="Search names, IDs, tags and notes" aria-label="Search" autocomplete="off" autofocus>{}</form>"#,
        escape_html(query),
        submit_button("Search", Btn::Primary)
    ));

    if q.is_empty() {
        body.push_str(&empty_state(
            "Search everything",
            &format!(
                "Find customers, projects, tasks, meetings and notes by name, ID, tag or text. Press {} to jump straight to one.",
                kbd("⌘K")
            ),
            "",
        ));
    } else if hits.is_empty() {
        body.push_str(&empty_state(
            &format!("No matches for “{}”", escape_html(q)),
            "Every word has to appear somewhere in an entity. Try fewer or shorter words.",
            "",
        ));
    } else {
        body.push_str(r#"<ol class="search-results" id="rows">"#);
        for hit in hits.iter().take(MAX_RESULTS) {
            let e = hit.record;
            let snippet = hit
                .snippet
                .as_ref()
                .map(|s| format!(r#"<p class="search-snippet">{s}</p>"#))
                .unwrap_or_default();
            body.push_str(&format!(
                r#"<li data-row data-id="{}"><div class="search-head"><span class="search-kind">{}</span><span class="search-title">{}{}</span>{}</div>{snippet}</li>"#,
                escape_html(&e.id),
                capitalize(e.kind.label()),
                entity_name_link(e),
                id_chip(&e.id),
                status_badge(frontmatter::get_str_or(&e.frontmatter, "status", "")),
            ));
        }
        body.push_str("</ol>");
        if hits.len() > MAX_RESULTS {
            body.push_str(&format!(
                r#"<p class="table-count">Showing the best {MAX_RESULTS} of {}. Add words to narrow it down.</p>"#,
                hits.len()
            ));
        }
    }
    layout(page, "Search", "", query, &body)
}

/// Compact JSON index of every entity for client-side jump-to:
/// `[{"id","t" (title),"k" (kind),"s" (status),"d" (due or meeting date)}]`.
pub fn index_json(catalog: &Catalog) -> String {
    let items: Vec<serde_json::Value> = catalog
        .records
        .iter()
        .map(|r| {
            let fm = &r.frontmatter;
            let date = frontmatter::get_str(fm, "due_date")
                .or_else(|| frontmatter::get_str(fm, "date"))
                .unwrap_or("");
            serde_json::json!({
                "id": r.id,
                "t": display_name(r),
                "k": r.kind.label(),
                "s": frontmatter::get_str_or(fm, "status", ""),
                "d": date,
            })
        })
        .collect();
    serde_json::Value::Array(items).to_string()
}

/// Tags shown and matched per entity in the palette.
const PALETTE_TAGS: usize = 8;

/// Data for the command palette: pages to jump to, entity kinds in nav order,
/// and every entity with its status tone and tags. Hrefs are relative to the
/// base path; the script prefixes it.
pub fn palette_json(page: &Page) -> String {
    let cfg = page.cfg;
    let kinds: Vec<EntityKind> = NAV_GROUPS
        .iter()
        .flat_map(|(_, kinds)| kinds.iter().copied())
        .filter(|k| cfg.entity_available(k))
        .collect();
    let mut pages = vec![json!({"label": "Overview", "href": "/", "keys": "g d"})];
    for kind in &kinds {
        let keys = go_key(*kind).map(|k| format!("g {k}")).unwrap_or_default();
        if *kind == EntityKind::Task {
            pages.push(json!({"label": "Task board", "href": "/tasks", "keys": keys}));
            pages.push(json!({"label": "Task list", "href": "/tasks/list", "keys": "g l"}));
        } else {
            let plural = kind.label_plural();
            pages.push(
                json!({"label": capitalize(plural), "href": format!("/{plural}"), "keys": keys}),
            );
            if *kind == EntityKind::Meeting {
                pages.push(
                    json!({"label": "Meeting calendar", "href": "/meetings/calendar", "keys": ""}),
                );
            }
        }
    }
    pages.push(json!({"label": "Search", "href": "/search", "keys": ""}));
    let kinds: Vec<serde_json::Value> = kinds
        .iter()
        .map(|k| json!({"k": k.label(), "label": capitalize(k.label_plural())}))
        .collect();
    let entities: Vec<serde_json::Value> = page
        .catalog
        .records
        .iter()
        .map(|r| {
            let fm = &r.frontmatter;
            let status = frontmatter::get_str_or(fm, "status", "");
            let date = frontmatter::get_str(fm, "due_date")
                .or_else(|| frontmatter::get_str(fm, "date"))
                .unwrap_or("");
            let mut tags = frontmatter::get_string_list(fm, "tags");
            tags.truncate(PALETTE_TAGS);
            json!({
                "id": r.id,
                "t": display_name(r),
                "k": r.kind.label(),
                "s": status,
                "tone": if status.is_empty() { "neutral" } else { status_tone(status) },
                "d": date,
                "tags": tags,
            })
        })
        .collect();
    json!({
        "editable": page.editable,
        "pages": pages,
        "kinds": kinds,
        "entities": entities,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityKind;
    use crate::html::catalog::tests::{catalog, rec, test_config};

    #[test]
    fn search_page_escapes_query() {
        let (_d, cfg) = test_config();
        let cat = catalog(
            vec![rec(EntityKind::Task, "TASK-001", "title: Robot")],
            &cfg,
        );
        let page = Page::new(&cfg, &cat, "");
        let html = search_page(&page, "<script>");
        assert!(!html.contains("<script>alert"));
        assert!(html.contains("No matches for “&lt;script&gt;”"));
        let html = search_page(&page, "robot");
        assert!(html.contains("1 result for “robot”"));
        assert!(html.contains(r#"data-id="TASK-001""#));
        assert!(search_page(&page, "").contains("Search everything"));
    }

    #[test]
    fn index_json_lists_every_record() {
        let (_d, cfg) = test_config();
        let cat = catalog(
            vec![
                rec(
                    EntityKind::Task,
                    "TASK-001",
                    "title: \"A \\\"q\\\"\"\nstatus: todo\ndue_date: 2026-10-09",
                ),
                rec(EntityKind::Meeting, "MTG-001", "title: M\ndate: 2026-10-15"),
            ],
            &cfg,
        );
        let v: serde_json::Value = serde_json::from_str(&index_json(&cat)).unwrap();
        assert_eq!(v[0]["id"], "TASK-001");
        assert_eq!(v[0]["t"], "A \"q\"");
        assert_eq!(v[0]["k"], "task");
        assert_eq!(v[0]["d"], "2026-10-09");
        assert_eq!(v[1]["d"], "2026-10-15");
    }

    #[test]
    fn palette_json_has_pages_kinds_and_tags() {
        let (_d, cfg) = test_config();
        let cat = catalog(
            vec![rec(
                EntityKind::Task,
                "TASK-001",
                "title: Robot\nstatus: done\ntags: [vla, ops]",
            )],
            &cfg,
        );
        let page = Page::new(&cfg, &cat, "").with_editable(true);
        let v: serde_json::Value = serde_json::from_str(&palette_json(&page)).unwrap();
        assert_eq!(v["editable"], true);
        assert_eq!(v["pages"][0]["href"], "/");
        assert!(v["pages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["href"] == "/tasks/list" && p["keys"] == "g l"));
        assert_eq!(v["kinds"][0]["k"], "task");
        assert_eq!(v["entities"][0]["tone"], "positive");
        assert_eq!(v["entities"][0]["tags"][1], "ops");
    }
}
