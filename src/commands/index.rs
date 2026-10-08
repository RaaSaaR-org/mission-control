use crate::cli::ui;
use crate::config::ResolvedConfig;
use crate::data::{self, EntityRecord};
use crate::entity::EntityKind;
use crate::error::McResult;
use crate::frontmatter;
use crate::util;
use colored::*;
use serde_json::Value as JsonValue;

/// Fields that may contain wiki-link brackets and should be stripped for JSON output.
const WIKILINK_FIELDS: &[&str] = &[
    "customers",
    "projects",
    "depends_on",
    "sprint",
    "milestone",
    "supersedes",
    "superseded_by",
    "customer",
];

/// Strip wiki-link brackets from known cross-reference fields in a JSON object.
fn strip_wikilinks_in_json(val: &mut JsonValue) {
    if let Some(obj) = val.as_object_mut() {
        for &field in WIKILINK_FIELDS {
            if let Some(v) = obj.get_mut(field) {
                match v {
                    JsonValue::String(s) => {
                        *s = frontmatter::strip_wikilink(s).to_string();
                    }
                    JsonValue::Array(arr) => {
                        for item in arr.iter_mut() {
                            if let JsonValue::String(s) = item {
                                *s = frontmatter::strip_wikilink(s).to_string();
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

pub fn run(cfg: &ResolvedConfig) -> McResult<()> {
    let result = run_quiet(cfg)?;
    let available: Vec<(EntityKind, usize)> = DISPLAY_ORDER
        .into_iter()
        .filter(|k| cfg.entity_available(k))
        .map(|k| (k, result.count(k)))
        .collect();

    if ui::get().json {
        let mut obj = serde_json::Map::new();
        for (k, n) in &available {
            obj.insert(k.label_plural().to_string(), JsonValue::from(*n));
        }
        let out = serde_json::json!({
            "path": rel_path(&cfg.data_dir, cfg),
            "counts": obj,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    let total: usize = available.iter().map(|(_, n)| n).sum();
    ui::success(format!(
        "Indexed {} into {}",
        ui::count(total, "entity", "entities").bold(),
        rel_path(&cfg.data_dir, cfg).cyan()
    ));
    let sep = format!(" {} ", ui::glyphs().sep);
    let parts: Vec<String> = available
        .iter()
        .map(|(k, n)| ui::count(*n, k.label(), k.label_plural()))
        .collect();
    println!("  {}", parts.join(&sep.dimmed().to_string()));
    Ok(())
}

/// Order of the per-kind counts in `mc index` output.
const DISPLAY_ORDER: [EntityKind; 9] = [
    EntityKind::Customer,
    EntityKind::Contact,
    EntityKind::Project,
    EntityKind::Meeting,
    EntityKind::Research,
    EntityKind::Task,
    EntityKind::Sprint,
    EntityKind::Milestone,
    EntityKind::Proposal,
];

fn rel_path(path: &std::path::Path, cfg: &ResolvedConfig) -> String {
    path.strip_prefix(&cfg.root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// JSON form of an entity: frontmatter with wiki-links stripped plus `_source`
/// (path relative to the repo root). Shared by `mc index` and `--json` output.
pub fn entity_json(entity: &EntityRecord, cfg: &ResolvedConfig) -> JsonValue {
    let mut json_val = data::yaml_to_json(&entity.frontmatter);
    strip_wikilinks_in_json(&mut json_val);
    if let Some(obj) = json_val.as_object_mut() {
        obj.insert(
            "_source".into(),
            JsonValue::String(rel_path(&entity.source_path, cfg)),
        );
    }
    json_val
}

/// Entities indexed per kind; kinds the repo doesn't enable count 0.
pub struct IndexResult {
    pub customers: usize,
    pub projects: usize,
    pub meetings: usize,
    pub research: usize,
    pub tasks: usize,
    pub sprints: usize,
    pub milestones: usize,
    pub proposals: usize,
    pub contacts: usize,
}

impl IndexResult {
    pub fn count(&self, kind: EntityKind) -> usize {
        match kind {
            EntityKind::Customer => self.customers,
            EntityKind::Project => self.projects,
            EntityKind::Meeting => self.meetings,
            EntityKind::Research => self.research,
            EntityKind::Task => self.tasks,
            EntityKind::Sprint => self.sprints,
            EntityKind::Milestone => self.milestones,
            EntityKind::Proposal => self.proposals,
            EntityKind::Contact => self.contacts,
        }
    }
}

/// Build indexes without printing to stdout: `index.json` with every
/// enabled kind, plus `<kind>.json` per enabled kind (e.g. `meetings.json`).
pub fn run_quiet(cfg: &ResolvedConfig) -> McResult<IndexResult> {
    let mut index = serde_json::Map::new();
    for kind in EntityKind::ALL {
        if cfg.entity_available(&kind) {
            index.insert(
                kind.label_plural().to_string(),
                JsonValue::Array(collect_json(kind, cfg)?),
            );
        }
    }

    std::fs::create_dir_all(&cfg.data_dir)?;

    let write = |name: &str, value: &JsonValue| -> McResult<()> {
        let data = serde_json::to_string_pretty(value)? + "\n";
        util::atomic_write(&cfg.data_dir.join(name), data.as_bytes())
    };
    for (key, entries) in &index {
        write(&format!("{key}.json"), entries)?;
    }
    let count = |kind: EntityKind| {
        index
            .get(kind.label_plural())
            .and_then(|v| v.as_array())
            .map_or(0, |a| a.len())
    };
    let result = IndexResult {
        customers: count(EntityKind::Customer),
        projects: count(EntityKind::Project),
        meetings: count(EntityKind::Meeting),
        research: count(EntityKind::Research),
        tasks: count(EntityKind::Task),
        sprints: count(EntityKind::Sprint),
        milestones: count(EntityKind::Milestone),
        proposals: count(EntityKind::Proposal),
        contacts: count(EntityKind::Contact),
    };
    write("index.json", &JsonValue::Object(index))?;
    Ok(result)
}

fn collect_json(kind: EntityKind, cfg: &ResolvedConfig) -> McResult<Vec<JsonValue>> {
    let entities = data::collect_entities(kind, cfg)?;
    let mut json_entries: Vec<JsonValue> = entities.iter().map(|e| entity_json(e, cfg)).collect();

    // Sort by ID
    json_entries.sort_by(|a, b| {
        let aid = a.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let bid = b.get("id").and_then(|v| v.as_str()).unwrap_or("");
        data::id_sort_key(aid).cmp(&data::id_sort_key(bid))
    });

    Ok(json_entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{init, new};
    use crate::config::{self, RepoMode};

    #[test]
    fn writes_a_file_per_enabled_kind_including_meetings() {
        let tmp = tempfile::TempDir::new().unwrap();
        init::run(tmp.path(), false, true, Some("E"), false, true).unwrap();
        let cfg = config::load_config(tmp.path(), RepoMode::Embedded).unwrap();
        new::create_meeting(&cfg, &new::MeetingInput::new("Kickoff")).unwrap();

        let result = run_quiet(&cfg).unwrap();
        assert_eq!(result.meetings, 1);
        let meetings: Vec<JsonValue> = serde_json::from_str(
            &std::fs::read_to_string(cfg.data_dir.join("meetings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(meetings[0]["id"], "MTG-001");

        // Embedded repos have no customers, projects or contacts.
        for name in ["customers.json", "projects.json", "contacts.json"] {
            assert!(!cfg.data_dir.join(name).exists(), "{name} written");
        }
        let index: JsonValue = serde_json::from_str(
            &std::fs::read_to_string(cfg.data_dir.join("index.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(index["meetings"][0]["id"], "MTG-001");
        assert!(index.get("customers").is_none());
        assert!(index["tasks"].as_array().unwrap().is_empty());
    }
}
