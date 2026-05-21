//! Shared helpers for integration tests.
//!
//! Two utilities:
//!
//! * [`spawn_mock_server`] — accept a single TCP connection on
//!   `127.0.0.1:0` and run a user-supplied closure that speaks
//!   whatever protocol the test needs.  The closure is handed a
//!   `BufReader<TcpStream>` for reads and a `TcpStream` for writes;
//!   it is responsible for the full protocol dance and for closing
//!   the connection (drop-on-return is enough).
//!
//! * [`LogCapture`] — a thread-local [`tracing`] subscriber that
//!   captures every `info!` / `warn!` / `error!` line emitted while
//!   the guard is alive.  Lets tests assert that the diagnostic
//!   translator (`smtp_hints_for`, …) actually fired with the
//!   expected hint strings, not just that the protocol function
//!   returned the expected `Ok(false)`.
//!
//! Lives under `tests/common/` so Cargo does NOT treat it as a test
//! binary itself; sibling test files pull it in with `mod common;`.

#![allow(dead_code)] // some helpers are protocol-specific; ignore in other tests

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Mock server
// ---------------------------------------------------------------------------

/// Returned to the test; dropping it joins the background thread.
pub struct MockServer {
    pub addr: SocketAddr,
    join: Option<JoinHandle<()>>,
}

impl Drop for MockServer {
    fn drop(&mut self) {
        // Best-effort join.  We don't return the closure's panics to
        // the test; the test will already have failed on a wrong
        // assertion if the protocol went off-script.
        if let Some(h) = self.join.take() {
            let _ = h.join();
        }
    }
}

/// Start a one-shot mock server on `127.0.0.1:<random>` and hand the
/// accepted connection to `handler`.  Returns the bound address and a
/// `MockServer` guard that joins the worker thread on drop.
///
/// The handler signature gives a buffered reader (line-oriented protocol
/// commands) and the raw stream for writes; closures should NOT use the
/// reader for writes because BufReader's internal cursor would diverge
/// from the kernel buffer.
pub fn spawn_mock_server<F>(handler: F) -> MockServer
where
    F: FnOnce(BufReader<TcpStream>, TcpStream) + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    let join = thread::spawn(move || {
        // Single connection, then close the listener.
        let (stream, _peer) = match listener.accept() {
            Ok(s) => s,
            Err(_) => return,
        };
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
        let writer = stream.try_clone().expect("try_clone");
        let reader = BufReader::new(stream);
        handler(reader, writer);
    });
    MockServer {
        addr,
        join: Some(join),
    }
}

/// Convenience: write `line` followed by CRLF.
pub fn writeln_crlf(w: &mut TcpStream, line: &str) {
    let _ = w.write_all(line.as_bytes());
    let _ = w.write_all(b"\r\n");
    let _ = w.flush();
}

/// Convenience: read a single CRLF-terminated line, return the line
/// WITHOUT the trailing CR/LF.  Returns an empty string on EOF.
pub fn read_line(r: &mut BufReader<TcpStream>) -> String {
    let mut buf = String::new();
    if r.read_line(&mut buf).is_err() {
        return String::new();
    }
    buf.trim_end_matches(['\r', '\n']).to_string()
}

// ---------------------------------------------------------------------------
// Log capture
// ---------------------------------------------------------------------------

/// Per-thread tracing subscriber that pushes every event's message field
/// into an `Arc<Mutex<Vec<String>>>`.  Tests construct one, then assert
/// against [`LogCapture::lines`].
pub struct LogCapture {
    lines: Arc<Mutex<Vec<String>>>,
    _guard: tracing::subscriber::DefaultGuard,
}

impl LogCapture {
    /// Install the capturing subscriber as the default for the calling
    /// thread.  The returned guard MUST be kept alive for the duration
    /// of the test - drop it before the protocol call finishes and the
    /// hint events go to a default no-op subscriber instead.
    pub fn install() -> Self {
        use tracing_subscriber::layer::SubscriberExt;

        let lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let layer = CaptureLayer {
            lines: lines.clone(),
        };
        let subscriber = tracing_subscriber::registry().with(layer);
        let guard = tracing::subscriber::set_default(subscriber);
        Self {
            lines,
            _guard: guard,
        }
    }

    pub fn lines(&self) -> Vec<String> {
        self.lines.lock().expect("log capture mutex").clone()
    }

    /// `true` iff any captured line contains `needle` as a substring.
    pub fn contains(&self, needle: &str) -> bool {
        self.lines().iter().any(|l| l.contains(needle))
    }
}

struct CaptureLayer {
    lines: Arc<Mutex<Vec<String>>>,
}

impl<S> tracing_subscriber::Layer<S> for CaptureLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = MsgVisitor::default();
        event.record(&mut visitor);
        if let Ok(mut g) = self.lines.lock() {
            g.push(visitor.msg);
        }
    }
}

#[derive(Default)]
struct MsgVisitor {
    msg: String,
}

impl tracing::field::Visit for MsgVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            // strip surrounding quotes Debug puts on str values
            let s = format!("{value:?}");
            self.msg = s
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .map(str::to_string)
                .unwrap_or(s);
        } else {
            self.msg.push_str(&format!(" {}={value:?}", field.name()));
        }
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.msg = value.to_string();
        } else {
            self.msg.push_str(&format!(" {}={value}", field.name()));
        }
    }
}
