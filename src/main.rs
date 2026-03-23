mod cli;
mod commands;
mod config;
mod data;
mod entity;
mod error;
mod frontmatter;
mod html;
mod mcp;
mod template;
mod util;

use clap::Parser;
use cli::{Cli, Command};
use colored::*;
use error::McResult;

fn main() {
    let cli = Cli::parse();

    if let Err(e) = run(&cli) {
        eprintln!("{} {}", "error:".red().bold(), e);
        if let Some(hint) = e.hint() {
            eprintln!("{} {}", "hint:".yellow().bold(), hint);
        }
        std::process::exit(1);
    }
}

fn run(cli: &Cli) -> McResult<()> {
    // Init is handled before config loading (config doesn't exist yet)
    if let Command::Init {
        project,
        embedded,
        name,
        path,
        force,
    } = &cli.command
    {
        let target = match path {
            Some(p) => std::path::PathBuf::from(p),
            None => std::env::current_dir()?,
        };
        return commands::init::run(
            &target,
            *project,
            *embedded,
            name.as_deref(),
            *force,
            cli.yes,
        );
    }

    // Determine repo root and mode
    let (root, mode) = match &cli.root {
        Some(path) => {
            let r = std::path::PathBuf::from(path);
            let m = config::detect_mode(&r);
            (r, m)
        }
        None => config::find_repo_root(&std::env::current_dir()?)?,
    };

    let cfg = config::load_config(&root, mode)?;

    match &cli.command {
        Command::Init { .. } => unreachable!("Init is handled before config loading"),
        Command::New { entity } => commands::new::run(entity, &cfg, cli.yes),
        Command::List { entity } => commands::list::run(entity, &cfg),
        Command::Show { id } => commands::show::run(id, &cfg),
        Command::Index => commands::index::run(&cfg),
        Command::Export { entity } => commands::export::run(entity, &cfg),
        Command::Print { entity } => commands::print::run(entity, &cfg),
        Command::Validate => commands::validate::run(&cfg),
        Command::Status => commands::status::run(&cfg),
        Command::Serve { port, base_path } => commands::serve::run(&cfg, *port, base_path),
        Command::Mcp => commands::mcp::run(&cfg),
        Command::Task { subcmd } => commands::task::run(subcmd, &cfg),
    }
}
