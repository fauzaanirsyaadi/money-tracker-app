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

    pub fn next_alias_version(&self) -> Result<u8> {
        Err(Error::Unsupported)
    }

    pub fn wrap_new_alias(&self, _version: u8, _plain: &[u8]) -> Result<Vec<u8>> {
        Err(Error::Unsupported)
    }

    pub fn delete_alias(&self, _version: u8) -> Result<()> {
        Err(Error::Unsupported)
    }

    pub fn prune_aliases_below(&self, _keep: u8) -> Result<()> {
        Err(Error::Unsupported)
    }

    pub fn security_level(&self) -> Result<SecurityLevel> {
        Err(Error::Unsupported)
    }
}
