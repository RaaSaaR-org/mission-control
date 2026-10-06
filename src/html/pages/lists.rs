//! Generic entity list pages (customers, projects, meetings, ...).

use crate::data::{self, EntityRecord};
use crate::entity::EntityKind;
use crate::frontmatter;
use crate::html::catalog::{display_name, split_wikilink};
use crate::html::components::{
    clip, clip_text, data_table, date_html, due_html, empty_state, entity_name_link, id_chip,
    link_button, meeting_view_toggle, owner_html, page_header, row_filter, status_badge,
    status_lamp, table_count, tag_chips_compact, th, Btn, Sort,
};
use crate::html::format::{capitalize, escape_html, fmt_day_relative, href_with, parse_date};
use crate::html::layout::layout;
use crate::html::Page;
use serde_yaml::Value;

/// Query parameters for a generic list page.
pub struct ListQuery<'a> {
    pub status: Option<&'a str>,
    pub tag: Option<&'a str>,
    pub sort: Option<&'a str>,
    pub dir: &'a str,
}

/// Sort entities by a frontmatter field (`name` sorts by display name).
pub fn sort_entities(entities: &mut [EntityRecord], field: &str, dir: &str) {
    let key = |e: &EntityRecord| -> String {
        if field == "name" || field == "title" {
            display_name(e).to_lowercase()
        } else if field == "id" {
            e.id.to_lowercase()
        } else {
            frontmatter::get_str(&e.frontmatter, field)
                .map(|s| s.to_lowercase())
                .or_else(|| data::get_number(&e.frontmatter, field).map(|n| format!("{n:010}")))
                .unwrap_or_default()
        }
    };
    entities.sort_by_cached_key(key);
    if dir == "desc" {
        entities.reverse();
    }
}

/// A table column on a list page.
enum Col {
    Id,
    Name(&'static str),
    Status,
    Owner,
    Date(&'static str, &'static str),
    /// Meeting date and time, stacked.
    When,
    Refs(&'static str, &'static str, &'static str),
    Tags,
    Summary,
    Text(&'static str, &'static str),
    Email,
    ParentCustomer,
}

impl Col {
    fn header(&self) -> (&'static str, Option<&'static str>) {
        match self {
            Col::Id => ("ID", Some("id")),
            Col::Name(label) => (label, Some("name")),
            Col::Status => ("Status", Some("status")),
            Col::Owner => ("Owner", Some("owner")),
            Col::Date(key, label) => (label, Some(key)),
            Col::When => ("Date", Some("date")),
            Col::Refs(_, _, label) => (label, None),
            Col::Tags => ("Tags", None),
            Col::Summary => ("Summary", None),
            Col::Text(key, label) => (label, Some(key)),
            Col::Email => ("Email", None),
            Col::ParentCustomer => ("Customer", None),
        }
    }

    fn class(&self) -> &'static str {
        match self {
            Col::Id => "col-id",
            Col::Name(_) => "col-name",
            Col::Status => "col-status",
            Col::Date(..) => "col-date",
            Col::When => "col-when",
            Col::Summary => "col-summary",
            Col::Tags => "col-tags",
            _ => "",
        }
    }
}

fn columns_for(kind: EntityKind) -> Vec<Col> {
    match kind {
        EntityKind::Customer => vec![
            Col::Id,
            Col::Name("Name"),
            Col::Status,
            Col::Owner,
            Col::Refs("projects", "project", "Projects"),
            Col::Tags,
            Col::Date("updated", "Updated"),
        ],
        EntityKind::Project => vec![
            Col::Id,
            Col::Name("Name"),
            Col::Status,
            Col::Owner,
            Col::Refs("customers", "customer", "Customers"),
            Col::Tags,
            Col::Date("target_date", "Target"),
        ],
        EntityKind::Meeting => vec![
            Col::When,
            Col::Name("Title"),
            Col::Status,
            Col::Refs("customers", "customer", "Customers"),
            Col::Tags,
        ],
        EntityKind::Research => vec![
            Col::Id,
            Col::Name("Title"),
            Col::Status,
            Col::Owner,
            Col::Summary,
            Col::Date("updated", "Updated"),
        ],
        EntityKind::Sprint => vec![
            Col::Id,
            Col::Name("Title"),
            Col::Status,
            Col::Owner,
            Col::Date("start_date", "Start"),
            Col::Date("end_date", "End"),
        ],
        EntityKind::Proposal => vec![
            Col::Id,
            Col::Name("Title"),
            Col::Status,
            Col::Refs("customers", "customer", "Customers"),
            Col::Refs("projects", "project", "Projects"),
            Col::Date("updated", "Updated"),
        ],
        EntityKind::Contact => vec![
            Col::Id,
            Col::Name("Name"),
            Col::Text("role", "Role"),
            Col::ParentCustomer,
            Col::Email,
            Col::Status,
        ],
        EntityKind::Task => vec![
            Col::Id,
            Col::Name("Title"),
            Col::Status,
            Col::Owner,
            Col::Date("due_date", "Due"),
        ],
    }
}

/// The customer a contact belongs to, derived from its directory.
fn parent_customer_html(e: &EntityRecord, page: &Page) -> String {
    let customers_dir = &page.cfg.customers_dir;
    let dir_name = e
        .source_path
        .strip_prefix(customers_dir)
        .ok()
        .and_then(|rel| rel.components().next())
        .map(|c| c.as_os_str().to_string_lossy().to_string());
    match dir_name {
        Some(d) => page.catalog.ref_html(&d),
        None => String::new(),
    }
}

fn link_list(fm: &Value, key: &str, alt_key: &str) -> Vec<String> {
    let mut items = frontmatter::get_string_list(fm, key);
    if items.is_empty() {
        if let Some(s) = frontmatter::get_str(fm, alt_key).filter(|s| !s.trim().is_empty()) {
            items.push(s.to_string());
        }
    }
    items
}

/// References as clipped links, with every name in the tooltip.
pub(crate) fn refs_clip(page: &Page, items: &[String]) -> String {
    let names: Vec<String> = items
        .iter()
        .map(|raw| {
            let (target, alias) = split_wikilink(raw);
            page.catalog
                .canonical_id(target)
                .and_then(|id| page.catalog.name(id))
                .or(alias)
                .unwrap_or(target)
                .to_string()
        })
        .filter(|n| !n.is_empty())
        .collect();
    clip(&page.catalog.refs_html(items), &names.join(", "))
}

fn cell_html(col: &Col, e: &EntityRecord, page: &Page, list_href: &str) -> String {
    let fm = &e.frontmatter;
    let get = |k: &str| frontmatter::get_str_or(fm, k, "");
    match col {
        Col::Id => id_chip(&e.id),
        Col::Name(_) => entity_name_link(e),
        Col::Status => status_badge(get("status")),
        Col::Owner => owner_html(get("owner")),
        Col::Date(key, _) => {
            let v = get(key);
            if *key == "due_date" {
                due_html(v, page.today)
            } else {
                date_html(v, page.today)
            }
        }
        Col::When => match parse_date(get("date")) {
            Some(d) => format!(
                r#"<span class="when"><time datetime="{}">{}</time><span>{}</span></span>"#,
                d.format("%Y-%m-%d"),
                fmt_day_relative(d, page.today),
                escape_html(get("time"))
            ),
            None => escape_html(get("date")),
        },
        Col::Refs(key, alt, _) => refs_clip(page, &link_list(fm, key, alt)),
        Col::Tags => tag_chips_compact(
            &frontmatter::get_string_list(fm, "tags"),
            Some(list_href),
            2,
        ),
        Col::Summary => clip_text(get("summary")),
        Col::Text(key, _) => clip_text(get(key)),
        Col::Email => {
            let email = get("email").trim();
            if email.is_empty() {
                String::new()
            } else {
                clip(
                    &format!(
                        r#"<a class="plain-link" href="mailto:{0}">{0}</a>"#,
                        escape_html(email)
                    ),
                    email,
                )
            }
        }
        Col::ParentCustomer => parent_customer_html(e, page),
    }
}

/// Render a list page for a given entity kind.
pub fn list_page(
    page: &Page,
    kind: EntityKind,
    entities: &[EntityRecord],
    query: &ListQuery,
) -> String {
    let plural = kind.label_plural();
    let title = capitalize(plural);
    let list_href = format!("/{plural}");
    let total = page.catalog.count_for(kind).map_or(0, |c| c.total);
    let filtered = query.status.is_some() || query.tag.is_some();

    let meta = if filtered {
        format!("{} of {} shown", entities.len(), total)
    } else {
        format!("{} total", total)
    };
    let actions = if kind == EntityKind::Meeting {
        meeting_view_toggle(false)
    } else {
        String::new()
    };
    let mut body = page_header(&title, &meta, &actions);

    if total == 0 {
        body.push_str(&empty_state(
            &format!("No {plural} yet."),
            &format!(
                r#"Create one with <code>mc new {} "Name"</code>."#,
                kind.label()
            ),
            "",
        ));
        return layout(page, &title, &list_href, "", &body);
    }

    body.push_str(&list_toolbar(page, kind, query, total));

    if entities.is_empty() {
        body.push_str(&empty_state(
            &format!("No {plural} match these filters"),
            "Clear the filters to see everything again.",
            &link_button(&list_href, "Clear filters", Btn::Secondary, ""),
        ));
        return layout(page, &title, &list_href, "", &body);
    }

    let cols = columns_for(kind);
    let sort = Sort {
        field: query.sort,
        dir: query.dir,
    };
    let sort_href = |field: &str, dir: &str| {
        href_with(
            &list_href,
            &[
                ("status", query.status.unwrap_or("")),
                ("tag", query.tag.unwrap_or("")),
                ("sort", field),
                ("dir", dir),
            ],
        )
    };
    let head: String = cols
        .iter()
        .map(|col| {
            let (label, key) = col.header();
            th(
                label,
                col.class(),
                key.map(|k| (k, &sort, &sort_href as &dyn Fn(&str, &str) -> String)),
            )
        })
        .collect();
    let rows: String = entities
        .iter()
        .map(|e| {
            let cells: String = cols
                .iter()
                .map(|col| {
                    format!(
                        r#"<td class="{}">{}</td>"#,
                        col.class(),
                        cell_html(col, e, page, &list_href)
                    )
                })
                .collect();
            format!(
                r#"<tr data-row data-id="{}">{cells}</tr>"#,
                escape_html(&e.id)
            )
        })
        .collect();
    body.push_str(&data_table(&format!("list-{plural}"), &head, &rows));
    body.push_str(&table_count(entities.len(), total));

    layout(page, &title, &list_href, "", &body)
}

/// Status chips, the active tag and the live row filter.
fn list_toolbar(page: &Page, kind: EntityKind, query: &ListQuery, total: usize) -> String {
    let plural = kind.label_plural();
    let list_href = format!("/{plural}");
    let tag = query.tag.unwrap_or("");
    let chip = |href: String, active: bool, inner: String| {
        let current = if active {
            r#" chip-active" aria-current="page"#
        } else {
            ""
        };
        format!(r#"<a class="chip{current}" href="{href}">{inner}</a>"#)
    };

    let mut html = String::from(
        r#"<div class="toolbar filter-form"><nav class="chips" aria-label="Filter by status">"#,
    );
    html.push_str(&chip(
        href_with(&list_href, &[("tag", tag)]),
        query.status.is_none(),
        format!(r#"All <span class="chip-count">{total}</span>"#),
    ));
    if let Some(sc) = page.catalog.count_for(kind) {
        for (status, n) in &sc.by_status {
            html.push_str(&chip(
                href_with(&list_href, &[("status", status), ("tag", tag)]),
                query.status == Some(status.as_str()),
                format!(
                    r#"{}{} <span class="chip-count">{n}</span>"#,
                    status_lamp(status),
                    escape_html(&crate::html::format::status_label(status)),
                ),
            ));
        }
    }
    html.push_str("</nav>");
    if let Some(tag) = query.tag {
        html.push_str(&format!(
            r#"<span class="chip chip-active chip-removable">Tag: {}<a href="{}" aria-label="Remove filter" title="Remove filter">×</a></span>"#,
            escape_html(tag),
            href_with(&list_href, &[("status", query.status.unwrap_or(""))]),
        ));
    }
    html.push_str(&row_filter("rows", &format!("Filter {plural}")));
    html.push_str("</div>");
    html
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::catalog::tests::{catalog, rec, test_config};

    #[test]
    fn sort_entities_by_name_and_number() {
        let mut v = vec![
            rec(EntityKind::Task, "TASK-002", "title: beta\npriority: 1"),
            rec(EntityKind::Task, "TASK-001", "title: Alpha\npriority: 3"),
        ];
        sort_entities(&mut v, "name", "asc");
        assert_eq!(v[0].id, "TASK-001");
        sort_entities(&mut v, "priority", "asc");
        assert_eq!(v[0].id, "TASK-002");
        sort_entities(&mut v, "id", "desc");
        assert_eq!(v[0].id, "TASK-002");
    }

    #[test]
    fn list_page_escapes_links_and_keeps_tag_filter() {
        let (_d, cfg) = test_config();
        let make = || {
            vec![rec(
                EntityKind::Customer,
                "CUST-001",
                "name: \"A&B <Corp>\"\nstatus: active\ntags: [\"x y\", b, c]\nowner: Jane",
            )]
        };
        let cat = catalog(make(), &cfg);
        let page = Page::new(&cfg, &cat, "");
        let q = ListQuery {
            status: None,
            tag: Some("<t>"),
            sort: Some("name"),
            dir: "asc",
        };
        let html = list_page(&page, EntityKind::Customer, &make(), &q);
        assert!(html.contains("A&amp;B &lt;Corp&gt;"));
        assert!(!html.contains("<Corp>"));
        assert!(html.contains("Tag: &lt;t&gt;"));
        assert!(html.contains(r#"href="/customers?tag=%3Ct%3E&amp;sort=name&amp;dir=desc""#));
        assert!(html.contains(r#"<span class="tag-more" title="c">+1</span>"#));
        assert!(html.contains(r#"data-id="CUST-001""#));
        assert!(html.contains("Showing <span data-filter-count>1</span> of 1"));
    }

    #[test]
    fn empty_kind_shows_create_hint() {
        let (_d, cfg) = test_config();
        let cat = catalog(Vec::new(), &cfg);
        let page = Page::new(&cfg, &cat, "");
        let q = ListQuery {
            status: None,
            tag: None,
            sort: None,
            dir: "asc",
        };
        let html = list_page(&page, EntityKind::Project, &[], &q);
        assert!(html.contains("No projects yet."));
        assert!(html.contains(r#"mc new project "Name""#));
    }
}
