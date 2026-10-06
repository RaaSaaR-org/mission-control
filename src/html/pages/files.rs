//! Markdown files that aren't entities (research notes, specs, reports),
//! reached through relative links in entity notes.

use crate::frontmatter;
use crate::html::catalog::display_name;
use crate::html::components::id_chip;
use crate::html::format::{capitalize, entity_href, escape_html};
use crate::html::layout::layout;
use crate::html::markdown::{render_markdown_in, strip_leading_h1, DocContext};
use crate::html::Page;
use std::path::Path;

/// Render a repo Markdown file at `path` (absolute, inside the repo root).
pub fn file_page(page: &Page, path: &Path, content: &str) -> String {
    let root = &page.cfg.root;
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string();
    let (fm, body) = match frontmatter::split_frontmatter(content) {
        Some((fm_str, body)) => (frontmatter::parse_raw(&fm_str, path).ok(), body),
        None => (None, content.to_string()),
    };
    let body = body.trim();
    let title = fm
        .as_ref()
        .and_then(|fm| frontmatter::get_str(fm, "title"))
        .map(str::to_string)
        .or_else(|| {
            body.strip_prefix("# ")
                .and_then(|rest| rest.lines().next())
                .map(|l| l.trim().to_string())
        })
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default()
        });

    // The entity this file sits under, e.g. RES-006 for
    // research/RES-006-x/claude/notes.md.
    let owner = path
        .ancestors()
        .skip(1)
        .take_while(|d| d.starts_with(root) && *d != root.as_path())
        .find_map(|dir| {
            let mut in_dir = page
                .catalog
                .records
                .iter()
                .filter(|r| r.source_path.parent() == Some(dir));
            match (in_dir.next(), in_dir.next()) {
                (Some(only), None) => Some(only),
                _ => None,
            }
        });
    let (crumbs, active_nav) = match owner {
        Some(rec) => {
            let plural = rec.kind.label_plural();
            (
                format!(
                    r#"<a href="/{plural}">{}</a><span class="sep" aria-hidden="true">/</span><a href="{}" title="{}">{}</a><span class="sep" aria-hidden="true">/</span>"#,
                    capitalize(plural),
                    entity_href(&rec.id),
                    escape_html(display_name(rec)),
                    id_chip(&rec.id)
                ),
                format!("/{plural}"),
            )
        }
        None => (String::new(), String::new()),
    };
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let dir = path.parent().unwrap_or(root);
    let doc = DocContext { root, dir };
    let notes = render_markdown_in(strip_leading_h1(body), page.catalog, Some(&doc));
    let notes = if notes.trim().is_empty() {
        r#"<p class="muted detail-empty">This file is empty.</p>"#.to_string()
    } else {
        format!(r#"<div class="detail-body prose">{notes}</div>"#)
    };

    let content = format!(
        r#"<article class="detail file-doc">
<nav class="breadcrumb" aria-label="Breadcrumb">{crumbs}<span class="crumb-id">{}</span></nav>
<header class="detail-hero"><h1 class="detail-title">{}</h1></header>
<div class="detail-source" title="Source file"><code>{}</code></div>
{notes}
</article>"#,
        escape_html(&file_name),
        escape_html(&title),
        escape_html(&rel),
    );
    layout(page, &title, &active_nav, "", &content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityKind;
    use crate::html::catalog::tests::{catalog, rec, test_config};

    #[test]
    fn file_page_links_back_to_owner_and_resolves_links() {
        let (_d, cfg) = test_config();
        let mut res = rec(EntityKind::Research, "RES-001", "title: Robots");
        let dir = cfg.research_dir.join("RES-001-robots");
        res.source_path = dir.join("RES-001.md");
        let cat = catalog(vec![res], &cfg);
        let page = Page::new(&cfg, &cat, "");
        let path = dir.join("claude").join("notes.md");
        let html = file_page(
            &page,
            &path,
            "# Deep <dive>\n\nSee [main](../RES-001.md), [next](more%20notes.md#top) and ![x](img/a.png).",
        );
        assert!(html.contains("Deep &lt;dive&gt;</h1>"));
        assert!(html.contains(r#"href="/entity/RES-001">main</a>"#));
        assert!(
            html.contains(r#"href="/files/research/RES-001-robots/claude/more%20notes.md#top""#)
        );
        assert!(html.contains(r#"src="/files/research/RES-001-robots/claude/img/a.png""#));
        assert!(html.contains(r#"<a href="/entity/RES-001" title="Robots">"#));
    }
}
