# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Initial Rust port of the email connectivity tester.
- CLI binary (`smtp-test-tool`) with TOML config + named profiles and
  Outlook.com defaults.
- GUI binary (`smtp-test-tool-gui`) built on eframe/egui with OS
  dark/light auto-follow and AccessKit screen-reader support.
- IT-actionable diagnostic translator for the most common Microsoft
  365 SMTP failure codes (5.7.60 SendAsDenied, 5.7.139 Basic-Auth-
  disabled, 5.7.57 unauthenticated MAIL FROM, …).
- Hand-rolled IMAP + POP3 clients over `rustls` so we own the full
  wire trace.

### Project conventions
- `AGENTS.md` captures the working agreement: WCAG 2.2 AAA is the
  baseline, dark+light mode on every OS is mandatory, atomic
  conventional commits, no shortcuts.

[Unreleased]: https://github.com/Struis112/smtp-test-tool/compare/HEAD...HEAD
