use crate::checklist;
use crate::cli::markdown::{self, Doc, RenderOptions};
use crate::cli::{pager, suggest, ui};
use crate::commands::{check, index};
use crate::comments::{self, Comment};
use crate::config::ResolvedConfig;
use crate::data::{self, EntityRecord};
use crate::entity::EntityKind;
use crate::error::McResult;
use crate::frontmatter;
use crate::html::{display_name, Catalog};
use colored::*;
use serde_json::Value as JsonValue;
use serde_yaml::Value;
use std::path::Path;

/// Fields rendered in the header rather than the field list.
const HEADER_FIELDS: &[&str] = &["id", "status", "title", "name", "slug"];

/// Widest the rendered body gets, for readability on wide terminals.
const BODY_WIDTH: usize = 100;

/// Backlinks listed before "+N more".
const MAX_BACKLINKS: usize = 10;

/// `mc show` flags beyond the ID.
#[derive(Clone, Copy, Debug, Default)]
pub struct ShowOptions {
    /// Print the markdown source even on a terminal.
    pub raw: bool,
    /// Never pipe through a pager.
    pub no_pager: bool,
    /// Open the file in an editor instead of printing it.
    pub open: bool,
}

pub fn run(id: &str, opts: ShowOptions, cfg: &ResolvedConfig) -> McResult<()> {
    let entity = suggest::find_entity(id, cfg, None)?;
    let ui = ui::get();

    if opts.open {
        if ui.json {
            let path = relative(&entity.source_path, cfg);
            println!("{}", serde_json::json!({ "id": entity.id, "path": path }));
        } else if ui.interactive {
            ui::info(format!("Opening {}", relative(&entity.source_path, cfg)));
        }
        return pager::open(&entity.source_path);
    }

    if ui.json {
        let mut json = index::entity_json(&entity, cfg);
        if let Some(obj) = json.as_object_mut() {
            obj.insert("_kind".into(), JsonValue::from(entity.kind.label()));
            obj.insert("_body".into(), JsonValue::from(entity.body.clone()));
        }
        println!("{}", serde_json::to_string_pretty(&json)?);
        return Ok(());
    }

    if opts.raw || !ui.interactive {
        print!("{}", join(&raw_view(&entity, cfg)));
        return Ok(());
    }

    let catalog = load_catalog(cfg);
    let mut hyperlinks = ui::hyperlinks();
    let mut lines = rendered_view(&entity, cfg, &catalog, hyperlinks);
    let pager = (!opts.no_pager).then(pager::command).flatten();
    if let (Some(cmd), Some(height)) = (pager, ui::height()) {
        if lines.len() >= height {
            // Only `less` is known to pass OSC 8 through; MC_HYPERLINKS forces.
            if hyperlinks && !pager::is_less(&cmd) && std::env::var_os("MC_HYPERLINKS").is_none() {
                hyperlinks = false;
                lines = rendered_view(&entity, cfg, &catalog, hyperlinks);
            }
            if pager::page(&cmd, &join(&lines)) {
                return Ok(());
            }
        }
    }
    print!("{}", join(&lines));
    Ok(())
}

fn join(lines: &[String]) -> String {
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Every entity in the repo, for resolving references and backlinks.
pub fn load_catalog(cfg: &ResolvedConfig) -> Catalog {
    let records = EntityKind::ALL
        .into_iter()
        .filter(|k| cfg.entity_available(k))
        .flat_map(|k| data::collect_entities(k, cfg).unwrap_or_default())
        .collect();
    Catalog::from_records(records, Vec::new(), cfg)
}

fn relative(path: &Path, cfg: &ResolvedConfig) -> String {
    path.strip_prefix(&cfg.root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn rule_width(rendered: bool) -> usize {
    ui::content_width(2, if rendered { BODY_WIDTH } else { 72 })
}

/// Header, fields and the markdown source as written: what piped output and
/// `--raw` print, kept stable for scripts. On a terminal, control characters
/// are dropped so a file can't send escape sequences.
pub fn raw_view(entity: &EntityRecord, cfg: &ResolvedConfig) -> Vec<String> {
    let mut out = header(entity, None);
    out.extend(fields(entity, cfg, None));
    let lines = tidy_body(&entity.body);
    if lines.is_empty() {
        out.push(String::new());
        return out;
    }
    out.push(format!("  {}", ui::rule(rule_width(false))));
    out.push(String::new());
    // Fenced code is dimmed and yellow when colours are on, as before.
    let mut in_code = false;
    for line in lines {
        let line = terminal_safe(line);
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            out.push(format!("  {}", line.dimmed()));
        } else if in_code {
            out.push(format!("  {}", line.yellow()));
        } else {
            out.push(format!("  {line}"));
        }
    }
    out.push(String::new());
    out
}

/// `s` without control characters on an interactive terminal; unchanged when
/// piped, so scripts see the file as written.
fn terminal_safe(s: &str) -> String {
    if ui::get().interactive {
        markdown::sanitize(s)
    } else {
        s.to_string()
    }
}

/// Header, fields, the rendered body and a footer of linked entities.
pub fn rendered_view(
    entity: &EntityRecord,
    cfg: &ResolvedConfig,
    catalog: &Catalog,
    hyperlinks: bool,
) -> Vec<String> {
    let links = Links {
        catalog,
        hyperlinks,
    };
    let mut out = header(entity, Some(&links));
    out.extend(fields(entity, cfg, Some(&links)));

    let width = ui::content_width(2, BODY_WIDTH);
    let dir = entity.source_path.parent().unwrap_or(Path::new(""));
    let title = frontmatter::get_str(&entity.frontmatter, "title")
        .or_else(|| frontmatter::get_str(&entity.frontmatter, "name"));
    let doc = Doc {
        catalog,
        root: &cfg.root,
        dir,
        title,
    };
    let opts = RenderOptions {
        width,
        unicode: ui::get().unicode,
        hyperlinks,
    };
    // Comments of tasks and meetings get their own section below the notes.
    let (notes, comment_list) = if comments::is_commentable(entity.kind) {
        comments::split(&entity.body)
    } else {
        (entity.body.clone(), Vec::new())
    };
    let mut body = markdown::render(&notes, &opts, Some(&doc));
    if !body.lines.is_empty() {
        out.push(format!("  {}", ui::rule(rule_width(true))));
        out.push(String::new());
        let (done, total) = checklist::progress(&checklist::body_items(entity.kind, &entity.body));
        if total > 0 {
            out.push(format!(
                "  {}  {}  {}",
                "checklist".dimmed(),
                check::progress_bar(done, total),
                format!("{done} of {total} done").dimmed()
            ));
            out.push(String::new());
        }
        out.extend(indent(&body.lines, 2));
    }
    out.push(String::new());
    if !comment_list.is_empty() {
        let doc = Doc { title: None, ..doc };
        let opts = RenderOptions {
            width: width.saturating_sub(2).max(20),
            ..opts
        };
        out.push(format!(
            "  {}  {}",
            ui::section("Comments"),
            comment_list.len().to_string().dimmed()
        ));
        for c in &comment_list {
            out.push(format!("  {}", comment_head(c)));
            let rendered = markdown::render(&c.body, &opts, Some(&doc));
            out.extend(indent(&rendered.lines, 4));
            out.push(String::new());
            body.refs.extend(rendered.refs);
        }
    }

    // Frontmatter references (project, depends_on, ...) first, then the body's.
    let mut refs: Vec<String> = Vec::new();
    for id in frontmatter_refs(entity, catalog)
        .into_iter()
        .chain(body.refs)
    {
        if id != entity.id && !refs.contains(&id) {
            refs.push(id);
        }
    }
    let linked: Vec<&EntityRecord> = refs
        .iter()
        .filter_map(|id| catalog.records.iter().find(|r| &r.id == id))
        .collect();
    let backlinks: Vec<&EntityRecord> = catalog
        .records
        .iter()
        .filter(|r| r.id != entity.id && !refs.contains(&r.id))
        .filter(|r| catalog.record_mentions(r, &entity.id))
        .collect();
    if !linked.is_empty() {
        out.push(format!("  {}", ui::section("Links")));
        out.extend(ref_lines(&linked, cfg, &links, true));
        out.push(String::new());
    }
    if !backlinks.is_empty() {
        out.push(format!("  {}", ui::section("Referenced by")));
        let shown = &backlinks[..backlinks.len().min(MAX_BACKLINKS)];
        out.extend(ref_lines(shown, cfg, &links, false));
        if backlinks.len() > shown.len() {
            out.push(format!(
                "  {}",
                format!("+{} more", backlinks.len() - shown.len()).dimmed()
            ));
        }
        out.push(String::new());
    }
    out
}

/// Prefix non-empty lines with `n` spaces.
fn indent(lines: &[String], n: usize) -> Vec<String> {
    let pad = " ".repeat(n);
    lines
        .iter()
        .map(|l| {
            if l.is_empty() {
                String::new()
            } else {
                format!("{pad}{l}")
            }
        })
        .collect()
}

/// "Jane Doe  2026-10-05 14:32" for a comment; hand-written headings that
/// don't parse are shown as written.
fn comment_head(c: &Comment) -> String {
    let when = [c.date.as_deref(), c.time.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    let who = match (&c.author, c.date.is_some()) {
        (Some(a), _) => markdown::sanitize(a),
        (None, false) if !c.heading.is_empty() => markdown::sanitize(&c.heading),
        _ => "Note".to_string(),
    };
    let who = format!("{} {}", ui::glyphs().bullet.cyan(), who.bold());
    if when.is_empty() {
        who
    } else {
        format!("{who}  {}", when.dimmed())
    }
}

/// Reference resolution for the rendered view.
struct Links<'a> {
    catalog: &'a Catalog,
    hyperlinks: bool,
}

impl Links<'_> {
    /// Link `text` to the file at `path` when hyperlinks are on.
    fn file(&self, path: &Path, text: &str) -> String {
        if self.hyperlinks {
            ui::hyperlink(&markdown::file_url(path), text)
        } else {
            text.to_string()
        }
    }

    /// Highlight known entity IDs in `s` and link them to their files.
    fn ids(&self, s: &str) -> String {
        let s = markdown::sanitize(s);
        self.catalog
            .id_regex()
            .replace_all(&s, |caps: &regex::Captures| {
                let id = &caps[0];
                match self.catalog.records.iter().find(|r| r.id == id) {
                    Some(rec) => self.file(&rec.source_path, &id.cyan().to_string()),
                    None => id.to_string(),
                }
            })
            .into_owned()
    }
}

fn header(entity: &EntityRecord, links: Option<&Links>) -> Vec<String> {
    let g = ui::glyphs();
    let fm = &entity.frontmatter;
    let status = terminal_safe(frontmatter::get_str_or(fm, "status", ""));
    let title = frontmatter::get_str(fm, "title")
        .or_else(|| frontmatter::get_str(fm, "name"))
        .unwrap_or("");

    let id = entity.id.cyan().bold().to_string();
    let id = match links {
        Some(l) => l.file(&entity.source_path, &id),
        None => id,
    };
    let mut line = format!(
        "  {} {id}  {}",
        g.brand.cyan(),
        entity.kind.label().to_uppercase().dimmed()
    );
    if !status.is_empty() {
        line = format!("{line}  {}", ui::status(&status));
    }
    let mut out = vec![String::new(), line];
    if !title.is_empty() {
        match links {
            Some(_) => {
                let title = markdown::sanitize(title);
                for part in ui::wrap(&title, ui::content_width(2, BODY_WIDTH), 0) {
                    out.push(format!("  {}", part.bold()));
                }
            }
            None => out.push(format!("  {}", terminal_safe(title).bold())),
        }
    }
    out.push(format!("  {}", ui::rule(rule_width(links.is_some()))));
    out
}

fn fields(entity: &EntityRecord, cfg: &ResolvedConfig, links: Option<&Links>) -> Vec<String> {
    let Some(map) = entity.frontmatter.as_mapping() else {
        return Vec::new();
    };
    let today = crate::util::today_str();
    let rows: Vec<(String, String)> = map
        .iter()
        .filter_map(|(k, v)| {
            let key = k.as_str()?;
            if key.starts_with('_') || HEADER_FIELDS.contains(&key) || is_empty(v) {
                return None;
            }
            // Obsidian aliases that just repeat the ID are noise.
            if key == "aliases" && format_value(v) == entity.id {
                return None;
            }
            let value = match key {
                "priority" => data::get_number(&entity.frontmatter, "priority")
                    .map(|p| format!("{} {}", ui::priority(p), ui::priority_label(p).dimmed()))
                    .unwrap_or_else(|| format_value(v)),
                "due_date" if entity.kind == EntityKind::Task => {
                    crate::commands::list::due_cell(entity, &today)
                }
                _ => match links {
                    Some(l) => l.ids(&format_value(v)),
                    None => terminal_safe(&format_value(v)),
                },
            };
            Some((key.replace('_', " "), value))
        })
        .collect();

    let key_w = rows.iter().map(|(k, _)| k.len()).max().unwrap_or(4).max(4);
    // The rendered view wraps long values (summaries) under themselves.
    let value_w = links.map(|_| ui::content_width(4 + key_w, BODY_WIDTH - key_w - 2));
    let source = terminal_safe(&relative(&entity.source_path, cfg));
    let source = match (links, value_w) {
        // The path keeps its tail; the link still carries all of it.
        (Some(l), Some(w)) => l.file(
            &entity.source_path,
            &ui::truncate_start(&source, w).dimmed().to_string(),
        ),
        _ => source.dimmed().to_string(),
    };
    let mut out: Vec<String> = Vec::new();
    for (k, v) in rows {
        let key = ui::pad(&k.dimmed().to_string(), key_w);
        match value_w {
            Some(w) => {
                for (i, part) in ui::wrap(&v, w, 0).into_iter().enumerate() {
                    let lead = if i == 0 {
                        key.clone()
                    } else {
                        " ".repeat(key_w)
                    };
                    out.push(format!("  {lead}  {part}"));
                }
            }
            None => out.push(format!("  {key}  {v}")),
        }
    }
    out.push(format!(
        "  {}  {}",
        ui::pad(&"file".dimmed().to_string(), key_w),
        source
    ));
    out
}

/// Known entity IDs mentioned in the frontmatter (project, depends_on, ...).
fn frontmatter_refs(entity: &EntityRecord, catalog: &Catalog) -> Vec<String> {
    fn walk(v: &Value, catalog: &Catalog, out: &mut Vec<String>) {
        match v {
            Value::String(s) => {
                for m in catalog.id_regex().find_iter(s) {
                    if catalog.name(m.as_str()).is_some() {
                        out.push(m.as_str().to_string());
                    }
                }
            }
            Value::Sequence(seq) => seq.iter().for_each(|v| walk(v, catalog, out)),
            Value::Mapping(m) => m.values().for_each(|v| walk(v, catalog, out)),
            Value::Tagged(t) => walk(&t.value, catalog, out),
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Some(map) = entity.frontmatter.as_mapping() {
        for (k, v) in map {
            if !matches!(k.as_str(), Some("id" | "aliases")) {
                walk(v, catalog, &mut out);
            }
        }
    }
    out
}

/// One line per entity (`ID  status  title`), optionally followed by its
/// file path so the list is useful without clickable links.
fn ref_lines(
    recs: &[&EntityRecord],
    cfg: &ResolvedConfig,
    links: &Links,
    with_path: bool,
) -> Vec<String> {
    let id_w = recs.iter().map(|r| r.id.len()).max().unwrap_or(0);
    let statuses: Vec<String> = recs
        .iter()
        .map(|r| {
            ui::status(&markdown::sanitize(frontmatter::get_str_or(
                &r.frontmatter,
                "status",
                "",
            )))
        })
        .collect();
    let status_w = statuses.iter().map(|s| ui::width_of(s)).max().unwrap_or(0);
    let width = ui::content_width(2, BODY_WIDTH);
    let title_w = width.saturating_sub(id_w + status_w + 4).max(12);
    let mut out = Vec::new();
    for (rec, status) in recs.iter().zip(statuses) {
        let id = links.file(&rec.source_path, &ui::pad(&rec.id.cyan().to_string(), id_w));
        let title = markdown::sanitize(display_name(rec));
        let mut line = format!("  {id}  ");
        if status_w > 0 {
            line.push_str(&ui::pad(&status, status_w));
            line.push_str("  ");
        }
        line.push_str(&ui::truncate(&title, title_w));
        out.push(line.trim_end().to_string());
        if with_path {
            let path = markdown::sanitize(&relative(&rec.source_path, cfg));
            let path = ui::truncate_start(&path, width.saturating_sub(id_w + 2).max(12))
                .dimmed()
                .to_string();
            let path = links.file(&rec.source_path, &path);
            out.push(format!("  {}  {path}", " ".repeat(id_w)));
        }
    }
    out
}

/// Trim surrounding blank lines and collapse runs of blank lines.
fn tidy_body(body: &str) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for line in body.lines() {
        let blank = line.trim().is_empty();
        if blank && out.last().is_none_or(|l| l.trim().is_empty()) {
            continue;
        }
        out.push(line.trim_end());
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    out
}

fn is_empty(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Sequence(seq) => seq.is_empty(),
        Value::Mapping(m) => m.is_empty(),
        _ => false,
    }
}

fn format_value(value: &Value) -> String {
    match value {
        Value::String(s) => link_text(s),
        Value::Sequence(seq) => seq.iter().map(format_value).collect::<Vec<_>>().join(", "),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Null => String::new(),
        Value::Mapping(m) => mapping_label(m),
        Value::Tagged(t) => format_value(&t.value),
    }
}

/// A wiki-link as its readable part: `[[CONT-003-alexander-david|Alexander
/// David]]` → `Alexander David`, `[[PROJ-001|Innovation Project]]` →
/// `Innovation Project (PROJ-001)`, `[[PROJ-001]]` → `PROJ-001`.
fn link_text(s: &str) -> String {
    let inner = s.strip_prefix("[[").and_then(|s| s.strip_suffix("]]"));
    match inner.and_then(|i| i.split_once('|')) {
        Some((target, alias)) if !alias.trim().is_empty() => {
            let bare_id = target.split_once('-').is_some_and(|(p, n)| {
                p.chars().all(|c| c.is_ascii_alphanumeric())
                    && !n.is_empty()
                    && n.chars().all(|c| c.is_ascii_digit())
            });
            if bare_id {
                format!("{} ({target})", alias.trim())
            } else {
                alias.trim().to_string()
            }
        }
        _ => frontmatter::strip_wikilink(s).to_string(),
    }
}

/// An object in the frontmatter (a contact, an attendee) as its name, with
/// the role in parentheses: `Daniel Lang (CTO)`.
fn mapping_label(m: &serde_yaml::Mapping) -> String {
    let field = |k: &str| {
        m.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let Some(name) = field("name")
        .or_else(|| field("title"))
        .or_else(|| field("id"))
    else {
        return format!("{{{} fields}}", m.len());
    };
    match field("role") {
        Some(role) => format!("{} ({role})", link_text(name)),
        None => link_text(name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tidy_body_collapses_blank_runs() {
        let body = "\n\n# Title\n\n\n\nPara\n\n- a\n\n\n";
        assert_eq!(tidy_body(body), vec!["# Title", "", "Para", "", "- a"]);
    }

    #[test]
    fn format_value_strips_wikilinks() {
        let v: Value = serde_yaml::from_str("['[[PROJ-001]]', '[[CUST-002]]']").unwrap();
        assert_eq!(format_value(&v), "PROJ-001, CUST-002");
    }

    #[test]
    fn format_value_shows_objects_and_link_aliases_readably() {
        let v: Value = serde_yaml::from_str(
            "- name: Daniel Lang\n  role: CTO\n  email: d@example.com\n- name: Bob\n- {email: x@example.com, phone: '1'}",
        )
        .unwrap();
        assert_eq!(format_value(&v), "Daniel Lang (CTO), Bob, {2 fields}");
        let v: Value = serde_yaml::from_str(
            "[florian-fromm, '[[CONT-003-alexander-david|Alexander David]]', '[[PROJ-001|Innovation Project]]']",
        )
        .unwrap();
        assert_eq!(
            format_value(&v),
            "florian-fromm, Alexander David, Innovation Project (PROJ-001)"
        );
    }
}
