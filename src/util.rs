use crate::error::McResult;
use regex::Regex;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

static SLUG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^a-z0-9]+").expect("static regex pattern is always valid"));

/// Convert a name to a URL-friendly slug.
pub fn slugify(name: &str) -> String {
    let lower = name.to_lowercase();
    let slug = SLUG_RE.replace_all(&lower, "-");
    let slug = slug.trim_matches('-');
    // Truncate to 80 chars to prevent overly long directory names
    if slug.len() > 80 {
        slug[..80].trim_end_matches('-').to_string()
    } else {
        slug.to_string()
    }
}

/// Today's date as YYYY-MM-DD.
pub fn today_str() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Parse a comma-separated string into a Vec of trimmed strings.
pub fn parse_comma_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Write data to a temporary file then rename for atomicity.
pub fn atomic_write(path: &Path, data: &[u8]) -> McResult<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Acme Inc."), "acme-inc");
        assert_eq!(slugify("Data Pipeline"), "data-pipeline");
        assert_eq!(slugify("  Hello   World  "), "hello-world");
        assert_eq!(slugify("LLM Benchmarks"), "llm-benchmarks");
    }

    #[test]
    fn test_slugify_truncation() {
        // A very long name should be truncated to at most 80 characters
        let long_name = "a ".repeat(100); // 200 chars
        let slug = slugify(&long_name);
        assert!(slug.len() <= 80);
        assert!(!slug.ends_with('-'));
    }

    #[test]
    fn test_parse_comma_list() {
        assert_eq!(parse_comma_list("a, b, c"), vec!["a", "b", "c"]);
        assert_eq!(parse_comma_list(""), Vec::<String>::new());
    }
}
