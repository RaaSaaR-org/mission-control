//! `mc api serve` — bootstrap the HTTP API.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;

use crate::api::auth::TokenStore;
use crate::api::{serve_with_lock, ApiServerConfig, RepoLock};
use crate::cli::ApiSubcommand;
use crate::config::ResolvedConfig;
use crate::error::{McError, McResult};

pub fn run(subcmd: &ApiSubcommand, cfg: &ResolvedConfig) -> McResult<()> {
    match subcmd {
        ApiSubcommand::Serve {
            port,
            bind,
            tokens_file,
            insecure_dev_token,
            read_only,
            log_format,
        } => run_serve(
            cfg,
            *port,
            bind,
            tokens_file.as_deref(),
            *insecure_dev_token,
            *read_only,
            log_format,
        ),
        ApiSubcommand::HashToken { secret } => run_hash_token(secret.as_deref()),
    }
}

pub fn run_hash_token(secret: Option<&str>) -> McResult<()> {
    use argon2::password_hash::{rand_core::OsRng, SaltString};
    use argon2::{Argon2, PasswordHasher};
    use std::io::{BufRead, IsTerminal};

    let plain = match secret {
        Some(s) => s.to_string(),
        None => {
            if std::io::stdin().is_terminal() {
                return Err(McError::Other(
                    "no secret provided and stdin is a terminal — pass the secret as an argument or pipe it in"
                        .into(),
                ));
            }
            let mut line = String::new();
            std::io::stdin()
                .lock()
                .read_line(&mut line)
                .map_err(McError::Io)?;
            line.trim_end_matches(['\n', '\r']).to_string()
        }
    };
    if plain.is_empty() {
        return Err(McError::Other("secret is empty".into()));
    }

    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map_err(|e| McError::Other(format!("argon2 hash: {e}")))?
        .to_string();
    println!("{hash}");
    Ok(())
}

fn run_serve(
    cfg: &ResolvedConfig,
    port: u16,
    bind: &str,
    tokens_file: Option<&str>,
    insecure_dev_token: bool,
    read_only: bool,
    log_format: &str,
) -> McResult<()> {
    init_tracing(log_format)?;

    // Acquire the repo lock FIRST — before generating any dev token or
    // printing any banner. A second `mc api serve` against the same repo
    // should fail fast and not leave the user holding an unusable bearer
    // token in their scrollback.
    let repo_lock = RepoLock::acquire(&cfg.root)?;

    let tokens = match (tokens_file, insecure_dev_token) {
        (Some(path), false) => {
            let tokens_path = PathBuf::from(path);
            TokenStore::from_file(&tokens_path).map_err(|e| {
                McError::Other(format!("load tokens file {}: {}", tokens_path.display(), e))
            })?
        }
        (None, true) => generate_dev_token_store()?,
        (None, false) => {
            return Err(McError::Other(
                "no token source — pass --tokens-file <path>, or use --insecure-dev-token for local development".into(),
            ));
        }
        (Some(_), true) => unreachable!("clap conflicts_with prevents this"),
    };

    let bind_ip: std::net::IpAddr = bind
        .parse()
        .map_err(|e| McError::Other(format!("invalid --bind {bind}: {e}")))?;
    let bind_addr = SocketAddr::new(bind_ip, port);

    let server_cfg = ApiServerConfig {
        bind: bind_addr,
        tokens,
        read_only,
    };

    let cfg_clone = cfg.clone();
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move { serve_with_lock(cfg_clone, server_cfg, repo_lock).await })
}

/// Build a one-token in-memory `TokenStore` containing a freshly generated
/// random secret. Print the plaintext token to stderr (with a loud warning)
/// so the developer can use it for the lifetime of the server. Used by
/// `--insecure-dev-token`.
fn generate_dev_token_store() -> McResult<TokenStore> {
    use argon2::password_hash::{rand_core::OsRng, SaltString};
    use argon2::{Argon2, PasswordHasher};
    use colored::Colorize;

    let secret = uuid::Uuid::new_v4().to_string();
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(secret.as_bytes(), &salt)
        .map_err(|e| McError::Other(format!("argon2 hash: {e}")))?
        .to_string();

    eprintln!();
    eprintln!(
        "{}",
        "════════════════════════════════════════════════════════════".yellow()
    );
    eprintln!(
        "{} --insecure-dev-token enabled. {} use in production.",
        "WARNING:".yellow().bold(),
        "DO NOT".red().bold()
    );
    eprintln!("Anyone with read access to this terminal has full control of your repo.");
    eprintln!();
    eprintln!("  Bearer token: {}", secret.cyan().bold());
    eprintln!();
    eprintln!("Use it like:");
    eprintln!(
        "  curl -H \"Authorization: Bearer {}\" http://127.0.0.1:5100/v1/tasks",
        secret
    );
    eprintln!(
        "{}",
        "════════════════════════════════════════════════════════════".yellow()
    );
    eprintln!();

    let yaml = format!(
        "tokens:\n  - name: dev-token\n    hash: \"{hash}\"\n    capabilities: [read, write]\n"
    );
    TokenStore::from_yaml(&yaml).map_err(|e| McError::Other(format!("dev token: {e}")))
}

fn init_tracing(format: &str) -> McResult<()> {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    match format {
        "json" => {
            // try_init returns Err if a subscriber is already installed (e.g. tests).
            let _ = tracing_subscriber::fmt()
                .with_env_filter(filter)
                .json()
                .try_init();
        }
        "human" => {
            let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
        }
        other => {
            return Err(McError::Other(format!(
                "invalid --log-format {other} (expected 'human' or 'json')"
            )));
        }
    }
    Ok(())
}

// SocketAddr does not implement FromStr the way we want for split bind+port,
// but the conversion above is enough. Keep this trait import live in case the
// bind string ever evolves to include a port.
#[allow(dead_code)]
fn _force_use_from_str() -> Option<SocketAddr> {
    SocketAddr::from_str("127.0.0.1:0").ok()
}
