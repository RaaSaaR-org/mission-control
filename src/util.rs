use crate::error::McResult;
use regex::Regex;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;

static SLUG_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^a-z0-9]+").expect("static regex pattern is always valid"));

/// ASCII spelling of common Latin letters with diacritics (`ü` -> `ue`,
/// `é` -> `e`) so slugs keep them instead of turning them into dashes.
fn transliterate(c: char) -> Option<&'static str> {
    Some(match c {
        'ä' | 'æ' => "ae",
        'ö' | 'œ' => "oe",
        'ü' => "ue",
        'ß' => "ss",
        'à' | 'á' | 'â' | 'ã' | 'å' | 'ā' => "a",
        'ç' | 'č' | 'ć' => "c",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ě' => "e",
        'ì' | 'í' | 'î' | 'ï' => "i",
        'ñ' | 'ń' | 'ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ø' => "o",
        'ù' | 'ú' | 'û' | 'ů' => "u",
        'ý' | 'ÿ' => "y",
        'š' | 'ś' => "s",
        'ž' | 'ź' | 'ż' => "z",
        'ł' => "l",
        'ř' => "r",
        _ => return None,
    })
}

/// Convert a name to a URL-friendly slug (ASCII `a-z0-9` and dashes, at most
/// 80 characters).
pub fn slugify(name: &str) -> String {
    slugify_with(name, true)
}

/// Slugs `name` may have on disk: the current [`slugify`] form and the form
/// used before letters with diacritics were transliterated (`Müller` ->
/// `m-ller` instead of `mueller`). Use this when matching existing folders.
pub fn slug_variants(name: &str) -> Vec<String> {
    let mut variants = vec![slugify(name)];
    let legacy = slugify_with(name, false);
    if !variants.contains(&legacy) {
        variants.push(legacy);
    }
    variants
}

fn slugify_with(name: &str, transliterate_letters: bool) -> String {
    let mut lower = String::with_capacity(name.len());
    for c in name.to_lowercase().chars() {
        // Decomposed (NFD) input, common on macOS, spells `ü` as `u` plus a
        // combining diaeresis: fold it like the precomposed letter (`ue`) and
        // drop other combining accents (`e` + U+0301 -> `e`).
        if transliterate_letters && ('\u{300}'..='\u{36f}').contains(&c) {
            if c == '\u{308}' && lower.ends_with(['a', 'o', 'u']) {
                lower.push('e');
            }
            continue;
        }
        match transliterate(c).filter(|_| transliterate_letters) {
            Some(ascii) => lower.push_str(ascii),
            None => lower.push(c),
        }
    }
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

/// Write data to a temporary file in the same directory, then rename it
/// over `path` so readers never see a partially written file. The temporary
/// name is hidden and unique per process and call (the MCP and API servers
/// write from several threads), and it is removed if the write fails.
///
/// A symlinked `path` is followed (the link's target is replaced, the link
/// stays a link), and an existing file keeps its permissions.
pub fn atomic_write(path: &Path, data: &[u8]) -> McResult<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let resolved = fs::canonicalize(path)
        .ok()
        .filter(|_| fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()));
    let path = resolved.as_deref().unwrap_or(path);
    let permissions = fs::metadata(path).ok().map(|m| m.permissions());
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".to_string());
    let tmp = path.with_file_name(format!(
        ".{}.{}-{}.tmp",
        file_name,
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = fs::write(&tmp, data)
        .and_then(|()| match &permissions {
            Some(p) => fs::set_permissions(&tmp, p.clone()),
            None => Ok(()),
        })
        .and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    Ok(result?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_variants_include_pre_transliteration_form() {
        assert_eq!(
            slug_variants("Müller GmbH"),
            vec!["mueller-gmbh", "m-ller-gmbh"]
        );
        assert_eq!(slug_variants("Acme Inc"), vec!["acme-inc"]);
    }

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
    fn test_slugify_transliterates_diacritics() {
        assert_eq!(slugify("Müller & Söhne GmbH"), "mueller-soehne-gmbh");
        assert_eq!(slugify("Café Straße"), "cafe-strasse");
        assert_eq!(slugify("Ærø Łódź"), "aero-lodz");
        assert_eq!(slugify("東京"), "");
    }

    #[test]
    fn test_atomic_write_replaces_and_leaves_no_temp_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("CUST-001.md");
        atomic_write(&path, b"one").unwrap();
        atomic_write(&path, b"two").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "two");
        let names: Vec<_> = fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1);
        assert!(atomic_write(&tmp.path().join("missing").join("x.md"), b"x").is_err());
    }

    #[test]
    fn test_slugify_decomposed_umlauts_match_precomposed() {
        let nfd = "Gru\u{308}n Ma\u{308}rz Cafe\u{301} O\u{308}l";
        assert_eq!(slugify(nfd), slugify("Grün März Café Öl"));
        assert_eq!(slugify(nfd), "gruen-maerz-cafe-oel");
    }

    #[cfg(unix)]
    #[test]
    fn test_atomic_write_follows_symlinks_and_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::TempDir::new().unwrap();
        let shared = tmp.path().join("shared");
        fs::create_dir(&shared).unwrap();
        let target = shared.join("linked.md");
        fs::write(&target, "old").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        let link = tmp.path().join("TASK-020-linked.md");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        atomic_write(&link, b"new").unwrap();
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        let mode = fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        // No temp files left next to the link or the target.
        assert_eq!(fs::read_dir(&shared).unwrap().count(), 1);
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 2);
    }

    #[test]
    fn test_parse_comma_list() {
        assert_eq!(parse_comma_list("a, b, c"), vec!["a", "b", "c"]);
        assert_eq!(parse_comma_list(""), Vec::<String>::new());
    }
}
