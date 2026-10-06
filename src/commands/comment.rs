//! `mc comment`: add a comment to a task or meeting.

use crate::cli::suggest;
use crate::cli::ui;
use crate::comments;
use crate::config::ResolvedConfig;
use crate::error::McResult;
use colored::*;
use std::io::Read;

/// Add `text` (or stdin when it is `-`) as a comment on `id`.
pub fn run(id: &str, text: &str, author: Option<&str>, cfg: &ResolvedConfig) -> McResult<()> {
    let entity = suggest::find_entity(id, cfg, None)?;
    let text = if text == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        text.to_string()
    };
    let added = comments::add(cfg, &entity, &text, author)?;
    if ui::get().json {
        println!("{}", serde_json::to_string_pretty(&added)?);
        return Ok(());
    }
    let who = added.comment.author.as_deref().unwrap_or("unknown");
    ui::success(format!(
        "Commented on {} as {}",
        added.id.cyan().bold(),
        who.bold()
    ));
    let c = &added.comment;
    let when = [c.date.as_deref(), c.time.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    println!(
        "  {}",
        format!(
            "{when} {} {} {} {}",
            ui::glyphs().sep,
            ui::count(added.count, "comment", "comments"),
            ui::glyphs().sep,
            added.path
        )
        .dimmed()
    );
    Ok(())
}
