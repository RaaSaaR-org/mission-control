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
pub fn parse_raw(fm_str: &str) -> McResult<Value> {
    let val: Value =
        serde_yaml::from_str(fm_str).map_err(|e| McError::Frontmatter {
            path: "".into(),
            message: e.to_string(),
        })?;
    Ok(val)
}

/// Parse frontmatter from a file, returning (Value, body).
pub fn parse_file(path: &Path) -> McResult<(Value, String)> {
    let content = std::fs::read_to_string(path)?;
    match split_frontmatter(&content) {
        Some((fm_str, body)) => {
            let val: Value =
                serde_yaml::from_str(&fm_str).map_err(|e| McError::Frontmatter {
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
    let yaml = serde_yaml::to_string(frontmatter).unwrap_or_default();
    // serde_yaml adds a trailing newline, remove it for cleanliness
    let yaml = yaml.trim_end();
    format!("---\n{}\n---\n{}", yaml, body)
}

/// Get a string field from a YAML Mapping Value.
pub fn get_str<'a>(val: &'a Value, key: &str) -> Option<&'a str> {
    val.as_mapping()
        .and_then(|m| m.get(&Value::String(key.to_string())))
        .and_then(|v| v.as_str())
}

/// Get a string field or empty string.
pub fn get_str_or<'a>(val: &'a Value, key: &str, default: &'a str) -> &'a str {
    get_str(val, key).unwrap_or(default)
}

/// Get a sequence of strings from a YAML value.
pub fn get_string_list(val: &Value, key: &str) -> Vec<String> {
    val.as_mapping()
        .and_then(|m| m.get(&Value::String(key.to_string())))
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

/// Set a sequence of strings on a YAML Mapping Value.
pub fn set_string_list(val: &mut Value, key: &str, values: &[String]) {
    if let Some(map) = val.as_mapping_mut() {
        let seq: Vec<Value> = values.iter().map(|s| Value::String(s.clone())).collect();
        map.insert(Value::String(key.to_string()), Value::Sequence(seq));
    }
}
