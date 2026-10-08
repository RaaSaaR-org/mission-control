use crate::cli::ui;
use crate::cli::{PrintEntity, PrintTemplate};
use crate::config::{RepoMode, ResolvedConfig};
use crate::data;
use crate::entity::EntityKind;
use crate::error::{McError, McResult};
use crate::frontmatter;
use crate::html::Catalog;
use crate::util;
use colored::*;
use genpdf::elements;
use genpdf::fonts;
use genpdf::style;
use genpdf::Alignment;
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use regex::Regex;
use serde_json::Value as JsonValue;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

/// Font sizes used throughout the PDF.
const H1_SIZE: u8 = 16;
const H2_SIZE: u8 = 14;
const H3_SIZE: u8 = 12;
const BODY_SIZE: u8 = 10;
const SMALL_SIZE: u8 = 8;

/// Cover page font sizes.
const COVER_BRAND_SIZE: u8 = 36;
const COVER_TITLE_SIZE: u8 = 28;
const COVER_TAGLINE_SIZE: u8 = 14;
const COVER_LABEL_SIZE: u8 = 16;
const COVER_META_SIZE: u8 = 11;

/// Footer and detail sizes.
const FOOTER_SIZE: u8 = 7;
const CODE_BLOCK_SIZE: u8 = 9;
const META_LABEL_SIZE: u8 = 9;

/// Page margins in mm.
const MARGIN_MM: f64 = 20.0;

pub fn run(entity: &PrintEntity, cfg: &ResolvedConfig) -> McResult<()> {
    let printed = match entity {
        PrintEntity::Meeting { id, output } => {
            meeting_pdf(cfg, id, output.as_deref(), Access::Cli)?
        }
        PrintEntity::Research { id, output, file } => {
            research_pdf(cfg, id, output.as_deref(), file.as_deref(), Access::Cli)?
        }
        PrintEntity::File {
            path,
            output,
            template,
            title,
        } => file_pdf(
            cfg,
            path,
            output.as_deref(),
            template,
            title.as_deref(),
            Access::Cli,
        )?,
    };
    report_written(&printed.path);
    Ok(())
}

/// Which paths a print may read and write. The CLI takes paths as typed
/// (relative to the working directory); MCP callers are confined to the
/// repo root, so a prompt-injected note can't make an agent read
/// `~/.ssh/id_rsa` into a PDF or overwrite `~/.zshrc`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    Cli,
    Repo,
}

/// A rendered PDF.
struct Printed {
    id: Option<String>,
    title: String,
    path: PathBuf,
}

impl Printed {
    /// `{id, title, path}` for MCP, with `path` relative to the repo root.
    fn to_json(&self, cfg: &ResolvedConfig) -> JsonValue {
        let root = cfg.root.canonicalize().unwrap_or_else(|_| cfg.root.clone());
        let path = self.path.strip_prefix(&root).unwrap_or(&self.path);
        let mut out = serde_json::json!({
            "title": self.title,
            "path": path.display().to_string(),
        });
        if let Some(id) = &self.id {
            out["id"] = JsonValue::from(id.as_str());
        }
        out
    }
}

/// Load a font family from the brand fonts directory, falling back to system fonts.
///
/// Priority: configured brand fonts → system font discovery → error.
fn load_fonts(cfg: &ResolvedConfig) -> McResult<fonts::FontFamily<fonts::FontData>> {
    // 1. Try configured brand fonts
    if let Some(ref fonts_dir) = cfg.brand.fonts_dir {
        match fonts::from_files(fonts_dir, &cfg.brand.font_name, None::<fonts::Builtin>) {
            Ok(family) => return Ok(family),
            Err(e) => {
                return Err(McError::Pdf(format!(
                    "Could not load fonts from {}: {}.\n\
                     Place TTF files in assets/brand/fonts/ (see assets/brand/README.md).",
                    fonts_dir.display(),
                    e
                )));
            }
        }
    }

    // 2. Try system font discovery
    if let Some(family) = discover_system_fonts() {
        return Ok(family);
    }

    // 3. No fonts found
    Err(McError::Pdf(
        "No fonts found. Set brand.fonts_dir in config.yml, \
         or install Liberation Sans (Linux) or ensure Arial is available (macOS)."
            .into(),
    ))
}

/// Search well-known system directories for a usable font family.
///
/// On Linux, looks for LiberationSans in standard font directories (hyphenated naming
/// matches genpdf's `from_files` convention). On macOS, loads Arial individually since
/// its space-separated filenames don't match the convention.
fn discover_system_fonts() -> Option<fonts::FontFamily<fonts::FontData>> {
    // Linux: LiberationSans uses hyphenated naming that matches from_files()
    for (dir, name) in [
        ("/usr/share/fonts/truetype/liberation", "LiberationSans"),
        ("/usr/share/fonts/TTF", "LiberationSans"),
    ] {
        if Path::new(dir).is_dir() {
            if let Ok(family) = fonts::from_files(dir, name, None::<fonts::Builtin>) {
                return Some(family);
            }
        }
    }

    // macOS: Arial uses space-separated naming, load each variant individually
    let macos_dir = Path::new("/System/Library/Fonts/Supplemental");
    if macos_dir.is_dir() {
        let load = |filename: &str| -> Option<fonts::FontData> {
            fonts::FontData::load(macos_dir.join(filename), None).ok()
        };
        if let (Some(r), Some(b), Some(i), Some(bi)) = (
            load("Arial.ttf"),
            load("Arial Bold.ttf"),
            load("Arial Italic.ttf"),
            load("Arial Bold Italic.ttf"),
        ) {
            return Some(fonts::FontFamily {
                regular: r,
                bold: b,
                italic: i,
                bold_italic: bi,
            });
        }
    }

    None
}

/// Create a configured genpdf Document with page decorator.
fn create_document(
    font_family: fonts::FontFamily<fonts::FontData>,
    doc_title: &str,
    brand_name: &str,
    primary_color: style::Color,
    accent_color: style::Color,
) -> genpdf::Document {
    let mut doc = genpdf::Document::new(font_family);
    doc.set_title(doc_title);
    doc.set_font_size(BODY_SIZE);
    doc.set_line_spacing(1.25);

    let mut decorator = genpdf::SimplePageDecorator::new();
    decorator.set_margins(MARGIN_MM);

    let brand = brand_name.to_string();
    let title = doc_title.to_string();
    let pc = primary_color;
    let ac = accent_color;

    decorator.set_header(move |page| {
        let mut layout = elements::LinearLayout::vertical();

        if page == 1 {
            // Cover page: minimal header (just spacing)
            layout.push(elements::Break::new(0.5));
        } else {
            // Pages 2+: brand + page number header with title and separator
            let mut header_line = elements::TableLayout::new(vec![1, 1]);
            header_line
                .row()
                .element(
                    elements::Paragraph::new(style::StyledString::new(
                        brand.clone(),
                        style::Style::new().bold().with_color(pc),
                    ))
                    .aligned(Alignment::Left),
                )
                .element(
                    elements::Paragraph::new(style::StyledString::new(
                        format!("Page {}", page),
                        style::Style::new()
                            .with_font_size(SMALL_SIZE)
                            .with_color(ac),
                    ))
                    .aligned(Alignment::Right),
                )
                .push()
                .ok();
            layout.push(header_line);

            // Document title subtitle
            layout.push(
                elements::Paragraph::new(style::StyledString::new(
                    title.clone(),
                    style::Style::new()
                        .italic()
                        .with_font_size(SMALL_SIZE)
                        .with_color(ac),
                ))
                .aligned(Alignment::Left),
            );

            // Thin separator line below header
            let rule_text = "─".repeat(90);
            layout.push(elements::Paragraph::new(style::StyledString::new(
                rule_text,
                style::Style::new()
                    .with_font_size(4)
                    .with_color(style::Color::Rgb(200, 200, 200)),
            )));

            layout.push(elements::Break::new(0.5));
        }

        layout
    });

    doc.set_page_decorator(decorator);
    doc
}

fn primary_color(cfg: &ResolvedConfig) -> style::Color {
    let c = cfg.brand.primary_color;
    style::Color::Rgb(c[0], c[1], c[2])
}

fn accent_color(cfg: &ResolvedConfig) -> style::Color {
    let c = cfg.brand.accent_color;
    style::Color::Rgb(c[0], c[1], c[2])
}

// ---------------------------------------------------------------------------
// Meeting PDF
// ---------------------------------------------------------------------------

fn report_written(path: &Path) {
    // Relative to the working directory when it's below it.
    let shown = std::env::current_dir()
        .ok()
        .and_then(|cwd| {
            let cwd = cwd.canonicalize().unwrap_or(cwd);
            let abs = path.canonicalize().ok()?;
            abs.strip_prefix(&cwd).ok().map(Path::to_path_buf)
        })
        .unwrap_or_else(|| path.to_path_buf())
        .display()
        .to_string();
    ui::success(format!("PDF written to {}", shown.bold()));
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "start"
    } else {
        "xdg-open"
    };
    ui::hint(format!(
        "view it with {}",
        ui::cmd(&format!("{opener} {}", shell_quote(&shown)))
    ));
}

fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// Where to write the PDF: `output` (or `default_name`), as typed for the
/// CLI. For [`Access::Repo`] the path is taken relative to the repo root and
/// must stay inside it, be visible (no `.git/...`), end in `.pdf` and not
/// replace a file that isn't a PDF.
fn output_path(
    cfg: &ResolvedConfig,
    output: Option<&str>,
    default_name: String,
    access: Access,
) -> McResult<PathBuf> {
    let output = output.map(str::to_string).unwrap_or(default_name);
    if access == Access::Cli {
        return Ok(PathBuf::from(output));
    }
    let usage = |msg: String| {
        McError::usage(
            msg,
            Some("give a .pdf path inside the repository, e.g. exports/report.pdf".into()),
        )
    };
    let root = cfg.root.canonicalize()?;
    let joined = root.join(&output);
    if joined
        .extension()
        .is_none_or(|e| !e.eq_ignore_ascii_case("pdf"))
    {
        return Err(usage(format!("output '{output}' must end in .pdf")));
    }
    let (Some(parent), Some(name)) = (joined.parent(), joined.file_name()) else {
        return Err(usage(format!("'{output}' is not a file path")));
    };
    // Resolving the parent folds `..` and symlinks, so the check below sees
    // where the file really lands.
    let parent = parent.canonicalize().map_err(|_| {
        McError::not_found(
            format!("the folder for '{output}' does not exist"),
            Some("write into an existing folder of the repository".into()),
        )
    })?;
    let path = parent.join(name);
    let inside = path
        .strip_prefix(&root)
        .is_ok_and(|rel| visible_in_repo(rel, cfg));
    if !inside {
        return Err(usage(format!(
            "output '{output}' is outside the repository or hidden"
        )));
    }
    if let Ok(meta) = path.symlink_metadata() {
        let is_pdf = || {
            use std::io::Read;
            let mut head = [0u8; 5];
            std::fs::File::open(&path)
                .and_then(|mut f| f.read_exact(&mut head))
                .is_ok_and(|()| &head == b"%PDF-")
        };
        if meta.file_type().is_symlink() || !meta.is_file() || !is_pdf() {
            return Err(McError::conflict(
                format!("'{output}' exists and is not a PDF; refusing to overwrite it"),
                Some("choose another output path".into()),
            ));
        }
    }
    Ok(path)
}

/// Whether a repo-relative path has only visible components (the `.mc`
/// folder of an embedded repo excepted), like the dashboard's `/files`.
fn visible_in_repo(rel: &Path, cfg: &ResolvedConfig) -> bool {
    rel.components().enumerate().all(|(i, c)| match c {
        Component::Normal(s) => {
            let s = s.to_string_lossy();
            !s.starts_with('.') || (i == 0 && s == ".mc" && cfg.mode == RepoMode::Embedded)
        }
        _ => false,
    })
}

/// Render the document and write it to `path` in one step, so a failed
/// render never leaves a truncated file behind.
fn write_pdf(doc: genpdf::Document, path: &Path) -> McResult<()> {
    let mut buf = Vec::new();
    doc.render(&mut buf)
        .map_err(|e| McError::Pdf(format!("Failed to render PDF: {e}")))?;
    util::atomic_write(path, &buf)
        .map_err(|e| McError::Pdf(format!("Failed to write {}: {e}", path.display())))
}

static OBSIDIAN_COMMENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)%%.*?%%").expect("static regex is valid"));
static WIKILINK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[\[([^\[\]|]+)(?:\|([^\[\]]*))?\]\]").expect("static regex is valid")
});

/// Text for a reference: `[[target|Alias]]` → `Alias`; `[[CUST-001]]` or
/// `CUST-001` → the entity's name when it's known; otherwise the target.
fn link_text(raw: &str, catalog: Option<&Catalog>) -> String {
    let raw = raw.trim();
    let (target, alias) = match WIKILINK_RE.captures(raw) {
        Some(c) if c.get(0).is_some_and(|m| m.len() == raw.len()) => (
            c.get(1).map_or("", |m| m.as_str().trim()),
            c.get(2)
                .map(|m| m.as_str().trim())
                .filter(|a| !a.is_empty()),
        ),
        _ => (raw, None),
    };
    if let Some(alias) = alias {
        return alias.to_string();
    }
    catalog
        .and_then(|c| c.canonical_id(target).and_then(|id| c.name(id)))
        .unwrap_or(target)
        .to_string()
}

/// Notes as a reader should see them: Obsidian `%% comments %%` (like the
/// `%% mc-links %%` footer) dropped and wiki-links shown as plain text.
fn printable_markdown(body: &str, catalog: Option<&Catalog>) -> String {
    let body = OBSIDIAN_COMMENT_RE.replace_all(body, "");
    WIKILINK_RE
        .replace_all(&body, |c: &regex::Captures| link_text(&c[0], catalog))
        .into_owned()
}

/// Cover-page metadata for a print template, from frontmatter. Empty values
/// are skipped when the cover page is rendered.
fn meta_pairs(
    template: &PrintTemplate,
    fm: &serde_yaml::Value,
    catalog: Option<&Catalog>,
) -> Vec<(&'static str, String)> {
    let get = |key: &str| frontmatter::get_str(fm, key).unwrap_or("").to_string();
    let joined = |key: &str| frontmatter::get_string_list(fm, key).join(", ");
    let links = |key: &str| {
        frontmatter::get_string_list(fm, key)
            .iter()
            .map(|l| link_text(l, catalog))
            .collect::<Vec<_>>()
            .join(", ")
    };

    match template {
        PrintTemplate::Standard => {
            let author = frontmatter::get_str(fm, "author")
                .or_else(|| frontmatter::get_str(fm, "owner"))
                .unwrap_or("")
                .to_string();
            vec![
                ("Date", get("date")),
                ("Author", author),
                ("Status", get("status")),
                ("Tags", joined("tags")),
            ]
        }
        PrintTemplate::Meeting => {
            let participants: Vec<String> = get_attendees(fm, catalog)
                .into_iter()
                .map(|a| a.name)
                .collect();
            vec![
                ("Date", get("date")),
                ("Time", get("time")),
                ("Duration", get("duration")),
                ("Status", get("status")),
                ("Participants", participants.join(", ")),
                ("Customers", links("customers")),
                ("Projects", links("projects")),
            ]
        }
        PrintTemplate::Research => vec![
            ("Owner", get("owner")),
            ("Status", get("status")),
            ("Tags", joined("tags")),
            ("Agents", joined("agents")),
        ],
        PrintTemplate::Sprint => vec![
            ("Owner", get("owner")),
            ("Status", get("status")),
            ("Goal", get("goal")),
            ("Start Date", get("start_date")),
            ("End Date", get("end_date")),
            ("Projects", links("projects")),
            ("Tags", joined("tags")),
        ],
    }
}

/// "Summary" section from the frontmatter `summary` field, if present.
fn push_summary(
    doc: &mut genpdf::Document,
    fm: &serde_yaml::Value,
    pc: style::Color,
    catalog: Option<&Catalog>,
) {
    let summary = printable_markdown(frontmatter::get_str(fm, "summary").unwrap_or(""), catalog);
    let summary = summary.trim();
    if summary.is_empty() {
        return;
    }
    doc.push(elements::Paragraph::new(style::StyledString::new(
        "Summary",
        style::Style::new()
            .bold()
            .with_font_size(H2_SIZE)
            .with_color(pc),
    )));
    doc.push(elements::Break::new(0.3));
    doc.push(elements::Paragraph::new(summary));
    push_section_separator(doc);
}

/// Print a meeting for MCP: the output path is relative to the repo root and
/// must stay inside it. Returns `{id, title, path}` (repo-relative path).
pub fn print_meeting_programmatic(
    cfg: &ResolvedConfig,
    id: &str,
    output: Option<&str>,
) -> McResult<JsonValue> {
    Ok(meeting_pdf(cfg, id, output, Access::Repo)?.to_json(cfg))
}

fn meeting_pdf(
    cfg: &ResolvedConfig,
    id: &str,
    output: Option<&str>,
    access: Access,
) -> McResult<Printed> {
    let out_path = output_path(cfg, output, format!("{id}.pdf"), access)?;
    let entity = data::find_entity_by_id(id, cfg)?;
    if entity.kind != EntityKind::Meeting {
        return Err(McError::usage(
            format!("{} is a {}, not a meeting", id, entity.kind.label()),
            None,
        ));
    }

    let fm = &entity.frontmatter;
    let title = frontmatter::get_str(fm, "title")
        .or_else(|| frontmatter::get_str(fm, "name"))
        .unwrap_or("Untitled Meeting");

    let font_family = load_fonts(cfg)?;
    let pc = primary_color(cfg);
    let ac = accent_color(cfg);
    let mut doc = create_document(font_family, title, &cfg.brand.name, pc, ac);
    let catalog = Catalog::load(cfg);
    let attendees = get_attendees(fm, Some(&catalog));
    let meta_pairs = meta_pairs(&PrintTemplate::Meeting, fm, Some(&catalog));

    // Cover page
    push_cover_page(
        &mut doc,
        &cfg.brand.name,
        &cfg.brand.tagline,
        "Meeting Notes",
        title,
        &meta_pairs,
        pc,
        ac,
    );

    // Attendees table (page 2+)
    if !attendees.is_empty() {
        doc.push(elements::Paragraph::new(style::StyledString::new(
            "Attendees",
            style::Style::new()
                .bold()
                .with_font_size(H2_SIZE)
                .with_color(pc),
        )));
        doc.push(elements::Break::new(0.3));

        let mut att_table = elements::TableLayout::new(vec![3, 3, 2]);
        att_table.set_cell_decorator(elements::FrameCellDecorator::new(true, true, false));

        // Header row
        {
            use genpdf::Element;
            let cell_pad = genpdf::Margins::trbl(1u8, 1u8, 1u8, 1u8);
            att_table
                .row()
                .element(
                    elements::Paragraph::new(style::StyledString::new(
                        "Name",
                        style::Style::new().bold(),
                    ))
                    .padded(cell_pad),
                )
                .element(
                    elements::Paragraph::new(style::StyledString::new(
                        "Role",
                        style::Style::new().bold(),
                    ))
                    .padded(cell_pad),
                )
                .element(
                    elements::Paragraph::new(style::StyledString::new(
                        "Company",
                        style::Style::new().bold(),
                    ))
                    .padded(cell_pad),
                )
                .push()
                .ok();
        }

        for att in &attendees {
            use genpdf::Element;
            let cell_pad = genpdf::Margins::trbl(1u8, 1u8, 1u8, 1u8);
            att_table
                .row()
                .element(elements::Paragraph::new(&*att.name).padded(cell_pad))
                .element(elements::Paragraph::new(&*att.role).padded(cell_pad))
                .element(elements::Paragraph::new(&*att.company).padded(cell_pad))
                .push()
                .ok();
        }

        doc.push(att_table);
        push_section_separator(&mut doc);
    }

    // Body content
    render_markdown(
        &mut doc,
        &printable_markdown(&entity.body, Some(&catalog)),
        pc,
    );

    // Footer
    push_document_footer(&mut doc);

    write_pdf(doc, &out_path)?;
    Ok(Printed {
        id: Some(id.to_string()),
        title: title.to_string(),
        path: out_path,
    })
}

// ---------------------------------------------------------------------------
// Research PDF
// ---------------------------------------------------------------------------

/// Print a research report for MCP: the output path is relative to the repo
/// root and must stay inside it. Returns `{id, title, path}`.
pub fn print_research_programmatic(
    cfg: &ResolvedConfig,
    id: &str,
    output: Option<&str>,
    file: Option<&str>,
) -> McResult<JsonValue> {
    Ok(research_pdf(cfg, id, output, file, Access::Repo)?.to_json(cfg))
}

fn research_pdf(
    cfg: &ResolvedConfig,
    id: &str,
    output: Option<&str>,
    file: Option<&str>,
    access: Access,
) -> McResult<Printed> {
    let out_path = output_path(cfg, output, format!("{id}-final-report.pdf"), access)?;
    let entity = data::find_entity_by_id(id, cfg)?;
    if entity.kind != EntityKind::Research {
        return Err(McError::usage(
            format!("{} is a {}, not a research topic", id, entity.kind.label()),
            None,
        ));
    }

    let fm = &entity.frontmatter;
    let title = frontmatter::get_str(fm, "title")
        .or_else(|| frontmatter::get_str(fm, "name"))
        .unwrap_or("Untitled Research");
    let font_family = load_fonts(cfg)?;
    let pc = primary_color(cfg);
    let ac = accent_color(cfg);
    let mut doc = create_document(font_family, title, &cfg.brand.name, pc, ac);
    let catalog = Catalog::load(cfg);
    let meta_pairs = meta_pairs(&PrintTemplate::Research, fm, Some(&catalog));

    // Cover page
    push_cover_page(
        &mut doc,
        &cfg.brand.name,
        &cfg.brand.tagline,
        "Research Report",
        title,
        &meta_pairs,
        pc,
        ac,
    );

    // Summary (page 2+)
    push_summary(&mut doc, fm, pc, Some(&catalog));

    // Find final/ directory
    let source_dir = entity
        .source_path
        .parent()
        .ok_or_else(|| McError::Other("Cannot determine research directory".into()))?;
    let final_dir = source_dir.join("final");

    let mut report_files = collect_report_files(&final_dir, file);

    if report_files.is_empty() {
        if let Some(f) = file {
            return Err(McError::not_found(
                format!("File '{}' not found in {}/final/", f, id),
                None,
            ));
        }
    }

    if report_files.is_empty() {
        // Fall back to _index.md body
        eprintln!(
            "{} No files in final/ directory, using entity file body.",
            "warning:".yellow().bold()
        );
        render_markdown(
            &mut doc,
            &printable_markdown(&entity.body, Some(&catalog)),
            pc,
        );
    } else {
        report_files.sort();
        let total = report_files.len();
        for (i, path) in report_files.iter().enumerate() {
            let content = std::fs::read_to_string(path)?;
            let body = match frontmatter::split_frontmatter(&content) {
                Some((_, body)) => body,
                None => content.clone(),
            };

            // Section header with filename
            if total > 1 {
                let fname = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                doc.push(elements::Paragraph::new(style::StyledString::new(
                    fname,
                    style::Style::new()
                        .bold()
                        .with_font_size(H1_SIZE)
                        .with_color(pc),
                )));
                doc.push(elements::Break::new(0.3));
            }

            render_markdown(&mut doc, &printable_markdown(&body, Some(&catalog)), pc);

            if i + 1 < total {
                push_section_separator(&mut doc);
                doc.push(elements::PageBreak::new());
            }
        }
    }

    // Footer
    push_document_footer(&mut doc);

    write_pdf(doc, &out_path)?;
    Ok(Printed {
        id: Some(id.to_string()),
        title: title.to_string(),
        path: out_path,
    })
}

// ---------------------------------------------------------------------------
// File PDF (generic markdown file)
// ---------------------------------------------------------------------------

/// Print a Markdown file for MCP: `path` must be a visible `.md` file inside
/// the repo and the output stays inside it too. Returns `{title, path}`.
pub fn print_file_programmatic(
    cfg: &ResolvedConfig,
    path: &str,
    output: Option<&str>,
    template: &PrintTemplate,
    title_override: Option<&str>,
) -> McResult<JsonValue> {
    Ok(file_pdf(cfg, path, output, template, title_override, Access::Repo)?.to_json(cfg))
}

fn file_pdf(
    cfg: &ResolvedConfig,
    path: &str,
    output: Option<&str>,
    template: &PrintTemplate,
    title_override: Option<&str>,
    access: Access,
) -> McResult<Printed> {
    let file_path = match access {
        Access::Cli => resolve_input_path(cfg, path)
            .ok_or_else(|| McError::not_found(format!("File not found: {}", path), None))?,
        Access::Repo => repo_input_path(cfg, path)?,
    };
    let stem = file_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let out_path = output_path(cfg, output, format!("{stem}.pdf"), access)?;

    let content = std::fs::read_to_string(&file_path)?;

    // Split frontmatter if present and parse YAML
    let (fm, body) = match frontmatter::split_frontmatter(&content) {
        Some((fm_str, body)) => {
            let parsed = frontmatter::parse_raw(&fm_str, &file_path).ok();
            (parsed, body)
        }
        None => (None, content.clone()),
    };

    // Determine title: flag > frontmatter > first H1 > filename
    let title = if let Some(t) = title_override {
        t.to_string()
    } else if let Some(ref fm) = fm {
        if let Some(t) =
            frontmatter::get_str(fm, "title").or_else(|| frontmatter::get_str(fm, "name"))
        {
            t.to_string()
        } else {
            detect_title_from_body(&body, &file_path)
        }
    } else {
        detect_title_from_body(&body, &file_path)
    };

    // Build metadata pairs based on template
    let entity_type_label = match template {
        PrintTemplate::Standard => "Document",
        PrintTemplate::Meeting => "Meeting Notes",
        PrintTemplate::Research => "Research Report",
        PrintTemplate::Sprint => "Sprint Report",
    };

    let catalog = Catalog::load(cfg);
    let meta_pairs = fm
        .as_ref()
        .map(|fm| meta_pairs(template, fm, Some(&catalog)))
        .unwrap_or_default();

    let font_family = load_fonts(cfg)?;
    let pc = primary_color(cfg);
    let ac = accent_color(cfg);
    let mut doc = create_document(font_family, &title, &cfg.brand.name, pc, ac);

    // Cover page
    push_cover_page(
        &mut doc,
        &cfg.brand.name,
        &cfg.brand.tagline,
        entity_type_label,
        &title,
        &meta_pairs,
        pc,
        ac,
    );

    // For research template: render summary before body if present
    if let (PrintTemplate::Research, Some(fm)) = (template, &fm) {
        push_summary(&mut doc, fm, pc, Some(&catalog));
    }

    // Body content
    render_markdown(&mut doc, &printable_markdown(&body, Some(&catalog)), pc);

    // Footer
    push_document_footer(&mut doc);

    write_pdf(doc, &out_path)?;
    Ok(Printed {
        id: None,
        title,
        path: out_path,
    })
}

/// A Markdown file named by an MCP caller: relative to the repo root (an
/// absolute path must point inside it), visible, and ending in `.md`.
fn repo_input_path(cfg: &ResolvedConfig, path: &str) -> McResult<PathBuf> {
    let root = cfg.root.canonicalize()?;
    let file = root
        .join(path)
        .canonicalize()
        .ok()
        .filter(|f| f.is_file())
        .ok_or_else(|| McError::not_found(format!("File not found: {path}"), None))?;
    let allowed = file
        .strip_prefix(&root)
        .is_ok_and(|rel| visible_in_repo(rel, cfg))
        && file
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("md"));
    if !allowed {
        return Err(McError::usage(
            format!("'{path}' is not a Markdown file inside the repository"),
            Some("give the path of a visible .md file relative to the repo root".into()),
        ));
    }
    Ok(file)
}

/// Resolve a user-supplied markdown path: as given (absolute or relative to
/// the working directory), else relative to the repo root.
fn resolve_input_path(cfg: &ResolvedConfig, path: &str) -> Option<PathBuf> {
    let given = PathBuf::from(path);
    if given.is_file() {
        return Some(given);
    }
    let in_repo = cfg.root.join(path);
    (given.is_relative() && in_repo.is_file()).then_some(in_repo)
}

/// Extract title from the first H1 heading in the markdown body, or fall back to filename.
fn detect_title_from_body(body: &str, file_path: &Path) -> String {
    let opts = Options::empty();
    let parser = Parser::new_ext(body, opts);
    let mut in_heading = false;

    for event in parser {
        match event {
            Event::Start(Tag::Heading {
                level: HeadingLevel::H1,
                ..
            }) => {
                in_heading = true;
            }
            Event::Text(text) if in_heading => {
                return text.to_string();
            }
            Event::End(TagEnd::Heading(_)) => {
                in_heading = false;
            }
            _ => {}
        }
    }

    // Fallback: filename stem
    file_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Thin gray horizontal rule (line of ─ characters).
fn push_horizontal_rule(doc: &mut genpdf::Document) {
    let rule_text = "─".repeat(90);
    doc.push(
        elements::Paragraph::new(style::StyledString::new(
            rule_text,
            style::Style::new()
                .with_font_size(4)
                .with_color(style::Color::Rgb(180, 180, 180)),
        ))
        .aligned(Alignment::Center),
    );
}

/// Vertical spacing between sections.
fn push_section_separator(doc: &mut genpdf::Document) {
    doc.push(elements::Break::new(1.5));
}

/// Styled metadata row: label in small gray caps, value in normal body size.
fn push_meta_row_styled(
    doc: &mut genpdf::Document,
    label: &str,
    value: &str,
    accent: style::Color,
) {
    let mut table = elements::TableLayout::new(vec![1, 3]);
    table
        .row()
        .element(elements::Paragraph::new(style::StyledString::new(
            label.to_uppercase(),
            style::Style::new()
                .with_font_size(META_LABEL_SIZE)
                .with_color(accent),
        )))
        .element(elements::Paragraph::new(style::StyledString::new(
            value.to_string(),
            style::Style::new().with_font_size(COVER_META_SIZE),
        )))
        .push()
        .ok();
    doc.push(table);
}

/// Footer at the end of the document: horizontal rule + generation line.
fn push_document_footer(doc: &mut genpdf::Document) {
    doc.push(elements::Break::new(1.0));
    push_horizontal_rule(doc);
    doc.push(elements::Break::new(0.3));
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    doc.push(
        elements::Paragraph::new(style::StyledString::new(
            format!("Generated by MissionControl · {}", date),
            style::Style::new()
                .with_font_size(FOOTER_SIZE)
                .with_color(style::Color::Rgb(140, 140, 140)),
        ))
        .aligned(Alignment::Center),
    );
}

/// Branded cover page for meeting/research PDFs.
#[allow(clippy::too_many_arguments)]
fn push_cover_page(
    doc: &mut genpdf::Document,
    brand: &str,
    tagline: &str,
    entity_type_label: &str,
    title: &str,
    meta_pairs: &[(&str, String)],
    primary_color: style::Color,
    accent_color: style::Color,
) {
    // Large vertical spacer (~80mm from top)
    for _ in 0..8 {
        doc.push(elements::Break::new(2.5));
    }

    // Brand name
    doc.push(
        elements::Paragraph::new(style::StyledString::new(
            brand.to_string(),
            style::Style::new()
                .bold()
                .with_font_size(COVER_BRAND_SIZE)
                .with_color(primary_color),
        ))
        .aligned(Alignment::Left),
    );

    // Tagline
    if !tagline.is_empty() {
        doc.push(
            elements::Paragraph::new(style::StyledString::new(
                tagline.to_string(),
                style::Style::new()
                    .italic()
                    .with_font_size(COVER_TAGLINE_SIZE)
                    .with_color(accent_color),
            ))
            .aligned(Alignment::Left),
        );
    }

    doc.push(elements::Break::new(0.5));
    push_horizontal_rule(doc);
    doc.push(elements::Break::new(0.8));

    // Entity type label (e.g., "MEETING NOTES")
    doc.push(
        elements::Paragraph::new(style::StyledString::new(
            entity_type_label.to_uppercase(),
            style::Style::new()
                .with_font_size(COVER_LABEL_SIZE)
                .with_color(accent_color),
        ))
        .aligned(Alignment::Left),
    );
    doc.push(elements::Break::new(3.0));

    // Title – set font_size on the element style so genpdf computes correct
    // line height for multi-line wrapping (StyledString font_size alone is
    // not used for line-height calculation).
    {
        use genpdf::Element;
        doc.push(
            elements::Paragraph::new(style::StyledString::new(
                title.to_string(),
                style::Style::new().bold(),
            ))
            .aligned(Alignment::Left)
            .styled(
                style::Style::new()
                    .with_font_size(COVER_TITLE_SIZE)
                    .with_line_spacing(1.15),
            ),
        );
    }
    doc.push(elements::Break::new(1.0));

    // Metadata pairs
    for (label, value) in meta_pairs {
        if !value.is_empty() {
            push_meta_row_styled(doc, label, value, accent_color);
        }
    }

    // Page break to start content on page 2
    doc.push(elements::PageBreak::new());
}

struct Attendee {
    name: String,
    role: String,
    company: String,
}

/// Extract attendees from frontmatter. Supports:
///   attendees:
///     - name: Alice
///       role: Engineer
///       company: Acme
///   OR
///     - Alice (Engineer, Acme)
///
/// Names may be wiki-links (`[[CONT-003-jane|Jane Doe]]`), shown as text.
fn get_attendees(fm: &serde_yaml::Value, catalog: Option<&Catalog>) -> Vec<Attendee> {
    let seq = fm
        .as_mapping()
        .and_then(|m| m.get(serde_yaml::Value::String("attendees".into())))
        .and_then(|v| v.as_sequence());

    let Some(seq) = seq else {
        return Vec::new();
    };

    seq.iter()
        .filter_map(|item| {
            if let Some(map) = item.as_mapping() {
                let name = link_text(
                    map.get(serde_yaml::Value::String("name".into()))
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                    catalog,
                );
                let role = map
                    .get(serde_yaml::Value::String("role".into()))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let company = map
                    .get(serde_yaml::Value::String("company".into()))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(Attendee {
                    name,
                    role,
                    company,
                })
            } else {
                item.as_str().map(|s| Attendee {
                    name: link_text(s, catalog),
                    role: String::new(),
                    company: String::new(),
                })
            }
        })
        .collect()
}

fn collect_report_files(final_dir: &Path, specific_file: Option<&str>) -> Vec<PathBuf> {
    if !final_dir.is_dir() {
        return Vec::new();
    }

    let Ok(entries) = std::fs::read_dir(final_dir) else {
        return Vec::new();
    };

    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "md"))
        .filter(|p| {
            if let Some(target) = specific_file {
                p.file_name()
                    .is_some_and(|name| name.to_string_lossy().contains(target))
            } else {
                true
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Markdown → genpdf rendering
// ---------------------------------------------------------------------------

/// A list being built; nested lists are pushed into their parent item.
enum ListBuilder {
    Ordered(elements::OrderedList),
    Unordered(elements::UnorderedList),
}

impl ListBuilder {
    fn new(ordered: bool) -> Self {
        if ordered {
            ListBuilder::Ordered(elements::OrderedList::new())
        } else {
            ListBuilder::Unordered(elements::UnorderedList::new())
        }
    }

    fn push_item(&mut self, item: elements::LinearLayout) {
        match self {
            ListBuilder::Ordered(l) => l.push(item),
            ListBuilder::Unordered(l) => l.push(item),
        }
    }

    fn push_into_layout(self, layout: &mut elements::LinearLayout) {
        match self {
            ListBuilder::Ordered(l) => layout.push(l),
            ListBuilder::Unordered(l) => layout.push(l),
        }
    }

    fn push_into_doc(self, doc: &mut genpdf::Document) {
        match self {
            ListBuilder::Ordered(l) => doc.push(l),
            ListBuilder::Unordered(l) => doc.push(l),
        }
    }
}

/// A list item being built: its text so far plus any nested lists.
struct ItemBuilder {
    layout: elements::LinearLayout,
    text: elements::Paragraph,
    has_text: bool,
}

impl ItemBuilder {
    fn new() -> Self {
        Self {
            layout: elements::LinearLayout::vertical(),
            text: elements::Paragraph::default(),
            has_text: false,
        }
    }

    /// Move the pending text into the layout (before a nested list or at the end).
    fn flush_text(&mut self) {
        if self.has_text {
            self.layout.push(std::mem::take(&mut self.text));
            self.has_text = false;
        }
    }

    fn finish(mut self) -> elements::LinearLayout {
        self.flush_text();
        self.layout
    }
}

fn render_markdown(doc: &mut genpdf::Document, markdown: &str, heading_color: style::Color) {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(markdown, opts);

    let mut current_paragraph = elements::Paragraph::default();
    let mut has_text = false;
    let mut in_heading = false;
    let mut heading_level = HeadingLevel::H1;
    let mut in_strong = false;
    let mut in_emphasis = false;
    let mut in_code = false;
    let mut lists: Vec<ListBuilder> = Vec::new();
    let mut items: Vec<ItemBuilder> = Vec::new();
    let mut in_table = false;
    let mut table_cols: usize = 0;
    let mut table_cell = String::new();
    let mut table_row_cells: Vec<String> = Vec::new();
    let mut table_rows: Vec<(Vec<String>, bool)> = Vec::new();

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                flush_paragraph(doc, &mut current_paragraph, &mut has_text);
                // Add breathing room before H2/H3 headings
                if matches!(level, HeadingLevel::H2 | HeadingLevel::H3) {
                    doc.push(elements::Break::new(2.5));
                }
                in_heading = true;
                heading_level = level;
            }
            Event::End(TagEnd::Heading(_)) => {
                in_heading = false;
                // current_paragraph already has the text with heading style
                flush_paragraph(doc, &mut current_paragraph, &mut has_text);
                doc.push(elements::Break::new(0.5));
            }
            Event::Start(Tag::Paragraph) => {
                if items.is_empty() {
                    current_paragraph = elements::Paragraph::default();
                    has_text = false;
                }
            }
            Event::End(TagEnd::Paragraph) => match items.last_mut() {
                // Loose lists: each paragraph of an item on its own line.
                Some(item) => item.flush_text(),
                None => {
                    flush_paragraph(doc, &mut current_paragraph, &mut has_text);
                    doc.push(elements::Break::new(0.6));
                }
            },
            Event::Start(Tag::Strong) => in_strong = true,
            Event::End(TagEnd::Strong) => in_strong = false,
            Event::Start(Tag::Emphasis) => in_emphasis = true,
            Event::End(TagEnd::Emphasis) => in_emphasis = false,
            Event::Start(Tag::CodeBlock(_)) => {
                flush_paragraph(doc, &mut current_paragraph, &mut has_text);
                in_code = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                // Flush code block with left padding
                if has_text {
                    use genpdf::Element;
                    doc.push(
                        std::mem::take(&mut current_paragraph)
                            .padded(genpdf::Margins::trbl(0u8, 0u8, 0u8, 2u8)),
                    );
                    has_text = false;
                }
                current_paragraph = elements::Paragraph::default();
                doc.push(elements::Break::new(0.3));
            }
            Event::Code(text) => {
                let s = text.to_string();
                if in_table {
                    table_cell.push_str(&s);
                } else if in_heading {
                    let size = heading_font_size(heading_level);
                    current_paragraph.push_styled(
                        s,
                        style::Style::new()
                            .with_font_size(size)
                            .with_color(heading_color),
                    );
                    has_text = true;
                } else if let Some(item) = items.last_mut() {
                    item.text.push_styled(s, style::Style::new().italic());
                    item.has_text = true;
                } else {
                    current_paragraph.push_styled(s, style::Style::new().italic());
                    has_text = true;
                }
            }
            Event::Start(Tag::List(first_number)) => {
                if let Some(item) = items.last_mut() {
                    item.flush_text();
                } else {
                    flush_paragraph(doc, &mut current_paragraph, &mut has_text);
                }
                lists.push(ListBuilder::new(first_number.is_some()));
            }
            Event::End(TagEnd::List(_)) => {
                if let Some(list) = lists.pop() {
                    match items.last_mut() {
                        // Nested list: becomes part of the enclosing item.
                        Some(parent) => list.push_into_layout(&mut parent.layout),
                        None => {
                            list.push_into_doc(doc);
                            doc.push(elements::Break::new(0.2));
                        }
                    }
                }
            }
            Event::Start(Tag::Item) => items.push(ItemBuilder::new()),
            Event::End(TagEnd::Item) => {
                if let (Some(item), Some(list)) = (items.pop(), lists.last_mut()) {
                    list.push_item(item.finish());
                }
            }
            Event::Start(Tag::Table(alignments)) => {
                flush_paragraph(doc, &mut current_paragraph, &mut has_text);
                in_table = true;
                table_cols = alignments.len();
                table_rows.clear();
            }
            Event::End(TagEnd::Table) => {
                render_table(doc, &table_rows, table_cols);
                table_rows.clear();
                in_table = false;
                doc.push(elements::Break::new(0.3));
            }
            Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => {
                table_row_cells.clear();
            }
            Event::End(TagEnd::TableHead) => {
                table_rows.push((std::mem::take(&mut table_row_cells), true));
            }
            Event::End(TagEnd::TableRow) => {
                table_rows.push((std::mem::take(&mut table_row_cells), false));
            }
            // A cell may contain several text/code events (e.g. `**bold** rest`);
            // collect them so every cell stays in its column.
            Event::Start(Tag::TableCell) => table_cell.clear(),
            Event::End(TagEnd::TableCell) => {
                table_row_cells.push(std::mem::take(&mut table_cell));
            }
            Event::Text(text) => {
                let s = text.to_string();
                if in_table {
                    table_cell.push_str(&s);
                } else if in_heading {
                    let size = heading_font_size(heading_level);
                    let st = style::Style::new()
                        .bold()
                        .with_font_size(size)
                        .with_color(heading_color);
                    current_paragraph.push_styled(s, st);
                    has_text = true;
                } else if in_code {
                    // Code block text — smaller font with distinct color
                    current_paragraph.push_styled(
                        s,
                        style::Style::new()
                            .with_font_size(CODE_BLOCK_SIZE)
                            .with_color(style::Color::Rgb(60, 60, 60)),
                    );
                    has_text = true;
                } else if let Some(item) = items.last_mut() {
                    item.text.push_styled(s, text_style(in_strong, in_emphasis));
                    item.has_text = true;
                } else {
                    current_paragraph.push_styled(s, text_style(in_strong, in_emphasis));
                    has_text = true;
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if in_table {
                    table_cell.push(' ');
                } else if let Some(item) = items.last_mut() {
                    item.text.push(" ");
                } else {
                    current_paragraph.push(" ");
                }
            }
            Event::Rule => {
                flush_paragraph(doc, &mut current_paragraph, &mut has_text);
                doc.push(elements::Break::new(0.3));
                push_horizontal_rule(doc);
                doc.push(elements::Break::new(0.3));
            }
            Event::TaskListMarker(checked) => {
                if let Some(item) = items.last_mut() {
                    let marker = if checked { "✓ " } else { "○ " };
                    let color = if checked {
                        style::Color::Rgb(0, 128, 0)
                    } else {
                        style::Color::Rgb(160, 160, 160)
                    };
                    item.text
                        .push_styled(marker, style::Style::new().bold().with_color(color));
                    item.has_text = true;
                }
            }
            _ => {}
        }
    }

    // Close anything left open by malformed input, innermost first.
    while let Some(list) = lists.pop() {
        if let Some(item) = items.pop() {
            let mut list = list;
            list.push_item(item.finish());
            list.push_into_doc(doc);
        } else {
            list.push_into_doc(doc);
        }
    }
    flush_paragraph(doc, &mut current_paragraph, &mut has_text);
}

fn text_style(bold: bool, italic: bool) -> style::Style {
    let mut st = style::Style::new();
    if bold {
        st = st.bold();
    }
    if italic {
        st = st.italic();
    }
    st
}

fn heading_font_size(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => H1_SIZE,
        HeadingLevel::H2 => H2_SIZE,
        HeadingLevel::H3 => H3_SIZE,
        _ => BODY_SIZE,
    }
}

fn flush_paragraph(
    doc: &mut genpdf::Document,
    para: &mut elements::Paragraph,
    has_text: &mut bool,
) {
    if *has_text {
        doc.push(std::mem::take(para));
        *has_text = false;
    }
    *para = elements::Paragraph::default();
}

fn render_table(doc: &mut genpdf::Document, rows: &[(Vec<String>, bool)], num_cols: usize) {
    if num_cols == 0 || rows.is_empty() {
        return;
    }

    let weights: Vec<usize> = vec![1; num_cols];
    let mut table = elements::TableLayout::new(weights);
    table.set_cell_decorator(elements::FrameCellDecorator::new(true, true, false));

    for (cells, is_header) in rows {
        use genpdf::Element;
        let cell_pad = genpdf::Margins::trbl(1u8, 1u8, 1u8, 1u8);
        let mut row = table.row();
        for i in 0..num_cols {
            let text = cells.get(i).map(|s| s.as_str()).unwrap_or("");
            if *is_header {
                row.push_element(
                    elements::Paragraph::new(style::StyledString::new(
                        text.to_string(),
                        style::Style::new().bold(),
                    ))
                    .padded(cell_pad),
                );
            } else {
                row.push_element(elements::Paragraph::new(text).padded(cell_pad));
            }
        }
        row.push().ok();
    }

    doc.push(table);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{init, new};
    use crate::config;
    use tempfile::TempDir;

    const RICH_MARKDOWN: &str = "# Title\n\nIntro with **bold** and `code`.\n\n\
        - Topic A\n  - detail 1\n  - detail 2\n    1. deep\n- Topic B\n\n\
        - [x] done item\n- [ ] open item\n\n\
        | Field | Value |\n|---|---|\n| **Owner** | Alice `ops` |\n| Plain | x |\n\n\
        ```\nfn main() {}\n```\n\n---\n\nEnd.\n";

    fn setup() -> (TempDir, config::ResolvedConfig) {
        let tmp = TempDir::new().unwrap();
        init::run(tmp.path(), false, false, Some("PrintCo"), false, true).unwrap();
        let cfg = config::load_config(tmp.path(), config::RepoMode::Standalone).unwrap();
        (tmp, cfg)
    }

    #[test]
    fn test_meta_pairs_strip_wikilinks() {
        let fm = frontmatter::parse_raw(
            "date: 2026-01-05\nstatus: scheduled\ncustomers: ['[[CUST-001]]']\nprojects: ['[[PROJ-002]]', PROJ-003]\nattendees: [Alice, {name: Bob, role: CTO}]",
            Path::new("m.md"),
        )
        .unwrap();
        let pairs = meta_pairs(&PrintTemplate::Meeting, &fm, None);
        let get = |k: &str| pairs.iter().find(|(l, _)| *l == k).map(|(_, v)| v.as_str());
        assert_eq!(get("Customers"), Some("CUST-001"));
        assert_eq!(get("Projects"), Some("PROJ-002, PROJ-003"));
        assert_eq!(get("Participants"), Some("Alice, Bob"));
        assert_eq!(get("Time"), Some(""));
    }

    #[test]
    fn links_and_obsidian_comments_print_as_plain_text() {
        let fm = frontmatter::parse_raw(
            "attendees: ['[[florian-fromm|Florian Fromm]]', '[[CONT-003]]', Plain Name, {name: '[[x|Bob]]'}]\ncustomers: ['[[CUST-004|Villeroy & Boch]]']",
            Path::new("m.md"),
        )
        .unwrap();
        let pairs = meta_pairs(&PrintTemplate::Meeting, &fm, None);
        let get = |k: &str| pairs.iter().find(|(l, _)| *l == k).map(|(_, v)| v.as_str());
        assert_eq!(
            get("Participants"),
            Some("Florian Fromm, CONT-003, Plain Name, Bob")
        );
        assert_eq!(get("Customers"), Some("Villeroy & Boch"));

        let body = "Notes on [[PROJ-001|the project]] and [[TASK-002]].\n\n%% private\nnote %%\nEnd.\n%% mc-links: [[CUST-004|Villeroy & Boch]] [[PROJ-001]] %%\n";
        let out = printable_markdown(body, None);
        assert_eq!(out, "Notes on the project and TASK-002.\n\n\nEnd.\n\n");
    }

    #[test]
    fn link_text_uses_entity_names_from_the_catalog() {
        let (_tmp, cfg) = setup();
        new::create_customer(&cfg, &new::CustomerInput::new("Acme")).unwrap();
        let mut input = new::MeetingInput::new("Kickoff");
        input.customers = vec!["CUST-001".into()];
        new::create_meeting(&cfg, &input).unwrap();
        let catalog = Catalog::load(&cfg);
        assert_eq!(link_text("[[MTG-001]]", Some(&catalog)), "Kickoff");
        assert_eq!(link_text("MTG-001", Some(&catalog)), "Kickoff");
        assert_eq!(link_text("[[MTG-001|Alias]]", Some(&catalog)), "Alias");
        assert_eq!(link_text("[[CUST-009]]", Some(&catalog)), "CUST-009");
    }

    #[test]
    fn mcp_prints_stay_inside_the_repo() {
        let (tmp, cfg) = setup();
        let outside = TempDir::new().unwrap();
        let secret = outside.path().join("secret.md");
        std::fs::write(&secret, "# Secret\n").unwrap();
        let victim = outside.path().join("victim.txt");
        std::fs::write(&victim, "precious\n").unwrap();
        std::fs::write(tmp.path().join("notes.md"), "# Notes\n").unwrap();
        std::fs::write(tmp.path().join("notes.txt"), "plain\n").unwrap();
        let std = &PrintTemplate::Standard;
        let print = |path: &str, output: Option<&str>| {
            print_file_programmatic(&cfg, path, output, std, None).unwrap_err()
        };

        // Inputs: outside the root, hidden, not Markdown.
        assert!(matches!(
            print(secret.to_str().unwrap(), None),
            McError::Usage { .. }
        ));
        assert!(matches!(
            print("../secret.md", None),
            McError::NotFound { .. }
        ));
        assert!(matches!(print("notes.txt", None), McError::Usage { .. }));
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
        std::fs::write(tmp.path().join(".git/x.md"), "x").unwrap();
        assert!(matches!(print(".git/x.md", None), McError::Usage { .. }));

        // Outputs: escaping the root, absolute elsewhere, not a PDF, hidden.
        for output in [
            "../escape.pdf".to_string(),
            outside.path().join("x.pdf").display().to_string(),
            "notes.txt.pdf/../notes.txt".to_string(),
            "report.txt".to_string(),
            ".git/x.pdf".to_string(),
        ] {
            let err = print("notes.md", Some(&output));
            assert!(matches!(err, McError::Usage { .. }), "{output}: {err}");
        }
        assert!(matches!(
            print("notes.md", Some(victim.to_str().unwrap())),
            McError::Usage { .. }
        ));
        std::fs::write(tmp.path().join("fake.pdf"), "not a pdf").unwrap();
        assert!(matches!(
            print("notes.md", Some("fake.pdf")),
            McError::Conflict { .. }
        ));
        let err = print_meeting_programmatic(&cfg, "MTG-001", Some("../outside/overwritten.pdf"))
            .unwrap_err();
        assert!(!matches!(err, McError::EntityNotFound(_)), "{err}");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "precious\n");
    }

    #[test]
    fn mcp_print_writes_inside_the_repo_and_returns_a_relative_path() {
        if discover_system_fonts().is_none() {
            return;
        }
        let (tmp, cfg) = setup();
        std::fs::write(tmp.path().join("notes.md"), "# Notes\n").unwrap();
        let std = &PrintTemplate::Standard;
        let result =
            print_file_programmatic(&cfg, "notes.md", Some("archive/notes.pdf"), std, None)
                .unwrap();
        assert_eq!(result["path"], "archive/notes.pdf");
        assert!(tmp.path().join("archive/notes.pdf").is_file());
        // Re-printing over an existing PDF is fine.
        print_file_programmatic(&cfg, "notes.md", Some("archive/notes.pdf"), std, None).unwrap();
    }

    #[test]
    fn test_resolve_input_path_falls_back_to_repo_root() {
        let (_tmp, cfg) = setup();
        std::fs::write(cfg.root.join("notes.md"), "# Notes\n").unwrap();
        let resolved = resolve_input_path(&cfg, "notes.md").unwrap();
        assert!(resolved.is_file());
        assert!(resolve_input_path(&cfg, "missing.md").is_none());
    }

    #[test]
    fn test_render_rich_markdown_to_pdf() {
        // Rendering needs real fonts; skip quietly on machines without them.
        if discover_system_fonts().is_none() {
            return;
        }
        let (tmp, cfg) = setup();
        let src = tmp.path().join("doc.md");
        std::fs::write(
            &src,
            format!("---\ntitle: Rich\nsummary: Short\n---\n{RICH_MARKDOWN}"),
        )
        .unwrap();
        let out = tmp.path().join("doc.pdf");
        let result = print_file_programmatic(
            &cfg,
            src.to_str().unwrap(),
            Some(out.to_str().unwrap()),
            &PrintTemplate::Research,
            None,
        )
        .unwrap();
        assert_eq!(result["title"], "Rich");
        assert!(std::fs::metadata(&out).unwrap().len() > 1000);
    }

    #[test]
    fn test_print_meeting_to_pdf() {
        if discover_system_fonts().is_none() {
            return;
        }
        let (tmp, cfg) = setup();
        new::create_customer(&cfg, &new::CustomerInput::new("Acme")).unwrap();
        let mut input = new::MeetingInput::new("Kickoff");
        input.attendees = vec!["Alice".into(), "Bob".into()];
        input.customers = vec!["CUST-001".into()];
        new::create_meeting(&cfg, &input).unwrap();
        let out = tmp.path().join("m.pdf");
        let result =
            print_meeting_programmatic(&cfg, "MTG-001", Some(out.to_str().unwrap())).unwrap();
        assert_eq!(result["title"], "Kickoff");
        assert!(out.is_file());

        let err = print_meeting_programmatic(&cfg, "TASK-001", None).unwrap_err();
        assert!(matches!(err, McError::EntityNotFound(_)));
    }
}
