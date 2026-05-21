//! End-to-end protocol tests for `pop3::run` against an in-process
//! mock POP3 server.

mod common;

use common::{read_line, spawn_mock_server, writeln_crlf, LogCapture};
use smtp_test_tool::outlook_defaults;
use smtp_test_tool::pop3;
use smtp_test_tool::tls::Security;

fn profile_for(addr: std::net::SocketAddr) -> smtp_test_tool::Profile {
    let mut p = outlook_defaults();
    p.pop_host = addr.ip().to_string();
    p.pop_port = addr.port();
    p.pop_security = Security::None;
    p.smtp_enabled = false;
    p.imap_enabled = false;
    p.pop_enabled = true;
    p.user = Some("ops@example.invalid".into());
    p.password = Some("hunter2".into());
    p.timeout_secs = 3;
    p
}

#[test]
fn pop3_auth_failure_triggers_hint() {
    let server = spawn_mock_server(|mut r, mut w| {
        writeln_crlf(&mut w, "+OK mock POP3 server ready");
        // Our client probes CAPA first.
        let cmd = read_line(&mut r);
        assert!(cmd.starts_with("CAPA"), "expected CAPA, got: {cmd}");
        writeln_crlf(&mut w, "+OK");
        writeln_crlf(&mut w, "TOP");
        writeln_crlf(&mut w, "USER");
        writeln_crlf(&mut w, "UIDL");
        writeln_crlf(&mut w, ".");
        // USER <name>
        let cmd = read_line(&mut r);
        assert!(cmd.starts_with("USER"), "expected USER, got: {cmd}");
        writeln_crlf(&mut w, "+OK");
        // PASS <password> -- reject with the wording our diagnostic
        // table recognises.
        let cmd = read_line(&mut r);
        assert!(cmd.starts_with("PASS"), "expected PASS, got: {cmd}");
        writeln_crlf(&mut w, "-ERR authentication failed");
        // Client follows with QUIT.
        let _ = read_line(&mut r);
        writeln_crlf(&mut w, "+OK bye");
    });

    let logs = LogCapture::install();
    let profile = profile_for(server.addr);

    let outcome = pop3::run(&profile);
    drop(server);

    assert!(
        matches!(outcome, Ok(false)),
        "expected Ok(false) on PASS rejection, got {outcome:?}"
    );
    assert!(
        logs.contains("authentication failed"),
        "expected the verbatim '-ERR authentication failed' to be logged; got:\n  {}",
        logs.lines().join("\n  ")
    );
    // The hint produced by diagnostics::pop_hints_for for this needle:
    assert!(
        logs.contains("POP disabled") || logs.contains("bad credentials"),
        "expected the POP-auth hint in log; got:\n  {}",
        logs.lines().join("\n  ")
    );
}

#[test]
fn pop3_disabled_phrase_triggers_dedicated_hint() {
    // Some tenants reply with the literal phrase 'POP3 is disabled' or
    // similar.  Our hint table catches the 'disabled' substring.
    let server = spawn_mock_server(|mut r, mut w| {
        writeln_crlf(&mut w, "+OK mock POP3 server ready");
        let _ = read_line(&mut r);
        writeln_crlf(&mut w, "+OK");
        writeln_crlf(&mut w, ".");
        let _ = read_line(&mut r);
        writeln_crlf(&mut w, "+OK");
        let _ = read_line(&mut r);
        writeln_crlf(&mut w, "-ERR POP3 is disabled for this account");
        let _ = read_line(&mut r);
        writeln_crlf(&mut w, "+OK bye");
    });

    let logs = LogCapture::install();
    let profile = profile_for(server.addr);

    let outcome = pop3::run(&profile);
    drop(server);

    assert!(matches!(outcome, Ok(false)));
    assert!(
        logs.contains("disabled"),
        "expected 'disabled' to appear in log; got:\n  {}",
        logs.lines().join("\n  ")
    );
}
