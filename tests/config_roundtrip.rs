//! Integration tests for `smtp_test_tool::config`.
//!
//! These deliberately live in `tests/` (not in `src/`) so they exercise
//! the crate exactly as a downstream user would: save a Config to disk,
//! read it back, compare.

use smtp_test_tool::config::{Config, Profile};
use smtp_test_tool::outlook_defaults;
use smtp_test_tool::smtp::AuthMech;
use smtp_test_tool::tls::Security;

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;

/// Returns a unique-per-test path inside the OS temp dir and a guard
/// that cleans it up when dropped.  We don't pull in tempfile/tempdir
/// just for one test - this is a few lines of std::env::temp_dir.
struct TempFile {
    path: PathBuf,
}

impl TempFile {
    fn new(stem: &str) -> Self {
        // Cargo runs tests in parallel; the thread name is unique per
        // test target + thread + run, which is good enough for our
        // "no clash" need without a uuid dep.
        let unique = format!(
            "smtp_test_tool_{stem}_{pid}_{ts:?}.toml",
            pid = std::process::id(),
            ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let path = env::temp_dir().join(unique);
        let _ = fs::remove_file(&path); // belt + braces
        Self { path }
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Build a sample profile that exercises every kind of field we
/// serialise: Option<String>, Vec<String>, custom enums, bools, ints.
fn fully_populated_profile() -> Profile {
    let mut p = outlook_defaults();
    p.user = Some("ops@example.com".into());
    p.password = Some("hunter2-please-rotate".into());
    p.oauth_token = None;
    p.smtp_enabled = true;
    p.smtp_security = Security::StartTls;
    p.auth_mech = AuthMech::Login;
    p.imap_enabled = true;
    p.imap_folder = "Archive".into();
    p.pop_enabled = false;
    p.send_test = true;
    p.mail_from = Some("ops@example.com".into());
    p.from_addr = Some("noreply@example.com".into()); // tests Send-As
    p.to = vec!["a@example.com".into(), "b@example.com".into()];
    p.cc = vec!["c@example.com".into()];
    p.bcc = vec![];
    p.subject = "Round-trip subject with UTF-8 \u{2192}".into();
    p.body = "First line.\nSecond line.\n".into();
    p.ehlo_name = Some("mailer.example.com".into());
    p.timeout_secs = 42;
    p.theme = "dark".into();
    p
}

#[test]
fn save_then_load_preserves_every_field_save_password_off() {
    let mut cfg = Config {
        active: "default".into(),
        profiles: BTreeMap::new(),
    };
    let mut p = fully_populated_profile();
    // Default behaviour: password should NOT round-trip unless the
    // caller explicitly opted in.  Drop it before saving to mirror
    // what the GUI does when save_pwd is false.
    p.password = None;
    cfg.upsert_profile("default", p.clone());

    let tmp = TempFile::new("default");
    cfg.save(&tmp.path).expect("save");

    let loaded = Config::load(&tmp.path).expect("load");
    let back = loaded.profile("default").expect("profile present").clone();

    assert_eq!(back.user, p.user);
    assert_eq!(back.password, None, "password must not round-trip by default");
    assert_eq!(back.oauth_token, p.oauth_token);
    assert_eq!(back.smtp_host, p.smtp_host);
    assert_eq!(back.smtp_port, p.smtp_port);
    assert_eq!(back.smtp_security, p.smtp_security);
    assert_eq!(back.auth_mech, p.auth_mech);
    assert_eq!(back.imap_folder, p.imap_folder);
    assert_eq!(back.pop_enabled, p.pop_enabled);
    assert_eq!(back.send_test, p.send_test);
    assert_eq!(back.mail_from, p.mail_from);
    assert_eq!(back.from_addr, p.from_addr);
    assert_eq!(back.to, p.to);
    assert_eq!(back.cc, p.cc);
    assert_eq!(back.bcc, p.bcc);
    assert_eq!(back.subject, p.subject);
    assert_eq!(back.body, p.body);
    assert_eq!(back.ehlo_name, p.ehlo_name);
    assert_eq!(back.timeout_secs, p.timeout_secs);
    assert_eq!(back.theme, p.theme);
}

#[test]
fn save_then_load_preserves_password_when_opted_in() {
    // The caller (GUI when save_pwd=true) keeps the password on the
    // Profile, which means the TOML serialiser will emit a
    // password_b64 = "..." entry, and the deserialiser will round-trip
    // it through base64.  Verify that loop.
    let p = fully_populated_profile();
    assert!(p.password.is_some(), "test setup: password set");

    let mut cfg = Config {
        active: "default".into(),
        profiles: BTreeMap::new(),
    };
    cfg.upsert_profile("default", p.clone());

    let tmp = TempFile::new("with_pwd");
    cfg.save(&tmp.path).expect("save");

    // Sanity check the file: must not contain the plain password.
    let on_disk = fs::read_to_string(&tmp.path).expect("read back");
    assert!(
        !on_disk.contains("hunter2-please-rotate"),
        "plain password leaked to disk: {on_disk}"
    );
    assert!(
        on_disk.contains("password_b64"),
        "expected password_b64 key in {on_disk}"
    );

    let loaded = Config::load(&tmp.path).expect("load");
    let back = loaded.profile("default").expect("profile present");
    assert_eq!(back.password.as_deref(), Some("hunter2-please-rotate"));
}

#[test]
fn multiple_profiles_coexist_with_active_selector() {
    let mut cfg = Config {
        active: "production".into(),
        profiles: BTreeMap::new(),
    };
    let mut prod = outlook_defaults();
    prod.user = Some("ops@prod.example.com".into());
    prod.smtp_host = "smtp.prod.example.com".into();
    let mut staging = outlook_defaults();
    staging.user = Some("ops@stage.example.com".into());
    staging.smtp_host = "smtp.stage.example.com".into();
    cfg.upsert_profile("production", prod.clone());
    cfg.upsert_profile("staging", staging.clone());

    let tmp = TempFile::new("multi");
    cfg.save(&tmp.path).expect("save");

    let loaded = Config::load(&tmp.path).expect("load");
    assert_eq!(loaded.active, "production");
    let names = loaded.profile_names();
    assert!(names.contains(&"production".to_string()));
    assert!(names.contains(&"staging".to_string()));
    assert_eq!(
        loaded.profile("production").map(|p| p.user.clone()).unwrap(),
        prod.user
    );
    assert_eq!(
        loaded.profile("staging").map(|p| p.smtp_host.clone()).unwrap(),
        staging.smtp_host
    );
}

#[test]
fn missing_optional_fields_default_to_outlook() {
    // An old / minimal config file with just the required scaffolding
    // must still load, falling back to outlook_defaults() for any
    // field that has a serde default.
    let minimal = r#"
active = "default"

[profiles.default]
smtp_host = "smtp.example.com"
smtp_port = 587
smtp_security = "starttls"
imap_host = "imap.example.com"
imap_port = 993
imap_security = "ssl"
pop_host = "pop.example.com"
pop_port = 995
pop_security = "ssl"
"#;
    let tmp = TempFile::new("minimal");
    fs::write(&tmp.path, minimal).expect("write");
    let loaded = Config::load(&tmp.path).expect("load");
    let p = loaded.profile("default").expect("profile present");
    // Defaults from the Profile struct must have filled in:
    assert!(p.smtp_enabled, "smtp_enabled defaults true");
    assert!(p.imap_enabled, "imap_enabled defaults true");
    assert!(!p.pop_enabled, "pop_enabled defaults false");
    assert_eq!(p.imap_folder, "INBOX");
    assert_eq!(p.timeout_secs, 20);
    assert_eq!(p.theme, "auto");
}
