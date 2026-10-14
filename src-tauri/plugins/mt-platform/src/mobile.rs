use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tauri::{
    AppHandle, Runtime,
    plugin::{PluginApi, PluginHandle},
};

use crate::{BiometricAvailability, BiometricOutcome, Result};

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

#[derive(Serialize)]
struct PromptText<'a> {
    title: &'a str,
    subtitle: &'a str,
    negative: &'a str,
}

#[derive(Deserialize)]
struct Empty {}

#[derive(Deserialize)]
struct Status {
    status: String,
}

#[derive(Deserialize)]
struct Outcome {
    outcome: String,
}

pub struct MtPlatform<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> MtPlatform<R> {
    /// Makes the status and navigation bar icons readable on the app's own
    /// theme: light icons when `dark`, dark icons otherwise.
    pub async fn set_system_bars_dark(&self, dark: bool) -> Result<()> {
        let _: Empty = self
            .0
            .run_mobile_plugin_async("setSystemBars", SystemBars { dark })
            .await?;
        Ok(())
    }

    pub async fn biometric_availability(&self) -> Result<BiometricAvailability> {
        let response: Status = self
            .0
            .run_mobile_plugin_async("biometricStatus", ())
            .await?;
        Ok(match response.status.as_str() {
            "available" => BiometricAvailability::Available,
            "none_enrolled" => BiometricAvailability::NoneEnrolled,
            _ => BiometricAvailability::Unavailable,
        })
    }

    /// Shows the system fingerprint/face prompt and waits for the user.
    ///
    /// The answer comes from native code straight to Rust; the WebView never
    /// supplies it, so it cannot be forged from script.
    pub async fn authenticate(
        &self,
        title: &str,
        subtitle: &str,
        negative: &str,
    ) -> Result<BiometricOutcome> {
        let response: Outcome = self
            .0
            .run_mobile_plugin_async(
                "authenticate",
                PromptText {
                    title,
                    subtitle,
                    negative,
                },
            )
            .await?;
        Ok(match response.outcome.as_str() {
            "success" => BiometricOutcome::Success,
            "cancelled" => BiometricOutcome::Cancelled,
            _ => BiometricOutcome::Failed,
        })
    }
}
