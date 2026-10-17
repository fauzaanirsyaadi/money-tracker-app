use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tauri::{
    AppHandle, Runtime,
    plugin::{PluginApi, PluginHandle},
};

use std::path::Path;

use zeroize::Zeroizing;

use crate::{
    BiometricAvailability, EnrollOutcome, Enrollment, Error, FileOutcome, Result, UnlockOutcome,
    hex,
};

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

#[derive(Serialize)]
struct UnlockRequest<'a> {
    title: &'a str,
    subtitle: &'a str,
    negative: &'a str,
    blob: &'a str,
}

#[derive(Serialize)]
struct SaveRequest<'a> {
    source: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
struct PickRequest<'a> {
    destination: &'a str,
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

// debug-safe: no Debug derive; the token and blob stay out of any log
#[derive(Deserialize)]
struct BiometricReply {
    outcome: String,
    token: Option<String>,
    blob: Option<String>,
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

    /// Creates a fresh biometric key, shows the system fingerprint/face prompt
    /// bound to it, and returns the token it released together with the sealed
    /// copy. The answer comes from native code straight to Rust; the WebView
    /// never supplies it, so it cannot be forged from script.
    pub async fn biometric_enroll(
        &self,
        title: &str,
        subtitle: &str,
        negative: &str,
    ) -> Result<EnrollOutcome> {
        let response: BiometricReply = self
            .0
            .run_mobile_plugin_async(
                "biometricEnroll",
                PromptText {
                    title,
                    subtitle,
                    negative,
                },
            )
            .await?;
        Ok(match response.outcome.as_str() {
            "success" => {
                let token = response.token.as_deref().and_then(hex::decode);
                let blob = response.blob.as_deref().and_then(hex::decode);
                match (token, blob) {
                    (Some(token), Some(blob)) => EnrollOutcome::Enrolled(Enrollment {
                        token: Zeroizing::new(token),
                        blob,
                    }),
                    _ => EnrollOutcome::Failed,
                }
            }
            "cancelled" => EnrollOutcome::Cancelled,
            _ => EnrollOutcome::Failed,
        })
    }

    /// Shows the prompt bound to the biometric key and returns the token that
    /// the key releases from `blob` after a real biometric.
    pub async fn biometric_unlock(
        &self,
        title: &str,
        subtitle: &str,
        negative: &str,
        blob: &[u8],
    ) -> Result<UnlockOutcome> {
        let blob = hex::encode(blob);
        let response: BiometricReply = self
            .0
            .run_mobile_plugin_async(
                "biometricUnlock",
                UnlockRequest {
                    title,
                    subtitle,
                    negative,
                    blob: &blob,
                },
            )
            .await?;
        Ok(match response.outcome.as_str() {
            "success" => response
                .token
                .as_deref()
                .and_then(hex::decode)
                .map_or(UnlockOutcome::Failed, |token| {
                    UnlockOutcome::Token(Zeroizing::new(token))
                }),
            "cancelled" => UnlockOutcome::Cancelled,
            "invalidated" => UnlockOutcome::Invalidated,
            _ => UnlockOutcome::Failed,
        })
    }

    /// Deletes the biometric key (and only that key) from the Keystore.
    pub async fn biometric_forget(&self) -> Result<()> {
        let response: Outcome = self
            .0
            .run_mobile_plugin_async("biometricForget", ())
            .await?;
        if response.outcome == "done" {
            Ok(())
        } else {
            Err(Error::Platform)
        }
    }

    /// Lets the user choose where to save `source` (Storage Access Framework)
    /// and copies it there. The native side only ever handles the file's
    /// bytes, which are already encrypted.
    pub async fn save_file(&self, source: &Path, name: &str) -> Result<FileOutcome> {
        let source = source.to_str().ok_or(Error::Platform)?;
        let response: Outcome = self
            .0
            .run_mobile_plugin_async("saveFile", SaveRequest { source, name })
            .await?;
        file_outcome(&response.outcome)
    }

    /// Lets the user choose a file and copies it to `destination` (an
    /// app-private path). Nothing is parsed on the native side.
    pub async fn pick_file(&self, destination: &Path) -> Result<FileOutcome> {
        let destination = destination.to_str().ok_or(Error::Platform)?;
        let response: Outcome = self
            .0
            .run_mobile_plugin_async("pickFile", PickRequest { destination })
            .await?;
        file_outcome(&response.outcome)
    }
}

fn file_outcome(outcome: &str) -> Result<FileOutcome> {
    match outcome {
        "done" => Ok(FileOutcome::Done),
        "cancelled" => Ok(FileOutcome::Cancelled),
        _ => Err(Error::Platform),
    }
}
