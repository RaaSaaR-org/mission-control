//! The entity catalog: every record in the repo plus derived lookups.

use super::all_kinds;
use super::format::{entity_href, escape_html};
use crate::config::ResolvedConfig;
use crate::data::{self, EntityRecord, StatusCounts};
use crate::entity::EntityKind;
use crate::frontmatter;
use regex::Regex;
use serde_yaml::Value;
use std::collections::HashMap;

/// Every entity in the repo, plus lookups derived from them.
pub struct Catalog {
    pub records: Vec<EntityRecord>,
    counts: Vec<(EntityKind, StatusCounts)>,
    names: HashMap<String, (EntityKind, String)>,
    /// Matches an ID at the start of a string, e.g. `CONT-003` in `CONT-003-jane-doe`.
    id_prefix_re: Regex,
    /// Matches IDs anywhere in a string.
    id_any_re: Regex,
}

impl Catalog {
    /// Load all entities available in this repo.
    pub fn load(cfg: &ResolvedConfig) -> Self {
        let mut records = Vec::new();
        let mut counts = Vec::new();
        for kind in all_kinds() {
            if !cfg.entity_available(&kind) {
                continue;
            }
            let recs = data::collect_entities(kind, cfg).unwrap_or_else(|e| {
                eprintln!("serve: error loading {}: {}", kind.label_plural(), e);
                Vec::new()
            });
            counts.push((kind, status_counts(kind, &recs, kind.statuses(cfg))));
            records.extend(recs);
        }
        Self::from_records(records, counts, cfg)
    }

    pub(crate) fn from_records(
        records: Vec<EntityRecord>,
        counts: Vec<(EntityKind, StatusCounts)>,
        cfg: &ResolvedConfig,
    ) -> Self {
        let names = records
            .iter()
            .map(|r| (r.id.clone(), (r.kind, display_name(r).to_string())))
            .collect();
        let prefixes = all_kinds()
            .map(|k| regex::escape(k.prefix(cfg)))
            .collect::<Vec<_>>()
            .join("|");
        Self {
            records,
            counts,
            names,
            id_prefix_re: Regex::new(&format!(r"^(?:{prefixes})-\d+")).expect("escaped regex"),
            id_any_re: Regex::new(&format!(r"\b(?:{prefixes})-\d+\b")).expect("escaped regex"),
        }
    }

    pub fn counts(&self) -> impl Iterator<Item = &StatusCounts> {
        self.counts.iter().map(|(_, c)| c)
    }

    pub(crate) fn count_for(&self, kind: EntityKind) -> Option<&StatusCounts> {
        self.counts.iter().find(|(k, _)| *k == kind).map(|(_, c)| c)
    }

    pub fn of_kind(&self, kind: EntityKind) -> impl Iterator<Item = &EntityRecord> {
        self.records.iter().filter(move |r| r.kind == kind)
    }

    /// Display name for an entity ID, if it exists.
    pub fn name(&self, id: &str) -> Option<&str> {
        self.names.get(id).map(|(_, n)| n.as_str())
    }

    /// Extract the entity ID at the start of a reference target
    /// (`CONT-003-jane-doe` → `CONT-003`). Returns `None` if the target doesn't
    /// start with a known ID prefix.
    pub fn canonical_id<'a>(&self, target: &'a str) -> Option<&'a str> {
        self.id_prefix_re.find(target.trim()).map(|m| m.as_str())
    }

    /// The entity stored at `path`, if any.
    pub fn id_for_path(&self, path: &std::path::Path) -> Option<&str> {
        self.records
            .iter()
            .find(|r| r.source_path == path)
            .map(|r| r.id.as_str())
    }

    pub(crate) fn id_regex(&self) -> &Regex {
        &self.id_any_re
    }

    /// Render a reference (`CUST-001`, `[[CUST-001|Alias]]`, `[[CONT-003-slug|Name]]`)
    /// as a link showing the entity's name. Unknown targets render as plain text.
    pub fn ref_html(&self, raw: &str) -> String {
        let (target, alias) = split_wikilink(raw);
        if let Some(id) = self.canonical_id(target) {
            if let Some(name) = self.name(id) {
                return format!(
                    r#"<a class="ref" href="{}" title="{}">{}</a>"#,
                    entity_href(id),
                    escape_html(id),
                    escape_html(name),
                );
            }
        }
        let text = alias.unwrap_or(target).trim();
        if text.is_empty() {
            String::new()
        } else {
            format!(r#"<span class="ref-plain">{}</span>"#, escape_html(text))
        }
    }

    /// All references as comma-separated links.
    pub(crate) fn refs_html(&self, items: &[String]) -> String {
        let refs: Vec<String> = items
            .iter()
            .map(|s| self.ref_html(s))
            .filter(|s| !s.is_empty())
            .collect();
        if refs.is_empty() {
            String::new()
        } else {
            format!(r#"<span class="refs">{}</span>"#, refs.join(", "))
        }
    }

    /// The first reference as a link, then `+N` with every name in its title.
    /// Duplicate targets are counted once.
    pub(crate) fn refs_html_compact(&self, items: &[String]) -> String {
        let mut seen: Vec<&str> = Vec::new();
        for raw in items {
            let (target, _) = split_wikilink(raw);
            let key = self.canonical_id(target).unwrap_or(target);
            if !key.is_empty() && !seen.contains(&key) {
                seen.push(key);
            }
        }
        let Some(first) = items
            .iter()
            .find(|raw| !split_wikilink(raw).0.is_empty())
            .map(|raw| self.ref_html(raw))
        else {
            return String::new();
        };
        if seen.len() <= 1 {
            return format!(r#"<span class="refs">{first}</span>"#);
        }
        let names: Vec<String> = items
            .iter()
            .map(|raw| {
                let (target, alias) = split_wikilink(raw);
                self.canonical_id(target)
                    .and_then(|id| self.name(id))
                    .or(alias)
                    .unwrap_or(target)
                    .to_string()
            })
            .filter(|n| !n.is_empty())
            .fold(Vec::new(), |mut acc, n| {
                if !acc.contains(&n) {
                    acc.push(n);
                }
                acc
            });
        format!(
            r#"<span class="refs">{first} <span class="more" title="{}">+{}</span></span>"#,
            escape_html(&names.join(", ")),
            seen.len() - 1
        )
    }

    /// Whether a YAML value mentions the given ID anywhere.
    fn value_mentions(&self, value: &Value, id: &str) -> bool {
        match value {
            Value::String(s) => self.id_any_re.find_iter(s).any(|m| m.as_str() == id),
            Value::Sequence(seq) => seq.iter().any(|v| self.value_mentions(v, id)),
            Value::Mapping(map) => map.values().any(|v| self.value_mentions(v, id)),
            Value::Tagged(t) => self.value_mentions(&t.value, id),
            _ => false,
        }
    }

    pub(crate) fn record_mentions(&self, rec: &EntityRecord, id: &str) -> bool {
        self.value_mentions(&rec.frontmatter, id)
            || self
                .id_any_re
                .find_iter(&rec.body)
                .any(|m| m.as_str() == id)
    }
}

/// Count records by status, ordered by the configured status list.
fn status_counts(kind: EntityKind, records: &[EntityRecord], order: &[String]) -> StatusCounts {
    let mut map: HashMap<String, usize> = HashMap::new();
    for r in records {
        let s = frontmatter::get_str(&r.frontmatter, "status").unwrap_or("unknown");
        *map.entry(s.to_string()).or_insert(0) += 1;
    }
    let mut by_status: Vec<(String, usize)> = order
        .iter()
        .filter_map(|s| map.remove(s).map(|n| (s.clone(), n)))
        .collect();
    let mut rest: Vec<(String, usize)> = map.into_iter().collect();
    rest.sort();
    by_status.extend(rest);
    StatusCounts {
        label: kind.label_plural().to_string(),
        total: records.len(),
        by_status,
    }
}

/// Split `[[target|alias]]` into its parts. Plain strings return `(s, None)`.
pub(crate) fn split_wikilink(raw: &str) -> (&str, Option<&str>) {
    let raw = raw.trim();
    match raw.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
        Some(inner) => match inner.split_once('|') {
            Some((t, a)) => (t.trim(), Some(a.trim())),
            None => (inner.trim(), None),
        },
        None => (raw, None),
    }
}

/// The best human-readable name for an entity: `name`, then `title`, then ID.
pub fn display_name(e: &EntityRecord) -> &str {
    frontmatter::get_str(&e.frontmatter, "name")
        .filter(|s| !s.trim().is_empty())
        .or_else(|| frontmatter::get_str(&e.frontmatter, "title").filter(|s| !s.trim().is_empty()))
        .unwrap_or(&e.id)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config::{load_config, RepoMode};
    use std::path::PathBuf;

    /// A config for a throwaway standalone repo, for tests that need prefixes.
    pub(crate) fn test_config() -> (tempfile::TempDir, ResolvedConfig) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        std::fs::write(
            dir.path().join("config/config.yml"),
            "site:\n  name: Test\n",
        )
        .unwrap();
        let cfg = load_config(dir.path(), RepoMode::Standalone).unwrap();
        (dir, cfg)
    }

    pub(crate) fn rec(kind: EntityKind, id: &str, yaml: &str) -> EntityRecord {
        EntityRecord {
            kind,
            id: id.to_string(),
            frontmatter: serde_yaml::from_str(yaml).unwrap(),
            body: String::new(),
            source_path: PathBuf::from(format!("/x/{id}.md")),
        }
    }

    /// A catalog with status counts for every kind available in `cfg`.
    pub(crate) fn catalog(records: Vec<EntityRecord>, cfg: &ResolvedConfig) -> Catalog {
        let counts = all_kinds()
            .filter(|k| cfg.entity_available(k))
            .map(|k| {
                let recs: Vec<EntityRecord> = records
                    .iter()
                    .filter(|r| r.kind == k)
                    .map(|r| {
                        rec(
                            r.kind,
                            &r.id,
                            &serde_yaml::to_string(&r.frontmatter).unwrap(),
                        )
                    })
                    .collect();
                (k, status_counts(k, &recs, k.statuses(cfg)))
            })
            .collect();
        Catalog::from_records(records, counts, cfg)
    }

    #[test]
    fn split_wikilink_handles_alias_and_plain() {
        assert_eq!(
            split_wikilink("[[CUST-001|Acme]]"),
            ("CUST-001", Some("Acme"))
        );
        assert_eq!(split_wikilink("[[PROJ-002]]"), ("PROJ-002", None));
        assert_eq!(split_wikilink("TASK-003"), ("TASK-003", None));
    }

    #[test]
    fn ref_html_escapes_names_and_resolves_slugs() {
        let (_d, cfg) = test_config();
        let cat = catalog(
            vec![rec(
                EntityKind::Contact,
                "CONT-003",
                "name: \"<b>Jane</b>\"",
            )],
            &cfg,
        );
        let html = cat.ref_html("[[CONT-003-jane|J]]");
        assert!(html.contains(r#"href="/entity/CONT-003""#));
        assert!(html.contains("&lt;b&gt;Jane&lt;/b&gt;"));
        assert_eq!(
            cat.ref_html("<script>"),
            r#"<span class="ref-plain">&lt;script&gt;</span>"#
        );
    }

    #[test]
    fn refs_compact_dedupes_and_counts() {
        let (_d, cfg) = test_config();
        let cat = catalog(
            vec![
                rec(EntityKind::Customer, "CUST-001", "name: Acme"),
                rec(EntityKind::Customer, "CUST-002", "name: Beta"),
            ],
            &cfg,
        );
        let items: Vec<String> = ["[[CUST-001|Acme]]", "CUST-001", "CUST-002", "Gamma"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let html = cat.refs_html_compact(&items);
        assert!(html.contains(">Acme</a>"));
        assert!(html.contains(r#"title="Acme, Beta, Gamma">+2<"#));
        assert_eq!(cat.refs_html_compact(&[]), "");
        assert!(!cat
            .refs_html_compact(&["CUST-002".to_string()])
            .contains("more"));
    }
}
