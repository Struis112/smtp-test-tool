//! DNS-side checks for a mail domain.
//!
//! Most mail-flow problems we get blamed for are actually somebody
//! else's DNS: a missing SPF record, a `p=none` DMARC policy that the
//! receiving side has tightened to `p=reject`, a forgotten MX, a
//! reverse-DNS that does not exist for the sending IP.  This module
//! runs the five lookups that catch ~90% of those failures and turns
//! the raw answers into IT-actionable hints.
//!
//! ## Design
//!
//! The public API is **synchronous** to fit the rest of the codebase
//! (`src/smtp.rs`, `src/imap.rs`, etc.).  Internally we spin up a
//! `tokio` `current_thread` runtime per call because hickory-resolver
//! 0.26 dropped its sync entry points.  The runtime lives for the
//! duration of one `audit_domain` and then drops.
//!
//! No DNSSEC validation, no DoH/DoT - those are valuable but
//! orthogonal additions for a future release.  We use the system
//! resolver configuration (`/etc/resolv.conf` or the Windows
//! equivalent) so the answers match what other tools on the same
//! machine see.

use std::net::IpAddr;
use std::time::Duration;

use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::proto::rr::{RData, RecordType};
use hickory_resolver::{Resolver, TokioResolver};
use serde::{Deserialize, Serialize};
use tokio::runtime::Builder;

// =====================================================================
// Public types
// =====================================================================

/// One MX record from the apex domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MxRecord {
    /// Preference / priority value (lower = preferred).
    pub preference: u16,
    /// Exchange hostname (trailing dot stripped).
    pub exchange: String,
    /// Forward A / AAAA addresses for the exchange host, if resolution
    /// succeeded.  Empty vec means we tried and failed - the receiver
    /// will not be able to deliver here.
    pub ips: Vec<IpAddr>,
}

/// SPF record (TXT at the apex starting with `v=spf1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpfRecord {
    /// The full raw `v=spf1 ...` string.
    pub raw: String,
    /// Detected "all" mechanism qualifier:
    ///   - `Some("-")` = `-all`     (fail / hard reject)
    ///   - `Some("~")` = `~all`     (softfail)
    ///   - `Some("?")` = `?all`     (neutral)
    ///   - `Some("+")` = `+all`     (pass anything - effectively no policy)
    ///   - `None`      = no `all` mechanism present
    pub all_qualifier: Option<String>,
}

/// DMARC record (TXT at `_dmarc.<domain>` starting with `v=DMARC1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DmarcRecord {
    pub raw: String,
    /// `p=` value: usually `none`, `quarantine`, `reject`.
    pub policy: Option<String>,
    /// `sp=` value (subdomain policy).
    pub subdomain_policy: Option<String>,
    /// `pct=` value (sampling rate).
    pub pct: Option<u8>,
}

/// Full audit report for one domain.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DnsReport {
    pub domain: String,
    pub mx: Vec<MxRecord>,
    pub spf: Option<SpfRecord>,
    pub dmarc: Option<DmarcRecord>,
}

/// One IT-actionable hint produced by `interpret`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsHint {
    /// Stable ID for translation lookup, e.g. `"no_mx"`, `"spf_plus_all"`.
    pub id: &'static str,
    /// English fallback text - the GUI can translate via i18n if the
    /// locale has the matching `diagnostics.dns.<id>` key.
    pub text: String,
    /// Severity for UI colouring.
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// "Your mail will be silently dropped" - bright red.
    Critical,
    /// "Your mail looks suspicious; deliverability suffers" - amber.
    Warning,
    /// "Worth knowing but not urgent."
    Info,
}

#[derive(Debug, thiserror::Error)]
pub enum DnsError {
    #[error("invalid domain name: {0}")]
    BadDomain(String),
    #[error("tokio runtime error: {0}")]
    Runtime(#[from] std::io::Error),
    #[error("resolver error: {0}")]
    Resolver(String),
}

// =====================================================================
// Public entry point
// =====================================================================

/// Run all five lookups against `domain` and return a fully populated
/// report.  Network-bound; ~1-2s on a healthy network, up to the
/// resolver timeout if DNS is broken.
pub fn audit_domain(domain: &str) -> Result<DnsReport, DnsError> {
    let domain = domain.trim().trim_end_matches('.').to_lowercase();
    if domain.is_empty() || !domain.contains('.') {
        return Err(DnsError::BadDomain(domain));
    }

    let rt = Builder::new_current_thread().enable_all().build()?;
    rt.block_on(audit_domain_async(&domain))
}

async fn audit_domain_async(domain: &str) -> Result<DnsReport, DnsError> {
    // hickory 0.26's builder_tokio reads the system resolver config
    // (/etc/resolv.conf on Unix, the registry on Windows) so our
    // answers match what other tools on the same host see.
    let mut builder =
        TokioResolver::builder_tokio().map_err(|e| DnsError::Resolver(e.to_string()))?;
    builder.options_mut().timeout = Duration::from_secs(5);
    let resolver = builder
        .build()
        .map_err(|e| DnsError::Resolver(e.to_string()))?;

    let mx = lookup_mx(&resolver, domain).await;
    let spf = lookup_spf(&resolver, domain).await;
    let dmarc = lookup_dmarc(&resolver, domain).await;

    Ok(DnsReport {
        domain: domain.to_string(),
        mx,
        spf,
        dmarc,
    })
}

// =====================================================================
// Individual lookups
// =====================================================================

type Rsv = Resolver<TokioRuntimeProvider>;

async fn lookup_mx(resolver: &Rsv, domain: &str) -> Vec<MxRecord> {
    let Ok(answers) = resolver.mx_lookup(domain).await else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for rec in answers.answers() {
        let RData::MX(mx) = &rec.data else { continue };
        let exchange = mx.exchange.to_utf8().trim_end_matches('.').to_string();
        // Forward-resolve the exchange to surface DNS-chain breaks.
        let ips: Vec<IpAddr> = resolver
            .lookup_ip(&exchange)
            .await
            .map(|x| x.iter().collect())
            .unwrap_or_default();
        out.push(MxRecord {
            preference: mx.preference,
            exchange,
            ips,
        });
    }
    out.sort_by_key(|r| r.preference);
    out
}

async fn lookup_spf(resolver: &Rsv, domain: &str) -> Option<SpfRecord> {
    let raw = txt_record_matching(resolver, domain, "v=spf1").await?;
    Some(parse_spf(&raw))
}

async fn lookup_dmarc(resolver: &Rsv, domain: &str) -> Option<DmarcRecord> {
    let raw = txt_record_matching(resolver, &format!("_dmarc.{domain}"), "v=DMARC1").await?;
    Some(parse_dmarc(&raw))
}

/// Look up TXT records at `name` and return the first one whose value
/// starts with `prefix` (case-sensitive, per RFCs 4408 / 7489).
async fn txt_record_matching(resolver: &Rsv, name: &str, prefix: &str) -> Option<String> {
    let answers = resolver.lookup(name, RecordType::TXT).await.ok()?;
    for rec in answers.answers() {
        let RData::TXT(txt) = &rec.data else { continue };
        // A TXT record can be a sequence of multiple <character-string>s
        // (RFC 1035 sec 3.3.14, RFC 7208 sec 3.3); concatenate them to
        // recover the logical record value.
        let joined: String = txt
            .txt_data
            .iter()
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .collect();
        if joined.starts_with(prefix) {
            return Some(joined);
        }
    }
    None
}

// =====================================================================
// Record parsers (pure - no I/O, easy to test)
// =====================================================================

pub(crate) fn parse_spf(raw: &str) -> SpfRecord {
    let all_qualifier = raw.split_whitespace().find_map(|tok| {
        let (q, m) = if let Some(rest) = tok.strip_prefix(['-', '~', '?', '+']) {
            (&tok[..1], rest)
        } else {
            ("+", tok)
        };
        if m == "all" {
            Some(q.to_string())
        } else {
            None
        }
    });
    SpfRecord {
        raw: raw.to_string(),
        all_qualifier,
    }
}

pub(crate) fn parse_dmarc(raw: &str) -> DmarcRecord {
    let mut record = DmarcRecord {
        raw: raw.to_string(),
        policy: None,
        subdomain_policy: None,
        pct: None,
    };
    for tag in raw.split(';') {
        let (k, v) = match tag.trim().split_once('=') {
            Some(pair) => (pair.0.trim(), pair.1.trim()),
            None => continue,
        };
        match k {
            "p" => record.policy = Some(v.to_string()),
            "sp" => record.subdomain_policy = Some(v.to_string()),
            "pct" => record.pct = v.parse().ok(),
            _ => {}
        }
    }
    record
}

// =====================================================================
// Interpretation - what would IT actually do about this report?
// =====================================================================

/// Convert a `DnsReport` into a flat list of hints, sorted by severity
/// (Critical first).  The English text in each hint is a usable
/// default; the GUI's i18n layer can substitute a localised version
/// keyed by `DnsHint::id`.
pub fn interpret(report: &DnsReport) -> Vec<DnsHint> {
    let mut out = Vec::new();

    // ---- MX ---------------------------------------------------------
    if report.mx.is_empty() {
        out.push(DnsHint {
            id: "no_mx",
            severity: Severity::Critical,
            text: format!(
                "{} has no MX records.  Mail to this domain will bounce \
                 with 'unrouteable address' on every receiver.",
                report.domain
            ),
        });
    } else {
        let mut broken = Vec::new();
        for mx in &report.mx {
            if mx.ips.is_empty() {
                broken.push(mx.exchange.clone());
            }
        }
        if !broken.is_empty() {
            out.push(DnsHint {
                id: "mx_no_a",
                severity: Severity::Critical,
                text: format!(
                    "MX hostname(s) without A/AAAA records: {}.  Receivers \
                     will fail to resolve where to deliver - fix the \
                     forward DNS for these hosts.",
                    broken.join(", ")
                ),
            });
        }
    }

    // ---- SPF --------------------------------------------------------
    match &report.spf {
        None => out.push(DnsHint {
            id: "no_spf",
            severity: Severity::Warning,
            text: format!(
                "{} has no SPF record.  Many receivers (Microsoft 365, \
                 Yahoo, AOL) treat this as 'suspicious' and may junk or \
                 reject the mail.",
                report.domain
            ),
        }),
        Some(spf) => match spf.all_qualifier.as_deref() {
            Some("+") => out.push(DnsHint {
                id: "spf_plus_all",
                severity: Severity::Critical,
                text: format!(
                    "SPF record ends with '+all' - this means 'anyone in \
                     the world may send as {}'.  Effectively no policy. \
                     Change to '-all' (strict) or '~all' (softfail).",
                    report.domain
                ),
            }),
            Some("?") => out.push(DnsHint {
                id: "spf_neutral_all",
                severity: Severity::Warning,
                text: "SPF record ends with '?all' (neutral).  Receivers \
                       cannot use SPF to distinguish legitimate mail \
                       from forgeries.  Tighten to '~all' or '-all'."
                    .to_string(),
            }),
            None => out.push(DnsHint {
                id: "spf_no_all",
                severity: Severity::Warning,
                text: "SPF record has no 'all' mechanism.  Per RFC 7208 \
                       this is treated as 'neutral' by most receivers. \
                       Append '~all' or '-all' to make the policy \
                       explicit."
                    .to_string(),
            }),
            _ => {}
        },
    }

    // ---- DMARC ------------------------------------------------------
    match &report.dmarc {
        None => out.push(DnsHint {
            id: "no_dmarc",
            severity: Severity::Warning,
            text: format!(
                "{} has no DMARC record at _dmarc.{}.  Without DMARC, \
                 receivers will not honour your SPF / DKIM alignment - \
                 spoofing of your domain is harder to block.",
                report.domain, report.domain
            ),
        }),
        Some(d) => {
            match d.policy.as_deref() {
                Some("none") => out.push(DnsHint {
                    id: "dmarc_p_none",
                    severity: Severity::Info,
                    text: "DMARC policy is 'p=none' (monitor-only).  \
                           Useful for the first weeks after deploying \
                           DMARC; tighten to p=quarantine then p=reject \
                           once aggregate reports look clean."
                        .to_string(),
                }),
                Some("quarantine") | Some("reject") => {
                    // Healthy: nothing to flag.
                }
                Some(other) => out.push(DnsHint {
                    id: "dmarc_p_unknown",
                    severity: Severity::Warning,
                    text: format!(
                        "DMARC policy 'p={other}' is not a value any \
                         receiver recognises.  Use 'none', 'quarantine', \
                         or 'reject'."
                    ),
                }),
                None => out.push(DnsHint {
                    id: "dmarc_no_p",
                    severity: Severity::Warning,
                    text: "DMARC record is missing the required 'p=' \
                           tag.  Receivers will ignore the record."
                        .to_string(),
                }),
            }
            if let Some(pct) = d.pct {
                if pct < 100 {
                    out.push(DnsHint {
                        id: "dmarc_pct_low",
                        severity: Severity::Info,
                        text: format!(
                            "DMARC pct={pct}: only that percentage of \
                             non-aligned mail is acted on.  Fine while \
                             rolling out, but raise to pct=100 once \
                             reports are clean."
                        ),
                    });
                }
            }
        }
    }

    out.sort_by_key(|h| match h.severity {
        Severity::Critical => 0,
        Severity::Warning => 1,
        Severity::Info => 2,
    });
    out
}

// =====================================================================
// Pretty-printing - used by both the CLI subcommand and the GUI tab.
// =====================================================================

/// Multi-line text rendering of a report + its hints, with no colour
/// escape codes.  Suitable for the GUI's monospace log panel and for
/// the CLI's stdout.
pub fn render_report(report: &DnsReport, hints: &[DnsHint]) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let _ = writeln!(s, "DNS audit for {}", report.domain);
    let _ = writeln!(s, "{}", "-".repeat(20 + report.domain.len()));
    let _ = writeln!(s);

    if report.mx.is_empty() {
        let _ = writeln!(s, "MX:    (none)");
    } else {
        let _ = writeln!(s, "MX:");
        for mx in &report.mx {
            let ips = if mx.ips.is_empty() {
                "<unresolved>".to_string()
            } else {
                mx.ips
                    .iter()
                    .map(|i| i.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let _ = writeln!(s, "  {:>3}  {:<40}  [{}]", mx.preference, mx.exchange, ips);
        }
    }
    let _ = writeln!(s);

    match &report.spf {
        Some(spf) => {
            let _ = writeln!(s, "SPF:   {}", spf.raw);
            if let Some(q) = &spf.all_qualifier {
                let _ = writeln!(s, "         (all-qualifier: '{q}all')");
            }
        }
        None => {
            let _ = writeln!(s, "SPF:   (none)");
        }
    }
    let _ = writeln!(s);

    match &report.dmarc {
        Some(d) => {
            let _ = writeln!(s, "DMARC: {}", d.raw);
        }
        None => {
            let _ = writeln!(s, "DMARC: (none)");
        }
    }
    let _ = writeln!(s);

    if hints.is_empty() {
        let _ = writeln!(s, "Hints: (none - the basics look healthy)");
    } else {
        let _ = writeln!(s, "Hints:");
        for h in hints {
            let badge = match h.severity {
                Severity::Critical => "[CRIT]",
                Severity::Warning => "[WARN]",
                Severity::Info => "[info]",
            };
            let _ = writeln!(s, "  {badge} {}", h.text);
        }
    }
    s
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- SPF parser -------------------------------------------------

    #[test]
    fn spf_parses_minus_all() {
        let r = parse_spf("v=spf1 ip4:192.0.2.0/24 -all");
        assert_eq!(r.all_qualifier.as_deref(), Some("-"));
    }

    #[test]
    fn spf_parses_tilde_all() {
        let r = parse_spf("v=spf1 mx ~all");
        assert_eq!(r.all_qualifier.as_deref(), Some("~"));
    }

    #[test]
    fn spf_parses_plus_all_explicit() {
        let r = parse_spf("v=spf1 +all");
        assert_eq!(r.all_qualifier.as_deref(), Some("+"));
    }

    #[test]
    fn spf_parses_question_all() {
        let r = parse_spf("v=spf1 ?all");
        assert_eq!(r.all_qualifier.as_deref(), Some("?"));
    }

    #[test]
    fn spf_handles_no_all() {
        let r = parse_spf("v=spf1 include:_spf.google.com");
        assert!(r.all_qualifier.is_none());
    }

    // ---- DMARC parser -----------------------------------------------

    #[test]
    fn dmarc_parses_reject_policy() {
        let r = parse_dmarc("v=DMARC1; p=reject; rua=mailto:postmaster@example.com");
        assert_eq!(r.policy.as_deref(), Some("reject"));
        assert!(r.subdomain_policy.is_none());
        assert!(r.pct.is_none());
    }

    #[test]
    fn dmarc_parses_quarantine_with_sp_and_pct() {
        let r = parse_dmarc("v=DMARC1; p=quarantine; sp=reject; pct=50");
        assert_eq!(r.policy.as_deref(), Some("quarantine"));
        assert_eq!(r.subdomain_policy.as_deref(), Some("reject"));
        assert_eq!(r.pct, Some(50));
    }

    #[test]
    fn dmarc_tolerates_whitespace_chaos() {
        let r = parse_dmarc("v=DMARC1 ; p = none ;   pct=100");
        assert_eq!(r.policy.as_deref(), Some("none"));
        assert_eq!(r.pct, Some(100));
    }

    // ---- Interpretation --------------------------------------------

    #[test]
    fn interpret_flags_missing_mx_as_critical() {
        let report = DnsReport {
            domain: "example.com".into(),
            mx: vec![],
            spf: None,
            dmarc: None,
        };
        let hints = interpret(&report);
        assert!(hints.iter().any(|h| h.id == "no_mx"));
        assert!(hints.iter().find(|h| h.id == "no_mx").unwrap().severity == Severity::Critical);
    }

    #[test]
    fn interpret_flags_plus_all_as_critical() {
        let report = DnsReport {
            domain: "example.com".into(),
            mx: vec![MxRecord {
                preference: 10,
                exchange: "mx.example.com".into(),
                ips: vec!["192.0.2.1".parse().unwrap()],
            }],
            spf: Some(parse_spf("v=spf1 +all")),
            dmarc: None,
        };
        let hints = interpret(&report);
        let spf_hint = hints.iter().find(|h| h.id == "spf_plus_all").unwrap();
        assert_eq!(spf_hint.severity, Severity::Critical);
    }

    #[test]
    fn interpret_quiet_when_everything_healthy() {
        let report = DnsReport {
            domain: "example.com".into(),
            mx: vec![MxRecord {
                preference: 10,
                exchange: "mx.example.com".into(),
                ips: vec!["192.0.2.1".parse().unwrap()],
            }],
            spf: Some(parse_spf("v=spf1 mx -all")),
            dmarc: Some(parse_dmarc(
                "v=DMARC1; p=reject; rua=mailto:dmarc@example.com",
            )),
        };
        let hints = interpret(&report);
        // Only acceptable hint here would be an info-level one, never
        // critical or warning.
        assert!(hints.iter().all(|h| h.severity == Severity::Info));
    }

    #[test]
    fn render_includes_domain_and_hint_badges() {
        let report = DnsReport {
            domain: "example.com".into(),
            mx: vec![],
            spf: None,
            dmarc: None,
        };
        let hints = interpret(&report);
        let s = render_report(&report, &hints);
        assert!(s.contains("example.com"));
        assert!(s.contains("[CRIT]"));
        assert!(s.contains("(none)"));
    }

    // ---- Live integration test (network-bound; off by default) -----

    /// Hits live DNS for `outlook.com`.  We only assert that the
    /// **resolver path works** (i.e. we got some MX records back
    /// without panicking); we deliberately do NOT assert specific
    /// SPF / DMARC content because Microsoft tweaks those records
    /// from time to time and we do not want a green-field deploy
    /// to break the build.  Enabled with `cargo test --features
    /// live-net`.
    #[cfg(feature = "live-net")]
    #[test]
    fn live_outlook_audit() {
        let report = audit_domain("outlook.com").unwrap();
        assert!(
            !report.mx.is_empty(),
            "outlook.com should have MX records (got: {:?})",
            report
        );
    }
}
