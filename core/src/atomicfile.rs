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

/// How many earlier copies `safe_write` keeps.
pub const BACKUPS: usize = 3;

fn backup_path(path: &Path, n: usize) -> std::path::PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(format!(".bak{n}"));
    std::path::PathBuf::from(p)
}

/// Like `write`, and first keeps the file as it was: `<name>.bak1` is the
/// copy just before this change, `.bak2` and `.bak3` the two before that.
/// An unchanged file isn't copied again (so a no-op save never pushes a good
/// copy out), and a failed backup never stops the save. Not for files that
/// hold a sign-in or session: those get `write`, which leaves no copies.
pub fn safe_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Ok(old) = std::fs::read(path) {
        if old != bytes && !old.is_empty() {
            for n in (1..BACKUPS).rev() {
                let _ = std::fs::rename(backup_path(path, n), backup_path(path, n + 1));
            }
            let _ = write(&backup_path(path, 1), &old);
        }
    }
    write(path, bytes)
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
    fn safe_write_keeps_the_last_three_copies_newest_first() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.json");
        for v in ["v1", "v2", "v3", "v4", "v5"] {
            super::safe_write(&p, v.as_bytes()).unwrap();
        }
        let read = |n: &str| std::fs::read_to_string(d.path().join(n)).unwrap();
        assert_eq!(read("config.json"), "v5");
        assert_eq!([read("config.json.bak1"), read("config.json.bak2"), read("config.json.bak3")], ["v4", "v3", "v2"]);
        assert!(!d.path().join("config.json.bak4").exists());
    }

    #[test]
    fn safe_write_does_not_copy_an_unchanged_or_empty_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.json");
        super::safe_write(&p, b"").unwrap();
        super::safe_write(&p, b"a").unwrap();
        assert!(!d.path().join("config.json.bak1").exists(), "nothing worth keeping yet");
        super::safe_write(&p, b"a").unwrap();
        assert!(!d.path().join("config.json.bak1").exists(), "same bytes are not a new copy");
        super::safe_write(&p, b"b").unwrap();
        assert_eq!(std::fs::read(d.path().join("config.json.bak1")).unwrap(), b"a");
    }

    #[test]
    fn safe_write_still_saves_when_a_backup_cannot_be_made() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.json");
        super::safe_write(&p, b"old").unwrap();
        // A directory where the first copy goes makes keeping it fail.
        std::fs::create_dir(d.path().join("config.json.bak1")).unwrap();
        super::safe_write(&p, b"new").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"new");
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
