//! End-to-end protocol tests for `smtp::run` against an in-process
//! mock SMTP server.  Each test exercises a specific failure mode and
//! verifies that the diagnostic translator (`diagnostics::smtp_hints_for`)
//! fired with the expected hint text on the captured log stream.
//!
//! Mock servers speak plain SMTP (no TLS) - tests pin
//! `smtp_security = Security::None` and the mock advertises EHLO without
//! STARTTLS so lettre never attempts an upgrade.

mod common;

use common::{read_line, spawn_mock_server, writeln_crlf, LogCapture};
use smtp_test_tool::outlook_defaults;
use smtp_test_tool::smtp::{self, AuthMech};
use smtp_test_tool::tls::Security;

/// Build a Profile that talks plain SMTP to the given address.
fn profile_for(addr: std::net::SocketAddr) -> smtp_test_tool::Profile {
    let mut p = outlook_defaults();
    p.smtp_host = addr.ip().to_string();
    p.smtp_port = addr.port();
    p.smtp_security = Security::None;
    // Disable IMAP and POP so smtp::run is the only thing exercised.
    p.imap_enabled = false;
    p.pop_enabled = false;
    p.user = Some("ops@example.invalid".into());
    p.password = Some("hunter2".into());
    // Keep send_test off; AUTH failure should surface from test_connection.
    p.send_test = false;
    // Force LOGIN so the mock's 334-challenge dance matches; with Auto,
    // lettre would try PLAIN first and complain that the mock's 334
    // challenge does not fit a one-shot PLAIN exchange.
    p.auth_mech = AuthMech::Login;
    p.timeout_secs = 3;
    p
}

#[test]
fn smtp_auth_failure_5_7_139_triggers_basic_auth_hint() {
    // Mock script: minimal SMTP server that rejects AUTH with the
    // verbatim Microsoft 365 'basic auth disabled' reply.  We do NOT
    // advertise STARTTLS so lettre stays plain.
    let server = spawn_mock_server(|mut r, mut w| {
        // 220 greeting
        writeln_crlf(&mut w, "220 mock.example.invalid ESMTP ready");
        // Read EHLO, respond with capabilities (no STARTTLS, AUTH LOGIN+PLAIN)
        let _ehlo = read_line(&mut r);
        writeln_crlf(&mut w, "250-mock.example.invalid Hello [127.0.0.1]");
        writeln_crlf(&mut w, "250-SIZE 157286400");
        writeln_crlf(&mut w, "250-AUTH LOGIN PLAIN");
        writeln_crlf(&mut w, "250 OK");
        // Read AUTH command + base64 user + base64 password (lettre uses LOGIN by
        // default in mech=auto).  We don't care about the exact strings; just
        // drain them.
        let _auth = read_line(&mut r);
        writeln_crlf(&mut w, "334 VXNlcm5hbWU6");
        let _user = read_line(&mut r);
        writeln_crlf(&mut w, "334 UGFzc3dvcmQ6");
        let _pass = read_line(&mut r);
        // The smoking-gun reply.
        writeln_crlf(
            &mut w,
            "535 5.7.139 Authentication unsuccessful, basic authentication is disabled",
        );
        // Drain QUIT if the client sends one.
        let _quit = read_line(&mut r);
        writeln_crlf(&mut w, "221 Bye");
    });

    let logs = LogCapture::install();
    let profile = profile_for(server.addr);

    let outcome = smtp::run(&profile);
    drop(server);

    // We expect Ok(false): the server is reachable, the protocol
    // completed, but AUTH was rejected.
    assert!(
        matches!(outcome, Ok(false)),
        "expected Ok(false) on auth rejection, got {outcome:?}"
    );

    // Diagnostic translator must have fired.  Look for both the raw
    // ESC and the actionable hint text in the captured log.
    assert!(
        logs.contains("5.7.139"),
        "expected the captured log to mention ESC 5.7.139; got lines:\n  {}",
        logs.lines().join("\n  ")
    );
    assert!(
        logs.contains("Conditional Access"),
        "expected the 5.7.139 'Conditional Access' hint in the log; got lines:\n  {}",
        logs.lines().join("\n  ")
    );
}

#[test]
fn smtp_send_as_denied_5_7_60_surfaces_hint() {
    // Same flow, but the failure happens on the AUTH step with
    // 'SendAsDenied' wording.  Microsoft 365 actually returns 5.7.60
    // during the RCPT TO / message submission stage; we approximate by
    // surfacing it during auth so the test stays a one-message dance.
    let server = spawn_mock_server(|mut r, mut w| {
        writeln_crlf(&mut w, "220 mock.example.invalid ESMTP ready");
        let _ehlo = read_line(&mut r);
        writeln_crlf(&mut w, "250-mock.example.invalid Hello [127.0.0.1]");
        writeln_crlf(&mut w, "250-AUTH LOGIN PLAIN");
        writeln_crlf(&mut w, "250 OK");
        let _auth = read_line(&mut r);
        writeln_crlf(&mut w, "334 VXNlcm5hbWU6");
        let _user = read_line(&mut r);
        writeln_crlf(&mut w, "334 UGFzc3dvcmQ6");
        let _pass = read_line(&mut r);
        writeln_crlf(
            &mut w,
            "550 5.7.60 SMTP; Client does not have permissions to send as this sender",
        );
        let _quit = read_line(&mut r);
        writeln_crlf(&mut w, "221 Bye");
    });

    let logs = LogCapture::install();
    let profile = profile_for(server.addr);

    let outcome = smtp::run(&profile);
    drop(server);

    assert!(matches!(outcome, Ok(false)));
    assert!(
        logs.contains("5.7.60"),
        "expected ESC 5.7.60 in log; got:\n  {}",
        logs.lines().join("\n  ")
    );
    assert!(
        logs.contains("Send As"),
        "expected the SendAs hint to mention 'Send As'; got:\n  {}",
        logs.lines().join("\n  ")
    );
}

#[test]
fn smtp_dns_failure_is_logged_not_panicked() {
    // No mock server; point at a port that nothing is listening on so
    // the connect itself fails.  Goal: smtp::run must return cleanly
    // (Ok(false) per the 'reachable + rejected' contract, or Err for a
    // hard error - either is fine, neither is a panic).
    let logs = LogCapture::install();
    let mut profile = outlook_defaults();
    profile.smtp_host = "127.0.0.1".into();
    profile.smtp_port = 1; // privileged + unused
    profile.smtp_security = Security::None;
    profile.user = Some("x@example.invalid".into());
    profile.password = Some("y".into());
    profile.send_test = false;
    profile.timeout_secs = 2;
    profile.imap_enabled = false;
    profile.pop_enabled = false;

    let outcome = smtp::run(&profile);
    // We accept either Ok(false) or Err - both prove no panic.  What
    // we ASSERT is that something was logged so the user gets a clue.
    assert!(outcome.is_ok() || outcome.is_err(), "must not panic");
    assert!(
        !logs.lines().is_empty(),
        "expected at least one log line about the connection attempt"
    );
}
