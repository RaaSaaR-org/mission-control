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
    // serde_yaml adds a trailing newline, remove it for cleanliness
    let yaml = yaml.trim_end();
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
    fn test_serialize_document_format() {
        let fm_str = "id: TEST-001\nname: Test";
        let fm = parse_raw(fm_str, std::path::Path::new("test.md")).unwrap();
        let body = "\n# Test\n";

        let doc = serialize_document(&fm, body);
        assert!(doc.starts_with("---\n"));
        assert!(doc.contains("\n---\n"));
        assert!(doc.contains("# Test"));
    }
}
