//! When the data was last backed up, kept on the device only.
//!
//! The app has no server, so the only reminder it can give is local: a small
//! file next to the database records the moment an export last reached the
//! user's chosen location, and the moment the app first looked (`since`), so
//! "never backed up" can still say for how long. It holds two timestamps and
//! nothing about the data. No clock is trusted beyond a reminder: a stamp in the
//! future simply counts as "today".

use std::{fs, io, io::Write, path::Path};

use serde::{Deserialize, Serialize};

pub(crate) const META_FILE: &str = "backup_meta.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupMeta {
    /// Epoch milliseconds of the first time this file was made.
    pub since_ms: u64,
    /// Epoch milliseconds of the last export the user saved, if any.
    pub last_ms: Option<u64>,
}

fn write(dir: &Path, meta: &BackupMeta) -> io::Result<()> {
    let bytes = serde_json::to_vec(meta).map_err(io::Error::other)?;
    let tmp = dir.join(format!("{META_FILE}.tmp"));
    let mut file = fs::File::create(&tmp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, dir.join(META_FILE))
}

/// Reads the file, creating it (with `since` = now) when it is missing or cannot
/// be read. A damaged file costs only the reminder's history, never data.
pub fn load_or_init(dir: &Path, now_ms: u64) -> BackupMeta {
    if let Ok(bytes) = fs::read(dir.join(META_FILE))
        && let Ok(meta) = serde_json::from_slice::<BackupMeta>(&bytes)
    {
        return meta;
    }
    let meta = BackupMeta {
        since_ms: now_ms,
        last_ms: None,
    };
    // Best effort: without a file the reminder just restarts from the next look.
    let _ = write(dir, &meta);
    meta
}

/// Records a saved export. Best effort for the caller: the backup itself already succeeded.
pub fn record_backup(dir: &Path, now_ms: u64) -> io::Result<()> {
    let mut meta = load_or_init(dir, now_ms);
    meta.last_ms = Some(now_ms);
    write(dir, &meta)
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mt-meta-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_first_look_creates_the_file_and_keeps_its_since() {
        let dir = dir("init");
        let first = load_or_init(&dir, 1_000);
        assert_eq!(
            first,
            BackupMeta {
                since_ms: 1_000,
                last_ms: None
            }
        );
        // A later look does not move `since`.
        assert_eq!(load_or_init(&dir, 9_000), first);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_saved_export_is_remembered_across_looks() {
        let dir = dir("record");
        load_or_init(&dir, 1_000);
        record_backup(&dir, 5_000).unwrap();
        assert_eq!(
            load_or_init(&dir, 9_000),
            BackupMeta {
                since_ms: 1_000,
                last_ms: Some(5_000)
            }
        );
        record_backup(&dir, 7_000).unwrap();
        assert_eq!(load_or_init(&dir, 9_000).last_ms, Some(7_000));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn recording_without_a_file_makes_one() {
        let dir = dir("record-first");
        record_backup(&dir, 4_000).unwrap();
        assert_eq!(
            load_or_init(&dir, 9_000),
            BackupMeta {
                since_ms: 4_000,
                last_ms: Some(4_000)
            }
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_damaged_file_restarts_the_reminder_and_nothing_else() {
        let dir = dir("damaged");
        fs::write(dir.join(META_FILE), b"{not json").unwrap();
        assert_eq!(
            load_or_init(&dir, 3_000),
            BackupMeta {
                since_ms: 3_000,
                last_ms: None
            }
        );
        // It was rewritten, so the next look is stable.
        assert_eq!(load_or_init(&dir, 8_000).since_ms, 3_000);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_file_holds_two_numbers_and_nothing_about_the_data() {
        let dir = dir("content");
        record_backup(&dir, 12_345).unwrap();
        let text = fs::read_to_string(dir.join(META_FILE)).unwrap();
        assert_eq!(text, r#"{"since_ms":12345,"last_ms":12345}"#);
        let _ = fs::remove_dir_all(dir);
    }
}
