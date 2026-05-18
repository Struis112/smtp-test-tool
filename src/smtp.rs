//! SMTP connectivity test using lettre.
//!
//! Emits `tracing` events; both CLI and GUI subscribe.  Translates any
//! server error into actionable hints via `diagnostics::smtp_hints_for`.

use crate::config::Profile;
use crate::diagnostics::smtp_hints_for;
use crate::tls::Security;
use anyhow::{anyhow, Context, Result};
use lettre::message::{header::ContentType, Mailbox, Message};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::client::{Tls, TlsParametersBuilder};
use lettre::transport::smtp::SmtpTransport;
use lettre::Transport;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::time::Duration;
use tracing::{error, info, warn};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMech {
    #[default]
    Auto,
    Login,
    Plain,
    #[serde(rename = "cram-md5")]
    CramMd5,
    #[serde(rename = "xoauth2")]
    XOauth2,
}

impl AuthMech {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthMech::Auto => "auto",
            AuthMech::Login => "login",
            AuthMech::Plain => "plain",
            AuthMech::CramMd5 => "cram-md5",
            AuthMech::XOauth2 => "xoauth2",
        }
    }
}

/// Run the SMTP test.  Returns `Ok(true)` on success; `Ok(false)` on a
/// reachable-server-but-rejected outcome; `Err` on lower-level failure
/// (DNS, connection refused, ...).
#[tracing::instrument(level = "info", skip(p), fields(protocol = "smtp"))]
pub fn run(p: &Profile) -> Result<bool> {
    info!(
        "SMTP target {}:{} ({})",
        p.smtp_host,
        p.smtp_port,
        p.smtp_security.as_str()
    );

    // ----- TLS parameters (lettre takes a rustls-backed builder) ------
    let tls_params = TlsParametersBuilder::new(p.smtp_host.clone())
        .dangerous_accept_invalid_certs(p.insecure_tls)
        .dangerous_accept_invalid_hostnames(p.insecure_tls)
        .build()
        .context("building TLS parameters")?;
    let tls = match p.smtp_security {
        Security::None => Tls::None,
        Security::StartTls => Tls::Required(tls_params),
        Security::Implicit => Tls::Wrapper(tls_params),
    };
    if p.insecure_tls {
        warn!("TLS certificate verification DISABLED (insecure_tls=true)");
    }

    // ----- transport --------------------------------------------------
    let mut builder = SmtpTransport::builder_dangerous(&p.smtp_host)
        .port(p.smtp_port)
        .tls(tls)
        .timeout(Some(Duration::from_secs(p.timeout_secs)));

    if let Some(ehlo) = &p.ehlo_name {
        builder = builder.hello_name(lettre::transport::smtp::extension::ClientId::Domain(ehlo.clone()));
    }

    // ----- credentials ------------------------------------------------
    if let (Some(user), Some(pass)) = (p.user.as_ref(), p.password.as_ref()) {
        builder = builder.credentials(Credentials::new(user.clone(), pass.clone()));
        let mech = match p.auth_mech {
            AuthMech::Auto => vec![Mechanism::Plain, Mechanism::Login],
            AuthMech::Login => vec![Mechanism::Login],
            AuthMech::Plain => vec![Mechanism::Plain],
            AuthMech::CramMd5 => vec![Mechanism::Xoauth2 /* placeholder, see below */],
            AuthMech::XOauth2 => vec![Mechanism::Xoauth2],
        };
        builder = builder.authentication(mech);
        info!("Configured SMTP AUTH as {user} (mech={})", p.auth_mech.as_str());
    } else if let Some(token) = p.oauth_token.as_ref() {
        // XOAUTH2: lettre accepts the bearer token in the password field.
        let user = p.user.clone().unwrap_or_default();
        builder = builder.credentials(Credentials::new(user.clone(), token.clone()));
        builder = builder.authentication(vec![Mechanism::Xoauth2]);
        info!("Configured SMTP XOAUTH2 as {user}");
    } else {
        info!("No credentials supplied - testing connectivity only (no AUTH)");
    }

    let transport = builder.build();

    // ----- optional test message --------------------------------------
    if p.send_test {
        match build_message(p) {
            Ok(msg) => match transport.send(&msg) {
                Ok(resp) => {
                    info!("Message accepted (code {})", resp.code());
                    return Ok(true);
                }
                Err(e) => {
                    error!("MESSAGE SUBMISSION FAILED: {e}");
                    for hint in smtp_hints_for(&e.to_string()) {
                        error!("{hint}");
                    }
                    return Ok(false);
                }
            },
            Err(e) => {
                error!("Could not build test message: {e}");
                return Ok(false);
            }
        }
    }

    // ----- otherwise just verify the AUTH/handshake -------------------
    match transport.test_connection() {
        Ok(true) => {
            info!("SMTP handshake + AUTH succeeded");
            Ok(true)
        }
        Ok(false) => {
            error!("SMTP server did not accept the connection probe");
            Ok(false)
        }
        Err(e) => {
            error!("SMTP test failed: {e}");
            for hint in smtp_hints_for(&e.to_string()) {
                error!("{hint}");
            }
            Ok(false)
        }
    }
}

fn build_message(p: &Profile) -> Result<Message> {
    let header_from = p.from_addr.clone()
        .or_else(|| p.mail_from.clone())
        .or_else(|| p.user.clone())
        .ok_or_else(|| anyhow!("no From: address (set 'from_addr', 'mail_from', or 'user')"))?;
    let envelope_from = p.mail_from.clone().or_else(|| p.user.clone()).unwrap_or(header_from.clone());

    let to_addrs: Vec<String> = if p.to.is_empty() {
        // default: send to ourselves so the test is harmless.
        vec![envelope_from.clone()]
    } else {
        p.to.clone()
    };

    if header_from != envelope_from {
        info!(
            "Header From <{}> differs from envelope MAIL FROM <{}> - this exercises 'Send As' rights",
            header_from, envelope_from
        );
    }

    let mut msg = Message::builder()
        .from(Mailbox::from_str(&header_from).context("invalid From: address")?)
        .subject(&p.subject);

    for t in &to_addrs {
        msg = msg.to(Mailbox::from_str(t).with_context(|| format!("invalid To: {t}"))?);
    }
    for c in &p.cc {
        msg = msg.cc(Mailbox::from_str(c).with_context(|| format!("invalid Cc: {c}"))?);
    }
    for b in &p.bcc {
        msg = msg.bcc(Mailbox::from_str(b).with_context(|| format!("invalid Bcc: {b}"))?);
    }
    if let Some(r) = &p.reply_to {
        msg = msg.reply_to(Mailbox::from_str(r).context("invalid Reply-To:")?);
    }

    let msg = msg
        .header(ContentType::TEXT_PLAIN)
        .body(p.body.clone())
        .context("building MIME body")?;
    Ok(msg)
}
