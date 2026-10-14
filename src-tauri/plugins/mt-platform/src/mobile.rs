use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tauri::{
    AppHandle, Runtime,
    plugin::{PluginApi, PluginHandle},
};

use crate::Result;

pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> Result<MtPlatform<R>> {
    let handle = api.register_android_plugin("com.plugin.mt_platform", "PlatformPlugin")?;
    Ok(MtPlatform(handle))
}

#[derive(Serialize)]
struct SystemBars {
    dark: bool,
}

#[derive(Deserialize)]
struct Empty {}

pub struct MtPlatform<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> MtPlatform<R> {
    /// Makes the status and navigation bar icons readable on the app's own
    /// theme: light icons when `dark`, dark icons otherwise.
    pub fn set_system_bars_dark(&self, dark: bool) -> Result<()> {
        let _: Empty = self
            .0
            .run_mobile_plugin("setSystemBars", SystemBars { dark })?;
        Ok(())
    }
}
