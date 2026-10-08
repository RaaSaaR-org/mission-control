use crate::cli::suggest;
use crate::cli::ui;
use crate::cli::ExportEntity;
use crate::config::ResolvedConfig;
use crate::data;
use crate::entity::EntityKind;
use crate::error::{McError, McResult};
use crate::frontmatter;
use crate::util;
use colored::*;
use regex::Regex;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

pub fn run(entity: &ExportEntity, cfg: &ResolvedConfig) -> McResult<()> {
    match entity {
        ExportEntity::Customer { id, folder_only } => export_customer(cfg, id, *folder_only),
    }
}

type Zip = ZipWriter<fs::File>;

fn export_customer(cfg: &ResolvedConfig, id_or_slug: &str, folder_only: bool) -> McResult<()> {
    if !cfg.entity_available(&EntityKind::Customer) {
        return Err(McError::not_available(EntityKind::Customer, cfg));
    }
    // Find the customer directory
    let dir = find_customer_dir(cfg, id_or_slug)?;
    let dir_name = dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    // Ensure archive/ exists
    fs::create_dir_all(&cfg.archive_dir)?;

    let today = util::today_str();
    let zip_name = format!("{}-{}.zip", dir_name, today);
    let zip_path = cfg.archive_dir.join(&zip_name);

    let file = fs::File::create(&zip_path)?;
    let mut zip = ZipWriter::new(file);

    // The customer folder at the top level, then (unless --folder-only) what
    // links to the customer from elsewhere under related/<kind>/.
    let mut files = add_tree(&mut zip, &dir, "", false)?;
    let mut related: Vec<(EntityKind, usize)> = Vec::new();
    if !folder_only {
        let id_re = Regex::new(&format!(
            r"^{}-\d+",
            regex::escape(EntityKind::Customer.prefix(cfg))
        ))
        .expect("regex with escaped prefix is always valid");
        if let Some(id) = id_re.find(&dir_name) {
            let (n, counts) = add_related(&mut zip, cfg, id.as_str(), &dir)?;
            files += n;
            related = counts;
        }
    }

    zip.finish()?;

    let rel = zip_path
        .strip_prefix(&cfg.root)
        .unwrap_or(&zip_path)
        .display()
        .to_string();
    let bytes = fs::metadata(&zip_path).map(|m| m.len()).unwrap_or(0);

    if ui::get().json {
        let related: serde_json::Map<String, serde_json::Value> = related
            .iter()
            .map(|(k, n)| (k.label_plural().to_string(), (*n).into()))
            .collect();
        let out = serde_json::json!({
            "customer": dir_name,
            "path": rel,
            "files": files,
            "bytes": bytes,
            "related": related,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    let g = ui::glyphs();
    ui::success(format!(
        "Exported {} {} {}",
        dir_name.cyan().bold(),
        g.arrow,
        rel.bold()
    ));
    println!(
        "  {}",
        format!(
            "{} {} {}",
            ui::count(files, "file", "files"),
            g.sep,
            human_size(bytes)
        )
        .dimmed()
    );
    let linked: Vec<String> = related
        .iter()
        .filter(|(_, n)| *n > 0)
        .map(|(k, n)| ui::count(*n, k.label(), k.label_plural()))
        .collect();
    if !linked.is_empty() {
        println!(
            "  {}",
            format!("including {}", linked.join(&format!(" {} ", g.sep))).dimmed()
        );
    }

    Ok(())
}

/// Hidden files (`.gitkeep`, `.DS_Store`, ...) are never exported.
fn is_hidden(name: &std::ffi::OsStr) -> bool {
    name.to_string_lossy().starts_with('.')
}

/// Zip entry name for `path` relative to `base`, under `prefix`, with `/`
/// separators on every platform.
fn entry_name(prefix: &str, base: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(base).unwrap_or(path);
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    format!("{prefix}{}", parts.join("/"))
}

fn add_file(zip: &mut Zip, path: &Path, name: &str) -> McResult<()> {
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file(name, options)?;
    zip.write_all(&fs::read(path)?)?;
    Ok(())
}

/// Add everything below `dir` (skipping hidden files, and with `notes_only`
/// everything but Markdown) under `prefix`. Returns the number of files added.
fn add_tree(zip: &mut Zip, dir: &Path, prefix: &str, notes_only: bool) -> McResult<usize> {
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    if !prefix.is_empty() {
        zip.add_directory(prefix, options)?;
    }
    let mut files = 0usize;
    let walker = WalkDir::new(dir)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !is_hidden(e.file_name()));
    for entry in walker.filter_map(|e| e.ok()).filter(|e| e.depth() > 0) {
        let name = entry_name(prefix, dir, entry.path());
        if entry.file_type().is_dir() {
            if !notes_only {
                zip.add_directory(format!("{name}/"), options)?;
            }
        } else if !notes_only || entry.path().extension().is_some_and(|e| e == "md") {
            add_file(zip, entry.path(), &name)?;
            files += 1;
        }
    }
    Ok(files)
}

/// Add the projects, research, meetings and tasks outside `customer_dir`
/// whose `customers` / `customer` field links to `id`, under
/// `related/<kind>/`. Project and research folders contribute all their
/// Markdown notes, but not their assets: a project shared by many customers
/// may hold gigabytes of media. Returns the number of files added and the
/// number of entities per kind.
fn add_related(
    zip: &mut Zip,
    cfg: &ResolvedConfig,
    id: &str,
    customer_dir: &Path,
) -> McResult<(usize, Vec<(EntityKind, usize)>)> {
    let links_here = |fm: &serde_yaml::Value| {
        ["customers", "customer"].iter().any(|key| {
            frontmatter::get_link_list(fm, key).iter().any(|target| {
                target
                    .get(..id.len())
                    .is_some_and(|h| h.eq_ignore_ascii_case(id))
                    && matches!(target.as_bytes().get(id.len()), None | Some(b'-'))
            })
        })
    };
    // Folders already in the zip: tasks inside an exported project folder
    // (or the customer's own) aren't added twice.
    let mut included: Vec<PathBuf> = vec![customer_dir.to_path_buf()];
    let mut files = 0usize;
    let mut counts = Vec::new();
    for kind in [
        EntityKind::Project,
        EntityKind::Research,
        EntityKind::Meeting,
        EntityKind::Task,
    ] {
        if !cfg.entity_available(&kind) {
            continue;
        }
        let base = kind.base_dir(cfg);
        let mut n = 0usize;
        for record in data::collect_entities(kind, cfg)? {
            let path = &record.source_path;
            if !links_here(&record.frontmatter) || included.iter().any(|d| path.starts_with(d)) {
                continue;
            }
            let prefix = format!("related/{}/", kind.label_plural());
            let folder = path.parent().filter(|p| {
                matches!(kind, EntityKind::Project | EntityKind::Research) && *p != base
            });
            match folder {
                Some(folder) => {
                    let name = folder.file_name().unwrap_or_default().to_string_lossy();
                    files += add_tree(zip, folder, &format!("{prefix}{name}/"), true)?;
                    included.push(folder.to_path_buf());
                }
                None => {
                    let name = path.file_name().unwrap_or_default().to_string_lossy();
                    add_file(zip, path, &format!("{prefix}{name}"))?;
                    files += 1;
                }
            }
            n += 1;
        }
        counts.push((kind, n));
    }
    Ok((files, counts))
}

fn human_size(bytes: u64) -> String {
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

fn find_customer_dir(cfg: &ResolvedConfig, id_or_slug: &str) -> McResult<PathBuf> {
    let not_found = |hint: Option<String>| {
        McError::not_found(format!("customer '{id_or_slug}' not found"), hint)
    };
    if !cfg.customers_dir.is_dir() {
        return Err(not_found(None));
    }

    let mut dirs: Vec<(String, PathBuf)> = Vec::new();
    for entry in fs::read_dir(&cfg.customers_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            dirs.push((
                entry.file_name().to_string_lossy().to_string(),
                entry.path(),
            ));
        }
    }
    dirs.sort();

    // 1. By ID (loose: CUST-1, cust-001, 1) — directory is `<ID>-<slug>`.
    if let Ok((id, _)) = suggest::normalize_id(id_or_slug, cfg, Some(EntityKind::Customer)) {
        let prefix = format!("{id}-");
        if let Some((_, p)) = dirs
            .iter()
            .find(|(n, _)| n.starts_with(&prefix) || *n == id)
        {
            return Ok(p.clone());
        }
    }

    // 2. By exact slug, then by unique partial slug. Folders created before
    //    slugify transliterated umlauts (`CUST-001-m-ller`) still match.
    let slugs: Vec<String> = util::slug_variants(id_or_slug)
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    let slug_of = |name: &str| -> String {
        let parts: Vec<&str> = name.splitn(3, '-').collect();
        parts.get(2).copied().unwrap_or(name).to_string()
    };
    if let Some((_, p)) = dirs.iter().find(|(n, _)| slugs.contains(&slug_of(n))) {
        return Ok(p.clone());
    }
    let partial: Vec<&(String, PathBuf)> = dirs
        .iter()
        .filter(|(n, _)| {
            let folder = slug_of(n);
            slugs
                .iter()
                .any(|s| s.len() >= 3 && folder.contains(s.as_str()))
        })
        .collect();
    match partial.as_slice() {
        [(_, p)] => Ok(p.clone()),
        [] => {
            let names: Vec<&str> = dirs.iter().map(|(n, _)| n.as_str()).collect();
            let hint = match suggest::did_you_mean(id_or_slug, names.iter().copied()) {
                Some(n) => format!("did you mean {n}? Run `mc list customers` to browse"),
                None => "run `mc list customers` to see IDs and names".into(),
            };
            Err(not_found(Some(hint)))
        }
        many => Err(McError::usage(
            format!("'{id_or_slug}' matches {} customers", many.len()),
            Some(format!(
                "be more specific: {}",
                many.iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::init;
    use crate::config;

    #[test]
    fn finds_customer_folders_slugged_under_old_and_new_rules() {
        let tmp = tempfile::TempDir::new().unwrap();
        init::run(tmp.path(), false, false, Some("T"), false, true).unwrap();
        let cfg = config::load_config(tmp.path(), config::RepoMode::Standalone).unwrap();
        let old = cfg.customers_dir.join("CUST-001-m-ller-gmbh");
        let new = cfg.customers_dir.join("CUST-002-schoene-tage");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();

        assert_eq!(find_customer_dir(&cfg, "Müller GmbH").unwrap(), old);
        assert_eq!(find_customer_dir(&cfg, "müller").unwrap(), old);
        assert_eq!(find_customer_dir(&cfg, "Schöne Tage").unwrap(), new);
        assert_eq!(find_customer_dir(&cfg, "CUST-1").unwrap(), old);
    }

    fn write(path: PathBuf, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn zip_entries(cfg: &ResolvedConfig) -> Vec<String> {
        let zip = fs::read_dir(&cfg.archive_dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "zip"))
            .unwrap();
        let archive = zip::ZipArchive::new(fs::File::open(&zip).unwrap()).unwrap();
        let mut names: Vec<String> = archive.file_names().map(str::to_string).collect();
        names.sort();
        fs::remove_file(zip).unwrap();
        names
    }

    #[test]
    fn export_includes_linked_entities_and_skips_dotfiles() {
        let tmp = tempfile::TempDir::new().unwrap();
        init::run(tmp.path(), false, false, Some("T"), false, true).unwrap();
        let cfg = config::load_config(tmp.path(), config::RepoMode::Standalone).unwrap();
        let cust = cfg.customers_dir.join("CUST-001-acme");
        write(
            cust.join("CUST-001.md"),
            "---\nid: CUST-001\nname: Acme\n---\n",
        );
        write(cust.join(".gitkeep"), "");
        write(cust.join("meetings/.DS_Store"), "x");
        write(
            cust.join("tasks/todo/TASK-001-own.md"),
            "---\nid: TASK-001\ntitle: Own\ncustomers: ['[[CUST-001]]']\n---\n",
        );
        write(
            cfg.meetings_dir.join("2026-01-01-kickoff.md"),
            "---\nid: MTG-001\ntitle: Kickoff\ncustomers: ['[[CUST-001|Acme]]']\n---\n",
        );
        write(
            cfg.meetings_dir.join("2026-01-02-other.md"),
            "---\nid: MTG-002\ntitle: Other\ncustomers: [CUST-0010]\n---\n",
        );
        let proj = cfg.projects_dir.join("PROJ-001-site");
        write(
            proj.join("PROJ-001.md"),
            "---\nid: PROJ-001\nname: Site\ncustomer: CUST-001-acme\n---\n",
        );
        write(proj.join("docs/spec.md"), "# Spec\n");
        write(proj.join("assets/video.mp4"), "binary");
        write(
            proj.join("tasks/todo/TASK-002-in-project.md"),
            "---\nid: TASK-002\ntitle: P\ncustomers: [CUST-001]\n---\n",
        );
        write(
            cfg.tasks_dir.join("todo/TASK-003-global.md"),
            "---\nid: TASK-003\ntitle: G\ncustomers: ['[[CUST-001]]']\n---\n",
        );

        export_customer(&cfg, "CUST-001", false).unwrap();
        let names = zip_entries(&cfg);
        let files: Vec<&str> = names
            .iter()
            .map(String::as_str)
            .filter(|n| !n.ends_with('/'))
            .collect();
        assert_eq!(
            files,
            [
                "CUST-001.md",
                "related/meetings/2026-01-01-kickoff.md",
                "related/projects/PROJ-001-site/PROJ-001.md",
                "related/projects/PROJ-001-site/docs/spec.md",
                "related/projects/PROJ-001-site/tasks/todo/TASK-002-in-project.md",
                "related/tasks/TASK-003-global.md",
                "tasks/todo/TASK-001-own.md",
            ]
        );

        export_customer(&cfg, "CUST-001", true).unwrap();
        let names = zip_entries(&cfg);
        assert!(names
            .iter()
            .all(|n| !n.starts_with("related/") && !n.contains("/.")));
        assert!(names.contains(&"CUST-001.md".to_string()));
    }
}
