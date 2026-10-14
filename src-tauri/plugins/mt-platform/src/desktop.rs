use std::marker::PhantomData;

use serde::de::DeserializeOwned;
use tauri::{AppHandle, Runtime, plugin::PluginApi};

use crate::Result;

pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> Result<MtPlatform<R>> {
    Ok(MtPlatform(PhantomData))
}

/// Desktop builds have no system bars to style; calls succeed and do nothing.
pub struct MtPlatform<R: Runtime>(PhantomData<fn() -> R>);

impl<R: Runtime> MtPlatform<R> {
    pub fn set_system_bars_dark(&self, _dark: bool) -> Result<()> {
        Ok(())
    }
}
