//! Android Keystore access for the database key.
//!
//! The plugin exposes a Rust API only (see [`MtKeystoreExt`]); it registers no
//! commands, so the WebView can never request key operations.

use tauri::{
    Manager, Runtime,
    plugin::{Builder, TauriPlugin},
};

#[cfg(desktop)]
mod desktop;
mod error;
mod hex;
#[cfg(mobile)]
mod mobile;

pub use error::{Error, Result};

#[cfg(desktop)]
use desktop::MtKeystore;
#[cfg(mobile)]
use mobile::MtKeystore;

pub use hex::{decode as hex_decode, encode as hex_encode};

/// Where the Keystore key lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityLevel {
    StrongBox,
    TrustedEnvironment,
    Software,
    Unknown,
}

pub trait MtKeystoreExt<R: Runtime> {
    fn mt_keystore(&self) -> &MtKeystore<R>;
}

impl<R: Runtime, T: Manager<R>> MtKeystoreExt<R> for T {
    fn mt_keystore(&self) -> &MtKeystore<R> {
        self.state::<MtKeystore<R>>().inner()
    }
}

pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("mt-keystore")
        .setup(|app, api| {
            #[cfg(mobile)]
            let keystore = mobile::init(app, api)?;
            #[cfg(desktop)]
            let keystore = desktop::init(app, api)?;
            app.manage(keystore);
            Ok(())
        })
        .build()
}
