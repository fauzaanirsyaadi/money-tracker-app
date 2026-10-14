use std::marker::PhantomData;

use serde::de::DeserializeOwned;
use tauri::{AppHandle, Runtime, plugin::PluginApi};

use crate::{BiometricAvailability, BiometricOutcome, Result};

pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    _api: PluginApi<R, C>,
) -> Result<MtPlatform<R>> {
    Ok(MtPlatform(PhantomData))
}

/// Desktop builds have no system bars and no biometric prompt: bar calls do
/// nothing and biometrics report unavailable, so the PIN is the only way in.
pub struct MtPlatform<R: Runtime>(PhantomData<fn() -> R>);

impl<R: Runtime> MtPlatform<R> {
    pub async fn set_system_bars_dark(&self, _dark: bool) -> Result<()> {
        Ok(())
    }

    pub async fn biometric_availability(&self) -> Result<BiometricAvailability> {
        Ok(BiometricAvailability::Unavailable)
    }

    pub async fn authenticate(
        &self,
        _title: &str,
        _subtitle: &str,
        _negative: &str,
    ) -> Result<BiometricOutcome> {
        Ok(BiometricOutcome::Failed)
    }
}
