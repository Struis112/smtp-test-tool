# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **OS-native keychain integration** behind a new `keychain` cargo
  feature (on by default).  Windows Credential Manager / macOS
  Keychain / Linux Secret Service via `keyring 3.6`.
  * `src/keystore.rs` exposes a `Keystore` trait with `save`, `load`,
    `forget`; `OsKeystore` (real) and `NullKeystore` (graceful no-op
    for builds without the feature).  6 unit tests use an in-memory
    `MockKeystore` so the suite passes on every OS, including headless
    Linux CI runners.
  * CLI: `--keychain-load` looks up the password at startup;
    `--keychain-save` writes it after a successful run only
    (failed AUTH never leaks a wrong password); `keychain status
    <user>` / `keychain forget <user>` subcommands inspect and manage
    entries.
  * GUI: *Save password to keychain* / *Forget keychain entry* buttons
    under the credentials block.  Auto-loads on startup when an entry
    exists for the current user, showing a small *(loaded from
    keychain)* hint next to the controls.
- **CLI provider parity** with the GUI: `--provider gmail` and
  friends (case-insensitive, unique-substring resolver), plus a
  `providers` subcommand that prints the full curated list.
- **Gmail 'Send mail as' client-side bounce diagnostic**: a user who
  pastes the bounce body (English or Dutch verbatim) into
  `smtp_hints_for` now gets an actionable pointer to *Gmail Settings
  > Accounts and Import > Send mail as > Edit info*.

### Changed
- `AGENTS.md` rule #8 expanded: OS keychain is now the documented
  one-and-only approved persistent store for credentials.

## [0.1.1] - 2026-05-19

### Security
- **Removed all persistence of credentials.**  `Profile::password` and
  `Profile::oauth_token` are now `#[serde(skip)]`; the base64 "opt-in"
  codepath, the GUI checkbox, and the `base64` crate dependency are
  all gone.  Enforced by `tests/config_roundtrip.rs::
  save_never_writes_credentials_even_when_set`.  Codified as rule #8
  in `AGENTS.md`.  **BREAKING**: a v0.1.0 user who had a saved
  `password_b64` in their config will need to re-enter the password
  on next launch.

### Changed
- **`send_test` now defaults to TRUE** (was false in v0.1.0).  Hitting
  *Run Test* on a fresh install will exercise the full end-to-end path
  including delivery, Send-As rights, and spam filters - not just AUTH.
  Users who want auth-only behaviour can untick the *Actually send a
  test email* box on the Send Mail tab.  Existing v0.1.0 configs that
  explicitly set `send_test = false` keep that value; configs missing
  the field will now load as true (matches fresh-install behaviour).

### Added
- **Provider-preset menu** (`src/providers.rs`) with eleven curated
  presets: Outlook.com, Microsoft 365, Gmail, Yahoo, iCloud, Proton
  Mail (Bridge), Fastmail, Zoho, AOL, GMX, Yandex.  Picking one from
  the top-bar *Provider preset ▾* menu rewrites the SMTP/IMAP/POP3
  host, port, and security fields on the active profile and logs an
  app-password / Bridge note where one applies.  Replaces the old
  one-shot "Reset to Outlook.com" button.
- **Manual theme override on the GUI Advanced tab.**  Auto / Dark /
  Light, with the resolved OS hint shown next to the Auto label;
  persisted through `Profile.theme` so existing v0.1.0 configs keep
  working without a migration.
- Dark + light GUI screenshots in `docs/screenshots/`, regenerable
  on Windows via `tools/screenshot.ps1`.  Linked from the README.
- 14 additional tests (now 17 total, was 3): full diagnostic
  translator coverage (SMTP enhanced-code boundaries, IMAP
  AUTHENTICATIONFAILED / LOGINDISABLED, POP -ERR phrases) plus a new
  integration test suite at `tests/config_roundtrip.rs` covering
  save+load symmetry, opt-in password obfuscation, multi-profile,
  and defaults-fallback for minimal hand-written configs.

### Fixed
- **Default config filename was still `email_tester.toml`** after the
  project rename, so `discover_config_path()` never found the file
  every doc told the user to create.  The GUI showed 'no config
  file' even with a valid TOML next to the exe; the CLI silently
  fell back to built-in Outlook defaults.
- GUI Servers tab body rendered empty due to a nested-ScrollArea
  height race in the egui 0.34 layout.  Split the log into its own
  resizable BottomPanel; tabs now get a properly-sized CentralPanel.
- GUI log panel was drowning in eframe / winit / glow / wgpu DEBUG
  noise.  Default tracing filter is now `info,eframe=warn,...`;
  power users can opt into more verbose output via `RUST_LOG`.
- `Config::load` parse errors are no longer silently swallowed; the
  GUI now logs the path and error so users can tell 'no config' from
  'config exists but won't parse'.

### Changed
- `cargo deny` `multiple-versions` ratcheted from `warn` to `deny`.
  The 20 known duplicate transitives are enumerated in `deny.toml`
  with a `reason` field each; any NEW duplicate will fail CI.

## [0.1.0] - 2026-05-19

### Added
- Initial Rust port of the email connectivity tester.
- CLI binary (`smtp-test-tool`) with TOML config + named profiles and
  Outlook.com defaults.
- GUI binary (`smtp-test-tool-gui`) built on eframe/egui with OS
  dark/light auto-follow (hand-rolled detection, no third-party crate)
  and AccessKit screen-reader support.
- IT-actionable diagnostic translator for the most common Microsoft
  365 SMTP failure codes (5.7.60 SendAsDenied, 5.7.139 Basic-Auth-
  disabled, 5.7.57 unauthenticated MAIL FROM, …).
- Hand-rolled IMAP + POP3 clients over `rustls` so we own the full
  wire trace.
- Public release infrastructure: GitHub Actions CI matrix (fmt,
  clippy, test on Linux/macOS/Windows, MSRV 1.92, cargo-deny,
  cargo-llvm-cov → Codecov, cargo doc → GitHub Pages); release
  workflow producing cross-OS binaries on tag push and publishing
  to crates.io.

### Project conventions
- `AGENTS.md` captures the working agreement: WCAG 2.2 AAA is the
  baseline, dark+light mode on every OS is mandatory, atomic
  conventional commits, no shortcuts.

[Unreleased]: https://github.com/Struis112/smtp-test-tool/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/Struis112/smtp-test-tool/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/Struis112/smtp-test-tool/releases/tag/v0.1.0
