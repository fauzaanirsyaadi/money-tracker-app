//! Making a file operation survive a power loss, not only a process crash.
//!
//! `File::sync_all` makes a file's *content* durable. A rename or a newly
//! created file lives in the **directory**, which must be synced too, or a power
//! cut can bring back the old name (or none) after the content was safely
//! written. Everything here is best effort: a filesystem that refuses to sync a
//! directory must not turn a successful operation into a failure.

use std::{fs, io, io::Write, path::Path};

/// Syncs the directory entry table of `dir`.
pub fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(handle) = fs::File::open(dir) {
        let _ = handle.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Writes `bytes` so that, after a crash or a power loss, `path` holds either
/// the old content or all of the new, never a truncated file: a temporary file
/// beside it is written and synced, renamed over `path`, and the directory is
/// synced. A leftover `*.tmp` from a crash is overwritten by the next write.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let mut file = fs::File::create(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path)?;
    if let Some(dir) = path.parent() {
        sync_dir(dir);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mt-durable-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_atomic_creates_and_replaces_without_leaving_a_temporary() {
        let dir = dir("atomic");
        let path = dir.join("db.key");
        write_atomic(&path, b"one").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"one");
        write_atomic(&path, b"two, longer").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"two, longer");
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["db.key"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_stale_temporary_from_a_crash_is_simply_overwritten() {
        let dir = dir("stale");
        let path = dir.join("lock.json");
        write_atomic(&path, b"good").unwrap();
        // A crash after the temporary was written but before the rename.
        fs::write(dir.join("lock.json.tmp"), b"half-written garb").unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            b"good",
            "readers never look at the temporary"
        );
        write_atomic(&path, b"next").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"next");
        assert!(!dir.join("lock.json.tmp").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn syncing_a_directory_that_is_missing_is_harmless() {
        sync_dir(Path::new("/definitely/not/here"));
    }
}
