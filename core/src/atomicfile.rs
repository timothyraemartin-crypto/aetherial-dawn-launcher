//! Whole-file writes that can't leave a half-written file behind.

use std::path::Path;

/// Writes `bytes` to a sibling temp file, then renames it over `path`, so a
/// crash or power cut leaves either the old file or the new one.
pub fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn replaces_the_old_file_and_leaves_no_temp() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.json");
        super::write(&p, b"old").unwrap();
        super::write(&p, b"new").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"new");
        assert!(!d.path().join("config.json.tmp").exists());
    }

    #[test]
    fn a_failed_write_keeps_the_old_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.json");
        super::write(&p, b"old").unwrap();
        // A directory in the temp file's place makes the write fail.
        std::fs::create_dir(d.path().join("config.json.tmp")).unwrap();
        assert!(super::write(&p, b"new").is_err());
        assert_eq!(std::fs::read(&p).unwrap(), b"old");
    }
}
