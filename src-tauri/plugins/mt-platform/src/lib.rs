//! Native Android window behaviour that the WebView cannot do itself.
//!
//! The plugin exposes a Rust API only (see [`MtPlatformExt`]); it registers no
//! commands, so the WebView reaches it only through commands the app defines.

use std::fmt;

use tauri::{
    Manager, Runtime,
    plugin::{Builder, TauriPlugin},
};

#[cfg(desktop)]
mod desktop;
mod error;
#[cfg(mobile)]
mod mobile;

use zeroize::Zeroizing;

pub use error::{Error, Result};

pub mod hex;

#[cfg(desktop)]
use desktop::MtPlatform;
#[cfg(mobile)]
use mobile::MtPlatform;

/// Whether the device can show a strong biometric prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BiometricAvailability {
    Available,
    /// Hardware exists but no fingerprint or face is enrolled.
    NoneEnrolled,
    Unavailable,
}

/// What a fingerprint/face enrolment produced: a random token, known to the
/// native side and to Rust only, and that token sealed under the biometric key.
/// Rust keeps the blob and a hash of the token, never the token itself.
// debug-safe: carries a secret token, so Debug is written by hand and prints neither field
pub struct Enrollment {
    pub token: Zeroizing<Vec<u8>>,
    pub blob: Vec<u8>,
}

impl fmt::Debug for Enrollment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Enrollment(redacted)")
    }
}

// debug-safe: Enrolled carries an Enrollment, whose Debug is redacted
#[derive(Debug)]
pub enum EnrollOutcome {
    Enrolled(Enrollment),
    /// The user dismissed the prompt.
    Cancelled,
    /// Too many tries, hardware error, or a key that could not be made.
    Failed,
}

// debug-safe: Token holds a Zeroizing buffer, printed as redacted by hand below
pub enum UnlockOutcome {
    /// The key opened after a real biometric: this is the token it released.
    Token(Zeroizing<Vec<u8>>),
    /// The user dismissed the prompt or chose the PIN instead.
    Cancelled,
    /// Too many tries, hardware error, or any failure not known to be permanent.
    Failed,
    /// The biometric key is permanently unusable (a fingerprint or face was
    /// enrolled, or the key is gone). This is about the biometric key only and
    /// is never a storage problem.
    Invalidated,
}

impl fmt::Debug for UnlockOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Token(_) => "UnlockOutcome::Token(redacted)",
            Self::Cancelled => "UnlockOutcome::Cancelled",
            Self::Failed => "UnlockOutcome::Failed",
            Self::Invalidated => "UnlockOutcome::Invalidated",
        })
    }
}

/// Result of a system file picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOutcome {
    /// The file was written to, or read from, the chosen location.
    Done,
    /// The user dismissed the picker.
    Cancelled,
}

pub trait MtPlatformExt<R: Runtime> {
    fn mt_platform(&self) -> &MtPlatform<R>;
}

impl<R: Runtime, T: Manager<R>> MtPlatformExt<R> for T {
    fn mt_platform(&self) -> &MtPlatform<R> {
        self.state::<MtPlatform<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("mt-platform")
        .setup(|app, api| {
            #[cfg(mobile)]
            let platform = mobile::init(app, api)?;
            #[cfg(desktop)]
            let platform = desktop::init(app, api)?;
            app.manage(platform);
            Ok(())
        })
        .build()
}
