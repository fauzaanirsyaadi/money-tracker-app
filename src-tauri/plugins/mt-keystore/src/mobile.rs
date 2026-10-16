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

#[derive(Serialize)]
struct Version {
    version: u32,
}

#[derive(Serialize)]
struct VersionedData {
    version: u32,
    data: String,
}

#[derive(Deserialize)]
struct VersionResponse {
    version: u32,
}

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

    /// The alias version a replacement key will get (one above every existing alias).
    pub fn next_alias_version(&self) -> Result<u8> {
        let response: VersionResponse = self.0.run_mobile_plugin("nextAliasVersion", ())?;
        u8::try_from(response.version).map_err(|_| Error::BadResponse)
    }

    /// Creates the alias `version` and wraps `plain` with it. Every older alias
    /// stays as it is, so data wrapped with it remains readable.
    pub fn wrap_new_alias(&self, version: u8, plain: &[u8]) -> Result<Vec<u8>> {
        let request = VersionedData {
            version: u32::from(version),
            data: hex::encode(plain),
        };
        let response: DataResponse = self.0.run_mobile_plugin("wrapNewAlias", request)?;
        hex::decode(&response.data).ok_or(Error::BadResponse)
    }

    /// Deletes one alias. For an unused new alias, or an old one after the new
    /// data was verified.
    pub fn delete_alias(&self, version: u8) -> Result<()> {
        let _: Empty = self.0.run_mobile_plugin(
            "deleteAlias",
            Version {
                version: u32::from(version),
            },
        )?;
        Ok(())
    }

    /// Deletes every alias older than `keep`.
    pub fn prune_aliases_below(&self, keep: u8) -> Result<()> {
        let _: Empty = self.0.run_mobile_plugin(
            "pruneBelow",
            Version {
                version: u32::from(keep),
            },
        )?;
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
