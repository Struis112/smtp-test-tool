//! Command-line entry point.

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use smtp_test_tool::config::{default_save_path, discover_config_path, Config};
use smtp_test_tool::{outlook_defaults, run_tests, Profile};
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "smtp-test-tool", version, about,
          long_about = "Test SMTP / IMAP / POP3 connectivity to any mail server.\n\
                        Defaults to Outlook.com / Office 365.")]
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
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&lvl));
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
        Some(p) => Config::load(p)
            .with_context(|| format!("loading {}", p.display()))?,
        None => Config {
            active: "default".into(),
            profiles: [("default".into(), outlook_defaults())].into_iter().collect(),
        },
    };

    let profile_name = cli
        .profile
        .clone()
        .unwrap_or_else(|| cfg.active.clone());

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
        Cmd::Init { output } => {
            let mut new_cfg = Config { active: "default".into(), profiles: Default::default() };
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
    if let Some(u) = cli.user { profile.user = Some(u); }
    if let Some(p) = cli.password { profile.password = Some(p); }
    if let Some(t) = cli.oauth_token { profile.oauth_token = Some(t); }
    if cli.insecure { profile.insecure_tls = true; }

    if profile.user.is_none() {
        profile.user = Some(prompt("Username / email: ")?);
    }
    if profile.password.is_none() && profile.oauth_token.is_none() {
        profile.password = Some(prompt_password("Password: ")?);
    }

    let results = run_tests(&profile);
    Ok(results.all_passed())
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
