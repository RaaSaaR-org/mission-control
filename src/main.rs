use clap::Parser;
use colored::*;
use mc::cli::{ui, ApiSubcommand, Cli, Command};
use mc::error::{McError, McResult};
use mc::{commands, config};
use std::io::IsTerminal;

fn main() {
    let cli = Cli::parse();
    ui::init(cli.color, cli.json);

    // Long-running servers keep Rust's default (ignore SIGPIPE); one-shot
    // commands die quietly when piped into `head` instead of panicking.
    if !matches!(
        cli.command,
        Command::Serve { .. } | Command::Mcp | Command::Api { .. }
    ) {
        reset_sigpipe();
    }

    if let Err(e) = run(&cli) {
        report_error(&e, cli.json);
        std::process::exit(e.exit_code());
    }
}

fn report_error(e: &McError, json: bool) {
    if json {
        let payload = serde_json::json!({
            "error": {
                "message": e.to_string(),
                "hint": e.hint(),
                "exit_code": e.exit_code(),
            }
        });
        eprintln!("{payload}");
        return;
    }
    // stderr may be a terminal even when stdout is piped (and vice versa).
    let paint = ui::get().color && std::io::stderr().is_terminal();
    let (label, hint) = if paint {
        (
            "error:".red().bold().to_string(),
            "hint:".cyan().bold().to_string(),
        )
    } else {
        ("error:".to_string(), "hint:".to_string())
    };
    eprintln!("{label} {}", ui::error_message(&e.to_string()));
    if let Some(h) = e.hint() {
        let mut lines = h.lines();
        if let Some(first) = lines.next() {
            eprintln!("  {hint} {first}");
        }
        for line in lines {
            eprintln!("        {line}");
        }
    }
}

#[cfg(unix)]
fn reset_sigpipe() {
    const SIGPIPE: i32 = 13;
    const SIG_DFL: usize = 0;
    extern "C" {
        fn signal(signum: i32, handler: usize) -> usize;
    }
    // SAFETY: restoring the default disposition for SIGPIPE is async-signal-safe
    // and happens before any other threads are spawned.
    unsafe {
        signal(SIGPIPE, SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {}

/// Commands that understand `--json`.
fn supports_json(cmd: &Command) -> bool {
    matches!(
        cmd,
        Command::List { .. }
            | Command::Show { .. }
            | Command::Index
            | Command::Export { .. }
            | Command::Validate
            | Command::Status
            | Command::Task { .. }
            | Command::Check { .. }
            | Command::Comment { .. }
    )
}

fn command_name(cmd: &Command) -> &'static str {
    match cmd {
        Command::New { .. } => "new",
        Command::Print { .. } => "print",
        Command::Serve { .. } => "serve",
        Command::Mcp => "mcp",
        Command::Api { .. } => "api",
        Command::Init { .. } => "init",
        _ => "this command",
    }
}

fn run(cli: &Cli) -> McResult<()> {
    if cli.json && !supports_json(&cli.command) {
        return Err(McError::usage(
            format!(
                "--json is not supported by `mc {}`",
                command_name(&cli.command)
            ),
            Some(
                "JSON output is available for list, show, status, validate, index, export, task, check and comment"
                    .into(),
            ),
        ));
    }

    // hash-token doesn't need a repo — it's a pure utility.
    if let Command::Api {
        subcmd: ApiSubcommand::HashToken { secret },
    } = &cli.command
    {
        return commands::api::run_hash_token(secret.as_deref());
    }

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
        Command::Show {
            id,
            raw,
            no_pager,
            open,
            full: _,
        } => commands::show::run(
            id,
            commands::show::ShowOptions {
                raw: *raw,
                no_pager: *no_pager,
                open: *open,
            },
            &cfg,
        ),
        Command::Check { id, item, uncheck } => commands::check::run(id, *item, *uncheck, &cfg),
        Command::Comment { id, text, author } => {
            commands::comment::run(id, text, author.as_deref(), &cfg)
        }
        Command::Index => commands::index::run(&cfg),
        Command::Export { entity } => commands::export::run(entity, &cfg),
        Command::Print { entity } => commands::print::run(entity, &cfg),
        Command::Validate => commands::validate::run(&cfg),
        Command::Status => commands::status::run(&cfg),
        Command::Serve {
            port,
            base_path,
            read_only,
            allow_edits,
        } => commands::serve::run(
            &cfg,
            *port,
            &commands::serve::ServeOptions {
                base_path: base_path.clone(),
                read_only: *read_only,
                allow_edits: *allow_edits,
            },
        ),
        Command::Mcp => commands::mcp::run(&cfg),
        Command::Api { subcmd } => commands::api::run(subcmd, &cfg),
        Command::Task { subcmd } => commands::task::run(subcmd, &cfg),
    }
}
