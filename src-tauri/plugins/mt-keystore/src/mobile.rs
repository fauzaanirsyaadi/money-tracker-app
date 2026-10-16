use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tauri::{
    AppHandle, Runtime,
    plugin::{PluginApi, PluginHandle},
};

use crate::{Error, Result, SecurityLevel, hex};

pub fn init<R: Runtime, C: DeserializeOwned>(
    _app: &AppHandle<R>,
    api: PluginApi<R, C>,
) -> Result<MtKeystore<R>> {
    let handle = api.register_android_plugin("com.plugin.mt_keystore", "KeystorePlugin")?;
    Ok(MtKeystore(handle))
}

#[derive(Serialize)]
struct DataRequest {
    data: String,
}

#[derive(Deserialize)]
struct Empty {}

#[derive(Deserialize)]
struct DataResponse {
    data: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LevelResponse {
    security_level: String,
}

pub struct MtKeystore<R: Runtime>(PluginHandle<R>);

impl<R: Runtime> MtKeystore<R> {
    /// Encrypts `plain` with the non-exportable Keystore key (AES-256-GCM).
    pub fn wrap(&self, plain: &[u8]) -> Result<Vec<u8>> {
        self.call("wrap", plain)
    }

    /// Reverses [`MtKeystore::wrap`]. Fails if the blob was altered or the key is gone.
    pub fn unwrap(&self, blob: &[u8]) -> Result<Vec<u8>> {
        self.call("unwrap", blob)
    }

    /// Deletes the wrapping key. Only for a restore that replaces data the old
    /// key can no longer open, after the user confirmed.
    pub fn reset(&self) -> Result<()> {
        let _: Empty = self.0.run_mobile_plugin("reset", ())?;
        Ok(())
    }

    pub fn security_level(&self) -> Result<SecurityLevel> {
        let response: LevelResponse = self.0.run_mobile_plugin("securityLevel", ())?;
        Ok(match response.security_level.as_str() {
            "strongbox" => SecurityLevel::StrongBox,
            "tee" => SecurityLevel::TrustedEnvironment,
            "software" => SecurityLevel::Software,
            _ => SecurityLevel::Unknown,
        })
    }

    fn call(&self, command: &str, bytes: &[u8]) -> Result<Vec<u8>> {
        let request = DataRequest {
            data: hex::encode(bytes),
        };
        let response: DataResponse = self.0.run_mobile_plugin(command, request)?;
        hex::decode(&response.data).ok_or(Error::BadResponse)
    }
}
