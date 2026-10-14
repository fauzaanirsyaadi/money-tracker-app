use std::fmt;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    /// Not an Android build.
    Unsupported,
    /// The native side refused or failed.
    Platform,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unsupported => "platform call unsupported here",
            Self::Platform => "platform call failed",
        })
    }
}

impl std::error::Error for Error {}

#[cfg(mobile)]
impl From<tauri::plugin::mobile::PluginInvokeError> for Error {
    fn from(_: tauri::plugin::mobile::PluginInvokeError) -> Self {
        Self::Platform
    }
}
