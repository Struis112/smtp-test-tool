//! email-tester core library.
//!
//! Pure-Rust, single-binary mail-server diagnostic.  Used by both the
//! CLI (`bin/cli.rs`) and the GUI (`bin/gui.rs`).
//!
//! All protocol tests emit progress through [`tracing`] events with a
//! `protocol` field (`"smtp"`, `"imap"`, or `"pop3"`) so any subscriber
//! (terminal formatter, GUI log widget, file writer) can route them.

pub mod config;
pub mod diagnostics;
pub mod imap;
pub mod pop3;
pub mod runner;
pub mod smtp;
pub mod theme;
pub mod tls;

pub use config::{Config, Profile};
pub use runner::{run_tests, TestOutcome, TestResults};

/// Built-in Outlook.com / Office 365 defaults.
pub fn outlook_defaults() -> Profile {
    Profile {
        user: None,
        password: None,
        oauth_token: None,

        smtp_enabled: true,
        smtp_host: "smtp-mail.outlook.com".into(),
        smtp_port: 587,
        smtp_security: tls::Security::StartTls,
        auth_mech: smtp::AuthMech::Auto,

        imap_enabled: true,
        imap_host: "outlook.office365.com".into(),
        imap_port: 993,
        imap_security: tls::Security::Implicit,
        imap_folder: "INBOX".into(),

        pop_enabled: false,
        pop_host: "outlook.office365.com".into(),
        pop_port: 995,
        pop_security: tls::Security::Implicit,

        send_test: false,
        mail_from: None,
        from_addr: None,
        to: Vec::new(),
        cc: Vec::new(),
        bcc: Vec::new(),
        reply_to: None,
        subject: "Email server connectivity test".into(),
        body: "This is a connectivity test sent by email-tester.\n".into(),

        ehlo_name: None,
        timeout_secs: 20,
        insecure_tls: false,
        ca_file: None,

        log_level: "info".into(),
        wire_trace: false,
        theme: "auto".into(),
    }
}
