//! Native Android window behaviour that the WebView cannot do itself.
//!
//! The plugin exposes a Rust API only (see [`MtPlatformExt`]); it registers no
//! commands, so the WebView reaches it only through commands the app defines.

use tauri::{
    Manager, Runtime,
    plugin::{Builder, TauriPlugin},
};

#[cfg(desktop)]
mod desktop;
mod error;
#[cfg(mobile)]
mod mobile;

pub use error::{Error, Result};

#[cfg(desktop)]
use desktop::MtPlatform;
#[cfg(mobile)]
use mobile::MtPlatform;

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
