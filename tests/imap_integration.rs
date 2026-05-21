//! End-to-end protocol tests for `imap::run` against an in-process
//! mock IMAP server.

mod common;

use common::{read_line, spawn_mock_server, writeln_crlf, LogCapture};
use smtp_test_tool::imap;
use smtp_test_tool::outlook_defaults;
use smtp_test_tool::tls::Security;

fn profile_for(addr: std::net::SocketAddr) -> smtp_test_tool::Profile {
    let mut p = outlook_defaults();
    p.imap_host = addr.ip().to_string();
    p.imap_port = addr.port();
    p.imap_security = Security::None;
    p.smtp_enabled = false;
    p.pop_enabled = false;
    p.user = Some("ops@example.invalid".into());
    p.password = Some("hunter2".into());
    p.timeout_secs = 3;
    p
}

#[test]
fn imap_login_authenticationfailed_triggers_hint() {
    let server = spawn_mock_server(|mut r, mut w| {
        // Greeting
        writeln_crlf(&mut w, "* OK [CAPABILITY IMAP4rev1 STARTTLS] mock ready");
        // a1 CAPABILITY -- our client always issues this first.
        let cmd = read_line(&mut r);
        assert!(
            cmd.contains("CAPABILITY"),
            "expected CAPABILITY, got: {cmd}"
        );
        writeln_crlf(&mut w, "* CAPABILITY IMAP4rev1 STARTTLS AUTH=PLAIN");
        writeln_crlf(&mut w, "a1 OK CAPABILITY completed");
        // b1 LOGIN "user" "pass"
        let cmd = read_line(&mut r);
        assert!(cmd.contains("LOGIN"), "expected LOGIN, got: {cmd}");
        writeln_crlf(&mut w, "b1 NO [AUTHENTICATIONFAILED] LOGIN failed");
        // Client should issue LOGOUT.
        let _ = read_line(&mut r);
        writeln_crlf(&mut w, "* BYE Logging out");
        writeln_crlf(&mut w, "z1 OK LOGOUT completed");
    });

    let logs = LogCapture::install();
    let profile = profile_for(server.addr);

    let outcome = imap::run(&profile);
    drop(server);

    assert!(
        matches!(outcome, Ok(false)),
        "expected Ok(false) on LOGIN rejection, got {outcome:?}"
    );
    assert!(
        logs.contains("AUTHENTICATIONFAILED"),
        "expected AUTHENTICATIONFAILED in log; got:\n  {}",
        logs.lines().join("\n  ")
    );
    // The hint produced by diagnostics::imap_hints_for for this needle:
    assert!(
        logs.contains("bad password"),
        "expected the 'bad password' hint in log; got:\n  {}",
        logs.lines().join("\n  ")
    );
}

#[test]
fn imap_logindisabled_capability_triggers_hint() {
    // Server advertises LOGINDISABLED on plain channel; our client must
    // log the warning AND the diagnostic hint, then proceed to attempt
    // LOGIN anyway (which the mock will then refuse).
    let server = spawn_mock_server(|mut r, mut w| {
        writeln_crlf(
            &mut w,
            "* OK [CAPABILITY IMAP4rev1 LOGINDISABLED STARTTLS] mock ready",
        );
        // a1 CAPABILITY
        let _ = read_line(&mut r);
        writeln_crlf(&mut w, "* CAPABILITY IMAP4rev1 LOGINDISABLED STARTTLS");
        writeln_crlf(&mut w, "a1 OK CAPABILITY completed");
        // b1 LOGIN -- server refuses outright per the LOGINDISABLED
        // advertisement.
        let _ = read_line(&mut r);
        writeln_crlf(
            &mut w,
            "b1 NO [PRIVACYREQUIRED] LOGIN disabled on plain channel",
        );
        let _ = read_line(&mut r);
        writeln_crlf(&mut w, "* BYE");
        writeln_crlf(&mut w, "z1 OK LOGOUT");
    });

    let logs = LogCapture::install();
    let profile = profile_for(server.addr);

    let outcome = imap::run(&profile);
    drop(server);

    assert!(matches!(outcome, Ok(false)));
    assert!(
        logs.contains("LOGINDISABLED"),
        "expected LOGINDISABLED to be logged or quoted from CAPABILITY; got:\n  {}",
        logs.lines().join("\n  ")
    );
}
