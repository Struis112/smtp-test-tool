//! eframe / egui GUI for smtp-test-tool.
//!
//! * Single static binary, no external runtime needed.
//! * Auto-follows OS dark/light via the `dark-light` crate.
//! * AccessKit screen-reader integration is enabled via the eframe feature.
//! * All status conveyed in text too - colour is never the only signal.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use eframe::egui;
use smtp_test_tool::config::{default_save_path, discover_config_path, Config};
use smtp_test_tool::runner::{TestOutcome, TestResults};
use smtp_test_tool::theme::{detect as detect_appearance, Appearance};
use smtp_test_tool::tls::Security;
use smtp_test_tool::{outlook_defaults, run_tests, Profile};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

// -----------------------------------------------------------------------
// Log capture - a custom tracing Layer that pushes formatted records
// into a Vec the GUI displays.  Thread-safe.
// -----------------------------------------------------------------------
#[derive(Clone, Copy, Debug)]
enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
struct LogLine {
    level: LogLevel,
    text: String,
}

#[derive(Default)]
struct LogSink {
    lines: Mutex<Vec<LogLine>>,
}

impl LogSink {
    fn push(&self, level: LogLevel, text: String) {
        if let Ok(mut g) = self.lines.lock() {
            // Cap memory at 5000 lines so a long --wire run stays responsive.
            if g.len() > 5000 {
                g.drain(..1000);
            }
            g.push(LogLine { level, text });
        }
    }
    fn drain_into(&self, dst: &mut Vec<LogLine>) {
        if let Ok(mut g) = self.lines.lock() {
            dst.extend(g.drain(..));
        }
    }
}

struct GuiLayer {
    sink: Arc<LogSink>,
}

impl<S> Layer<S> for GuiLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut visitor = FieldFmt::default();
        event.record(&mut visitor);
        let lvl = match *event.metadata().level() {
            tracing::Level::TRACE => LogLevel::Trace,
            tracing::Level::DEBUG => LogLevel::Debug,
            tracing::Level::INFO => LogLevel::Info,
            tracing::Level::WARN => LogLevel::Warn,
            tracing::Level::ERROR => LogLevel::Error,
        };
        self.sink.push(lvl, visitor.message);
    }
}

#[derive(Default)]
struct FieldFmt {
    message: String,
}

impl tracing::field::Visit for FieldFmt {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}");
            // strip the surrounding quotes Debug puts on strings
            if self.message.starts_with('"') && self.message.ends_with('"') {
                self.message = self.message[1..self.message.len() - 1].to_string();
            }
        } else {
            self.message
                .push_str(&format!(" {}={value:?}", field.name()));
        }
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else {
            self.message.push_str(&format!(" {}={value}", field.name()));
        }
    }
}

// -----------------------------------------------------------------------
// Application state
// -----------------------------------------------------------------------
struct App {
    cfg_path: Option<PathBuf>,
    cfg: Config,
    profile_name: String,
    profile: Profile,
    log_sink: Arc<LogSink>,
    log_buf: Vec<LogLine>,
    show_pwd: bool,
    save_pwd: bool,
    busy: bool,
    result_rx: Option<Receiver<TestResults>>,
    last_results: TestResults,
    to_csv: String,
    cc_csv: String,
    bcc_csv: String,
    tab: Tab,
}

#[derive(PartialEq, Copy, Clone)]
enum Tab {
    Servers,
    Send,
    Tls,
    Advanced,
}

impl App {
    fn new(sink: Arc<LogSink>, cc: &eframe::CreationContext<'_>) -> Self {
        // OS theme follow (rule #4: always dark/light, all OS).
        // Uses our own theme::detect() so we do not depend on dark-light,
        // whose v2 pulls in the unmaintained async-std (RUSTSEC-2025-0052).
        let visuals = match detect_appearance() {
            Appearance::Dark => egui::Visuals::dark(),
            Appearance::Light => egui::Visuals::light(),
            Appearance::Unknown => {
                tracing::info!("OS did not advertise a colour scheme; defaulting to dark");
                egui::Visuals::dark()
            }
        };
        cc.egui_ctx.set_visuals(visuals);

        let cfg_path = discover_config_path();
        let cfg = cfg_path
            .as_ref()
            .and_then(|p| Config::load(p).ok())
            .unwrap_or_else(|| Config {
                active: "default".into(),
                profiles: [("default".into(), outlook_defaults())]
                    .into_iter()
                    .collect(),
            });
        let profile_name = cfg.active.clone();
        let profile = cfg
            .profile(&profile_name)
            .cloned()
            .unwrap_or_else(outlook_defaults);
        let to_csv = profile.to.join(", ");
        let cc_csv = profile.cc.join(", ");
        let bcc_csv = profile.bcc.join(", ");

        Self {
            cfg_path,
            cfg,
            profile_name,
            profile,
            log_sink: sink,
            log_buf: Vec::new(),
            show_pwd: false,
            save_pwd: false,
            busy: false,
            result_rx: None,
            last_results: TestResults::default(),
            to_csv,
            cc_csv,
            bcc_csv,
            tab: Tab::Servers,
        }
    }

    fn apply_outlook_defaults(&mut self) {
        let d = outlook_defaults();
        self.profile.smtp_host = d.smtp_host;
        self.profile.smtp_port = d.smtp_port;
        self.profile.smtp_security = d.smtp_security;
        self.profile.imap_host = d.imap_host;
        self.profile.imap_port = d.imap_port;
        self.profile.imap_security = d.imap_security;
        self.profile.pop_host = d.pop_host;
        self.profile.pop_port = d.pop_port;
        self.profile.pop_security = d.pop_security;
    }

    fn run_tests_async(&mut self) {
        // Sync CSV fields back into profile.
        self.profile.to = csv_to_vec(&self.to_csv);
        self.profile.cc = csv_to_vec(&self.cc_csv);
        self.profile.bcc = csv_to_vec(&self.bcc_csv);

        let (tx, rx): (Sender<TestResults>, Receiver<TestResults>) = std::sync::mpsc::channel();
        self.result_rx = Some(rx);
        self.busy = true;
        let profile = self.profile.clone();
        thread::spawn(move || {
            let r = run_tests(&profile);
            let _ = tx.send(r);
        });
    }

    fn save_config(&mut self) -> anyhow::Result<()> {
        // sync CSVs first
        self.profile.to = csv_to_vec(&self.to_csv);
        self.profile.cc = csv_to_vec(&self.cc_csv);
        self.profile.bcc = csv_to_vec(&self.bcc_csv);

        if !self.save_pwd {
            // Strip password before persisting unless user opted in.
            self.profile.password = None;
        }
        self.cfg
            .upsert_profile(&self.profile_name, self.profile.clone());
        self.cfg.active = self.profile_name.clone();
        let target = self.cfg_path.clone().unwrap_or_else(default_save_path);
        self.cfg.save(&target)?;
        self.cfg_path = Some(target);
        Ok(())
    }
}

fn csv_to_vec(s: &str) -> Vec<String> {
    s.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

impl eframe::App for App {
    // egui 0.34: `ui()` is the required entry; `update(ctx, frame)` is
    // deprecated and provided as a no-op default by the trait.
    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root_ui.ctx().clone();
        // Drain log lines from the sink.
        self.log_sink.drain_into(&mut self.log_buf);
        if self.log_buf.len() > 5000 {
            let drop = self.log_buf.len() - 5000;
            self.log_buf.drain(..drop);
        }

        // Poll background test thread.
        if let Some(rx) = &self.result_rx {
            if let Ok(r) = rx.try_recv() {
                self.last_results = r;
                self.busy = false;
                self.result_rx = None;
            }
        }
        if self.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }

        // ----- top bar ------------------------------------------------
        egui::Panel::top("top").show_inside(root_ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Profile:");
                let names = self.cfg.profile_names();
                egui::ComboBox::from_id_salt("profile")
                    .selected_text(&self.profile_name)
                    .show_ui(ui, |ui| {
                        for n in &names {
                            if ui.selectable_label(&self.profile_name == n, n).clicked() {
                                self.profile_name = n.clone();
                                if let Some(p) = self.cfg.profile(n) {
                                    self.profile = p.clone();
                                    self.to_csv = self.profile.to.join(", ");
                                    self.cc_csv = self.profile.cc.join(", ");
                                    self.bcc_csv = self.profile.bcc.join(", ");
                                }
                            }
                        }
                    });
                if ui.button("Save Config").clicked() {
                    if let Err(e) = self.save_config() {
                        tracing::error!("save failed: {e:#}");
                    } else {
                        tracing::info!(
                            "Saved profile [{}] to {}",
                            self.profile_name,
                            self.cfg_path
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_default()
                        );
                    }
                }
                if ui.button("Reset to Outlook.com").clicked() {
                    self.apply_outlook_defaults();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        self.cfg_path
                            .as_ref()
                            .map(|p| format!("{}", p.display()))
                            .unwrap_or_else(|| "no config file".into()),
                    );
                });
            });
        });

        // ----- bottom: action bar + summary ---------------------------
        egui::Panel::bottom("bottom").show_inside(root_ui, |ui| {
            ui.horizontal(|ui| {
                let run = ui.add_enabled(
                    !self.busy,
                    egui::Button::new(if self.busy { "Running..." } else { "Run Test" }),
                );
                if run.clicked() {
                    self.run_tests_async();
                }
                ui.separator();
                outcome_chip(ui, "SMTP", self.last_results.smtp);
                outcome_chip(ui, "IMAP", self.last_results.imap);
                outcome_chip(ui, "POP3", self.last_results.pop3);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Clear Log").clicked() {
                        self.log_buf.clear();
                    }
                });
            });
        });

        // ----- main: tabs + log split ---------------------------------
        // CentralPanel is the implicit "everything left over" area. It must
        // be added AFTER the top/bottom panels per the egui 0.34 docs.
        egui::CentralPanel::default().show_inside(root_ui, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Servers, "Servers");
                ui.selectable_value(&mut self.tab, Tab::Send, "Send Mail");
                ui.selectable_value(&mut self.tab, Tab::Tls, "TLS / Auth");
                ui.selectable_value(&mut self.tab, Tab::Advanced, "Advanced");
            });
            ui.separator();

            let avail = ui.available_height();
            egui::ScrollArea::vertical()
                .max_height(avail * 0.45)
                .show(ui, |ui| match self.tab {
                    Tab::Servers => tab_servers(ui, self),
                    Tab::Send => tab_send(ui, self),
                    Tab::Tls => tab_tls(ui, self),
                    Tab::Advanced => tab_advanced(ui, self),
                });

            ui.separator();
            ui.label(egui::RichText::new("Log").strong());
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for line in &self.log_buf {
                        let (color, tag) = level_style(line.level, ui.visuals().dark_mode);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new(tag).color(color).monospace());
                            ui.label(egui::RichText::new(&line.text).monospace());
                        });
                    }
                });
        });
    }
}

// ---------- helpers -------------------------------------------------------
fn outcome_chip(ui: &mut egui::Ui, name: &str, o: Option<TestOutcome>) {
    let (txt, col) = match o {
        Some(TestOutcome::Pass) => (
            format!("{name}: PASS"),
            egui::Color32::from_rgb(0x0e, 0x7c, 0x0e),
        ),
        Some(TestOutcome::Fail) => (
            format!("{name}: FAIL"),
            egui::Color32::from_rgb(0xa3, 0x00, 0x00),
        ),
        Some(TestOutcome::Skipped) => (format!("{name}: skip"), egui::Color32::GRAY),
        None => (format!("{name}: -"), egui::Color32::GRAY),
    };
    ui.label(egui::RichText::new(txt).color(col).monospace());
}

fn level_style(lvl: LogLevel, dark: bool) -> (egui::Color32, &'static str) {
    if dark {
        match lvl {
            LogLevel::Trace => (egui::Color32::from_rgb(0xa0, 0xa0, 0xa0), "[TRACE]"),
            LogLevel::Debug => (egui::Color32::from_rgb(0xa0, 0xa0, 0xa0), "[DEBUG]"),
            LogLevel::Info => (egui::Color32::from_rgb(0xf0, 0xf0, 0xf0), "[INFO ]"),
            LogLevel::Warn => (egui::Color32::from_rgb(0xff, 0xd1, 0x66), "[WARN ]"),
            LogLevel::Error => (egui::Color32::from_rgb(0xff, 0x6b, 0x6b), "[ERROR]"),
        }
    } else {
        match lvl {
            LogLevel::Trace => (egui::Color32::from_rgb(0x55, 0x55, 0x55), "[TRACE]"),
            LogLevel::Debug => (egui::Color32::from_rgb(0x55, 0x55, 0x55), "[DEBUG]"),
            LogLevel::Info => (egui::Color32::from_rgb(0x11, 0x11, 0x11), "[INFO ]"),
            LogLevel::Warn => (egui::Color32::from_rgb(0x8a, 0x4b, 0x00), "[WARN ]"),
            LogLevel::Error => (egui::Color32::from_rgb(0xa3, 0x00, 0x00), "[ERROR]"),
        }
    }
}

// ---------- tabs ----------------------------------------------------------
fn tab_servers(ui: &mut egui::Ui, a: &mut App) {
    egui::Grid::new("creds")
        .num_columns(3)
        .striped(false)
        .show(ui, |ui| {
            ui.label("Username:");
            let user = a.profile.user.clone().unwrap_or_default();
            let mut u = user;
            if ui.text_edit_singleline(&mut u).changed() {
                a.profile.user = Some(u).filter(|s| !s.is_empty());
            }
            ui.end_row();

            ui.label("Password:");
            let mut pwd = a.profile.password.clone().unwrap_or_default();
            let resp = ui.add(egui::TextEdit::singleline(&mut pwd).password(!a.show_pwd));
            if resp.changed() {
                a.profile.password = Some(pwd).filter(|s| !s.is_empty());
            }
            ui.checkbox(&mut a.show_pwd, "Show");
            ui.end_row();

            ui.label("OAuth token:");
            let mut t = a.profile.oauth_token.clone().unwrap_or_default();
            if ui
                .add(egui::TextEdit::singleline(&mut t).password(true))
                .changed()
            {
                a.profile.oauth_token = Some(t).filter(|s| !s.is_empty());
            }
            ui.label("(XOAUTH2, optional)");
            ui.end_row();
        });
    ui.separator();
    proto_block(
        ui,
        "SMTP",
        &mut a.profile.smtp_enabled,
        &mut a.profile.smtp_host,
        &mut a.profile.smtp_port,
        &mut a.profile.smtp_security,
    );
    proto_block(
        ui,
        "IMAP",
        &mut a.profile.imap_enabled,
        &mut a.profile.imap_host,
        &mut a.profile.imap_port,
        &mut a.profile.imap_security,
    );
    proto_block(
        ui,
        "POP3",
        &mut a.profile.pop_enabled,
        &mut a.profile.pop_host,
        &mut a.profile.pop_port,
        &mut a.profile.pop_security,
    );
}

fn proto_block(
    ui: &mut egui::Ui,
    name: &str,
    enabled: &mut bool,
    host: &mut String,
    port: &mut u16,
    sec: &mut Security,
) {
    ui.horizontal(|ui| {
        ui.checkbox(enabled, format!("Test {name}"));
        ui.label("Host:");
        ui.text_edit_singleline(host);
        ui.label("Port:");
        ui.add(egui::DragValue::new(port).range(1..=65535));
        ui.label("Security:");
        egui::ComboBox::from_id_salt(format!("{name}-sec"))
            .selected_text(sec.as_str())
            .show_ui(ui, |ui| {
                ui.selectable_value(sec, Security::None, "none");
                ui.selectable_value(sec, Security::StartTls, "starttls");
                ui.selectable_value(sec, Security::Implicit, "ssl");
            });
    });
}

fn tab_send(ui: &mut egui::Ui, a: &mut App) {
    ui.checkbox(
        &mut a.profile.send_test,
        "Actually send a test email (otherwise only AUTH is tested)",
    );
    ui.separator();
    egui::Grid::new("msg").num_columns(2).show(ui, |ui| {
        opt_line(ui, "MAIL FROM (envelope):", &mut a.profile.mail_from);
        opt_line(ui, "From: (header)       :", &mut a.profile.from_addr);
        ui.label("To  (comma sep):");
        ui.text_edit_singleline(&mut a.to_csv);
        ui.end_row();
        ui.label("Cc  (comma sep):");
        ui.text_edit_singleline(&mut a.cc_csv);
        ui.end_row();
        ui.label("Bcc (comma sep):");
        ui.text_edit_singleline(&mut a.bcc_csv);
        ui.end_row();
        opt_line(ui, "Reply-To:", &mut a.profile.reply_to);
        ui.label("Subject:");
        ui.text_edit_singleline(&mut a.profile.subject);
        ui.end_row();
    });
    ui.label("Body:");
    ui.add(
        egui::TextEdit::multiline(&mut a.profile.body)
            .desired_rows(6)
            .desired_width(f32::INFINITY),
    );
}

fn opt_line(ui: &mut egui::Ui, label: &str, v: &mut Option<String>) {
    ui.label(label);
    let mut buf = v.clone().unwrap_or_default();
    if ui.text_edit_singleline(&mut buf).changed() {
        *v = if buf.is_empty() { None } else { Some(buf) };
    }
    ui.end_row();
}

fn tab_tls(ui: &mut egui::Ui, a: &mut App) {
    ui.checkbox(
        &mut a.profile.insecure_tls,
        "Disable certificate verification (INSECURE - testing only)",
    );
    ui.horizontal(|ui| {
        ui.label("CA bundle (PEM):");
        let mut buf = a
            .profile
            .ca_file
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        if ui.text_edit_singleline(&mut buf).changed() {
            a.profile.ca_file = if buf.is_empty() {
                None
            } else {
                Some(buf.into())
            };
        }
    });
}

fn tab_advanced(ui: &mut egui::Ui, a: &mut App) {
    egui::Grid::new("adv").num_columns(2).show(ui, |ui| {
        ui.label("Timeout (s):");
        ui.add(egui::DragValue::new(&mut a.profile.timeout_secs).range(1..=600));
        ui.end_row();

        ui.label("EHLO/HELO name:");
        let mut e = a.profile.ehlo_name.clone().unwrap_or_default();
        if ui.text_edit_singleline(&mut e).changed() {
            a.profile.ehlo_name = if e.is_empty() { None } else { Some(e) };
        }
        ui.end_row();

        ui.label("IMAP folder:");
        ui.text_edit_singleline(&mut a.profile.imap_folder);
        ui.end_row();

        ui.label("Log level:");
        egui::ComboBox::from_id_salt("loglvl")
            .selected_text(&a.profile.log_level)
            .show_ui(ui, |ui| {
                for lv in ["trace", "debug", "info", "warn", "error"] {
                    ui.selectable_value(&mut a.profile.log_level, lv.into(), lv);
                }
            });
        ui.end_row();

        ui.label("Save password in config:");
        ui.checkbox(&mut a.save_pwd, "(base64, NOT encryption)");
        ui.end_row();
    });
}

// -----------------------------------------------------------------------
// main
// -----------------------------------------------------------------------
fn main() -> eframe::Result<()> {
    let sink = Arc::new(LogSink::default());
    let layer = GuiLayer { sink: sink.clone() };
    tracing_subscriber::registry()
        .with(LevelFilter::DEBUG)
        .with(layer)
        .init();

    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 760.0])
            .with_min_inner_size([720.0, 520.0])
            .with_title("SMTP Test Tool"),
        ..Default::default()
    };
    eframe::run_native(
        "SMTP Test Tool",
        opts,
        Box::new(|cc| Ok(Box::new(App::new(sink, cc)))),
    )
}
