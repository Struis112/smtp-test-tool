//! Minimal hand-rolled POP3 client - USER/PASS, CAPA, STAT, STLS, QUIT.

use crate::config::Profile;
use crate::diagnostics::pop_hints_for;
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
    fn write_all(&mut self, b: &[u8]) -> std::io::Result<()> {
        match self {
            Stream::Plain(_, w) => w.write_all(b),
            Stream::Tls(r) => r.get_mut().write_all(b),
        }
    }
    fn read_line(&mut self, b: &mut String) -> std::io::Result<usize> {
        match self {
            Stream::Plain(r, _) => r.read_line(b),
            Stream::Tls(r) => r.read_line(b),
        }
    }
}

pub fn run(p: &Profile) -> Result<bool> {
    info!(
        protocol = "pop3",
        "POP3 target {}:{} ({})",
        p.pop_host,
        p.pop_port,
        p.pop_security.as_str()
    );

    let addr = format!("{}:{}", p.pop_host, p.pop_port);
    let tcp = TcpStream::connect(&addr).with_context(|| format!("connecting to {addr}"))?;
    tcp.set_read_timeout(Some(Duration::from_secs(p.timeout_secs)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(p.timeout_secs)))?;
    info!(protocol = "pop3", "TCP connection established");

    let tls_cfg = build_client_config(p.ca_file.as_deref(), p.insecure_tls)?;
    if p.insecure_tls {
        warn!(protocol = "pop3", "TLS certificate verification DISABLED");
    }

    let mut stream = match p.pop_security {
        Security::Implicit => Stream::Tls(BufReader::new(tls_wrap(&tls_cfg, tcp, &p.pop_host)?)),
        _ => {
            let tcp2 = tcp.try_clone()?;
            Stream::Plain(BufReader::new(tcp), tcp2)
        }
    };

    let greet = read_one(&mut stream)?;
    info!(protocol = "pop3", "Greeting: {}", greet.trim_end());
    if !greet.starts_with("+OK") {
        error!(protocol = "pop3", "Unexpected greeting: {greet}");
        return Ok(false);
    }

    // CAPA (optional, some old servers don't support it)
    if let Ok(caps) = multiline(&mut stream, "CAPA") {
        for line in caps.lines() {
            info!(protocol = "pop3", "CAPA: {line}");
        }
    }

    if matches!(p.pop_security, Security::StartTls) {
        single(&mut stream, "STLS")?;
        info!(protocol = "pop3", "STLS negotiated");
        let tcp = match stream {
            Stream::Plain(_, w) => w,
            _ => unreachable!(),
        };
        stream = Stream::Tls(BufReader::new(tls_wrap(&tls_cfg, tcp, &p.pop_host)?));
    }

    let user = p.user.as_ref().ok_or_else(|| anyhow!("POP3 needs a username"))?;
    let pass = p.password.as_ref().ok_or_else(|| anyhow!("POP3 needs a password"))?;

    if let Err(e) = single(&mut stream, &format!("USER {user}")) {
        error!(protocol = "pop3", "USER FAILED: {e}");
        return Ok(false);
    }
    if let Err(e) = single(&mut stream, &format!("PASS {pass}")) {
        error!(protocol = "pop3", "AUTH FAILED: {e}");
        for h in pop_hints_for(&e.to_string()) {
            error!(protocol = "pop3", "{h}");
        }
        let _ = single(&mut stream, "QUIT");
        return Ok(false);
    }
    info!(protocol = "pop3", "AUTH succeeded as {user}");

    if let Ok(stat) = single(&mut stream, "STAT") {
        info!(protocol = "pop3", "STAT: {}", stat.trim_end());
    }

    let _ = single(&mut stream, "QUIT");
    info!(protocol = "pop3", "Session closed cleanly");
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
    let mut s = StreamOwned::new(conn, tcp);
    s.flush().ok();
    if let Some(suite) = s.conn.negotiated_cipher_suite() {
        info!(protocol = "pop3", "TLS established: {:?}", suite.suite());
    }
    Ok(s)
}

fn read_one(s: &mut Stream) -> Result<String> {
    let mut line = String::new();
    let n = s.read_line(&mut line).context("reading POP3 response")?;
    if n == 0 {
        bail!("connection closed");
    }
    debug!(protocol = "pop3", "S: {}", line.trim_end());
    Ok(line)
}

fn single(s: &mut Stream, cmd: &str) -> Result<String> {
    debug!(protocol = "pop3", "C: {cmd}");
    s.write_all(format!("{cmd}\r\n").as_bytes())?;
    let line = read_one(s)?;
    if line.starts_with("+OK") {
        Ok(line)
    } else {
        bail!(line.trim_end().to_string())
    }
}

fn multiline(s: &mut Stream, cmd: &str) -> Result<String> {
    debug!(protocol = "pop3", "C: {cmd}");
    s.write_all(format!("{cmd}\r\n").as_bytes())?;
    let first = read_one(s)?;
    if !first.starts_with("+OK") {
        bail!(first.trim_end().to_string());
    }
    let mut acc = String::new();
    loop {
        let line = read_one(s)?;
        if line.trim_end() == "." {
            break;
        }
        acc.push_str(&line);
    }
    Ok(acc)
}
