//! Command-line entry point.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use smtp_test_tool::config::{default_save_path, discover_config_path, Config};
use smtp_test_tool::providers::{self, Provider};
use smtp_test_tool::{outlook_defaults, run_tests, Profile};
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "smtp-test-tool",
    version,
    about,
    long_about = "Test SMTP / IMAP / POP3 connectivity to any mail server.\n\
                        Defaults to Outlook.com / Office 365."
)]
struct Cli {
    /// TOML config file to load.
    #[arg(short, long, env = "SMTP_TEST_TOOL_CONFIG")]
    config: Option<PathBuf>,

    /// Profile within the config file (default: 'default').
    #[arg(short, long)]
    profile: Option<String>,

    /// Username (overrides config).
    #[arg(short, long)]
    user: Option<String>,

    /// Password (omit to prompt).
    #[arg(short = 'P', long)]
    password: Option<String>,

    /// Bearer token for XOAUTH2 (overrides --password).
    #[arg(long)]
    oauth_token: Option<String>,

    /// Apply a built-in provider preset (overwrites smtp/imap/pop3 host,
    /// port, and security on the active profile).  Run `smtp-test-tool
    /// providers` for the list of valid names.  Matching is case-insensitive
    /// and accepts any unique substring, so `--provider gmail` and
    /// `--provider "google workspace"` both pick Gmail.
    #[arg(long, value_name = "NAME")]
    provider: Option<String>,

    /// Disable certificate verification (testing only).
    #[arg(long)]
    insecure: bool,

    /// Override log level (trace, debug, info, warn, error).
    #[arg(long)]
    log_level: Option<String>,

    /// Sub-commands.  Default action is 'test' against the loaded profile.
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run the connectivity test (default action).
    Test,
    /// List profiles in the loaded config file.
    Profiles,
    /// Print the Outlook.com defaults as a starter TOML.
    Init {
        /// File to write (default: ./smtp_test_tool.toml).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// List the built-in provider presets that --provider accepts.
    Providers,
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<bool> {
    let cli = Cli::parse();

    // ---- logging --------------------------------------------------------
    let lvl = cli.log_level.clone().unwrap_or_else(|| "info".into());
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&lvl));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_level(true)
        .with_ansi(supports_colour())
        .with_writer(io::stderr)
        .init();

    // ---- locate config --------------------------------------------------
    let cfg_path = cli.config.clone().or_else(discover_config_path);
    let cfg = match &cfg_path {
        Some(p) => Config::load(p).with_context(|| format!("loading {}", p.display()))?,
        None => Config {
            active: "default".into(),
            profiles: [("default".into(), outlook_defaults())]
                .into_iter()
                .collect(),
        },
    };

    let profile_name = cli.profile.clone().unwrap_or_else(|| cfg.active.clone());

    match cli.cmd.unwrap_or(Cmd::Test) {
        Cmd::Profiles => {
            match &cfg_path {
                Some(p) => println!("Profiles in {}:", p.display()),
                None => println!("No config file loaded; using built-in defaults."),
            }
            for n in cfg.profile_names() {
                println!("  {n}{}", if n == cfg.active { "  (active)" } else { "" });
            }
            return Ok(true);
        }
        Cmd::Providers => {
            println!("Built-in provider presets (use with --provider):");
            let mut max_name = 0;
            for p in providers::PROVIDERS {
                max_name = max_name.max(p.name.len());
            }
            for p in providers::PROVIDERS {
                println!(
                    "  {:<width$}  smtp={}:{}  imap={}:{}{}",
                    p.name,
                    p.smtp.host,
                    p.smtp.port,
                    p.imap.host,
                    p.imap.port,
                    if p.pop.is_none() { "  (no POP3)" } else { "" },
                    width = max_name
                );
                if let Some(note) = p.note {
                    println!("  {:width$}    note: {}", "", note, width = max_name);
                }
            }
            return Ok(true);
        }
        Cmd::Init { output } => {
            let mut new_cfg = Config {
                active: "default".into(),
                profiles: Default::default(),
            };
            new_cfg.upsert_profile("default", outlook_defaults());
            let target = output.unwrap_or_else(default_save_path);
            new_cfg.save(&target)?;
            println!("Wrote starter config to {}", target.display());
            return Ok(true);
        }
        Cmd::Test => { /* fall through */ }
    }

    // ---- build the effective profile (CLI overrides config) ------------
    let mut profile: Profile = cfg
        .profile(&profile_name)
        .cloned()
        .unwrap_or_else(outlook_defaults);

    // Apply --provider BEFORE the credential overrides so user/password
    // entered on the command line win over anything the preset implies
    // (though presets never populate credentials).
    if let Some(name) = cli.provider.as_deref() {
        let p = resolve_provider(name)?;
        apply_provider_to(&mut profile, p);
        tracing::info!("applied provider preset: {}", p.name);
        if let Some(note) = p.note {
            tracing::info!("  note: {note}");
        }
    }

    if let Some(u) = cli.user {
        profile.user = Some(u);
    }
    if let Some(p) = cli.password {
        profile.password = Some(p);
    }
    if let Some(t) = cli.oauth_token {
        profile.oauth_token = Some(t);
    }
    if cli.insecure {
        profile.insecure_tls = true;
    }

    if profile.user.is_none() {
        profile.user = Some(prompt("Username / email: ")?);
    }
    if profile.password.is_none() && profile.oauth_token.is_none() {
        profile.password = Some(prompt_password("Password: ")?);
    }

    let results = run_tests(&profile);
    Ok(results.all_passed())
}

/// Resolve a `--provider` argument to a curated preset.  Case-insensitive
/// exact match first, then case-insensitive unique-substring match.
/// Ambiguous matches are an error rather than silently picking the first.
fn resolve_provider(name: &str) -> Result<&'static Provider> {
    let needle = name.to_ascii_lowercase();
    // 1. Exact match (case-insensitive).
    if let Some(p) = providers::PROVIDERS
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
    {
        return Ok(p);
    }
    // 2. Unique substring match.
    let hits: Vec<&'static Provider> = providers::PROVIDERS
        .iter()
        .filter(|p| p.name.to_ascii_lowercase().contains(&needle))
        .collect();
    match hits.len() {
        1 => Ok(hits[0]),
        0 => Err(anyhow::anyhow!(
            "unknown provider {name:?}; run `smtp-test-tool providers` for the list"
        )),
        _ => {
            let names: Vec<&str> = hits.iter().map(|p| p.name).collect();
            Err(anyhow::anyhow!(
                "provider {name:?} is ambiguous; matched: {}",
                names.join(", ")
            ))
        }
    }
}

/// Mirror of GUI's `App::apply_provider`, on a free-standing Profile.
fn apply_provider_to(profile: &mut Profile, p: &Provider) {
    profile.smtp_host = p.smtp.host.into();
    profile.smtp_port = p.smtp.port;
    profile.smtp_security = p.smtp.security;
    profile.imap_host = p.imap.host.into();
    profile.imap_port = p.imap.port;
    profile.imap_security = p.imap.security;
    match p.pop {
        Some(pop) => {
            profile.pop_host = pop.host.into();
            profile.pop_port = pop.port;
            profile.pop_security = pop.security;
        }
        None => {
            profile.pop_enabled = false;
        }
    }
}

fn prompt(msg: &str) -> Result<String> {
    print!("{msg}");
    io::stdout().flush().ok();
    let mut s = String::new();
    io::stdin().read_line(&mut s)?;
    Ok(s.trim().to_string())
}

fn prompt_password(msg: &str) -> Result<String> {
    // Minimal "hidden" prompt - on Windows / Unix we just read a line and
    // hope the terminal is not echoing.  Pulling in `rpassword` would add
    // another dep; for an internal tool the trade-off is fine.
    eprint!("{msg}");
    io::stderr().flush().ok();
    let mut s = String::new();
    io::stdin().read_line(&mut s)?;
    Ok(s.trim_end_matches(['\r', '\n']).to_string())
}

fn supports_colour() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    // tracing_subscriber's ansi detection is conservative on Windows;
    // we trust stderr being a TTY.
    use std::io::IsTerminal;
    io::stderr().is_terminal()
}
