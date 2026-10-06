//! `mc check`: list an entity's Markdown checklist, or tick/untick an item.

use crate::checklist::{self, Target};
use crate::cli::markdown::sanitize;
use crate::cli::suggest;
use crate::cli::ui::{self, Tone};
use crate::config::ResolvedConfig;
use crate::error::McResult;
use colored::*;
use serde_json::json;

pub fn run(id: &str, item: Option<usize>, uncheck: bool, cfg: &ResolvedConfig) -> McResult<()> {
    let entity = suggest::find_entity(id, cfg, None)?;
    let Some(n) = item else {
        let content = std::fs::read_to_string(&entity.source_path)?;
        let items = checklist::entity_items(entity.kind, &content);
        let (done, total) = checklist::progress(&items);
        if ui::get().json {
            let out = json!({"id": entity.id, "items": items, "done": done, "total": total});
            println!("{}", serde_json::to_string_pretty(&out)?);
            return Ok(());
        }
        if items.is_empty() {
            ui::info(format!(
                "{} has no checklist items",
                entity.id.cyan().bold()
            ));
            ui::hint("add lines like `- [ ] Do it` to its Markdown file");
            return Ok(());
        }
        println!(
            "{}  {}  {}",
            entity.id.cyan().bold(),
            progress_bar(done, total),
            format!("{done} of {total} done").dimmed()
        );
        let width = total.to_string().len();
        // Item text comes straight from the file: no escape sequences.
        for i in &items {
            let n = ui::pad_left(&i.index.to_string(), width);
            let text = sanitize(&i.text);
            if i.checked {
                println!("  {}  {} {}", n.dimmed(), "[x]".green(), text.dimmed());
            } else {
                println!("  {}  [ ] {}", n.dimmed(), text);
            }
        }
        ui::hint(format!(
            "tick one with {}",
            ui::cmd(&format!("mc check {} <number>", entity.id))
        ));
        return Ok(());
    };

    let change = checklist::set_checked(&entity, Target::Index(n), !uncheck, None, None)?;
    if ui::get().json {
        let out = json!({
            "id": entity.id,
            "item": change.item,
            "changed": change.changed,
            "done": change.done,
            "total": change.total,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }
    let state = if uncheck { "unchecked" } else { "checked" };
    let what = format!(
        "{} #{} {}",
        entity.id.cyan().bold(),
        n,
        sanitize(&change.item.text).bold()
    );
    if change.changed {
        ui::success(format!("{} {what}", capitalize(state)));
    } else {
        ui::info(format!(
            "{what} is already {state} {}",
            "(nothing changed)".dimmed()
        ));
    }
    println!(
        "  {}  {}",
        progress_bar(change.done, change.total),
        format!("{} of {} done", change.done, change.total).dimmed()
    );
    Ok(())
}

/// A 12-cell green bar for `done` of `total` checklist items.
pub(crate) fn progress_bar(done: usize, total: usize) -> String {
    let fraction = if total == 0 {
        0.0
    } else {
        done as f64 / total as f64
    };
    ui::bar(fraction, 12, Tone::Good)
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next()
        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
        .unwrap_or_default()
}
