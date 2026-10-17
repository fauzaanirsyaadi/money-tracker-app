use std::marker::PhantomData;

use serde::de::DeserializeOwned;
use tauri::{AppHandle, Runtime, plugin::PluginApi};

use std::path::Path;

use crate::{BiometricAvailability, EnrollOutcome, Error, FileOutcome, Result, UnlockOutcome};

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

    pub async fn biometric_enroll(
        &self,
        _title: &str,
        _subtitle: &str,
        _negative: &str,
    ) -> Result<EnrollOutcome> {
        Ok(EnrollOutcome::Failed)
    }

    pub async fn biometric_unlock(
        &self,
        _title: &str,
        _subtitle: &str,
        _negative: &str,
        _blob: &[u8],
    ) -> Result<UnlockOutcome> {
        Ok(UnlockOutcome::Failed)
    }

    pub async fn biometric_forget(&self) -> Result<()> {
        Ok(())
    }

    /// There is no Storage Access Framework on desktop builds.
    pub async fn save_file(&self, _source: &Path, _name: &str) -> Result<FileOutcome> {
        Err(Error::Unsupported)
    }

    pub async fn pick_file(&self, _destination: &Path) -> Result<FileOutcome> {
        Err(Error::Unsupported)
    }
}
