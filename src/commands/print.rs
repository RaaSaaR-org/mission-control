use crate::cli::PrintEntity;
use crate::config::ResolvedConfig;
use crate::data;
use crate::entity::EntityKind;
use crate::error::{McError, McResult};
use crate::frontmatter;
use colored::*;
use genpdf::elements;
use genpdf::fonts;
use genpdf::style;
use genpdf::Alignment;
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use serde_json::Value as JsonValue;
use std::path::{Path, PathBuf};

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
    match entity {
        PrintEntity::Meeting { id, output } => print_meeting(id, output.as_deref(), cfg),
        PrintEntity::Research { id, output, file } => {
            print_research(id, output.as_deref(), file.as_deref(), cfg)
        }
    }
}

/// Load a font family from the brand fonts directory.
/// Uses Builtin::Helvetica so the font data is not embedded in the PDF (smaller files)
/// when the font is metrically compatible (Liberation Sans is).
fn load_fonts(cfg: &ResolvedConfig) -> McResult<fonts::FontFamily<fonts::FontData>> {
    if let Some(ref fonts_dir) = cfg.brand.fonts_dir {
        match fonts::from_files(fonts_dir, &cfg.brand.font_name, None::<fonts::Builtin>)
        {
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

    Err(McError::Pdf(
        "No fonts directory configured. Set brand.fonts_dir in config/config.yml \
         and place TTF files there (see assets/brand/README.md)."
            .into(),
    ))
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

fn print_meeting(id: &str, output: Option<&str>, cfg: &ResolvedConfig) -> McResult<()> {
    let result = print_meeting_programmatic(cfg, id, output)?;
    println!(
        "{} PDF written to {}",
        "success:".green().bold(),
        result["path"].as_str().unwrap_or("?")
    );
    Ok(())
}

/// Programmatic variant of `print_meeting` -- returns JSON instead of printing.
pub fn print_meeting_programmatic(
    cfg: &ResolvedConfig,
    id: &str,
    output: Option<&str>,
) -> McResult<JsonValue> {
    let entity = data::find_entity_by_id(id, cfg)?;
    if entity.kind != EntityKind::Meeting {
        return Err(McError::Other(format!(
            "{} is a {}, not a meeting",
            id,
            entity.kind.label()
        )));
    }

    let fm = &entity.frontmatter;
    let title = frontmatter::get_str(fm, "title")
        .or_else(|| frontmatter::get_str(fm, "name"))
        .unwrap_or("Untitled Meeting");
    let date = frontmatter::get_str(fm, "date").unwrap_or("");
    let time = frontmatter::get_str(fm, "time").unwrap_or("");
    let duration = frontmatter::get_str(fm, "duration").unwrap_or("");
    let status = frontmatter::get_str(fm, "status").unwrap_or("");
    let customers = frontmatter::get_string_list(fm, "customers");
    let projects = frontmatter::get_string_list(fm, "projects");

    let font_family = load_fonts(cfg)?;
    let pc = primary_color(cfg);
    let ac = accent_color(cfg);
    let mut doc = create_document(font_family, title, &cfg.brand.name, pc, ac);

    // Build cover page metadata pairs
    let attendees = get_attendees(fm);
    let participant_names: Vec<&str> = attendees.iter().map(|a| a.name.as_str()).collect();
    let mut meta_pairs: Vec<(&str, String)> = Vec::new();
    meta_pairs.push(("Date", date.to_string()));
    if !time.is_empty() {
        meta_pairs.push(("Time", time.to_string()));
    }
    if !duration.is_empty() {
        meta_pairs.push(("Duration", duration.to_string()));
    }
    meta_pairs.push(("Status", status.to_string()));
    if !participant_names.is_empty() {
        meta_pairs.push(("Participants", participant_names.join(", ")));
    }
    if !customers.is_empty() {
        meta_pairs.push(("Customers", customers.join(", ")));
    }
    if !projects.is_empty() {
        meta_pairs.push(("Projects", projects.join(", ")));
    }

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
    render_markdown(&mut doc, &entity.body, pc);

    // Footer
    push_document_footer(&mut doc);

    // Write PDF
    let out_path = match output {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(format!("{}.pdf", id)),
    };
    doc.render_to_file(&out_path)
        .map_err(|e| McError::Pdf(format!("Failed to write PDF: {e}")))?;

    let display_path = out_path.canonicalize().unwrap_or(out_path);
    Ok(serde_json::json!({
        "id": id,
        "title": title,
        "path": display_path.display().to_string(),
    }))
}

// ---------------------------------------------------------------------------
// Research PDF
// ---------------------------------------------------------------------------

fn print_research(
    id: &str,
    output: Option<&str>,
    specific_file: Option<&str>,
    cfg: &ResolvedConfig,
) -> McResult<()> {
    let result = print_research_programmatic(cfg, id, output, specific_file)?;
    println!(
        "{} PDF written to {}",
        "success:".green().bold(),
        result["path"].as_str().unwrap_or("?")
    );
    Ok(())
}

/// Programmatic variant of `print_research` -- returns JSON instead of printing.
pub fn print_research_programmatic(
    cfg: &ResolvedConfig,
    id: &str,
    output: Option<&str>,
    file: Option<&str>,
) -> McResult<JsonValue> {
    let entity = data::find_entity_by_id(id, cfg)?;
    if entity.kind != EntityKind::Research {
        return Err(McError::Other(format!(
            "{} is a {}, not a research topic",
            id,
            entity.kind.label()
        )));
    }

    let fm = &entity.frontmatter;
    let title = frontmatter::get_str(fm, "title")
        .or_else(|| frontmatter::get_str(fm, "name"))
        .unwrap_or("Untitled Research");
    let owner = frontmatter::get_str(fm, "owner").unwrap_or("");
    let status = frontmatter::get_str(fm, "status").unwrap_or("");
    let summary = frontmatter::get_str(fm, "summary").unwrap_or("");
    let tags = frontmatter::get_string_list(fm, "tags");
    let agents = frontmatter::get_string_list(fm, "agents");

    let font_family = load_fonts(cfg)?;
    let pc = primary_color(cfg);
    let ac = accent_color(cfg);
    let mut doc = create_document(font_family, title, &cfg.brand.name, pc, ac);

    // Build cover page metadata pairs
    let mut meta_pairs: Vec<(&str, String)> = Vec::new();
    if !owner.is_empty() {
        meta_pairs.push(("Owner", owner.to_string()));
    }
    meta_pairs.push(("Status", status.to_string()));
    if !tags.is_empty() {
        meta_pairs.push(("Tags", tags.join(", ")));
    }
    if !agents.is_empty() {
        meta_pairs.push(("Agents", agents.join(", ")));
    }

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
    if !summary.is_empty() {
        doc.push(elements::Paragraph::new(style::StyledString::new(
            "Summary",
            style::Style::new()
                .bold()
                .with_font_size(H2_SIZE)
                .with_color(pc),
        )));
        doc.push(elements::Break::new(0.3));
        doc.push(elements::Paragraph::new(summary));
        push_section_separator(&mut doc);
    }

    // Find final/ directory
    let source_dir = entity
        .source_path
        .parent()
        .ok_or_else(|| McError::Other("Cannot determine research directory".into()))?;
    let final_dir = source_dir.join("final");

    let mut report_files = collect_report_files(&final_dir, file);

    if report_files.is_empty() && file.is_some() {
        return Err(McError::Other(format!(
            "File '{}' not found in {}/final/",
            file.unwrap(),
            id
        )));
    }

    if report_files.is_empty() {
        // Fall back to _index.md body
        eprintln!(
            "{} No files in final/ directory, using _index.md body.",
            "warning:".yellow().bold()
        );
        render_markdown(&mut doc, &entity.body, pc);
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
                    style::Style::new().bold().with_font_size(H1_SIZE).with_color(pc),
                )));
                doc.push(elements::Break::new(0.3));
            }

            render_markdown(&mut doc, &body, pc);

            if i + 1 < total {
                push_section_separator(&mut doc);
                doc.push(elements::PageBreak::new());
            }
        }
    }

    // Footer
    push_document_footer(&mut doc);

    // Write PDF
    let out_path = match output {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from(format!("{}-final-report.pdf", id)),
    };
    doc.render_to_file(&out_path)
        .map_err(|e| McError::Pdf(format!("Failed to write PDF: {e}")))?;

    let display_path = out_path.canonicalize().unwrap_or(out_path);
    Ok(serde_json::json!({
        "id": id,
        "title": title,
        "path": display_path.display().to_string(),
    }))
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
fn get_attendees(fm: &serde_yaml::Value) -> Vec<Attendee> {
    let seq = fm
        .as_mapping()
        .and_then(|m| m.get(&serde_yaml::Value::String("attendees".into())))
        .and_then(|v| v.as_sequence());

    let Some(seq) = seq else {
        return Vec::new();
    };

    seq.iter()
        .filter_map(|item| {
            if let Some(map) = item.as_mapping() {
                let name = map
                    .get(&serde_yaml::Value::String("name".into()))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let role = map
                    .get(&serde_yaml::Value::String("role".into()))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let company = map
                    .get(&serde_yaml::Value::String("company".into()))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(Attendee {
                    name,
                    role,
                    company,
                })
            } else if let Some(s) = item.as_str() {
                Some(Attendee {
                    name: s.to_string(),
                    role: String::new(),
                    company: String::new(),
                })
            } else {
                None
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
        .filter(|p| p.extension().map_or(false, |ext| ext == "md"))
        .filter(|p| {
            if let Some(target) = specific_file {
                p.file_name()
                    .map_or(false, |name| name.to_string_lossy().contains(target))
            } else {
                true
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Markdown → genpdf rendering
// ---------------------------------------------------------------------------

fn render_markdown(doc: &mut genpdf::Document, markdown: &str, heading_color: style::Color) {
    let opts = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(markdown, opts);

    let mut current_paragraph = elements::Paragraph::default();
    let mut has_text = false;
    let mut in_heading = false;
    let mut heading_level = HeadingLevel::H1;
    let mut in_strong = false;
    let mut in_emphasis = false;
    let mut in_code = false;
    let mut list_ordered = false;
    let mut list_items: Vec<elements::Paragraph> = Vec::new();
    let mut current_list_item = elements::Paragraph::default();
    let mut in_list_item = false;
    let mut in_table = false;
    let mut table_cols: usize = 0;
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
                if !in_list_item {
                    current_paragraph = elements::Paragraph::default();
                    has_text = false;
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if in_list_item {
                    // Don't flush; collect into list item
                } else {
                    flush_paragraph(doc, &mut current_paragraph, &mut has_text);
                    doc.push(elements::Break::new(0.6));
                }
            }
            Event::Start(Tag::Strong) => {
                in_strong = true;
            }
            Event::End(TagEnd::Strong) => {
                in_strong = false;
            }
            Event::Start(Tag::Emphasis) => {
                in_emphasis = true;
            }
            Event::End(TagEnd::Emphasis) => {
                in_emphasis = false;
            }
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
                if in_heading {
                    let size = heading_font_size(heading_level);
                    current_paragraph.push_styled(
                        s,
                        style::Style::new().with_font_size(size).with_color(heading_color),
                    );
                } else if in_list_item {
                    current_list_item.push_styled(s, style::Style::new().italic());
                } else {
                    current_paragraph.push_styled(s, style::Style::new().italic());
                }
                if !in_list_item {
                    has_text = true;
                }
            }
            Event::Start(Tag::List(first_number)) => {
                flush_paragraph(doc, &mut current_paragraph, &mut has_text);
                list_ordered = first_number.is_some();
                list_items.clear();
            }
            Event::End(TagEnd::List(_)) => {
                // Push collected list items
                if list_ordered {
                    let mut ol = elements::OrderedList::new();
                    for item in list_items.drain(..) {
                        ol.push(item);
                    }
                    doc.push(ol);
                } else {
                    let mut ul = elements::UnorderedList::new();
                    for item in list_items.drain(..) {
                        ul.push(item);
                    }
                    doc.push(ul);
                }
                doc.push(elements::Break::new(0.2));
            }
            Event::Start(Tag::Item) => {
                in_list_item = true;
                current_list_item = elements::Paragraph::default();
            }
            Event::End(TagEnd::Item) => {
                in_list_item = false;
                list_items.push(std::mem::take(&mut current_list_item));
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
            Event::Start(Tag::TableHead) => {
                table_row_cells.clear();
            }
            Event::End(TagEnd::TableHead) => {
                table_rows.push((table_row_cells.clone(), true));
                table_row_cells.clear();
            }
            Event::Start(Tag::TableRow) => {
                table_row_cells.clear();
            }
            Event::End(TagEnd::TableRow) => {
                table_rows.push((table_row_cells.clone(), false));
                table_row_cells.clear();
            }
            Event::Start(Tag::TableCell) => {}
            Event::End(TagEnd::TableCell) => {}
            Event::Text(text) => {
                let s = text.to_string();
                if in_table {
                    table_row_cells.push(s);
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
                } else if in_list_item {
                    let st = text_style(in_strong, in_emphasis);
                    current_list_item.push_styled(s, st);
                } else {
                    let st = text_style(in_strong, in_emphasis);
                    current_paragraph.push_styled(s, st);
                    has_text = true;
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if in_list_item {
                    current_list_item.push(" ");
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
                let marker = if checked { "✓ " } else { "○ " };
                if in_list_item {
                    current_list_item.push_styled(
                        marker,
                        style::Style::new()
                            .bold()
                            .with_color(if checked {
                                style::Color::Rgb(0, 128, 0)
                            } else {
                                style::Color::Rgb(160, 160, 160)
                            }),
                    );
                }
            }
            _ => {}
        }
    }

    // Flush anything remaining
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

fn render_table(
    doc: &mut genpdf::Document,
    rows: &[(Vec<String>, bool)],
    num_cols: usize,
) {
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
