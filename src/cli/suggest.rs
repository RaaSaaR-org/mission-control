//! Forgiving input resolution: "did you mean" suggestions, status normalization
//! and loose entity-ID parsing (`task-7`, `7`, `task7` → `TASK-007`).

use crate::config::ResolvedConfig;
use crate::data::{self, EntityRecord};
use crate::entity::EntityKind;
use crate::error::{McError, McResult};
use crate::frontmatter;

const ALL_KINDS: [EntityKind; 8] = [
    EntityKind::Customer,
    EntityKind::Contact,
    EntityKind::Project,
    EntityKind::Meeting,
    EntityKind::Research,
    EntityKind::Task,
    EntityKind::Sprint,
    EntityKind::Proposal,
];

/// Edit distance counting insertions, deletions, substitutions and adjacent
/// transpositions (optimal string alignment), so `doen` is one step from `done`.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

/// Lowercase and strip everything but letters and digits.
fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Best candidate for a mistyped `input`, if any is reasonably close.
pub fn did_you_mean<'a, I>(input: &str, candidates: I) -> Option<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    let needle = squash(input);
    if needle.is_empty() {
        return None;
    }
    let len = needle.chars().count();
    let threshold = if len < 4 { 1 } else { (len / 3).max(2) };
    let mut best: Option<(usize, &'a str)> = None;
    for cand in candidates {
        let hay = squash(cand);
        let substring = needle.len() >= 3 && hay.contains(&needle);
        let score = if hay.starts_with(&needle) || substring {
            0
        } else {
            let d = levenshtein(&needle, &hay);
            if d > threshold {
                continue;
            }
            d
        };
        if best.is_none_or(|(s, _)| score < s) {
            best = Some((score, cand));
        }
    }
    best.map(|(_, c)| c)
}

/// Words people commonly type for task-ish statuses.
const STATUS_ALIASES: &[(&str, &str)] = &[
    ("wip", "in-progress"),
    ("doing", "in-progress"),
    ("started", "in-progress"),
    ("progress", "in-progress"),
    ("complete", "done"),
    ("completed", "done"),
    ("finished", "done"),
    ("closed", "done"),
    ("open", "todo"),
    ("canceled", "cancelled"),
    ("inreview", "review"),
];

/// Map user input onto one of `valid` statuses, tolerating case, `_`/space
/// instead of `-`, and a few common aliases. Returns `None` if nothing matches.
pub fn match_status<'a>(input: &str, valid: &'a [String]) -> Option<&'a str> {
    let key = squash(input);
    if let Some(v) = valid.iter().find(|v| squash(v) == key) {
        return Some(v.as_str());
    }
    STATUS_ALIASES
        .iter()
        .filter(|(alias, _)| *alias == key)
        .find_map(|(_, target)| valid.iter().find(|v| v == target))
        .map(|s| s.as_str())
}

/// Resolve a status for `kind`, or fail with a "did you mean" usage error.
pub fn resolve_status(input: &str, valid: &[String], kind: EntityKind) -> McResult<String> {
    if let Some(s) = match_status(input, valid) {
        return Ok(s.to_string());
    }
    Err(McError::usage(
        format!("'{input}' is not a valid {} status", kind.label()),
        Some(status_hint(input, valid)),
    ))
}

/// Hint text listing valid statuses with a suggestion first when possible.
pub fn status_hint(input: &str, valid: &[String]) -> String {
    let list = valid.join(", ");
    match did_you_mean(input, valid.iter().map(String::as_str)) {
        Some(s) => format!("did you mean '{s}'? Valid: {list}"),
        None => format!("valid statuses: {list}"),
    }
}

/// Parse loose ID input into a canonical `PREFIX-NNN` and its kind.
///
/// Accepts any case, a missing dash (`task7`), unpadded numbers (`TASK-7`)
/// and, when `default_kind` is given, a bare number (`7`).
pub fn normalize_id(
    input: &str,
    cfg: &ResolvedConfig,
    default_kind: Option<EntityKind>,
) -> McResult<(String, EntityKind)> {
    let raw = input.trim();

    // Configured prefixes may contain digits (e.g. `P2P`), so match them
    // literally first, longest prefix wins.
    let literal = ALL_KINDS
        .into_iter()
        .filter_map(|k| {
            let p = k.prefix(cfg);
            let head = raw.get(..p.len())?;
            let rest = raw[p.len()..].trim_start_matches(['-', '_', ' ']);
            (!p.is_empty()
                && head.eq_ignore_ascii_case(p)
                && !rest.is_empty()
                && rest.chars().all(|c| c.is_ascii_digit()))
            .then_some((k, p.len(), rest))
        })
        .max_by_key(|(_, len, _)| *len);
    if let Some((kind, _, digits)) = literal {
        let digits = digits.trim_start_matches('0');
        return Ok((format!("{}-{:0>3}", kind.prefix(cfg), digits), kind));
    }

    let split = raw
        .char_indices()
        .find(|(_, c)| c.is_ascii_digit())
        .map(|(i, _)| i)
        .unwrap_or(raw.len());
    let (head, digits) = raw.split_at(split);
    let prefix = head.trim_end_matches(['-', '_', ' ']);
    let number_ok = !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit());

    let kind = if prefix.is_empty() {
        default_kind
    } else {
        ALL_KINDS
            .into_iter()
            .find(|k| k.prefix(cfg).eq_ignore_ascii_case(prefix))
    };

    match kind {
        Some(kind) if number_ok => {
            let digits = digits.trim_start_matches('0');
            let id = format!("{}-{:0>3}", kind.prefix(cfg), digits);
            Ok((id, kind))
        }
        Some(kind) if prefix.is_empty() || digits.is_empty() => Err(McError::usage(
            format!("'{raw}' is not a valid ID"),
            Some(format!(
                "{} IDs look like {}-001",
                kind.label(),
                kind.prefix(cfg)
            )),
        )),
        _ => {
            let prefixes: Vec<&str> = ALL_KINDS
                .iter()
                .filter(|k| cfg.entity_available(k))
                .map(|k| k.prefix(cfg))
                .collect();
            let hint = match did_you_mean(prefix, prefixes.iter().copied()) {
                Some(p) if !prefix.is_empty() => format!(
                    "did you mean {p}-{:0>3}? Known prefixes: {}",
                    if number_ok { digits } else { "001" },
                    prefixes.join(", ")
                ),
                _ => format!(
                    "IDs look like {}-001; known prefixes: {}",
                    default_kind.unwrap_or(EntityKind::Task).prefix(cfg),
                    prefixes.join(", ")
                ),
            };
            Err(McError::usage(
                format!("'{raw}' is not a valid ID"),
                Some(hint),
            ))
        }
    }
}

/// Find an entity from loose input, with suggestions when it does not exist.
pub fn find_entity(
    input: &str,
    cfg: &ResolvedConfig,
    default_kind: Option<EntityKind>,
) -> McResult<EntityRecord> {
    let (id, kind) = normalize_id(input, cfg, default_kind)?;
    if !cfg.entity_available(&kind) {
        return Err(McError::not_available(kind, cfg));
    }
    match data::find_entity_by_id(&id, cfg) {
        Err(McError::EntityNotFound(_)) => {
            // Hand-written IDs may use other padding (TASK-0001); try verbatim.
            let raw = input.trim();
            if raw != id {
                if let Ok(e) = data::find_entity_by_id(raw, cfg) {
                    return Ok(e);
                }
            }
            Err(not_found(&id, kind, cfg))
        }
        other => other,
    }
}

fn not_found(id: &str, kind: EntityKind, cfg: &ResolvedConfig) -> McError {
    let existing = data::collect_entities(kind, cfg).unwrap_or_default();
    let message = format!("{} {id} not found", kind.label());
    if existing.is_empty() {
        return McError::not_found(
            message,
            Some(format!(
                "there are no {} yet; create one with `mc new {} \"...\"`",
                kind.label_plural(),
                kind.label()
            )),
        );
    }
    let close = existing
        .iter()
        .map(|e| (levenshtein(id, &e.id), e))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, e)| (*d, e.id.clone()))
        .map(|(_, e)| e);
    let browse = format!(
        "run `mc list {}` to browse all {}",
        kind.label_plural(),
        existing.len()
    );
    let hint = match close {
        Some(e) => {
            let name = frontmatter::get_str(&e.frontmatter, "title")
                .or_else(|| frontmatter::get_str(&e.frontmatter, "name"))
                .unwrap_or("");
            format!("did you mean {} ({name})? Or {browse}", e.id)
        }
        None => browse,
    };
    McError::not_found(message, Some(hint))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::init;
    use crate::config;

    fn statuses() -> Vec<String> {
        [
            "backlog",
            "todo",
            "in-progress",
            "review",
            "done",
            "cancelled",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    #[test]
    fn levenshtein_basics() {
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("same", "same"), 0);
        assert_eq!(levenshtein("doen", "done"), 1);
    }

    #[test]
    fn did_you_mean_finds_close_values() {
        let s = statuses();
        let c = s.iter().map(String::as_str);
        assert_eq!(did_you_mean("progres", c.clone()), Some("in-progress"));
        assert_eq!(did_you_mean("revew", c.clone()), Some("review"));
        assert_eq!(did_you_mean("dne", c.clone()), Some("done"));
        assert_eq!(did_you_mean("doen", c.clone()), Some("done"));
        assert_eq!(did_you_mean("zzzzzz", c), None);
    }

    #[test]
    fn match_status_is_forgiving() {
        let s = statuses();
        assert_eq!(match_status("In_Progress", &s), Some("in-progress"));
        assert_eq!(match_status("inprogress", &s), Some("in-progress"));
        assert_eq!(match_status("WIP", &s), Some("in-progress"));
        assert_eq!(match_status("canceled", &s), Some("cancelled"));
        assert_eq!(match_status("nope", &s), None);
    }

    #[test]
    fn resolve_status_suggests() {
        let err = resolve_status("revw", &statuses(), EntityKind::Task).unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(err.hint().unwrap().contains("did you mean 'review'"));
    }

    #[test]
    fn normalize_id_variants() {
        let tmp = tempfile::TempDir::new().unwrap();
        init::run(tmp.path(), false, false, Some("T"), false, true).unwrap();
        let cfg = config::load_config(tmp.path(), config::RepoMode::Standalone).unwrap();

        let n = |s: &str, d| normalize_id(s, &cfg, d).map(|(id, _)| id);
        assert_eq!(n("TASK-001", None).unwrap(), "TASK-001");
        assert_eq!(n("task-7", None).unwrap(), "TASK-007");
        assert_eq!(n("task7", None).unwrap(), "TASK-007");
        assert_eq!(n("TASK-0007", None).unwrap(), "TASK-007");
        assert_eq!(n("proj-1234", None).unwrap(), "PROJ-1234");
        assert_eq!(n("7", Some(EntityKind::Task)).unwrap(), "TASK-007");
        assert!(n("7", None).is_err());

        let err = n("TSK-1", None).unwrap_err();
        assert!(err.hint().unwrap().contains("did you mean TASK-001"));
    }

    #[test]
    fn normalize_id_handles_prefixes_with_digits() {
        let tmp = tempfile::TempDir::new().unwrap();
        init::run(tmp.path(), false, false, Some("T"), false, true).unwrap();
        let mut cfg = config::load_config(tmp.path(), config::RepoMode::Standalone).unwrap();
        cfg.id_prefixes.task = "T2".into();

        let n = |s: &str| normalize_id(s, &cfg, None);
        assert_eq!(n("T2-001").unwrap(), ("T2-001".into(), EntityKind::Task));
        assert_eq!(n("t2-7").unwrap(), ("T2-007".into(), EntityKind::Task));
    }

    #[test]
    fn find_entity_falls_back_to_verbatim_id() {
        let tmp = tempfile::TempDir::new().unwrap();
        init::run(tmp.path(), false, false, Some("T"), false, true).unwrap();
        let cfg = config::load_config(tmp.path(), config::RepoMode::Standalone).unwrap();
        let dir = cfg.tasks_dir.join("todo");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("TASK-0042-odd.md"),
            "---\nid: TASK-0042\ntitle: Odd padding\nstatus: todo\n---\n",
        )
        .unwrap();

        let e = find_entity("TASK-0042", &cfg, None).unwrap();
        assert_eq!(e.id, "TASK-0042");
        assert!(find_entity("TASK-43", &cfg, None).is_err());
    }
}
