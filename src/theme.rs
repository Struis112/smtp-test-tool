//! Cross-platform OS appearance (dark/light) detection.
//!
//! We hand-roll this so we don't depend on `dark-light`, which in v2.x
//! pulls in the now-unmaintained `async-std` (RUSTSEC-2025-0052).
//!
//! Per AGENTS.md §4: dark + light mode follow MUST work on Windows,
//! macOS, and Linux without third-party crates beyond the standard
//! ecosystem.  The Python reference implementation (the previous
//! generation of this tool) demonstrated that the algorithm fits in
//! about thirty lines per platform; this is the Rust translation.
//!
//! Precedence:
//!
//! 1. `NO_COLOR`             -> [`Appearance::Unknown`] (let caller decide).
//! 2. `COLORFGBG`            -> parsed foreground;background colours.
//! 3. Per-OS native probe    -> Windows registry / macOS `defaults` /
//!    GNOME gsettings / KDE `kdeglobals`.
//! 4. Fallback               -> [`Appearance::Unknown`].

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Dark,
    Light,
    /// OS did not advertise a preference, or detection failed.
    Unknown,
}

/// Best-effort current OS appearance.
///
/// Never panics, never blocks for more than a short subprocess call on
/// macOS / Linux, never touches the filesystem outside the OS-provided
/// settings store.
pub fn detect() -> Appearance {
    if std::env::var_os("NO_COLOR").is_some() {
        return Appearance::Unknown;
    }

    if let Some(a) = from_colorfgbg() {
        return a;
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(a) = windows::detect() {
            return a;
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(a) = macos::detect() {
            return a;
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(a) = linux::detect() {
            return a;
        }
    }

    Appearance::Unknown
}

/// `COLORFGBG` is set by xterm / Konsole / rxvt / iTerm2 in the form
/// `<fg>;<bg>` or `<fg>;<extra>;<bg>` (8 or 16 ANSI colours).  ANSI
/// background codes 0..6 and 8 are dark; 7 and 9..15 are light.
fn from_colorfgbg() -> Option<Appearance> {
    let raw = std::env::var("COLORFGBG").ok()?;
    let bg: u8 = raw.rsplit(';').next()?.trim().parse().ok()?;
    Some(if matches!(bg, 0..=6 | 8) {
        Appearance::Dark
    } else {
        Appearance::Light
    })
}

// =====================================================================
// Windows
// =====================================================================
#[cfg(target_os = "windows")]
mod windows {
    use super::Appearance;
    use std::ffi::{c_void, OsStr};
    use std::os::windows::ffi::OsStrExt;

    // Minimal hand-rolled binding to RegGetValueW to avoid the
    // `winreg` crate (extra dependency).  We only need to read one
    // DWORD from HKEY_CURRENT_USER, so the surface is tiny.
    // The Win32 type is HKEY (handle).  Lower-cased here so clippy's
    // upper-case-acronym lint stays quiet without an allow attribute.
    type Hkey = *mut c_void;
    const HKEY_CURRENT_USER: Hkey = 0x8000_0001 as Hkey;
    const RRF_RT_REG_DWORD: u32 = 0x0000_0010;
    const ERROR_SUCCESS: i32 = 0;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegGetValueW(
            hkey: Hkey,
            lp_subkey: *const u16,
            lp_value: *const u16,
            dw_flags: u32,
            pdw_type: *mut u32,
            pv_data: *mut c_void,
            pcb_data: *mut u32,
        ) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    pub fn detect() -> Option<Appearance> {
        // The user-mode "Apps" theme - the one humans actually toggle
        // via Settings > Personalisation > Colours > "Choose your mode".
        let subkey = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
        let value = wide("AppsUseLightTheme");
        let mut data: u32 = 0;
        let mut data_len: u32 = std::mem::size_of::<u32>() as u32;
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                &mut data as *mut u32 as *mut c_void,
                &mut data_len,
            )
        };
        if rc != ERROR_SUCCESS {
            return None;
        }
        Some(if data == 1 {
            Appearance::Light
        } else {
            Appearance::Dark
        })
    }
}

// =====================================================================
// macOS
// =====================================================================
#[cfg(target_os = "macos")]
mod macos {
    use super::Appearance;
    use std::process::Command;

    pub fn detect() -> Option<Appearance> {
        // `defaults read -g AppleInterfaceStyle` returns "Dark" iff the
        // key exists; in Light mode the key is absent and the command
        // exits non-zero.  We map both signals into Appearance.
        let out = Command::new("defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .output()
            .ok()?;
        if out.status.success() && String::from_utf8_lossy(&out.stdout).contains("Dark") {
            Some(Appearance::Dark)
        } else {
            // Missing key = Light, per Apple's documented behaviour.
            Some(Appearance::Light)
        }
    }
}

// =====================================================================
// Linux / other Unix - GNOME, KDE, and freedesktop conventions
// =====================================================================
#[cfg(all(unix, not(target_os = "macos")))]
mod linux {
    use super::Appearance;
    use std::process::Command;

    pub fn detect() -> Option<Appearance> {
        // 1. GNOME (and forks that respect this key).
        if let Some(a) = gsettings("org.gnome.desktop.interface", "color-scheme") {
            return Some(a);
        }
        // 2. KDE Plasma writes ColorScheme=BreezeDark in kdeglobals; reading
        //    that requires the `dirs` crate which we already depend on.
        if let Some(a) = kde_kdeglobals() {
            return Some(a);
        }
        // 3. Fallback: not detected.
        None
    }

    fn gsettings(schema: &str, key: &str) -> Option<Appearance> {
        let out = Command::new("gsettings")
            .args(["get", schema, key])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).to_lowercase();
        if s.contains("dark") {
            Some(Appearance::Dark)
        } else if s.contains("light") || s.contains("default") {
            Some(Appearance::Light)
        } else {
            None
        }
    }

    fn kde_kdeglobals() -> Option<Appearance> {
        let home = dirs::config_dir()?;
        let path = home.join("kdeglobals");
        let text = std::fs::read_to_string(path).ok()?;
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("ColorScheme=") {
                let v = v.trim().to_lowercase();
                return Some(if v.contains("dark") {
                    Appearance::Dark
                } else {
                    Appearance::Light
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Cargo runs tests in parallel by default; environment variables are
    // process-global, so we serialise the three cases into one ordered test
    // rather than fight the test runner.
    #[test]
    fn colorfgbg_parsing() {
        // SAFETY: this is the only test in the crate that touches
        // COLORFGBG, and it runs all three cases in sequence.
        unsafe {
            std::env::set_var("COLORFGBG", "15;0");
            assert_eq!(from_colorfgbg(), Some(Appearance::Dark), "bg=0 -> dark");

            std::env::set_var("COLORFGBG", "0;15");
            assert_eq!(from_colorfgbg(), Some(Appearance::Light), "bg=15 -> light");

            // ANSI bg code 8 is grey-on-black for many terminals - still dark.
            std::env::set_var("COLORFGBG", "7;8");
            assert_eq!(from_colorfgbg(), Some(Appearance::Dark), "bg=8 -> dark");

            // Malformed values return None and let the caller fall through.
            std::env::set_var("COLORFGBG", "nonsense");
            assert_eq!(from_colorfgbg(), None);

            std::env::remove_var("COLORFGBG");
            assert_eq!(from_colorfgbg(), None);
        }
    }
}
