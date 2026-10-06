//! Brand theming (colours, fonts, custom CSS) and base-path rewriting.

use crate::config::{ResolvedBrand, DEFAULT_ACCENT, DEFAULT_PRIMARY};
use regex::Regex;
use std::sync::LazyLock;

/// Rewrite absolute URLs in generated HTML to include a base path prefix.
/// No-op when base_path is empty.
pub fn prefix_base_path(html: &str, base_path: &str) -> String {
    if base_path.is_empty() {
        return html.to_string();
    }
    html.replace("href=\"/", &format!("href=\"{}/", base_path))
        .replace("src=\"/", &format!("src=\"{}/", base_path))
        .replace("action=\"/", &format!("action=\"{}/", base_path))
        .replace("url(\"/", &format!("url(\"{}/", base_path))
}

/// WCAG relative luminance of an sRGB colour.
fn luminance(c: [u8; 3]) -> f32 {
    let lin = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.039_28 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// Text colour for content placed on a filled primary colour.
fn on_color(c: [u8; 3]) -> &'static str {
    if luminance(c) > 0.30 {
        "#141A21"
    } else {
        "#FFFFFF"
    }
}

/// Generate CSS token overrides from the configured brand colours.
///
/// The output joins the app's `mc-tokens` cascade layer, so any unlayered
/// brand stylesheet still wins over it.
pub fn brand_css(brand: &ResolvedBrand) -> String {
    let mut light = String::new();
    let mut dark = String::new();

    // Darken for text variants, lighten for dark mode.
    let darken = |c: [u8; 3]| c.map(|v| (v as f32 * 0.75) as u8);
    let lighten = |c: [u8; 3]| c.map(|v| (v as f32 + (255.0 - v as f32) * 0.35) as u8);
    let rgb = |c: [u8; 3]| format!("rgb({},{},{})", c[0], c[1], c[2]);
    let rgba = |c: [u8; 3], a: f32| format!("rgba({},{},{},{})", c[0], c[1], c[2], a);

    for (var, color, default) in [
        ("blue", brand.primary_color, DEFAULT_PRIMARY),
        ("amber", brand.accent_color, DEFAULT_ACCENT),
    ] {
        if color == default {
            continue;
        }
        light.push_str(&format!(
            "--mc-{var}: {}; --mc-{var}-bg: {}; --mc-{var}-text: {};",
            rgb(color),
            rgba(color, 0.1),
            rgb(darken(color)),
        ));
        dark.push_str(&format!(
            "--mc-{var}: {}; --mc-{var}-bg: {}; --mc-{var}-text: {};",
            rgb(lighten(color)),
            rgba(color, 0.16),
            rgb(lighten(color)),
        ));
        if var == "blue" {
            light.push_str(&format!(" --mc-on-blue: {};", on_color(color)));
            dark.push_str(&format!(" --mc-on-blue: {};", on_color(lighten(color))));
        }
        light.push('\n');
        dark.push('\n');
    }

    if light.is_empty() {
        return String::new();
    }
    format!(
        "<style>\n@layer mc-tokens {{\n:root {{\n{light}}}\n@media (prefers-color-scheme: dark) {{\n:root:not([data-theme=\"light\"]) {{\n{dark}}}\n}}\n:root[data-theme=\"dark\"] {{\n{dark}}}\n}}\n</style>"
    )
}

/// Generate @font-face CSS for the brand font.
///
/// Only files whose name starts with the configured font name are registered
/// (e.g. `Inter-Bold.ttf` for `font_name: Inter`), so a fonts directory can
/// hold several families without them being mixed up.
pub fn font_face_css(brand: &ResolvedBrand) -> String {
    let fonts_dir = match &brand.fonts_dir {
        Some(d) => d,
        None => return String::new(),
    };
    let font_name = &brand.font_name;
    let family_key = normalize_font_key(font_name);

    let mut files: Vec<String> = std::fs::read_dir(fonts_dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    files.sort();

    let mut faces = String::new();
    for name in files {
        let lower = name.to_lowercase();
        let format = if lower.ends_with(".woff2") {
            "woff2"
        } else if lower.ends_with(".woff") {
            "woff"
        } else if lower.ends_with(".ttf") {
            "truetype"
        } else {
            continue;
        };
        let stem = normalize_font_key(lower.rsplit_once('.').map_or(&*lower, |(s, _)| s));
        let Some(variant) = stem.strip_prefix(&family_key) else {
            continue;
        };
        // Skip other families sharing a prefix, e.g. "InterDisplay" for "Inter".
        let (weight, style) = font_variant(variant);
        if weight == 0 {
            continue;
        }
        faces.push_str(&format!(
            "@font-face {{ font-family: \"{font_name}\"; src: url(\"/brand/fonts/{name}\") format(\"{format}\"); font-weight: {weight}; font-style: {style}; font-display: swap; }}\n"
        ));
    }

    if faces.is_empty() {
        return String::new();
    }
    format!(
        "<style>\n{faces}:root {{ --font-sans: \"{font_name}\", system-ui, -apple-system, \"Segoe UI\", Roboto, sans-serif; }}\n</style>"
    )
}

fn normalize_font_key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Map a font file's variant suffix (after the family name) to weight and style.
/// Returns weight 0 for unrecognized suffixes.
fn font_variant(variant: &str) -> (u16, &'static str) {
    let (base, style) = match variant.strip_suffix("italic") {
        Some(b) => (b, "italic"),
        None => (variant, "normal"),
    };
    let weight = match base {
        "" | "regular" | "book" | "normal" => 400,
        "thin" | "hairline" => 100,
        "extralight" | "ultralight" => 200,
        "light" => 300,
        "medium" => 500,
        "semibold" | "demibold" => 600,
        "bold" => 700,
        "extrabold" | "ultrabold" => 800,
        "black" | "heavy" => 900,
        _ => 0,
    };
    (weight, style)
}

/// Rewrite relative `url(...)` references in custom CSS so they resolve
/// against the stylesheet's own directory (served at `/brand/asset/`).
/// Without this, `url('fonts/x.ttf')` in an inlined stylesheet resolves
/// against the page URL and 404s.
pub fn rewrite_css_urls(css: &str) -> String {
    static URL_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"url\(\s*(?:"([^"]*)"|'([^']*)'|([^)'"\s]*))\s*\)"#).expect("static regex")
    });
    URL_RE
        .replace_all(css, |caps: &regex::Captures| {
            let url = caps
                .get(1)
                .or(caps.get(2))
                .or(caps.get(3))
                .map_or("", |m| m.as_str());
            let is_relative = !url.is_empty()
                && !url.starts_with('/')
                && !url.starts_with('#')
                && !url.contains(':');
            if is_relative {
                let url = url.strip_prefix("./").unwrap_or(url);
                format!("url(\"/brand/asset/{url}\")")
            } else {
                caps[0].to_string()
            }
        })
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brand(primary: [u8; 3]) -> ResolvedBrand {
        ResolvedBrand {
            name: "X".into(),
            tagline: String::new(),
            fonts_dir: None,
            font_name: "Inter".into(),
            primary_color: primary,
            accent_color: DEFAULT_ACCENT,
            logo: None,
            custom_css: None,
        }
    }

    #[test]
    fn rewrite_css_urls_resolves_relative_paths() {
        let css = "a{src:url('fonts/A.ttf')} b{src:url(\"./img/b.png\")} c{src:url(x.woff2)}";
        let out = rewrite_css_urls(css);
        assert!(out.contains(r#"url("/brand/asset/fonts/A.ttf")"#));
        assert!(out.contains(r#"url("/brand/asset/img/b.png")"#));
        assert!(out.contains(r#"url("/brand/asset/x.woff2")"#));
    }

    #[test]
    fn rewrite_css_urls_leaves_absolute_urls() {
        let css = "a{src:url('/x.ttf')} b{src:url(https://e.com/f.woff)} c{background:url(data:image/png;base64,AA)} d{mask:url(#m)}";
        assert_eq!(rewrite_css_urls(css), css);
    }

    #[test]
    fn font_variant_maps_weights() {
        assert_eq!(font_variant(""), (400, "normal"));
        assert_eq!(font_variant("regular"), (400, "normal"));
        assert_eq!(font_variant("bold"), (700, "normal"));
        assert_eq!(font_variant("bolditalic"), (700, "italic"));
        assert_eq!(font_variant("italic"), (400, "italic"));
        assert_eq!(font_variant("semibold"), (600, "normal"));
        assert_eq!(font_variant("display"), (0, "normal"));
    }

    #[test]
    fn brand_css_is_layered_with_theme_overrides() {
        assert_eq!(brand_css(&brand(DEFAULT_PRIMARY)), "");
        let css = brand_css(&brand([255, 103, 0]));
        assert!(css.contains("@layer mc-tokens"));
        assert!(css.contains(r#":root[data-theme="dark"]"#));
        assert!(css.contains(r#":root:not([data-theme="light"])"#));
        // Orange is light enough to need ink text.
        assert!(css.contains("--mc-on-blue: #141A21"));
        let css = brand_css(&brand([20, 40, 120]));
        assert!(css.contains("--mc-on-blue: #FFFFFF"));
    }

    #[test]
    fn prefix_base_path_rewrites_urls() {
        let html = r##"<a href="/x"><img src="/y"><form action="/s"><style>a{b:url("/f")}</style><a href="#z">"##;
        let out = prefix_base_path(html, "/hq");
        assert!(out.contains(r#"href="/hq/x""#));
        assert!(out.contains(r#"src="/hq/y""#));
        assert!(out.contains(r#"action="/hq/s""#));
        assert!(out.contains(r#"url("/hq/f")"#));
        assert!(out.contains(r##"href="#z""##));
        assert_eq!(prefix_base_path(html, ""), html);
    }
}
