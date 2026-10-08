//! The entity catalog: every record in the repo plus derived lookups.

use super::format::{entity_href, escape_html};
use super::markdown::WIKILINK_RE;
use crate::config::ResolvedConfig;
use crate::data::{self, EntityRecord, StatusCounts};
use crate::entity::EntityKind;
use crate::frontmatter;
use crate::util::{slug_variants, slugify};
use regex::Regex;
use serde_yaml::Value;
use std::collections::HashMap;

/// Every entity in the repo, plus lookups derived from them.
pub struct Catalog {
    pub records: Vec<EntityRecord>,
    counts: Vec<(EntityKind, StatusCounts)>,
    names: HashMap<String, (EntityKind, String)>,
    /// Slugs each entity is known by (name, `slug` field, file and folder
    /// name), to check that `[[CONT-003-jane-doe|Jane]]` still means it.
    slugs: HashMap<String, Vec<String>>,
    /// Kind for each configured ID prefix.
    prefixes: Vec<(String, EntityKind)>,
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
        for kind in EntityKind::ALL.into_iter() {
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
        let slugs = records
            .iter()
            .map(|r| (r.id.clone(), record_slugs(r)))
            .collect();
        let kinds: Vec<(String, EntityKind)> = EntityKind::ALL
            .into_iter()
            .map(|k| (k.prefix(cfg).to_string(), k))
            .collect();
        let prefixes = kinds
            .iter()
            .map(|(p, _)| regex::escape(p))
            .collect::<Vec<_>>()
            .join("|");
        Self {
            records,
            counts,
            names,
            slugs,
            prefixes: kinds,
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

    /// Resolve a reference target (`CONT-003`, `CONT-003-jane-doe`) and its
    /// alias. IDs get renumbered, so a slug or alias that names a different
    /// entity than the ID now holds makes the link stale; when exactly one
    /// other entity of that kind has that slug or name, that's the one meant.
    pub(crate) fn resolve(&self, target: &str, alias: Option<&str>) -> Resolved<'_> {
        let target = target.trim();
        let Some(id) = self.canonical_id(target) else {
            return Resolved::Unknown;
        };
        let suffix = target[id.len()..].trim_start_matches('-').to_lowercase();
        let alias = alias.map(str::trim).filter(|a| !a.is_empty());
        let current = self.names.get_key_value(id);
        let slugs_of = |id: &str| self.slugs.get(id).map_or(&[][..], Vec::as_slice);
        if let Some((key, _)) = current {
            let fits = if suffix.is_empty() {
                alias.is_none_or(|a| alias_fits(a, slugs_of(key)))
            } else {
                slugs_of(key).contains(&suffix)
            };
            if fits {
                return Resolved::Entity(key);
            }
        }
        let kind = current.map(|(_, (k, _))| *k).or_else(|| {
            self.prefixes
                .iter()
                .find(|(p, _)| {
                    id.strip_prefix(p.as_str())
                        .is_some_and(|r| r.starts_with('-'))
                })
                .map(|(_, k)| *k)
        });
        let wanted: Vec<String> = [Some(suffix.clone()), alias.map(slugify)]
            .into_iter()
            .flatten()
            .filter(|w| !w.is_empty())
            .collect();
        let mut meant = self
            .records
            .iter()
            .filter(|r| Some(r.kind) == kind && r.id != id)
            .filter(|r| wanted.iter().any(|w| slugs_of(&r.id).contains(w)))
            .map(|r| r.id.as_str());
        if let (Some(other), None) = (meant.next(), meant.next()) {
            return Resolved::Moved {
                id: other,
                written: id.to_string(),
            };
        }
        match current {
            // A file renamed by hand: the alias still names the entity.
            Some((key, _)) if alias.is_some_and(|a| alias_fits(a, slugs_of(key))) => {
                Resolved::Entity(key)
            }
            Some((key, (_, name))) => Resolved::Stale { id: key, now: name },
            None => Resolved::Unknown,
        }
    }

    /// The text a reference shows: the entity's name when it resolves,
    /// otherwise the alias or target as written.
    pub(crate) fn ref_label(&self, raw: &str) -> String {
        let (target, alias) = split_wikilink(raw);
        match self.resolve(target, alias) {
            Resolved::Entity(id) | Resolved::Moved { id, .. } => {
                self.name(id).unwrap_or(id).to_string()
            }
            Resolved::Stale { .. } | Resolved::Unknown => alias.unwrap_or(target).to_string(),
        }
    }

    /// Render a reference (`CUST-001`, `[[CUST-001|Alias]]`, `[[CONT-003-slug|Name]]`)
    /// as a link showing the entity's name. Unknown targets render as plain
    /// text; stale ones (see [`Catalog::resolve`]) carry a warning marker.
    pub fn ref_html(&self, raw: &str) -> String {
        let (target, alias) = split_wikilink(raw);
        let link = |id: &str| {
            format!(
                r#"<a class="ref" href="{}" title="{}">{}</a>"#,
                entity_href(id),
                escape_html(id),
                escape_html(self.name(id).unwrap_or(id)),
            )
        };
        let text = alias.unwrap_or(target).trim();
        match self.resolve(target, alias) {
            Resolved::Entity(id) => link(id),
            moved @ Resolved::Moved { id, .. } => {
                format!("{}{}", link(id), stale_marker(&self.stale_note(&moved)))
            }
            stale @ Resolved::Stale { .. } => format!(
                r#"<span class="ref-plain ref-stale">{}</span>{}"#,
                escape_html(text),
                stale_marker(&self.stale_note(&stale))
            ),
            Resolved::Unknown if text.is_empty() => String::new(),
            Resolved::Unknown => {
                format!(r#"<span class="ref-plain">{}</span>"#, escape_html(text))
            }
        }
    }

    /// Why a reference is flagged as stale, for its tooltip.
    pub(crate) fn stale_note(&self, resolved: &Resolved) -> String {
        let holder = |id: &str| match self.name(id) {
            Some(name) => format!("{id} is now {name}"),
            None => format!("{id} doesn't exist"),
        };
        match resolved {
            Resolved::Moved { id, written } => format!(
                "Stale link: {}. Showing {} ({id}), the entity it names.",
                holder(written),
                self.name(id).unwrap_or(id)
            ),
            Resolved::Stale { id, .. } => format!("Stale link: {}.", holder(id)),
            Resolved::Entity(_) | Resolved::Unknown => String::new(),
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
        let mut seen: Vec<String> = Vec::new();
        for raw in items {
            let (target, alias) = split_wikilink(raw);
            let key = match self.resolve(target, alias) {
                Resolved::Entity(id) | Resolved::Moved { id, .. } => id.to_string(),
                _ => target.to_string(),
            };
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
            .map(|raw| self.ref_label(raw))
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
            Value::String(s) => {
                // A whole value like `CONT-003-jane-doe` is one reference.
                let s = s.trim();
                if !s.contains(char::is_whitespace) && !s.contains("[[") && s.len() > id.len() {
                    if let Some(found) = self.canonical_id(s) {
                        if s[found.len()..].starts_with('-') {
                            return self.resolve(s, None).id() == Some(id);
                        }
                    }
                }
                self.text_mentions(s, id)
            }
            Value::Sequence(seq) => seq.iter().any(|v| self.value_mentions(v, id)),
            Value::Mapping(map) => map.values().any(|v| self.value_mentions(v, id)),
            Value::Tagged(t) => self.value_mentions(&t.value, id),
            _ => false,
        }
    }

    /// Whether text mentions `id`, as a `[[wikilink]]` that resolves to it
    /// or as a bare ID.
    fn text_mentions(&self, text: &str, id: &str) -> bool {
        if !text.contains("[[") {
            return text.contains(id) && self.id_any_re.find_iter(text).any(|m| m.as_str() == id);
        }
        let mut rest = String::with_capacity(text.len());
        let mut last = 0;
        for caps in WIKILINK_RE.captures_iter(text) {
            let whole = caps.get(0).expect("match");
            let alias = caps.get(2).map(|a| a.as_str());
            if self.resolve(&caps[1], alias).id() == Some(id) {
                return true;
            }
            rest.push_str(&text[last..whole.start()]);
            rest.push(' ');
            last = whole.end();
        }
        rest.push_str(&text[last..]);
        self.id_any_re.find_iter(&rest).any(|m| m.as_str() == id)
    }

    /// Whether `rec` references `id` in its frontmatter (`project`,
    /// `attendees`, ...).
    pub(crate) fn record_links(&self, rec: &EntityRecord, id: &str) -> bool {
        self.value_mentions(&rec.frontmatter, id)
    }

    /// Whether `rec` references `id` in its frontmatter or anywhere in its body.
    pub(crate) fn record_mentions(&self, rec: &EntityRecord, id: &str) -> bool {
        self.record_links(rec, id) || self.text_mentions(&rec.body, id)
    }
}

/// What a reference points at; see [`Catalog::resolve`].
#[derive(Debug, PartialEq)]
pub(crate) enum Resolved<'a> {
    /// The entity the reference names.
    Entity(&'a str),
    /// The written ID now belongs to someone else; `id` is the entity the
    /// slug or alias names.
    Moved { id: &'a str, written: String },
    /// The written ID now belongs to an entity the slug or alias doesn't
    /// name, and no other entity matches.
    Stale { id: &'a str, now: &'a str },
    /// No such entity.
    Unknown,
}

impl<'a> Resolved<'a> {
    /// The entity a reference ends up at, if any.
    pub(crate) fn id(&self) -> Option<&'a str> {
        match self {
            Resolved::Entity(id) | Resolved::Moved { id, .. } => Some(id),
            Resolved::Stale { .. } | Resolved::Unknown => None,
        }
    }
}

/// The warning marker after a stale reference; `note` is its tooltip.
pub(crate) fn stale_marker(note: &str) -> String {
    let note = escape_html(note);
    format!(r#"<span class="ref-warn" role="img" aria-label="{note}" title="{note}"></span>"#)
}

/// Slugs an entity is known by: its name (both slug forms), its `slug`
/// field, and the part after the ID in its file or folder name.
fn record_slugs(r: &EntityRecord) -> Vec<String> {
    let mut slugs = slug_variants(display_name(r));
    if let Some(s) = frontmatter::get_str(&r.frontmatter, "slug") {
        slugs.push(slugify(s));
    }
    let stem = r.source_path.file_stem();
    let dir = r.source_path.parent().and_then(|p| p.file_name());
    for name in [stem, dir].into_iter().flatten() {
        let name = name.to_string_lossy();
        if let Some(rest) = name
            .strip_prefix(r.id.as_str())
            .and_then(|s| s.strip_prefix('-'))
        {
            slugs.push(rest.to_lowercase());
        }
    }
    let mut unique: Vec<String> = Vec::with_capacity(slugs.len());
    for s in slugs {
        if !s.is_empty() && !unique.contains(&s) {
            unique.push(s);
        }
    }
    unique
}

/// Whether an alias plausibly names an entity with these slugs. Aliases
/// are often shortened or inflected (`[[PROJ-001|Innovation Project]]`,
/// `[[PROJ-002|Robots]]` for "Robot Arm"), so one shared word stem of three
/// or more letters is enough, and aliases without such words
/// (`[[PROJ-001|IP]]`) are never doubted.
fn alias_fits(alias: &str, slugs: &[String]) -> bool {
    let alias = slugify(alias);
    let words: Vec<&str> = alias.split('-').filter(|w| w.len() >= 3).collect();
    if words.is_empty() {
        return true;
    }
    let same_stem = |a: &str, b: &str| b.len() >= 3 && (a.starts_with(b) || b.starts_with(a));
    slugs.iter().any(|s| {
        s.contains(alias.as_str())
            || alias.contains(s.as_str())
            || s.split('-').any(|w| words.iter().any(|a| same_stem(a, w)))
    })
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
        let counts = EntityKind::ALL
            .into_iter()
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

    /// Contacts renumbered after links were written: CONT-001 is now Daniel.
    fn renumbered(cfg: &ResolvedConfig) -> Catalog {
        let contact = |id: &str, slug: &str, name: &str| {
            let mut r = rec(EntityKind::Contact, id, &format!("name: {name}"));
            r.source_path = PathBuf::from(format!(
                "/x/customers/CUST-001-acme/contacts/{id}-{slug}.md"
            ));
            r
        };
        catalog(
            vec![
                contact("CONT-001", "daniel-lang", "Daniel Lang"),
                contact("CONT-008", "christian-becker", "Christian Becker"),
                contact("CONT-011", "alexander-david", "Alexander David"),
                rec(
                    EntityKind::Project,
                    "PROJ-001",
                    "name: East Side Fab Innovation Project",
                ),
                rec(EntityKind::Project, "PROJ-002", "name: Robot Arm"),
            ],
            cfg,
        )
    }

    #[test]
    fn stale_slugs_and_aliases_resolve_to_the_entity_they_name() {
        let (_d, cfg) = test_config();
        let cat = renumbered(&cfg);
        // Slug and alias agree with the ID: fine.
        assert_eq!(
            cat.resolve("CONT-001-daniel-lang", Some("Daniel Lang")),
            Resolved::Entity("CONT-001")
        );
        // The slug names someone else: that's who is meant.
        assert_eq!(
            cat.resolve("CONT-001-christian-becker", Some("Christian Becker")),
            Resolved::Moved {
                id: "CONT-008",
                written: "CONT-001".into()
            }
        );
        // No slug, but the alias is another contact's exact name.
        assert_eq!(
            cat.resolve("CONT-011", Some("Christian Becker")).id(),
            Some("CONT-008")
        );
        // Nobody by that name: flagged, not silently shown as Alexander.
        assert_eq!(
            cat.resolve("CONT-011-michael-valentin", Some("M. Valentin")),
            Resolved::Stale {
                id: "CONT-011",
                now: "Alexander David"
            }
        );
        // Shortened, inflected or initial-only aliases are not doubted.
        for (target, alias) in [
            ("PROJ-001", "Innovation Project"),
            ("PROJ-002", "Robots"),
            ("PROJ-001", "IP"),
        ] {
            assert_eq!(
                cat.resolve(target, Some(alias)).id(),
                Some(target),
                "{alias}"
            );
        }
        assert_eq!(cat.resolve("CONT-099-nobody", None), Resolved::Unknown);
    }

    #[test]
    fn stale_refs_render_with_a_warning_and_count_for_the_right_entity() {
        let (_d, cfg) = test_config();
        let cat = renumbered(&cfg);
        let moved = cat.ref_html("[[CONT-001-christian-becker|Christian Becker]]");
        assert!(moved.contains(r#"href="/entity/CONT-008""#), "{moved}");
        assert!(moved.contains(">Christian Becker</a>"));
        assert!(moved.contains(r#"class="ref-warn""#));
        assert!(moved.contains("CONT-001 is now Daniel Lang"));
        let stale = cat.ref_html("[[CONT-011-michael-valentin|M. Valentin]]");
        assert!(stale.contains(r#"<span class="ref-plain ref-stale">M. Valentin</span>"#));
        assert!(stale.contains("CONT-011 is now Alexander David"));
        assert!(!stale.contains("href"));
        assert!(!cat
            .ref_html("[[CONT-001|Daniel Lang]]")
            .contains("ref-warn"));

        let mut meeting = rec(
            EntityKind::Meeting,
            "MTG-004",
            "attendees: ['[[CONT-001-christian-becker|Christian Becker]]', 'CONT-011-michael-valentin']",
        );
        meeting.body = "Met [[CONT-001-christian-becker]] again.".into();
        assert!(cat.record_links(&meeting, "CONT-008"));
        assert!(!cat.record_links(&meeting, "CONT-001"));
        assert!(!cat.record_mentions(&meeting, "CONT-001"));
        assert!(!cat.record_mentions(&meeting, "CONT-011"));
        meeting.body = "Ask CONT-011 too.".into();
        assert!(cat.record_mentions(&meeting, "CONT-011"));
        assert_eq!(
            cat.ref_label("[[CONT-001-christian-becker|C. B.]]"),
            "Christian Becker"
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
