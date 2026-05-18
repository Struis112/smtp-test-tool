//! High-level orchestrator: run whichever of SMTP / IMAP / POP3 are enabled.

use crate::config::Profile;
use crate::{imap, pop3, smtp};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tracing::{error, info};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestOutcome {
    Pass,
    Fail,
    Skipped,
}

impl TestOutcome {
    pub fn as_tag(self) -> &'static str {
        match self {
            TestOutcome::Pass => "[ PASS ]",
            TestOutcome::Fail => "[ FAIL ]",
            TestOutcome::Skipped => "[ SKIP ]",
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TestResults {
    pub smtp: Option<TestOutcome>,
    pub imap: Option<TestOutcome>,
    pub pop3: Option<TestOutcome>,
    pub elapsed_ms: u128,
}

impl TestResults {
    pub fn all_passed(&self) -> bool {
        let xs = [self.smtp, self.imap, self.pop3];
        let any_run = xs.iter().any(|o| matches!(o, Some(TestOutcome::Pass | TestOutcome::Fail)));
        any_run
            && xs
                .iter()
                .all(|o| !matches!(o, Some(TestOutcome::Fail)))
    }
}

/// Run every enabled protocol test on the given profile.
pub fn run_tests(p: &Profile) -> TestResults {
    let t0 = Instant::now();
    let mut r = TestResults::default();

    if p.smtp_enabled {
        r.smtp = Some(run_one("SMTP", || smtp::run(p)));
    } else {
        r.smtp = Some(TestOutcome::Skipped);
    }
    if p.imap_enabled {
        r.imap = Some(run_one("IMAP", || imap::run(p)));
    } else {
        r.imap = Some(TestOutcome::Skipped);
    }
    if p.pop_enabled {
        r.pop3 = Some(run_one("POP3", || pop3::run(p)));
    } else {
        r.pop3 = Some(TestOutcome::Skipped);
    }
    r.elapsed_ms = t0.elapsed().as_millis();

    info!("===== SUMMARY  ({} ms) =====", r.elapsed_ms);
    log_outcome("SMTP", r.smtp);
    log_outcome("IMAP", r.imap);
    log_outcome("POP3", r.pop3);
    r
}

fn run_one<F>(name: &str, f: F) -> TestOutcome
where
    F: FnOnce() -> anyhow::Result<bool>,
{
    match f() {
        Ok(true) => TestOutcome::Pass,
        Ok(false) => TestOutcome::Fail,
        Err(e) => {
            error!("{name} aborted: {e:#}");
            TestOutcome::Fail
        }
    }
}

fn log_outcome(name: &str, o: Option<TestOutcome>) {
    match o {
        Some(TestOutcome::Pass) => info!("  {}  {name}", TestOutcome::Pass.as_tag()),
        Some(TestOutcome::Fail) => error!("  {}  {name}", TestOutcome::Fail.as_tag()),
        Some(TestOutcome::Skipped) => info!("  {}  {name}", TestOutcome::Skipped.as_tag()),
        None => {}
    }
}
