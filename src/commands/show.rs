use crate::config::ResolvedConfig;
use crate::data;
use crate::error::McResult;
use colored::*;
use serde_yaml::Value;

pub fn run(id: &str, cfg: &ResolvedConfig) -> McResult<()> {
    let entity = data::find_entity_by_id(id, cfg)?;

    let kind_label = entity.kind.label().to_uppercase();
    let kind_colored = match entity.kind.label() {
        "customer" => kind_label.blue().bold(),
        "project" => kind_label.green().bold(),
        "meeting" => kind_label.magenta().bold(),
        "research" => kind_label.cyan().bold(),
        "task" => kind_label.purple().bold(),
        _ => kind_label.bold(),
    };

    // Get status for inline display
    let status_str = if let Some(map) = entity.frontmatter.as_mapping() {
        map.iter()
            .find(|(k, _)| k.as_str() == Some("status"))
            .and_then(|(_, v)| v.as_str())
            .unwrap_or("")
    } else {
        ""
    };
    let status_display = crate::commands::list::format_status(status_str);

    println!();
    println!("  {} {} {}", kind_colored, id.cyan().bold(), status_display);
    println!(
        "  {}",
        "────────────────────────────────────────────────".dimmed()
    );

    // Print all frontmatter fields
    if let Some(map) = entity.frontmatter.as_mapping() {
        for (key, value) in map {
            let key_str = key.as_str().unwrap_or("");
            if key_str.starts_with('_') || key_str == "id" || key_str == "status" {
                continue;
            }
            let val_str = format_value(value);
            println!("  {:<14} {}", format!("{}:", key_str).dimmed(), val_str);
        }
    }

    println!(
        "  {}",
        "------------------------------------------------".dimmed()
    );
    println!(
        "  {} {}",
        "source:".dimmed(),
        entity.source_path.display().to_string().dimmed()
    );
    println!();

    // Print body preview (first 20 non-empty lines)
    let body_lines: Vec<&str> = entity
        .body
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(20)
        .collect();
    if !body_lines.is_empty() {
        for line in body_lines {
            if line.starts_with("# ") {
                println!("  {}", line.bold());
            } else if line.starts_with("## ") {
                println!("  {}", line.yellow());
            } else {
                println!("  {}", line);
            }
        }
        println!();
    }

    Ok(())
}

fn format_value(value: &Value) -> String {
    match value {
        Value::String(s) => {
            if s.is_empty() {
                "(empty)".dimmed().to_string()
            } else {
                s.clone()
            }
        }
        Value::Sequence(seq) => {
            if seq.is_empty() {
                "[]".dimmed().to_string()
            } else {
                let items: Vec<String> = seq
                    .iter()
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        Value::Mapping(_) => "(object)".to_string(),
                        _ => format!("{:?}", v),
                    })
                    .collect();
                items.join(", ")
            }
        }
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Null => "(null)".dimmed().to_string(),
        _ => format!("{:?}", value),
    }
}
