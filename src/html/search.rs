//! Full-text search over the catalog.

use super::catalog::{display_name, Catalog};
use super::format::escape_html;
use crate::data::EntityRecord;
use crate::frontmatter;

/// A single search hit.
pub struct SearchHit<'a> {
    pub record: &'a EntityRecord,
    pub score: u32,
    pub snippet: Option<String>,
}

/// Search all entities by ID, name, tags, and body text.
pub fn search<'a>(catalog: &'a Catalog, query: &str) -> Vec<SearchHit<'a>> {
    let terms: Vec<String> = query.split_whitespace().map(|t| t.to_lowercase()).collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<SearchHit> = catalog
        .records
        .iter()
        .filter_map(|rec| {
            let id = rec.id.to_lowercase();
            let name = display_name(rec).to_lowercase();
            let tags = frontmatter::get_string_list(&rec.frontmatter, "tags")
                .join(" ")
                .to_lowercase();
            let summary = frontmatter::get_str_or(&rec.frontmatter, "summary", "").to_lowercase();
            let body = rec.body.to_lowercase();
            let mut score = 0;
            let mut snippet_term: Option<&str> = None;
            for term in &terms {
                let s = if id == *term {
                    100
                } else if name.contains(term.as_str()) {
                    if name.starts_with(term.as_str()) {
                        40
                    } else {
                        30
                    }
                } else if id.contains(term.as_str()) {
                    25
                } else if tags.contains(term.as_str()) {
                    15
                } else if summary.contains(term.as_str()) {
                    10
                } else if body.contains(term.as_str()) {
                    snippet_term.get_or_insert(term);
                    5
                } else {
                    return None; // every term must match somewhere
                };
                score += s;
            }
            let snippet = snippet_term.and_then(|t| make_snippet(&rec.body, t));
            Some(SearchHit {
                record: rec,
                score,
                snippet,
            })
        })
        .collect();
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.record.id.cmp(&b.record.id))
    });
    hits
}

/// A short excerpt of `text` around the first occurrence of `term`, with the match highlighted.
fn make_snippet(text: &str, term: &str) -> Option<String> {
    let lower = text.to_lowercase();
    // Lowercasing can change byte lengths for some scripts; bail out rather than mis-slice.
    if lower.len() != text.len() {
        return None;
    }
    let pos = lower.find(term)?;
    let floor = |mut i: usize| {
        while !text.is_char_boundary(i) {
            i -= 1;
        }
        i
    };
    let start = floor(pos.saturating_sub(60));
    let end = floor((pos + term.len() + 80).min(text.len()));
    // Collapse whitespace and strip Markdown markers, keeping edge spaces so
    // the highlighted term stays separated from its neighbours.
    let clean = |s: &str| {
        let s = s
            .replace("**", "")
            .replace("[[", "")
            .replace("]]", "")
            .replace(['#', '`'], "");
        let mut out = s.split_whitespace().collect::<Vec<_>>().join(" ");
        if s.starts_with(char::is_whitespace) {
            out.insert(0, ' ');
        }
        if s.ends_with(char::is_whitespace) && !out.ends_with(' ') {
            out.push(' ');
        }
        out
    };
    Some(format!(
        "{}{}<mark>{}</mark>{}{}",
        if start > 0 { "…" } else { "" },
        escape_html(&clean(&text[start..pos])),
        escape_html(&text[pos..pos + term.len()]),
        escape_html(&clean(&text[pos + term.len()..end])),
        if end < text.len() { "…" } else { "" },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityKind;
    use crate::html::catalog::tests::{catalog, rec, test_config};

    #[test]
    fn snippet_highlights_match() {
        let s = make_snippet("The quick brown fox jumps", "brown").unwrap();
        assert!(s.contains("quick <mark>brown</mark> fox"));
        let s = make_snippet("**Use case:** pick parts", "pick").unwrap();
        assert!(s.starts_with("Use case: <mark>pick</mark>"));
        let s = make_snippet("a <b> c", "c").unwrap();
        assert!(s.contains("&lt;b&gt;"));
    }

    #[test]
    fn search_requires_every_term_and_ranks_ids_first() {
        let (_d, cfg) = test_config();
        let mut body_hit = rec(EntityKind::Research, "RES-001", "title: Notes");
        body_hit.body = "We talked about robots".into();
        let cat = catalog(
            vec![
                rec(EntityKind::Task, "TASK-001", "title: Robot arm"),
                body_hit,
            ],
            &cfg,
        );
        let hits = search(&cat, "robot");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].record.id, "TASK-001");
        assert!(hits[1]
            .snippet
            .as_deref()
            .unwrap()
            .contains("<mark>robot</mark>"));
        assert!(search(&cat, "robot zebra").is_empty());
        assert_eq!(search(&cat, "task-001")[0].score, 100);
    }
}
