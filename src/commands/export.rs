use crate::cli::suggest;
use crate::cli::ui;
use crate::cli::ExportEntity;
use crate::config::ResolvedConfig;
use crate::entity::EntityKind;
use crate::error::{McError, McResult};
use crate::util;
use colored::*;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

pub fn run(entity: &ExportEntity, cfg: &ResolvedConfig) -> McResult<()> {
    match entity {
        ExportEntity::Customer { id } => export_customer(cfg, id),
    }
}

fn export_customer(cfg: &ResolvedConfig, id_or_slug: &str) -> McResult<()> {
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
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // Walk the customer directory and add all files
    let mut files = 0usize;
    for entry in WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let rel_path = path.strip_prefix(&dir).unwrap_or(path);

        if path.is_dir() {
            if rel_path.to_string_lossy().is_empty() {
                continue;
            }
            let dir_entry = format!("{}/", rel_path.to_string_lossy());
            zip.add_directory(&dir_entry, options)?;
        } else {
            let file_entry = rel_path.to_string_lossy().to_string();
            zip.start_file(&file_entry, options)?;
            let data = fs::read(path)?;
            zip.write_all(&data)?;
            files += 1;
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
        let out = serde_json::json!({
            "customer": dir_name,
            "path": zip_path.display().to_string(),
            "files": files,
            "bytes": bytes,
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

    Ok(())
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
}
