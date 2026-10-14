use std::marker::PhantomData;

use serde::de::DeserializeOwned;
use tauri::{AppHandle, Runtime, plugin::PluginApi};

use crate::{Error, Result, SecurityLevel};

pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> Result<MtKeystore<R>> {
    Ok(MtKeystore(PhantomData))
}

/// Desktop builds have no Keystore; every operation reports `Unsupported`.
pub struct MtKeystore<R: Runtime>(PhantomData<fn() -> R>);

impl<R: Runtime> MtKeystore<R> {
    pub fn wrap(&self, _plain: &[u8]) -> Result<Vec<u8>> {
        Err(Error::Unsupported)
    }

    pub fn unwrap(&self, _blob: &[u8]) -> Result<Vec<u8>> {
        Err(Error::Unsupported)
    }

    pub fn security_level(&self) -> Result<SecurityLevel> {
        Err(Error::Unsupported)
    }
}
