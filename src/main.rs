use clap::error::ErrorKind;
use clap::{CommandFactory, FromArgMatches};
use colored::*;
use mc::cli::{suggest, ui, ApiSubcommand, Cli, Command, ListEntity, PrintEntity, TaskSubcommand};
use mc::config::ResolvedConfig;
use mc::entity::EntityKind;
use mc::error::{McError, McResult};
use mc::{commands, config};
use std::ffi::OsString;
use std::io::IsTerminal;

fn main() {
    let mut cli = parse_cli();
    ui::init(cli.color, cli.json);

    // Long-running servers keep Rust's default (ignore SIGPIPE); one-shot
    // commands die quietly when piped into `head` instead of panicking.
    if !matches!(
        cli.command,
        Command::Serve { .. } | Command::Mcp | Command::Api { .. }
    ) {
        reset_sigpipe();
    }

    if let Err(e) = run(&mut cli) {
        report_error(&e, cli.json);
        std::process::exit(e.exit_code());
    }
}

/// Parse the command line. `--color` also applies to clap's own help and
/// errors, and with `--json` argument errors are JSON like every other error.
fn parse_cli() -> Cli {
    let args: Vec<OsString> = std::env::args_os().collect();
    // Only flags before a `--` separator count.
    let flags = || {
        args.iter()
            .skip(1)
            .map(|a| a.to_string_lossy())
            .take_while(|a| a != "--")
    };
    let json = flags().any(|a| a == "--json");
    let mut color = None;
    let mut prev_color = false;
    for a in flags() {
        let value = match a.strip_prefix("--color=") {
            Some(v) => Some(v.to_string()),
            None if prev_color => Some(a.to_string()),
            None => None,
        };
        prev_color = a == "--color";
        match value.as_deref() {
            Some("never") => color = Some(clap::ColorChoice::Never),
            Some("always") => color = Some(clap::ColorChoice::Always),
            Some("auto") => color = Some(clap::ColorChoice::Auto),
            _ => {}
        }
    }

    let mut cmd = Cli::command();
    if let Some(c) = color {
        cmd = cmd.color(c);
    }
    let parsed = cmd
        .try_get_matches_from_mut(&args)
        .and_then(|m| Cli::from_arg_matches(&m).map_err(|e| e.format(&mut cmd)));
    match parsed {
        Ok(cli) => cli,
        Err(e) => {
            let informational = matches!(
                e.kind(),
                ErrorKind::DisplayHelp
                    | ErrorKind::DisplayVersion
                    | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            );
            if !json || informational {
                e.exit();
            }
            let (message, hint) = clap_error_parts(&e.to_string());
            let payload = serde_json::json!({
                "error": { "message": message, "hint": hint, "exit_code": e.exit_code() }
            });
            eprintln!("{payload}");
            std::process::exit(e.exit_code());
        }
    }
}

/// Message and hint (clap's `tip:` lines) of a rendered clap error.
fn clap_error_parts(rendered: &str) -> (String, Option<String>) {
    let mut lines = rendered.lines();
    let message = lines
        .next()
        .unwrap_or_default()
        .trim_start_matches("error: ")
        .to_string();
    let tips: Vec<&str> = lines
        .filter_map(|l| l.trim().strip_prefix("tip: "))
        .collect();
    (message, (!tips.is_empty()).then(|| tips.join("\n")))
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
            | Command::New { .. }
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
        Command::Completions { .. } => "completions",
        _ => "this command",
    }
}

/// Canonical IDs for the ID arguments that commands compare literally
/// (`--project proj-1` → `PROJ-001`), so loose input works there as it does
/// for `mc show`. An unknown ID fails with a did-you-mean instead of
/// quietly matching nothing.
fn normalize_ids(cmd: &mut Command, cfg: &ResolvedConfig) -> McResult<()> {
    let canon = |v: &mut Option<String>, kind| -> McResult<()> {
        if let Some(s) = v {
            *s = canonical_id(s, kind, cfg)?;
        }
        Ok(())
    };
    match cmd {
        Command::List {
            entity: ListEntity::Tasks {
                project, customer, ..
            },
        }
        | Command::Task {
            subcmd:
                TaskSubcommand::Board {
                    project, customer, ..
                }
                | TaskSubcommand::Next {
                    project, customer, ..
                },
        } => {
            canon(project, EntityKind::Project)?;
            canon(customer, EntityKind::Customer)?;
        }
        Command::List {
            entity: ListEntity::Contacts { customer, .. },
        } => canon(customer, EntityKind::Customer)?,
        Command::Print {
            entity: PrintEntity::Meeting { id, .. },
        } => *id = canonical_id(id, EntityKind::Meeting, cfg)?,
        Command::Print {
            entity: PrintEntity::Research { id, .. },
        } => *id = canonical_id(id, EntityKind::Research, cfg)?,
        _ => {}
    }
    Ok(())
}

/// The ID of the `kind` entity `input` names. Input that isn't ID-shaped (a
/// slug, a free-form label) and kinds this repo doesn't have pass through.
fn canonical_id(input: &str, kind: EntityKind, cfg: &ResolvedConfig) -> McResult<String> {
    if !cfg.entity_available(&kind) {
        return Ok(input.to_string());
    }
    match suggest::normalize_id(input, cfg, Some(kind)) {
        Ok((_, k)) if k != kind => Err(McError::usage(
            format!("'{input}' is a {} ID, not a {} ID", k.label(), kind.label()),
            Some(format!(
                "{} IDs look like {}-001",
                kind.label(),
                kind.prefix(cfg)
            )),
        )),
        Ok(_) => suggest::find_entity(input, cfg, Some(kind)).map(|e| e.id),
        Err(_) => Ok(input.to_string()),
    }
}

fn run(cli: &mut Cli) -> McResult<()> {
    if cli.json && !supports_json(&cli.command) {
        return Err(McError::usage(
            format!(
                "--json is not supported by `mc {}`",
                command_name(&cli.command)
            ),
            Some(
                "JSON output is available for list, show, status, validate, index, export, task, check, comment and new"
                    .into(),
            ),
        ));
    }

    // Completions and hash-token don't need a repo; they're pure utilities.
    if let Command::Completions { shell } = &cli.command {
        clap_complete::generate(*shell, &mut Cli::command(), "mc", &mut std::io::stdout());
        return Ok(());
    }

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
    normalize_ids(&mut cli.command, &cfg)?;

    match &cli.command {
        Command::Init { .. } | Command::Completions { .. } => {
            unreachable!("handled before config loading")
        }
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
            commands::comment::run(id, text.as_deref(), author.as_deref(), &cfg)
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
