use crate::cli::ExportEntity;
use crate::config::ResolvedConfig;
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
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // Walk the customer directory and add all files
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
        }
    }

    zip.finish()?;

    println!(
        "{} Exported {} to {}",
        "✓".green().bold(),
        dir_name.cyan().bold(),
        zip_path.display().to_string().dimmed()
    );

    Ok(())
}

fn find_customer_dir(cfg: &ResolvedConfig, id_or_slug: &str) -> McResult<PathBuf> {
    if !cfg.customers_dir.is_dir() {
        return Err(McError::EntityNotFound(id_or_slug.to_string()));
    }

    let slug = util::slugify(id_or_slug);

    for entry in fs::read_dir(&cfg.customers_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        // Match by ID prefix (e.g., "CUST-001") or by slug
        if name.starts_with(id_or_slug) || name.contains(&slug) {
            return Ok(entry.path());
        }
    }

    Err(McError::EntityNotFound(id_or_slug.to_string()))
}
