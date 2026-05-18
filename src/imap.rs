//! Minimal hand-rolled IMAP client - just enough for connectivity
//! diagnostics (CAPABILITY, STARTTLS, LOGIN, LIST, SELECT, LOGOUT).
//!
//! Hand-rolling avoids dragging in a heavy IMAP crate, and lets us surface
//! every server response verbatim into the diagnostic log.

use crate::config::Profile;
use crate::diagnostics::imap_hints_for;
use crate::tls::{build_client_config, Security};
use anyhow::{anyhow, bail, Context, Result};
use rustls::pki_types::ServerName;
use rustls::{ClientConnection, StreamOwned};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

enum Stream {
    Plain(BufReader<TcpStream>, TcpStream),
    Tls(BufReader<StreamOwned<ClientConnection, TcpStream>>),
}

impl Stream {
    fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
        match self {
            Stream::Plain(_, w) => w.write_all(data),
            Stream::Tls(r) => r.get_mut().write_all(data),
        }
    }
    fn read_line(&mut self, buf: &mut String) -> std::io::Result<usize> {
        match self {
            Stream::Plain(r, _) => r.read_line(buf),
            Stream::Tls(r) => r.read_line(buf),
        }
    }
}

pub fn run(p: &Profile) -> Result<bool> {
    info!(
        protocol = "imap",
        "IMAP target {}:{} ({})",
        p.imap_host,
        p.imap_port,
        p.imap_security.as_str()
    );

    // ----- TCP --------------------------------------------------------
    let addr = format!("{}:{}", p.imap_host, p.imap_port);
    let tcp = TcpStream::connect(&addr).with_context(|| format!("connecting to {addr}"))?;
    tcp.set_read_timeout(Some(Duration::from_secs(p.timeout_secs)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(p.timeout_secs)))?;
    info!(protocol = "imap", "TCP connection established");

    // ----- TLS (implicit) or stay plain (STARTTLS upgraded later) ----
    let tls_cfg = build_client_config(p.ca_file.as_deref(), p.insecure_tls)?;
    if p.insecure_tls {
        warn!(protocol = "imap", "TLS certificate verification DISABLED");
    }
    let mut stream = match p.imap_security {
        Security::Implicit => Stream::Tls(BufReader::new(tls_wrap(&tls_cfg, tcp, &p.imap_host)?)),
        _ => {
            let tcp2 = tcp.try_clone()?;
            Stream::Plain(BufReader::new(tcp), tcp2)
        }
    };

    // ----- read server greeting --------------------------------------
    let greet = read_response(&mut stream, None)?;
    info!(protocol = "imap", "Greeting: {}", greet.trim_end());

    // ----- CAPABILITY ------------------------------------------------
    let caps = imap_cmd(&mut stream, "a1", "CAPABILITY")?;
    info!(protocol = "imap", "CAPABILITY: {}", caps.trim_end());
    if p.imap_security == Security::None && caps.to_uppercase().contains("LOGINDISABLED") {
        warn!(protocol = "imap", "Server advertises LOGINDISABLED on cleartext - require STARTTLS/SSL");
    }

    // ----- STARTTLS upgrade ------------------------------------------
    if matches!(p.imap_security, Security::StartTls) {
        imap_cmd(&mut stream, "a2", "STARTTLS")?;
        info!(protocol = "imap", "STARTTLS negotiated");
        let tcp = match stream {
            Stream::Plain(_, w) => w,
            _ => unreachable!(),
        };
        stream = Stream::Tls(BufReader::new(tls_wrap(&tls_cfg, tcp, &p.imap_host)?));
        // Re-issue CAPABILITY post-TLS.
        let caps2 = imap_cmd(&mut stream, "a3", "CAPABILITY")?;
        info!(protocol = "imap", "CAPABILITY (post-TLS): {}", caps2.trim_end());
    }

    // ----- LOGIN -----------------------------------------------------
    let user = p.user.as_ref().ok_or_else(|| anyhow!("IMAP needs a username"))?;
    let pass = p
        .password
        .as_ref()
        .ok_or_else(|| anyhow!("IMAP needs a password (OAuth2 not yet supported by this client)"))?;
    let cmd = format!("LOGIN {} {}", quote(user), quote(pass));
    match imap_cmd(&mut stream, "b1", &cmd) {
        Ok(_) => info!(protocol = "imap", "LOGIN succeeded as {user}"),
        Err(e) => {
            error!(protocol = "imap", "LOGIN FAILED: {e}");
            for h in imap_hints_for(&e.to_string()) {
                error!(protocol = "imap", "{h}");
            }
            let _ = imap_cmd(&mut stream, "z1", "LOGOUT");
            return Ok(false);
        }
    }

    // ----- LIST + SELECT --------------------------------------------
    match imap_cmd(&mut stream, "b2", "LIST \"\" \"*\"") {
        Ok(list) => {
            let n = list.lines().filter(|l| l.starts_with("* LIST")).count();
            info!(protocol = "imap", "LIST returned {n} mailboxes");
        }
        Err(e) => warn!(protocol = "imap", "LIST failed: {e}"),
    }
    let folder = if p.imap_folder.is_empty() { "INBOX" } else { p.imap_folder.as_str() };
    match imap_cmd(&mut stream, "b3", &format!("EXAMINE {}", quote(folder))) {
        Ok(sel) => {
            let count = sel
                .lines()
                .find_map(|l| l.strip_prefix("* ").and_then(|s| s.split_whitespace().nth(0)).filter(|t| t.chars().all(|c| c.is_ascii_digit())))
                .unwrap_or("?");
            info!(protocol = "imap", "EXAMINE {folder} (read-only, {count} messages)");
        }
        Err(e) => error!(protocol = "imap", "EXAMINE failed: {e}"),
    }

    let _ = imap_cmd(&mut stream, "z9", "LOGOUT");
    info!(protocol = "imap", "Session closed cleanly");
    Ok(true)
}

fn tls_wrap(
    cfg: &Arc<rustls::ClientConfig>,
    tcp: TcpStream,
    host: &str,
) -> Result<StreamOwned<ClientConnection, TcpStream>> {
    let server = ServerName::try_from(host.to_string())
        .map_err(|_| anyhow!("invalid TLS server name {host}"))?;
    let conn = ClientConnection::new(cfg.clone(), server).context("rustls handshake init")?;
    let mut stream = StreamOwned::new(conn, tcp);
    // Force the handshake so we can log version/cipher early.
    stream.flush().ok();
    if let Some(suite) = stream.conn.negotiated_cipher_suite() {
        info!(protocol = "imap", "TLS established: {:?}", suite.suite());
    }
    Ok(stream)
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Issue a tagged IMAP command, read response lines until "<tag> OK/NO/BAD".
fn imap_cmd(s: &mut Stream, tag: &str, cmd: &str) -> Result<String> {
    debug!(protocol = "imap", "C: {tag} {cmd}");
    s.write_all(format!("{tag} {cmd}\r\n").as_bytes())
        .context("writing IMAP command")?;
    read_response(s, Some(tag))
}

fn read_response(s: &mut Stream, tag: Option<&str>) -> Result<String> {
    let mut acc = String::new();
    loop {
        let mut line = String::new();
        let n = s.read_line(&mut line).context("reading IMAP response")?;
        if n == 0 {
            bail!("connection closed by server");
        }
        debug!(protocol = "imap", "S: {}", line.trim_end());
        acc.push_str(&line);
        match tag {
            None => return Ok(acc), // greeting: first line is enough
            Some(t) => {
                if line.starts_with(t) {
                    let rest = &line[t.len()..].trim_start();
                    if rest.starts_with("OK") {
                        return Ok(acc);
                    }
                    if rest.starts_with("NO") || rest.starts_with("BAD") {
                        bail!("{}", line.trim_end().to_string());
                    }
                }
            }
        }
    }
}
