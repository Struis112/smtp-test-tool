//! Translates raw server replies into human-readable, IT-actionable hints.
//!
//! Tables ported verbatim from the Python `email_tester.py` so output stays
//! interchangeable between the two implementations.

use once_cell::sync::Lazy;
use std::collections::HashMap;

/// (short_explanation, remediation_hint)
pub type Hint = (&'static str, &'static str);

/// SMTP enhanced status codes ("5.7.139", "5.7.60", ...) -> Hint.
pub static SMTP_ENHANCED: Lazy<HashMap<&'static str, Hint>> = Lazy::new(|| {
    let mut m = HashMap::new();
    // 5.x.x permanent failures
    m.insert(
        "5.7.0",
        (
            "Authentication required, or chosen AUTH mechanism not permitted.",
            "Enable SMTP AUTH for the mailbox; verify LOGIN / PLAIN / XOAUTH2 is allowed.",
        ),
    );
    m.insert("5.7.1",   ("Relay access denied - server refuses to forward this message.",
                          "Either the sender is not authenticated, the recipient is external and relaying is disabled, or a transport rule is blocking the message."));
    m.insert("5.7.3",   ("Authentication unsuccessful.",
                          "Bad password, MFA enabled without app-password, or Basic/Legacy auth disabled on the tenant (O365: 'SMTP AUTH disabled')."));
    m.insert(
        "5.7.8",
        (
            "Authentication credentials invalid.",
            "Username/password rejected by the SASL layer.",
        ),
    );
    m.insert("5.7.57",  ("Client was not authenticated to send anonymous mail during MAIL FROM.",
                          "Force STARTTLS + AUTH before MAIL FROM. In O365 this is the typical error when the client connects without AUTH."));
    m.insert("5.7.60",  ("SendAsDenied - authenticated user is not allowed to send AS this From: address.",
                          "Grant the mailbox 'Send As' (or 'Send on Behalf') rights to the authenticated account, or change the From: header to match the login."));
    m.insert(
        "5.7.64",
        (
            "TenantAttribution; Relay Access Denied.",
            "Authenticated SMTP submission requires a licensed mailbox in O365.",
        ),
    );
    m.insert(
        "5.7.124",
        (
            "The user is not authorised to send mail.",
            "Disabled mailbox, blocked by Conditional Access, or licence missing.",
        ),
    );
    m.insert(
        "5.7.135",
        (
            "Authentication unsuccessful, the user credentials have expired.",
            "Reset the password / refresh the OAuth token.",
        ),
    );
    m.insert(
        "5.7.139",
        (
            "Authentication unsuccessful, the request did not meet the criteria.",
            "Conditional Access policy denied the login (location, device, MFA).",
        ),
    );
    m.insert(
        "5.7.500",
        (
            "Access denied, sending domain disabled.",
            "The sender's domain is blocked for outbound mail on this tenant.",
        ),
    );
    m.insert(
        "5.7.501",
        (
            "Access denied, banned sender.",
            "Sender address is on a tenant blocklist.",
        ),
    );
    m.insert(
        "5.7.508",
        (
            "Access denied, rate or traffic threshold exceeded.",
            "Throttled; wait and retry, or ask admin to raise the limit.",
        ),
    );
    m.insert(
        "5.7.511",
        (
            "Access denied, banned sender.",
            "Sender flagged as spam source.",
        ),
    );
    m.insert(
        "5.7.606",
        (
            "Access denied, banned sending IP.",
            "The submitting IP is on a Microsoft blocklist.",
        ),
    );
    m.insert(
        "5.7.708",
        (
            "Service refused. Source IP has bad reputation.",
            "Submit from a different IP or request delisting.",
        ),
    );
    m.insert(
        "5.7.750",
        (
            "Client blocked from sending from unregistered domains.",
            "Verify the sender domain in the tenant or use an accepted domain.",
        ),
    );
    m.insert(
        "5.1.0",
        (
            "Sender address rejected.",
            "From/MAIL FROM not accepted; usually format or domain policy.",
        ),
    );
    m.insert(
        "5.1.1",
        (
            "Bad destination mailbox - recipient does not exist.",
            "Check the recipient address.",
        ),
    );
    m.insert(
        "5.1.7",
        (
            "Invalid sender address (malformed).",
            "Fix the MAIL FROM syntax.",
        ),
    );
    m.insert(
        "5.1.8",
        (
            "Sender domain not allowed.",
            "Domain not accepted by the server.",
        ),
    );
    m.insert(
        "5.1.10",
        (
            "Recipient address rejected - user unknown.",
            "Typo or non-existent recipient.",
        ),
    );
    m.insert(
        "5.4.1",
        (
            "Recipient address rejected: access denied.",
            "Recipient mailbox refuses messages from this sender / domain.",
        ),
    );
    m.insert(
        "5.2.1",
        (
            "Mailbox disabled, not accepting messages.",
            "Recipient mailbox suspended.",
        ),
    );
    m.insert("5.2.2", ("Mailbox full.", "Recipient quota exceeded."));
    m.insert(
        "5.3.4",
        ("Message too big for system.", "Reduce message size."),
    );
    // 4.x.x temporary failures
    m.insert(
        "4.7.0",
        (
            "Temporary authentication failure / throttling.",
            "Try again later; could also be tarpit for repeated bad logins.",
        ),
    );
    m.insert(
        "4.4.2",
        (
            "Connection dropped.",
            "Network glitch or server restart; retry.",
        ),
    );
    m.insert(
        "4.3.2",
        (
            "System not accepting network messages.",
            "Server maintenance.",
        ),
    );
    m
});

/// IMAP error-string substring -> hint.
pub const IMAP_HINTS: &[(&str, &str)] = &[
    (
        "AUTHENTICATIONFAILED",
        "IMAP login rejected - bad password, MFA without app-password, or Basic Auth disabled.",
    ),
    ("LOGIN failed", "IMAP login rejected by server."),
    (
        "[ALERT]",
        "Server returned an ALERT response - read it; admin-defined message.",
    ),
    ("[UNAVAILABLE]", "Mailbox/server temporarily unavailable."),
    (
        "[PRIVACYREQUIRED]",
        "Server requires TLS before LOGIN - use STARTTLS or implicit SSL.",
    ),
    (
        "[CLIENTBUG]",
        "Client did something the server considers wrong; usually missing STARTTLS or wrong state.",
    ),
    (
        "LOGINDISABLED",
        "Plain LOGIN is disabled on this server - use STARTTLS/SSL or XOAUTH2.",
    ),
];

pub const POP_HINTS: &[(&str, &str)] = &[
    (
        "authentication failed",
        "POP3 login rejected - bad credentials or POP disabled for this mailbox.",
    ),
    ("Logon failure", "POP3 login rejected."),
    (
        "not implemented",
        "Server does not support the issued command.",
    ),
    (
        "disabled",
        "POP3 access is disabled for this account / tenant.",
    ),
];

/// Given any SMTP error message, return the matched enhanced-status hints.
pub fn smtp_hints_for(msg: &str) -> Vec<String> {
    let mut out = Vec::new();
    for esc in extract_enhanced_codes(msg) {
        if let Some((what, fix)) = SMTP_ENHANCED.get(esc.as_str()) {
            out.push(format!("  ESC {esc}: {what}"));
            out.push(format!("  -> Action: {fix}"));
        }
    }
    out
}

pub fn imap_hints_for(msg: &str) -> Vec<String> {
    let lower = msg.to_lowercase();
    IMAP_HINTS
        .iter()
        .filter(|(needle, _)| lower.contains(&needle.to_lowercase()))
        .map(|(_, hint)| format!("  -> {hint}"))
        .collect()
}

pub fn pop_hints_for(msg: &str) -> Vec<String> {
    let lower = msg.to_lowercase();
    POP_HINTS
        .iter()
        .filter(|(needle, _)| lower.contains(&needle.to_lowercase()))
        .map(|(_, hint)| format!("  -> {hint}"))
        .collect()
}

/// Find enhanced status codes like "5.7.139" or "4.7.0" inside an arbitrary
/// server reply.  Tiny regex-free scanner.
fn extract_enhanced_codes(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // First char must be '2', '4' or '5' preceded by a word boundary.
        let prev_is_boundary = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if prev_is_boundary && matches!(bytes[i], b'2' | b'4' | b'5') {
            // Pattern: D '.' D{1,3} '.' D{1,3} not followed by digit/alnum.
            let mut j = i + 1;
            if j < bytes.len() && bytes[j] == b'.' {
                j += 1;
                let mid_start = j;
                while j < bytes.len() && bytes[j].is_ascii_digit() && j - mid_start < 3 {
                    j += 1;
                }
                if j > mid_start && j < bytes.len() && bytes[j] == b'.' {
                    j += 1;
                    let tail_start = j;
                    while j < bytes.len() && bytes[j].is_ascii_digit() && j - tail_start < 3 {
                        j += 1;
                    }
                    if j > tail_start && (j == bytes.len() || !bytes[j].is_ascii_alphanumeric()) {
                        out.push(s[i..j].to_string());
                        i = j;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- enhanced-code scanner ------------------------------------

    #[test]
    fn extracts_codes_with_boundaries() {
        let v = extract_enhanced_codes("535 5.7.139 Authentication unsuccessful; 4.7.0 throttle");
        assert_eq!(v, vec!["5.7.139", "4.7.0"]);
    }

    #[test]
    fn ignores_version_strings_and_partials() {
        // 1.x.y is not an enhanced status code (1 is not 2/4/5).
        assert!(extract_enhanced_codes("running 1.2.3 release").is_empty());
        // 5.7 alone (no third octet) must not match.
        assert!(extract_enhanced_codes("see 5.7 spec").is_empty());
        // Embedded inside a word should not match (must be word-bounded).
        assert!(extract_enhanced_codes("v5.7.60suffix").is_empty());
    }

    #[test]
    fn extracts_at_start_and_end_of_string() {
        assert_eq!(extract_enhanced_codes("5.1.1"), vec!["5.1.1"]);
        assert_eq!(extract_enhanced_codes("foo 4.4.2"), vec!["4.4.2"]);
    }

    // ---- SMTP hint mapping ---------------------------------------

    #[test]
    fn hint_includes_send_as_for_5_7_60() {
        let h = smtp_hints_for("550 5.7.60 SendAsDenied");
        let joined = h.join("\n");
        assert!(joined.contains("SendAsDenied"));
        assert!(joined.contains("Send As"));
        assert!(joined.contains("ESC 5.7.60"));
    }

    #[test]
    fn hint_includes_basic_auth_for_5_7_139() {
        let h = smtp_hints_for(
            "535 5.7.139 Authentication unsuccessful, basic authentication is disabled",
        );
        let joined = h.join("\n");
        assert!(joined.contains("5.7.139"));
        assert!(joined.contains("Conditional Access"));
    }

    #[test]
    fn hint_for_unknown_code_is_empty() {
        // A code we don't have a mapping for (yet) should produce no
        // spurious lines, only the unmatched scanner result.
        let h = smtp_hints_for("550 5.9.999 Made up code");
        assert!(h.is_empty(), "expected no hints, got {h:?}");
    }

    #[test]
    fn hint_collects_multiple_codes_in_one_reply() {
        let h = smtp_hints_for("550 5.7.60 SendAsDenied; also see 5.1.1 for the recipient");
        let joined = h.join("\n");
        assert!(joined.contains("5.7.60"));
        assert!(joined.contains("5.1.1"));
    }

    // ---- IMAP hint mapping ---------------------------------------

    #[test]
    fn imap_hint_for_authenticationfailed() {
        let h = imap_hints_for("a1 NO [AUTHENTICATIONFAILED] LOGIN failed");
        assert!(!h.is_empty());
        assert!(h.iter().any(|s| s.contains("bad password")));
    }

    #[test]
    fn imap_hint_for_logindisabled() {
        let h = imap_hints_for("* CAPABILITY IMAP4rev1 LOGINDISABLED STARTTLS");
        assert!(h
            .iter()
            .any(|s| s.contains("STARTTLS") || s.contains("XOAUTH2")));
    }

    // ---- POP hint mapping ----------------------------------------

    #[test]
    fn pop_hint_for_authentication_failed() {
        let h = pop_hints_for("-ERR authentication failed");
        assert!(!h.is_empty());
        assert!(h
            .iter()
            .any(|s| s.contains("POP disabled") || s.contains("bad credentials")));
    }

    #[test]
    fn pop_hint_for_disabled() {
        let h = pop_hints_for("-ERR POP is disabled for this account");
        assert!(h.iter().any(|s| s.contains("disabled")));
    }
}
