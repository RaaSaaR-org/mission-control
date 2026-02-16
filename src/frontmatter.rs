use crate::error::{McError, McResult};
use serde_yaml::Value;
use std::path::Path;

/// Split a markdown string into optional frontmatter (without delimiters) and body.
pub fn split_frontmatter(content: &str) -> Option<(String, String)> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }

    // Skip the opening ---
    let after_open = &trimmed[3..];
    let after_open = after_open.strip_prefix('\n').unwrap_or(after_open);

    if let Some(end_idx) = after_open.find("\n---") {
        let fm = after_open[..end_idx].to_string();
        let body = after_open[end_idx + 4..].to_string();
        Some((fm, body))
    } else {
        None
    }
}

/// Parse raw YAML frontmatter string into a serde_yaml::Value (should be a Mapping).
pub fn parse_raw(fm_str: &str, source: &Path) -> McResult<Value> {
    let val: Value = serde_yaml::from_str(fm_str).map_err(|e| McError::Frontmatter {
        path: source.to_path_buf(),
        message: e.to_string(),
    })?;
    Ok(val)
}

/// Parse frontmatter from a file, returning (Value, body).
pub fn parse_file(path: &Path) -> McResult<(Value, String)> {
    let content = std::fs::read_to_string(path)?;
    match split_frontmatter(&content) {
        Some((fm_str, body)) => {
            let val: Value = serde_yaml::from_str(&fm_str).map_err(|e| McError::Frontmatter {
                path: path.to_path_buf(),
                message: e.to_string(),
            })?;
            Ok((val, body))
        }
        None => Err(McError::Frontmatter {
            path: path.to_path_buf(),
            message: "No YAML frontmatter found".into(),
        }),
    }
}

/// Serialize a YAML Value back into a complete markdown file with frontmatter.
pub fn serialize_document(frontmatter: &Value, body: &str) -> String {
    let yaml = serde_yaml::to_string(frontmatter)
        .expect("serializing a serde_yaml::Value to YAML should never fail");
    let yaml = yaml.trim_end();

    // Replace single-quoted wiki-links with double-quoted for Obsidian compatibility.
    // serde_yaml uses single quotes for strings containing `[`/`]`, but Obsidian
    // only recognises wiki-links inside double quotes in frontmatter.
    let re_quote = regex::Regex::new(r"'(\[\[.+?\]\])'").unwrap();
    let yaml = re_quote.replace_all(yaml, "\"$1\"");

    // Extract all [[...]] links from the frontmatter so we can mirror them in the
    // document body.  Obsidian's graph view reliably picks up links from body text
    // but not always from frontmatter properties.
    let re_links = regex::Regex::new(r"\[\[(.+?)\]\]").unwrap();
    let links: Vec<String> = re_links
        .captures_iter(&yaml)
        .map(|c| format!("[[{}]]", &c[1]))
        .collect();

    // Strip any existing mc-links comment so repeated serialisation is idempotent.
    let re_mc = regex::Regex::new(r"\n?%% mc-links:.*%%\n?").unwrap();
    let body = re_mc.replace_all(body, "");

    // Append an Obsidian comment listing all frontmatter links.
    let body = if links.is_empty() {
        body.to_string()
    } else {
        let link_str = links.join(" ");
        format!("{}\n%% mc-links: {} %%\n", body.trim_end(), link_str)
    };

    format!("---\n{}\n---\n{}", yaml, body)
}

/// Get a string field from a YAML Mapping Value.
pub fn get_str<'a>(val: &'a Value, key: &str) -> Option<&'a str> {
    val.as_mapping()
        .and_then(|m| m.get(Value::String(key.to_string())))
        .and_then(|v| v.as_str())
}

/// Get a string field or empty string.
pub fn get_str_or<'a>(val: &'a Value, key: &str, default: &'a str) -> &'a str {
    get_str(val, key).unwrap_or(default)
}

/// Get a sequence of strings from a YAML value.
pub fn get_string_list(val: &Value, key: &str) -> Vec<String> {
    val.as_mapping()
        .and_then(|m| m.get(Value::String(key.to_string())))
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// Strip `[[...]]` wiki-link brackets from a string.
/// Handles `[[target|alias]]` by returning just the target.
/// Returns the input unchanged if no brackets are found (backwards compat).
pub fn strip_wikilink(s: &str) -> &str {
    if let Some(inner) = s.strip_prefix("[[").and_then(|s| s.strip_suffix("]]")) {
        // Handle [[target|alias]] -- return target
        inner.split('|').next().unwrap_or(inner)
    } else {
        s
    }
}

/// Wrap a non-empty string in `[[...]]` wiki-link brackets.
pub fn wrap_wikilink(s: &str) -> String {
    if s.is_empty() {
        String::new()
    } else {
        format!("[[{}]]", s)
    }
}

/// Get a string field, stripping any wiki-link brackets.
pub fn get_link_str<'a>(val: &'a Value, key: &str) -> Option<&'a str> {
    get_str(val, key).map(strip_wikilink)
}

/// Get a sequence of strings, stripping wiki-link brackets from each.
pub fn get_link_list(val: &Value, key: &str) -> Vec<String> {
    get_string_list(val, key)
        .into_iter()
        .map(|s| strip_wikilink(&s).to_string())
        .collect()
}

/// Set a string field on a YAML Mapping Value.
pub fn set_str(val: &mut Value, key: &str, value: &str) {
    if let Some(map) = val.as_mapping_mut() {
        map.insert(
            Value::String(key.to_string()),
            Value::String(value.to_string()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_frontmatter_basic() {
        let content = "---\nid: CUST-001\nname: Acme\n---\n# Acme\n\nBody text.";
        let (fm, body) = split_frontmatter(content).unwrap();
        assert!(fm.contains("id: CUST-001"));
        assert!(fm.contains("name: Acme"));
        assert!(body.contains("Body text."));
    }

    #[test]
    fn test_split_frontmatter_no_frontmatter() {
        let content = "# Just a heading\n\nSome body.";
        assert!(split_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_raw_and_accessors() {
        let fm_str = "id: TASK-001\ntitle: Fix bug\nstatus: todo\ntags:\n  - urgent\n  - backend";
        let fm = parse_raw(fm_str, std::path::Path::new("test.md")).unwrap();

        assert_eq!(get_str(&fm, "id").unwrap(), "TASK-001");
        assert_eq!(get_str(&fm, "title").unwrap(), "Fix bug");
        assert_eq!(get_str(&fm, "status").unwrap(), "todo");
        assert_eq!(get_str(&fm, "nonexistent"), None);

        let tags = get_string_list(&fm, "tags");
        assert_eq!(tags, vec!["urgent", "backend"]);
    }

    #[test]
    fn test_set_str_modifies_value() {
        let fm_str = "id: TASK-001\nstatus: todo";
        let mut fm = parse_raw(fm_str, std::path::Path::new("test.md")).unwrap();

        set_str(&mut fm, "status", "done");
        assert_eq!(get_str(&fm, "status").unwrap(), "done");

        // Setting a new key
        set_str(&mut fm, "owner", "alice");
        assert_eq!(get_str(&fm, "owner").unwrap(), "alice");
    }

    #[test]
    fn test_frontmatter_round_trip() {
        let fm_str =
            "id: RES-001\ntitle: LLM Benchmarks\nstatus: draft\ntags:\n  - ai\n  - research";
        let fm = parse_raw(fm_str, std::path::Path::new("test.md")).unwrap();
        let body = "\n# LLM Benchmarks\n\nResearch body.\n";

        let doc = serialize_document(&fm, body);

        // Re-parse the serialized document
        let (fm_str2, body2) = split_frontmatter(&doc).unwrap();
        let fm2 = parse_raw(&fm_str2, std::path::Path::new("test.md")).unwrap();

        assert_eq!(get_str(&fm2, "id").unwrap(), "RES-001");
        assert_eq!(get_str(&fm2, "title").unwrap(), "LLM Benchmarks");
        assert_eq!(get_str(&fm2, "status").unwrap(), "draft");
        assert_eq!(get_string_list(&fm2, "tags"), vec!["ai", "research"]);
        assert!(body2.contains("Research body."));
    }

    #[test]
    fn test_strip_wikilink() {
        assert_eq!(strip_wikilink("[[CUST-001]]"), "CUST-001");
        assert_eq!(strip_wikilink("[[target|alias]]"), "target");
        assert_eq!(strip_wikilink("CUST-001"), "CUST-001");
        assert_eq!(strip_wikilink(""), "");
        assert_eq!(strip_wikilink("[[]]"), "");
        assert_eq!(
            strip_wikilink("[[nested[[brackets]]]]"),
            "nested[[brackets]]"
        );
    }

    #[test]
    fn test_wrap_wikilink() {
        assert_eq!(wrap_wikilink("CUST-001"), "[[CUST-001]]");
        assert_eq!(wrap_wikilink(""), "");
    }

    #[test]
    fn test_get_link_str() {
        let fm_str = "sprint: '[[SPR-001]]'\ncustomer: CUST-001";
        let fm = parse_raw(fm_str, std::path::Path::new("test.md")).unwrap();
        assert_eq!(get_link_str(&fm, "sprint"), Some("SPR-001"));
        assert_eq!(get_link_str(&fm, "customer"), Some("CUST-001"));
        assert_eq!(get_link_str(&fm, "missing"), None);
    }

    #[test]
    fn test_get_link_list() {
        let fm_str = "projects:\n  - '[[PROJ-001]]'\n  - PROJ-002";
        let fm = parse_raw(fm_str, std::path::Path::new("test.md")).unwrap();
        assert_eq!(get_link_list(&fm, "projects"), vec!["PROJ-001", "PROJ-002"]);
    }

    #[test]
    fn test_serialize_document_format() {
        let fm_str = "id: TEST-001\nname: Test";
        let fm = parse_raw(fm_str, std::path::Path::new("test.md")).unwrap();
        let body = "\n# Test\n";

        let doc = serialize_document(&fm, body);
        assert!(doc.starts_with("---\n"));
        assert!(doc.contains("\n---\n"));
        assert!(doc.contains("# Test"));
    }

    #[test]
    fn test_serialize_double_quotes_wikilinks() {
        let fm_str = "id: PROJ-001\ncustomer: '[[CUST-001]]'";
        let fm = parse_raw(fm_str, std::path::Path::new("test.md")).unwrap();
        let doc = serialize_document(&fm, "\n");

        // Must be double-quoted, not single-quoted
        assert!(
            doc.contains("\"[[CUST-001]]\""),
            "expected double-quoted wiki-link, got:\n{doc}"
        );
        assert!(
            !doc.contains("'[[CUST-001]]'"),
            "single-quoted wiki-link should not appear"
        );
    }

    #[test]
    fn test_serialize_mc_links_comment() {
        let fm_str = "id: PROJ-001\ncustomer: '[[CUST-001]]'";
        let fm = parse_raw(fm_str, std::path::Path::new("test.md")).unwrap();
        let doc = serialize_document(&fm, "\n# Project\n");

        assert!(
            doc.contains("%% mc-links: [[CUST-001]] %%"),
            "expected mc-links comment, got:\n{doc}"
        );
    }

    #[test]
    fn test_serialize_mc_links_multiple() {
        let fm_str = "id: MTG-001\ncustomers:\n  - '[[CUST-001]]'\nprojects:\n  - '[[PROJ-001]]'";
        let fm = parse_raw(fm_str, std::path::Path::new("test.md")).unwrap();
        let doc = serialize_document(&fm, "\n");

        assert!(
            doc.contains("[[CUST-001]]") && doc.contains("[[PROJ-001]]"),
            "expected both links in mc-links comment, got:\n{doc}"
        );
        // The comment should contain both
        let mc_line = doc.lines().find(|l| l.contains("%% mc-links:")).unwrap();
        assert!(mc_line.contains("[[CUST-001]]"));
        assert!(mc_line.contains("[[PROJ-001]]"));
    }

    #[test]
    fn test_serialize_mc_links_idempotent() {
        let fm_str = "id: PROJ-001\ncustomer: '[[CUST-001]]'";
        let fm = parse_raw(fm_str, std::path::Path::new("test.md")).unwrap();

        // Serialize once
        let doc1 = serialize_document(&fm, "\n# Project\n");
        // Extract body from first serialisation and re-serialize
        let (_, body1) = split_frontmatter(&doc1).unwrap();
        let doc2 = serialize_document(&fm, &body1);

        // Count occurrences of mc-links -- should be exactly one
        let count = doc2.matches("%% mc-links:").count();
        assert_eq!(count, 1, "mc-links duplicated after re-serialise:\n{doc2}");
    }

    #[test]
    fn test_serialize_no_links_no_comment() {
        let fm_str = "id: RES-001\ntitle: Plain research";
        let fm = parse_raw(fm_str, std::path::Path::new("test.md")).unwrap();
        let doc = serialize_document(&fm, "\n# Research\n");

        assert!(
            !doc.contains("%% mc-links:"),
            "no mc-links comment expected when no wiki-links:\n{doc}"
        );
    }
}
