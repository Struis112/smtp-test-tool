//! TOML config file with named profiles.  Auto-load order:
//!   1. `--config <FILE>` if explicit.
//!   2. `email_tester.toml` in the executable's directory.
//!   3. `email_tester.toml` in the current working directory.
//!   4. OS-standard config dir (e.g. `%APPDATA%/email-tester/email_tester.toml`).

use crate::smtp::AuthMech;
use crate::tls::Security;
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::{env, fs};

pub const DEFAULT_FILE_NAME: &str = "email_tester.toml";

/// Full config = many named profiles.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    /// Profile selected when none is specified on the command line.
    #[serde(default = "default_active")]
    pub active: String,
    /// All named profiles.  TOML representation: `[profiles.default]`, `[profiles.on-prem]` etc.
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

fn default_active() -> String { "default".into() }

/// All testable settings.  This is what gets serialised to TOML and what
/// the GUI/CLI render and edit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    // ---- credentials ----
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Stored base64 (NOT encryption!) - users opt in via --save-password.
    #[serde(default, rename = "password_b64", skip_serializing_if = "Option::is_none",
            serialize_with = "ser_b64", deserialize_with = "de_b64")]
    pub password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_token: Option<String>,

    // ---- SMTP ----
    #[serde(default = "yes")]
    pub smtp_enabled: bool,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: Security,
    #[serde(default)]
    pub auth_mech: AuthMech,

    // ---- IMAP ----
    #[serde(default = "yes")]
    pub imap_enabled: bool,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: Security,
    #[serde(default = "inbox")]
    pub imap_folder: String,

    // ---- POP3 ----
    #[serde(default)]
    pub pop_enabled: bool,
    pub pop_host: String,
    pub pop_port: u16,
    pub pop_security: Security,

    // ---- message (only when send_test) ----
    #[serde(default)]
    pub send_test: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mail_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_addr: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub to: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cc: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bcc: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    #[serde(default = "default_subject")]
    pub subject: String,
    #[serde(default = "default_body")]
    pub body: String,

    // ---- advanced ----
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ehlo_name: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub insecure_tls: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_file: Option<PathBuf>,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default)]
    pub wire_trace: bool,
    #[serde(default = "default_theme")]
    pub theme: String,
}

fn yes() -> bool { true }
fn inbox() -> String { "INBOX".into() }
fn default_subject() -> String { "Email server connectivity test".into() }
fn default_body() -> String { "This is a connectivity test sent by email-tester.\n".into() }
fn default_timeout() -> u64 { 20 }
fn default_log_level() -> String { "info".into() }
fn default_theme() -> String { "auto".into() }

impl Default for Profile {
    fn default() -> Self { crate::outlook_defaults() }
}

// ---- base64 (de)serialiser for the optional password -----------------
fn ser_b64<S: serde::Serializer>(v: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(p) => s.serialize_str(&B64.encode(p.as_bytes())),
        None => s.serialize_none(),
    }
}
fn de_b64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let opt = Option::<String>::deserialize(d)?;
    match opt {
        None => Ok(None),
        Some(s) => {
            let bytes = B64.decode(s.as_bytes()).map_err(serde::de::Error::custom)?;
            Ok(Some(String::from_utf8(bytes).map_err(serde::de::Error::custom)?))
        }
    }
}

// ===========================================================================
// File handling
// ===========================================================================
impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading config file {}", path.display()))?;
        let cfg: Config = toml::from_str(&text)
            .with_context(|| format!("parsing TOML {}", path.display()))?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        let mut text = String::from(
            "# email-tester configuration\n\
             # Multiple [profiles.<name>] sections can coexist; pick one with --profile.\n\
             # The file 'email_tester.toml' next to the executable is auto-loaded.\n\n");
        text.push_str(&toml::to_string_pretty(self)
            .context("serialising config to TOML")?);
        fs::write(path, text)
            .with_context(|| format!("writing config file {}", path.display()))?;
        Ok(())
    }

    /// Replace (or insert) one profile and persist.
    pub fn upsert_profile(&mut self, name: &str, p: Profile) {
        self.profiles.insert(name.to_string(), p);
    }

    pub fn profile_names(&self) -> Vec<String> {
        self.profiles.keys().cloned().collect()
    }

    pub fn profile(&self, name: &str) -> Option<&Profile> {
        self.profiles.get(name)
    }
}

/// Locate the most relevant config file on this machine.
pub fn discover_config_path() -> Option<PathBuf> {
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            let p = dir.join(DEFAULT_FILE_NAME);
            if p.exists() { return Some(p); }
        }
    }
    if let Ok(cwd) = env::current_dir() {
        let p = cwd.join(DEFAULT_FILE_NAME);
        if p.exists() { return Some(p); }
    }
    if let Some(dir) = dirs::config_dir() {
        let p = dir.join("email-tester").join(DEFAULT_FILE_NAME);
        if p.exists() { return Some(p); }
    }
    None
}

/// Where to save a brand-new config when the user clicks 'Save'.
pub fn default_save_path() -> PathBuf {
    if let Ok(exe) = env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.join(DEFAULT_FILE_NAME);
        }
    }
    if let Some(dir) = dirs::config_dir() {
        return dir.join("email-tester").join(DEFAULT_FILE_NAME);
    }
    PathBuf::from(DEFAULT_FILE_NAME)
}
