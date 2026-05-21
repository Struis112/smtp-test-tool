//! OS-native credential storage abstraction.
//!
//! Provides a [`Keystore`] trait with three operations - `save`, `load`,
//! `forget` - backed by:
//!
//! * [`OsKeystore`] when the `keychain` cargo feature is on: dispatches
//!   to the `keyring` crate, which uses Windows Credential Manager on
//!   Windows, the macOS Keychain on macOS, and the Secret Service API
//!   (`gnome-keyring`, KWallet, etc.) on Linux.
//! * [`NullKeystore`] when the feature is off: `load` returns `None`,
//!   `forget` is a no-op, `save` returns an explicit error so callers
//!   surface "rebuild with --features keychain" rather than silently
//!   succeed.
//!
//! Tests use a small in-memory [`MockKeystore`] (see the `#[cfg(test)]`
//! block) so they pass on every OS without touching a real keychain -
//! crucial for headless CI Linux runners where there is no Secret
//! Service daemon.
//!
//! The service name shared by every entry is [`SERVICE`].  The account
//! key is always the user's email address; if a user works with several
//! accounts they get one keychain entry per account, with no
//! cross-contamination.
//!
//! This module is the **one** exception to AGENTS.md rule #8: credentials
//! may live in an OS keychain (which is real at-rest encryption, gated
//! by the OS login/keychain prompt), but NEVER in our own config file.

use anyhow::Result;

/// Service identifier used for every entry this crate creates.  Anything
/// stored under this name in the OS keychain came from `smtp-test-tool`.
pub const SERVICE: &str = "smtp-test-tool";

/// Abstract OS-native secret store.  Implementations MUST be safe to
/// share across threads (the GUI calls them from both the UI thread and
/// background test threads).
pub trait Keystore: Send + Sync {
    /// Persist `secret` under (SERVICE, `user`).  Overwrites any existing
    /// value.  Returns an error only on real backend failure (e.g. no
    /// Secret Service daemon on Linux).
    fn save(&self, user: &str, secret: &str) -> Result<()>;

    /// Look up the secret for (SERVICE, `user`).  `Ok(None)` means
    /// "looked, none stored" - which is the expected case on first use
    /// and must NOT be reported as an error.
    fn load(&self, user: &str) -> Result<Option<String>>;

    /// Delete the entry for (SERVICE, `user`).  Idempotent: deleting an
    /// entry that does not exist returns `Ok(())`.
    fn forget(&self, user: &str) -> Result<()>;
}

// ============================================================================
// Real OS-backed implementation
// ============================================================================
#[cfg(feature = "keychain")]
mod os_impl {
    use super::{Keystore, SERVICE};
    use anyhow::{Context, Result};

    /// Routes calls through the `keyring` crate to the OS-native store.
    #[derive(Default, Debug, Clone, Copy)]
    pub struct OsKeystore;

    impl Keystore for OsKeystore {
        fn save(&self, user: &str, secret: &str) -> Result<()> {
            let entry =
                keyring::Entry::new(SERVICE, user).context("opening OS keychain entry for save")?;
            entry
                .set_password(secret)
                .context("writing secret to OS keychain")
        }

        fn load(&self, user: &str) -> Result<Option<String>> {
            let entry =
                keyring::Entry::new(SERVICE, user).context("opening OS keychain entry for load")?;
            match entry.get_password() {
                Ok(s) => Ok(Some(s)),
                // 'NoEntry' is the documented "not found" signal; treat
                // it as a None return rather than an error so callers
                // can do the natural `if let Some(p) = ks.load(u)?`.
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(e).context("reading secret from OS keychain"),
            }
        }

        fn forget(&self, user: &str) -> Result<()> {
            let entry = keyring::Entry::new(SERVICE, user)
                .context("opening OS keychain entry for forget")?;
            match entry.delete_credential() {
                Ok(()) => Ok(()),
                Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(e).context("deleting OS keychain entry"),
            }
        }
    }
}

#[cfg(feature = "keychain")]
pub use os_impl::OsKeystore;

// ============================================================================
// Feature-off no-op implementation
// ============================================================================
/// Returned by [`default_keystore`] when the crate was built without the
/// `keychain` feature.  Lets the rest of the codebase call `load()`
/// without `#[cfg]` gates - it just always says "nothing stored".
#[derive(Default, Debug, Clone, Copy)]
pub struct NullKeystore;

impl Keystore for NullKeystore {
    fn save(&self, _user: &str, _secret: &str) -> Result<()> {
        anyhow::bail!("keychain support is not compiled in - rebuild with `--features keychain`")
    }
    fn load(&self, _user: &str) -> Result<Option<String>> {
        Ok(None)
    }
    fn forget(&self, _user: &str) -> Result<()> {
        Ok(())
    }
}

// ============================================================================
// Factory
// ============================================================================
/// Return the keystore appropriate for this build.
pub fn default_keystore() -> Box<dyn Keystore> {
    #[cfg(feature = "keychain")]
    {
        Box::new(OsKeystore)
    }
    #[cfg(not(feature = "keychain"))]
    {
        Box::new(NullKeystore)
    }
}

// ============================================================================
// Tests
// ============================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// In-memory mock used by unit tests.  We deliberately do NOT rely
    /// on the real OS keychain in tests because Linux CI runners do not
    /// have a Secret Service daemon and we want the suite to pass on
    /// every platform.
    #[derive(Default)]
    struct MockKeystore {
        map: Mutex<HashMap<String, String>>,
    }

    impl Keystore for MockKeystore {
        fn save(&self, user: &str, secret: &str) -> Result<()> {
            self.map
                .lock()
                .expect("mock keystore mutex poisoned")
                .insert(user.into(), secret.into());
            Ok(())
        }
        fn load(&self, user: &str) -> Result<Option<String>> {
            Ok(self
                .map
                .lock()
                .expect("mock keystore mutex poisoned")
                .get(user)
                .cloned())
        }
        fn forget(&self, user: &str) -> Result<()> {
            self.map
                .lock()
                .expect("mock keystore mutex poisoned")
                .remove(user);
            Ok(())
        }
    }

    #[test]
    fn mock_load_returns_none_for_missing_entry() {
        let ks = MockKeystore::default();
        assert_eq!(ks.load("alice@example.com").unwrap(), None);
    }

    #[test]
    fn mock_save_then_load_round_trips() {
        let ks = MockKeystore::default();
        ks.save("alice@example.com", "s3cret").unwrap();
        assert_eq!(
            ks.load("alice@example.com").unwrap().as_deref(),
            Some("s3cret")
        );
    }

    #[test]
    fn mock_forget_is_idempotent() {
        let ks = MockKeystore::default();
        ks.forget("never-was-here").unwrap();
        ks.save("user@example.com", "x").unwrap();
        ks.forget("user@example.com").unwrap();
        ks.forget("user@example.com").unwrap();
        assert_eq!(ks.load("user@example.com").unwrap(), None);
    }

    #[test]
    fn mock_overwrites_existing_secret() {
        let ks = MockKeystore::default();
        ks.save("u", "a").unwrap();
        ks.save("u", "b").unwrap();
        assert_eq!(ks.load("u").unwrap().as_deref(), Some("b"));
    }

    #[test]
    fn null_keystore_load_is_none() {
        let ks = NullKeystore;
        assert_eq!(ks.load("any").unwrap(), None);
    }

    #[test]
    fn null_keystore_save_errors_clearly() {
        let ks = NullKeystore;
        let err = ks.save("u", "p").unwrap_err();
        let s = err.to_string();
        assert!(
            s.contains("--features keychain"),
            "error must hint at the cargo feature, got: {s}"
        );
    }
}
