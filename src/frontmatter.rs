use crate::error::{McError, McResult};
use regex::{Captures, Regex};
use serde_yaml::Value;
use std::path::Path;
use std::sync::LazyLock;

/// A whole single-quoted scalar that is a wiki-link, as the entire value of a
/// `key: ` or `- ` line. The content may only contain `''` escapes, so the
/// closing quote is the real end of the scalar.
static SINGLE_QUOTED_LINK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^([ \t]*(?:- |[^\s#'"][^\n]*?: ))'(\[\[(?:[^'\n]|'')*\]\])'[ \t]*$"#)
        .expect("static regex is valid")
});
static LINK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[(.+?)\]\]").expect("static regex is valid"));
/// mc's link footer as a whole line (with its own line break, if any).
static MC_LINKS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^%% mc-links:[^\n]*%%[ \t]*(?:\r?\n|\z)").expect("static regex is valid")
});

/// Split a markdown string into optional frontmatter (without delimiters) and body.
///
/// The opening and closing `---` must each be on a line of their own (trailing
/// whitespace and CRLF line endings are tolerated). The returned body starts
/// on the line after the closing `---` (its line ending is consumed), which is
/// exactly what [`serialize_document`] writes back, so parse -> serialize is
/// byte-stable for an unchanged body and frontmatter.
pub fn split_frontmatter(content: &str) -> Option<(String, String)> {
    let trimmed = content.trim_start_matches('\u{feff}').trim_start();
    let rest = trimmed.strip_prefix("---")?;
    let open_end = rest.find('\n')?;
    if !rest[..open_end].trim().is_empty() {
        return None;
    }
    let after_open = &rest[open_end + 1..];

    let mut pos = 0;
    for line in after_open.split_inclusive('\n') {
        if line.trim_end() == "---" {
            let fm = &after_open[..pos];
            let fm = fm.strip_suffix('\n').unwrap_or(fm);
            let body = &after_open[pos + line.len()..];
            return Some((fm.to_string(), body.to_string()));
        }
        pos += line.len();
    }
    None
}

/// Parse raw YAML frontmatter string into a serde_yaml::Value (should be a Mapping).
pub fn parse_raw(fm_str: &str, source: &Path) -> McResult<Value> {
    let val: Value = serde_yaml::from_str(fm_str).map_err(|e| McError::Frontmatter {
        path: source.to_path_buf(),
        message: e.to_string(),
    })?;
    Ok(val)
}

static YAML_LINE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bline (\d+)").expect("static regex is valid"));

/// The YAML parser's message for the frontmatter of `content`, with line
/// numbers counted from the top of the file (serde_yaml counts from the first
/// frontmatter line).
pub fn yaml_error_in_file(content: &str, e: &serde_yaml::Error) -> String {
    // Lines up to and including the opening `---` (see `split_frontmatter`).
    let trimmed = content.trim_start_matches('\u{feff}').trim_start();
    let lead = content.len() - trimmed.len();
    let offset = content[..lead].matches('\n').count() + 1;
    YAML_LINE_RE
        .replace_all(&e.to_string(), |caps: &Captures| {
            let line: usize = caps[1].parse().unwrap_or(0);
            format!("line {}", line + offset)
        })
        .into_owned()
}

/// Like [`parse_raw`] for the frontmatter `fm_str` split from `content`, but
/// a YAML error names lines of the file (see [`yaml_error_in_file`]).
pub fn parse_in_file(content: &str, fm_str: &str, source: &Path) -> McResult<Value> {
    serde_yaml::from_str(fm_str).map_err(|e| McError::Frontmatter {
        path: source.to_path_buf(),
        message: yaml_error_in_file(content, &e),
    })
}

/// Parse frontmatter from a file, returning (Value, body).
pub fn parse_file(path: &Path) -> McResult<(Value, String)> {
    let content = std::fs::read_to_string(path)?;
    match split_frontmatter(&content) {
        Some((fm_str, body)) => Ok((parse_in_file(&content, &fm_str, path)?, body)),
        None => Err(McError::Frontmatter {
            path: path.to_path_buf(),
            message: "No YAML frontmatter found".into(),
        }),
    }
}

/// Read `path`, let `edit` change its frontmatter, and write the file back
/// atomically, holding the repo's write lock. The body is kept byte for byte
/// (apart from mc's link footer, which is kept up to date).
pub fn update_file(path: &Path, edit: impl FnOnce(&mut Value)) -> McResult<()> {
    let _lock = crate::lock::acquire_for_file(path)?;
    let (mut fm, body) = parse_file(path)?;
    edit(&mut fm);
    crate::util::atomic_write(path, serialize_document(&fm, &body).as_bytes())
}

/// Serialize a YAML Value back into a complete markdown file with frontmatter.
///
/// Files with CRLF line endings in the body get CRLF in the frontmatter and
/// the link footer too.
pub fn serialize_document(frontmatter: &Value, body: &str) -> String {
    let nl = if body.contains("\r\n") { "\r\n" } else { "\n" };
    // Serializing an in-memory `Value` cannot fail in practice (no I/O, all keys
    // are YAML values); fall back to an empty mapping rather than panicking.
    let yaml = serde_yaml::to_string(frontmatter).unwrap_or_else(|_| "{}".to_string());
    let yaml = yaml.trim_end();

    // Replace single-quoted wiki-links with double-quoted for Obsidian compatibility.
    // serde_yaml uses single quotes for strings containing `[`/`]`, but Obsidian
    // only recognises wiki-links inside double quotes in frontmatter. The
    // content is re-escaped for double quotes (`''` -> `'`, `"` -> `\"`).
    let requoted = SINGLE_QUOTED_LINK_RE.replace_all(yaml, |c: &Captures| {
        let inner = c[2]
            .replace("''", "'")
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        format!("{}\"{}\"", &c[1], inner)
    });
    // A look-alike line inside a block scalar must not change; keep the
    // plain serde output unless the rewrite reads back as the same values.
    let yaml = if requoted == yaml
        || serde_yaml::from_str::<Value>(&requoted).is_ok_and(|v| v == *frontmatter)
    {
        requoted
    } else {
        std::borrow::Cow::Borrowed(yaml)
    };

    // Mirror all [[...]] links of the frontmatter values in the document body.
    // Obsidian's graph view reliably picks up links from body text but not
    // always from frontmatter properties.
    let mut links = Vec::new();
    collect_links(frontmatter, &mut links);

    // The existing footer (the last one outside code). It is replaced so
    // repeated serialisation is idempotent; text after it stays where it is.
    let footer = find_links_footer(body);

    // A footer that already links the same entities (aliases, order and
    // duplicates aside) is kept byte for byte: rewriting it would only be
    // diff noise.
    let keep =
        footer.filter(|m| !links.is_empty() && link_ids(m.as_str()) == link_ids(&links.join(" ")));
    let body = match keep {
        // Kept and already the last thing in the body: nothing changes.
        Some(m) if m.end() == body.len() => body.to_string(),
        _ => {
            let rest = match footer {
                Some(m) => format!("{}{}", &body[..m.start()], &body[m.end()..]),
                None => body.to_string(),
            };
            // Append an Obsidian comment listing all frontmatter links.
            let line = match keep {
                Some(m) => m.as_str().trim_end_matches(['\r', '\n']).to_string(),
                None => format!("%% mc-links: {} %%", links.join(" ")),
            };
            if links.is_empty() {
                rest
            } else {
                format!("{}{nl}{line}{nl}", rest.trim_end())
            }
        }
    };

    let yaml = if nl == "\n" {
        yaml.into_owned()
    } else {
        yaml.replace('\n', nl)
    };
    format!("---{nl}{yaml}{nl}---{nl}{body}")
}

/// `[[...]]` links in every string of `v`, in document order.
fn collect_links(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::String(s) => out.extend(LINK_RE.captures_iter(s).map(|c| format!("[[{}]]", &c[1]))),
        Value::Sequence(seq) => seq.iter().for_each(|i| collect_links(i, out)),
        Value::Mapping(map) => map.iter().for_each(|(k, v)| {
            collect_links(k, out);
            collect_links(v, out);
        }),
        Value::Tagged(t) => collect_links(&t.value, out),
        _ => {}
    }
}

/// The last `%% mc-links: ... %%` line of `body` (with its line break).
/// Look-alikes inside code (e.g. documentation of the footer) are ignored.
fn find_links_footer(body: &str) -> Option<regex::Match<'_>> {
    let code = crate::checklist::code_ranges(body);
    MC_LINKS_RE
        .find_iter(body)
        .filter(|m| !code.iter().any(|r| r.contains(&m.start())))
        .last()
}

/// The link targets of every `[[...]]` in `text`, without `|alias`,
/// upper-cased so `[[proj-001|Project]]` and `[[PROJ-001]]` compare equal.
fn link_ids(text: &str) -> std::collections::BTreeSet<String> {
    LINK_RE
        .captures_iter(text)
        .map(|c| c[1].split('|').next().unwrap_or("").trim().to_uppercase())
        .collect()
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
///
/// Hand-edited files often contain `tags: urgent` instead of a list, or
/// numeric items like `tags: [2026]`; both are accepted. Other non-scalar
/// items are skipped.
pub fn get_string_list(val: &Value, key: &str) -> Vec<String> {
    let Some(v) = val
        .as_mapping()
        .and_then(|m| m.get(Value::String(key.to_string())))
    else {
        return Vec::new();
    };
    match v {
        Value::Sequence(seq) => seq.iter().filter_map(scalar_to_string).collect(),
        Value::String(s) if !s.trim().is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

fn scalar_to_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
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
/// Already-wrapped input is returned unchanged (no `[[[[X]]]]`).
pub fn wrap_wikilink(s: &str) -> String {
    if s.is_empty() || (s.starts_with("[[") && s.ends_with("]]")) {
        s.to_string()
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
    fn yaml_errors_name_file_lines() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("t.md");
        for (content, line) in [
            ("---\nid: T-1\ntitle: \"open\n---\n", 3),
            ("\u{feff}\n\n---\nid: T-1\ntitle: \"open\n---\nbody\n", 5),
        ] {
            std::fs::write(&path, content).unwrap();
            let Err(McError::Frontmatter { message, .. }) = parse_file(&path) else {
                panic!("expected a frontmatter error");
            };
            let want = format!("quoted scalar at line {line} column");
            assert!(message.contains(&want), "{message}");
        }
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

    #[test]
    fn test_split_frontmatter_body_starts_after_closing_line() {
        let (fm, body) = split_frontmatter("---\nid: X-1\n---\n# T\n").unwrap();
        assert_eq!(fm, "id: X-1");
        assert_eq!(body, "# T\n");
        let (_, body) = split_frontmatter("---\nid: X-1\n---\n\n# T\n").unwrap();
        assert_eq!(body, "\n# T\n");
        let (_, body) = split_frontmatter("---\nid: X-1\n---").unwrap();
        assert_eq!(body, "");
    }

    #[test]
    fn test_parse_serialize_round_trip_is_byte_stable() {
        // Blank line after the frontmatter, wiki-links (mc-links footer), and none.
        for doc in [
            "---\nid: TASK-001\nstatus: todo\n---\n\n# Title\n\n## Notes\n",
            "---\nid: TASK-001\nstatus: todo\n---\n# Title\n",
            "---\nid: PROJ-001\ncustomer: \"[[CUST-001]]\"\n---\n\n# P\n%% mc-links: [[CUST-001]] %%\n",
        ] {
            let mut current = doc.to_string();
            for _ in 0..3 {
                let (fm_str, body) = split_frontmatter(&current).unwrap();
                let fm = parse_raw(&fm_str, Path::new("t.md")).unwrap();
                current = serialize_document(&fm, &body);
                assert_eq!(current, doc, "round trip changed the file");
            }
        }
    }

    #[test]
    fn test_split_frontmatter_crlf() {
        let content = "---\r\nid: CUST-001\r\nname: Acme\r\n---\r\n# Acme\r\n";
        let (fm, body) = split_frontmatter(content).unwrap();
        let val = parse_raw(&fm, Path::new("x.md")).unwrap();
        assert_eq!(get_str(&val, "id"), Some("CUST-001"));
        assert_eq!(get_str(&val, "name"), Some("Acme"));
        assert!(body.contains("# Acme"));
    }

    #[test]
    fn test_split_frontmatter_empty_block() {
        let (fm, body) = split_frontmatter("---\n---\nBody").unwrap();
        assert_eq!(fm, "");
        assert_eq!(body, "Body");
    }

    #[test]
    fn test_split_frontmatter_ignores_longer_dash_runs() {
        // A `----` line inside the YAML block must not close it.
        let content = "---\nnotes: |\n  ----\n  text\nid: A-1\n---\nbody";
        let (fm, body) = split_frontmatter(content).unwrap();
        let val = parse_raw(&fm, Path::new("x.md")).unwrap();
        assert_eq!(get_str(&val, "id"), Some("A-1"));
        assert_eq!(body, "body");
    }

    #[test]
    fn test_split_frontmatter_rejects_non_delimiter_opening() {
        assert!(split_frontmatter("----\nid: A\n---\n").is_none());
        assert!(split_frontmatter("---id: A\n---\n").is_none());
        assert!(split_frontmatter("---").is_none());
        assert!(split_frontmatter("---\nid: A\n").is_none());
    }

    #[test]
    fn test_split_frontmatter_bom_and_trailing_spaces() {
        let (fm, _) = split_frontmatter("\u{feff}--- \nid: A-1\n---  \nbody").unwrap();
        assert_eq!(fm, "id: A-1");
    }

    #[test]
    fn test_update_file_keeps_body_and_size_stable() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("t.md");
        let doc = "---\nid: TASK-001\nstatus: todo\n---\n\n# T\n\nBody.\n";
        std::fs::write(&path, doc).unwrap();
        update_file(&path, |fm| set_str(fm, "status", "done")).unwrap();
        update_file(&path, |fm| set_str(fm, "status", "todo")).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), doc);
    }

    #[test]
    fn test_parse_file_reports_path_on_bad_yaml() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("bad.md");
        std::fs::write(&path, "---\nid: [unclosed\n---\n").unwrap();
        let err = parse_file(&path).unwrap_err();
        assert!(err.to_string().contains("bad.md"), "{err}");
    }

    #[test]
    fn test_wrap_wikilink_idempotent() {
        assert_eq!(wrap_wikilink("[[CUST-001]]"), "[[CUST-001]]");
        assert_eq!(wrap_wikilink(&wrap_wikilink("X")), "[[X]]");
    }

    #[test]
    fn test_get_string_list_scalar_and_numbers() {
        let fm = parse_raw(
            "tags: urgent\nnums: [2026, true, x]\nempty: ''",
            Path::new("t.md"),
        )
        .unwrap();
        assert_eq!(get_string_list(&fm, "tags"), vec!["urgent"]);
        assert_eq!(get_string_list(&fm, "nums"), vec!["2026", "true", "x"]);
        assert!(get_string_list(&fm, "empty").is_empty());
        assert!(get_string_list(&fm, "missing").is_empty());
    }

    #[test]
    fn test_get_link_list_scalar() {
        let fm = parse_raw("customers: '[[CUST-001]]'", Path::new("t.md")).unwrap();
        assert_eq!(get_link_list(&fm, "customers"), vec!["CUST-001"]);
    }

    /// Serialize `fm` with `body` twice (the second time from the first
    /// result) and check the output is stable and parses back to `fm`.
    fn stable(fm: &Value, body: &str) -> String {
        let one = serialize_document(fm, body);
        let (fm_str, body1) = split_frontmatter(&one).unwrap();
        let parsed = parse_raw(&fm_str, Path::new("t.md")).expect(&one);
        assert_eq!(&parsed, fm, "{one}");
        let two = serialize_document(&parsed, &body1);
        assert_eq!(one, two, "not stable");
        one
    }

    #[test]
    fn test_wikilink_values_with_quotes_stay_valid_yaml() {
        for (value, expect) in [
            ("[[O'Neil]]", r#"x: "[[O'Neil]]""#),
            (r#"[[The "Big" Plan]]"#, r#"x: "[[The \"Big\" Plan]]""#),
            (
                r#"[[Q3]] "final" [[deck]]"#,
                r#"x: "[[Q3]] \"final\" [[deck]]""#,
            ),
            (r"[[a\b]]", r#"x: "[[a\\b]]""#),
            ("[[TASK-001]]", r#"x: "[[TASK-001]]""#),
        ] {
            let mut fm = parse_raw("id: T-1", Path::new("t.md")).unwrap();
            set_str(&mut fm, "x", value);
            let doc = stable(&fm, "\n# T\n");
            assert!(doc.contains(expect), "{value}: {doc}");
        }
        // Text around a quoted link is not a link value: left as serde wrote it.
        let mut fm = parse_raw("id: T-1", Path::new("t.md")).unwrap();
        set_str(&mut fm, "title", "Ask '[[Bob]]' about it");
        let doc = stable(&fm, "\n");
        assert_eq!(get_str(&fm, "title"), Some("Ask '[[Bob]]' about it"));
        assert!(doc.contains("%% mc-links: [[Bob]] %%"), "{doc}");
        // Link footers are built from the values, not the YAML escapes.
        let mut fm = parse_raw("id: T-1", Path::new("t.md")).unwrap();
        set_str(&mut fm, "owner", "[[Jane's]]");
        assert!(stable(&fm, "\n").contains("%% mc-links: [[Jane's]] %%"));
    }

    #[test]
    fn test_block_scalar_lookalikes_are_not_requoted() {
        let fm = parse_raw("id: T-1\nnotes: |-\n  a: '[[x]]'\n  b\n", Path::new("t.md")).unwrap();
        let doc = stable(&fm, "\n");
        assert!(doc.contains("a: '[[x]]'"), "{doc}");
    }

    #[test]
    fn test_text_after_the_footer_keeps_its_line() {
        let doc = "---\nid: T-1\nsprint: \"[[SPR-001]]\"\n---\n\n## Notes\n%% mc-links: [[SPR-001]] %%\n- [ ] appended item\n";
        let (fm_str, body) = split_frontmatter(doc).unwrap();
        let fm = parse_raw(&fm_str, Path::new("t.md")).unwrap();
        let out = stable(&fm, &body);
        assert!(
            out.ends_with("\n## Notes\n- [ ] appended item\n%% mc-links: [[SPR-001]] %%\n"),
            "{out}"
        );
        assert_eq!(out.matches("%% mc-links:").count(), 1);
    }

    #[test]
    fn test_footer_lookalikes_in_code_survive() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("t.md");
        let doc = "---\nid: T-1\nsprint: \"[[SPR-001]]\"\n---\n\nDocs:\n\n```\nline A\n%% mc-links: [[CUST-001]] %%\nline B\n```\n\nend\n%% mc-links: [[SPR-001]] %%\n";
        std::fs::write(&path, doc).unwrap();
        update_file(&path, |fm| set_str(fm, "status", "todo")).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(
            out.contains("```\nline A\n%% mc-links: [[CUST-001]] %%\nline B\n```"),
            "{out}"
        );
        assert!(
            out.ends_with("\nend\n%% mc-links: [[SPR-001]] %%\n"),
            "{out}"
        );
        // Without links the old footer goes, the line before it stays.
        let mut fm = parse_raw("id: T-1", Path::new("t.md")).unwrap();
        set_str(&mut fm, "x", "y");
        assert_eq!(
            serialize_document(&fm, "a\n%% mc-links: [[OLD]] %%\nb\n"),
            "---\nid: T-1\nx: y\n---\na\nb\n"
        );
    }

    #[test]
    fn test_crlf_files_stay_crlf() {
        let doc = "---\r\nid: T-1\r\nsprint: \"[[SPR-001]]\"\r\nstatus: todo\r\n---\r\n\r\n# T\r\n\r\nBody\r\n%% mc-links: [[SPR-001]] %%\r\n";
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("t.md");
        std::fs::write(&path, doc).unwrap();
        update_file(&path, |fm| set_str(fm, "status", "done")).unwrap();
        update_file(&path, |fm| set_str(fm, "status", "todo")).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), doc);
    }

    /// `update_file` on `doc` (a status round trip) and the result.
    fn status_round_trip(doc: &str) -> String {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("t.md");
        std::fs::write(&path, doc).unwrap();
        update_file(&path, |fm| set_str(fm, "status", "in-progress")).unwrap();
        update_file(&path, |fm| set_str(fm, "status", "todo")).unwrap();
        std::fs::read_to_string(&path).unwrap()
    }

    #[test]
    fn test_footer_with_aliases_for_the_same_links_is_kept() {
        let doc = "---\nid: T-1\nstatus: todo\nprojects:\n- \"[[PROJ-001]]\"\n---\n\n# T\n%% mc-links: [[PROJ-001|Innovation Project]] %%\n";
        assert_eq!(status_round_trip(doc), doc);
        // Case and duplicates don't count either; CRLF stays CRLF.
        let doc = "---\r\nid: T-1\r\nstatus: todo\r\nprojects:\r\n- \"[[PROJ-001]]\"\r\n---\r\n\r\nBody\r\n%% mc-links: [[proj-001|P]] [[PROJ-001]] %%\r\n";
        assert_eq!(status_round_trip(doc), doc);
    }

    #[test]
    fn test_footer_with_the_same_links_in_another_order_is_kept() {
        let doc = "---\nid: T-1\nstatus: todo\nsprint: \"[[SPR-001]]\"\ncustomers:\n- \"[[CUST-001]]\"\n---\n\n# T\n%% mc-links: [[CUST-001|Acme]] [[SPR-001]] %%\n";
        assert_eq!(status_round_trip(doc), doc);
        // Text after a kept footer still moves above it.
        let (fm_str, body) = split_frontmatter(doc).unwrap();
        let fm = parse_raw(&fm_str, Path::new("t.md")).unwrap();
        let body = body.replace("%%\n", "%%\n- [ ] item\n");
        assert!(
            stable(&fm, &body)
                .ends_with("\n# T\n- [ ] item\n%% mc-links: [[CUST-001|Acme]] [[SPR-001]] %%\n"),
            "{body}"
        );
    }

    #[test]
    fn test_footer_with_other_links_is_regenerated() {
        let doc = "---\nid: T-1\nstatus: todo\nsprint: \"[[SPR-001]]\"\nprojects:\n- \"[[PROJ-001]]\"\n---\n\n# T\n%% mc-links: [[PROJ-001|Innovation Project]] %%\n";
        let out = status_round_trip(doc);
        assert!(
            out.ends_with("\n# T\n%% mc-links: [[SPR-001]] [[PROJ-001]] %%\n"),
            "{out}"
        );
        let doc = "---\nid: T-1\nstatus: todo\nprojects:\n- \"[[PROJ-001]]\"\n---\n\n# T\n%% mc-links: [[PROJ-001]] [[CUST-001]] %%\n";
        let out = status_round_trip(doc);
        assert!(
            out.ends_with("\n# T\n%% mc-links: [[PROJ-001]] %%\n"),
            "{out}"
        );
    }
}
