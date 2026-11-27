//! Per-build Windows OS identity derived from the app's bundle `identifier`
//! and `productName`.
//!
//! Production (`io.github.hiyabye.rhythm` / `Rhythm`) keeps its historical
//! AUMID, `rhythm-win:` protocol scheme, `Rhythm.lnk` shortcut and `Run` value
//! name. Any non-production build (such as `.validation`) derives isolated
//! names from its own identifier and product name so it cannot read, hijack or
//! delete a production installation in the same Windows user account (#70).
//!
//! Keep the derivation rules in sync with `scripts/installer-hooks.nsh` and
//! `scripts/verify-windows-install.ps1`.

use std::sync::OnceLock;

pub const PRODUCTION_IDENTIFIER: &str = "io.github.hiyabye.rhythm";
pub const PRODUCTION_PRODUCT_NAME: &str = "Rhythm";
pub const PRODUCTION_PROTOCOL_SCHEME: &str = "rhythm-win";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowsIdentity {
    pub aumid: String,
    pub shortcut_filename: String,
    pub notification_prefix: String,
    pub studium_prefix: String,
    pub reminder_prefix: String,
    pub protocol_reg_key: String,
    pub protocol_command_reg_key: String,
}

static CURRENT: OnceLock<WindowsIdentity> = OnceLock::new();

/// Initialize the process-wide Windows identity from the app's resolved config.
/// Call once at the start of `main()` before registering the process AUMID or
/// handling single-instance protocol arguments.
#[cfg(windows)]
pub fn init(identifier: &str, product_name: Option<&str>) {
    let _ = CURRENT.set(WindowsIdentity::from_config(identifier, product_name));
}

/// Active build's Windows identity. Falls back to production defaults in unit
/// tests that do not run `main()`.
pub fn current() -> &'static WindowsIdentity {
    CURRENT.get_or_init(|| {
        WindowsIdentity::from_config(PRODUCTION_IDENTIFIER, Some(PRODUCTION_PRODUCT_NAME))
    })
}

fn sanitize_scheme_suffix(identifier: &str) -> String {
    let cleaned: String = identifier
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '.') {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    if cleaned.trim_matches(['-', '.']).is_empty() {
        "test-build".into()
    } else {
        cleaned
    }
}

/// Protocol scheme for the build with `identifier`.
///
/// Production owns `rhythm-win`. Every other identifier gets
/// `rhythm-win.<identifier>` (RFC 3986 allows ASCII letters, digits, `.` and
/// `-` in URI schemes), matching `RHYTHM_PROTOCOL_SCHEME` in
/// `scripts/installer-hooks.nsh`.
pub fn protocol_scheme(identifier: &str) -> String {
    if identifier == PRODUCTION_IDENTIFIER {
        return PRODUCTION_PROTOCOL_SCHEME.into();
    }
    let suffix = sanitize_scheme_suffix(identifier);
    format!("{PRODUCTION_PROTOCOL_SCHEME}.{suffix}")
}

impl WindowsIdentity {
    pub fn from_config(identifier: &str, product_name: Option<&str>) -> Self {
        let product_name = product_name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(PRODUCTION_PRODUCT_NAME);
        let aumid = identifier.to_string();
        let shortcut_filename = format!("{product_name}.lnk");
        let scheme = protocol_scheme(identifier);
        let notification_prefix = format!("{scheme}://notification/");
        let studium_prefix = format!("{scheme}://studium/");
        let reminder_prefix = format!("{scheme}://reminder/");
        let protocol_reg_key = format!("Software\\Classes\\{scheme}");
        let protocol_command_reg_key = format!("Software\\Classes\\{scheme}\\shell\\open\\command");
        Self {
            aumid,
            shortcut_filename,
            notification_prefix,
            studium_prefix,
            reminder_prefix,
            protocol_reg_key,
            protocol_command_reg_key,
        }
    }

    #[cfg(any(windows, test, feature = "test-support"))]
    pub fn notification_token_from_uri<'a>(&self, uri: &'a str) -> Option<&'a str> {
        let token = uri.strip_prefix(self.notification_prefix.as_str())?;
        crate::storage::notification_activation::valid_token(token).then_some(token)
    }

    pub fn studium_token_from_uri<'a>(&self, uri: &'a str) -> Option<&'a str> {
        let token = uri.strip_prefix(self.studium_prefix.as_str())?;
        crate::storage::notification_activation::valid_token(token).then_some(token)
    }

    pub fn reminder_token_from_uri<'a>(&self, uri: &'a str) -> Option<&'a str> {
        let token = uri.strip_prefix(self.reminder_prefix.as_str())?;
        crate::storage::notification_activation::valid_token(token).then_some(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_identity_preserves_legacy_registrations() {
        let id = WindowsIdentity::from_config(PRODUCTION_IDENTIFIER, Some("Rhythm"));
        assert_eq!(id.aumid, "io.github.hiyabye.rhythm");
        assert_eq!(id.shortcut_filename, "Rhythm.lnk");
        assert_eq!(protocol_scheme(PRODUCTION_IDENTIFIER), "rhythm-win");
        assert_eq!(id.notification_prefix, "rhythm-win://notification/");
        assert_eq!(id.studium_prefix, "rhythm-win://studium/");
        assert_eq!(id.reminder_prefix, "rhythm-win://reminder/");
        assert_eq!(id.protocol_reg_key, r"Software\Classes\rhythm-win");
        assert_eq!(
            id.protocol_command_reg_key,
            r"Software\Classes\rhythm-win\shell\open\command"
        );
    }

    #[test]
    fn validation_identity_isolates_every_windows_registration() {
        let prod = WindowsIdentity::from_config(PRODUCTION_IDENTIFIER, Some("Rhythm"));
        let val = WindowsIdentity::from_config(
            "io.github.hiyabye.rhythm.validation",
            Some("Rhythm Validation"),
        );

        assert_eq!(val.aumid, "io.github.hiyabye.rhythm.validation");
        assert_eq!(val.shortcut_filename, "Rhythm Validation.lnk");
        assert_eq!(
            protocol_scheme("io.github.hiyabye.rhythm.validation"),
            "rhythm-win.io.github.hiyabye.rhythm.validation"
        );
        assert_eq!(
            val.protocol_reg_key,
            r"Software\Classes\rhythm-win.io.github.hiyabye.rhythm.validation"
        );

        assert_ne!(val.aumid, prod.aumid);
        assert_ne!(val.shortcut_filename, prod.shortcut_filename);
        assert_ne!(val.protocol_reg_key, prod.protocol_reg_key);
        assert_ne!(val.protocol_command_reg_key, prod.protocol_command_reg_key);

        let token = "ab".repeat(32);
        let prod_notif = format!("{}{token}", prod.notification_prefix);
        let val_notif = format!("{}{token}", val.notification_prefix);
        assert_eq!(
            val.notification_token_from_uri(&val_notif),
            Some(token.as_str())
        );
        assert_eq!(val.notification_token_from_uri(&prod_notif), None);
        assert_eq!(prod.notification_token_from_uri(&val_notif), None);

        let val_studium = format!("{}{token}", val.studium_prefix);
        assert_eq!(
            val.studium_token_from_uri(&val_studium),
            Some(token.as_str())
        );
        assert_eq!(prod.studium_token_from_uri(&val_studium), None);

        let val_reminder = format!("{}{token}", val.reminder_prefix);
        assert_eq!(
            val.reminder_token_from_uri(&val_reminder),
            Some(token.as_str())
        );
        assert_eq!(prod.reminder_token_from_uri(&val_reminder), None);
    }

    #[test]
    fn unusual_identifiers_still_produce_valid_isolated_schemes() {
        for identifier in ["", "   ", "///", "Com.Example.Rhythm Test/빌드"] {
            let scheme = protocol_scheme(identifier);
            assert_ne!(scheme, PRODUCTION_PROTOCOL_SCHEME);
            assert!(scheme.starts_with("rhythm-win."));
            assert!(scheme.chars().all(|ch| ch.is_ascii_lowercase()
                || ch.is_ascii_digit()
                || matches!(ch, '-' | '.')));
        }
    }
}
