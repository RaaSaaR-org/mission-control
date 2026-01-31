use crate::error::McResult;
use regex::Regex;
use std::fs;
use std::path::Path;

/// Convert a name to a URL-friendly slug.
pub fn slugify(name: &str) -> String {
    let re = Regex::new(r"[^a-z0-9]+").unwrap();
    let lower = name.to_lowercase();
    let slug = re.replace_all(&lower, "-");
    slug.trim_matches('-').to_string()
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
    fn test_parse_comma_list() {
        assert_eq!(
            parse_comma_list("a, b, c"),
            vec!["a", "b", "c"]
        );
        assert_eq!(parse_comma_list(""), Vec::<String>::new());
    }
}
