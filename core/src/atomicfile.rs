//! Whole-file writes that can't leave a half-written file behind.

use std::path::Path;
use std::io::Write;

/// Writes and syncs `bytes` to an exclusively created sibling temp file, then
/// renames it over `path`. Each writer owns its temp file. This syncs file data;
/// directory-entry durability after power loss still depends on the filesystem.
pub fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    write_with_sync(path, bytes, std::fs::File::sync_all)
}

fn write_with_sync(path: &Path, bytes: &[u8], sync: impl FnOnce(&std::fs::File) -> std::io::Result<()>) -> std::io::Result<()> {
    if path.file_name().is_none() {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "A file name is required"));
    }
    let mut owned = None;
    for _ in 0..16 {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(format!(".tmp.{:016x}", rand::random::<u64>()));
        let tmp = std::path::PathBuf::from(tmp);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(file) => { owned = Some((tmp, file)); break; }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    let (tmp, mut file) = owned.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::AlreadyExists, "Could not reserve a temporary file"))?;
    let result = (|| {
        file.write_all(bytes)?;
        sync(&file)?;
        drop(file);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
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
    fn a_preexisting_temp_hard_link_cannot_overwrite_another_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.json");
        let victim = d.path().join("unrelated.txt");
        std::fs::write(&p, b"old").unwrap();
        std::fs::write(&victim, b"keep").unwrap();
        std::fs::hard_link(&victim, d.path().join("config.json.tmp")).unwrap();
        super::write(&p, b"new").unwrap();
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
        assert_eq!(std::fs::read(&p).unwrap(), b"new");
    }

    #[test]
    fn concurrent_writers_never_share_a_temporary_file() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.json");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(4));
        let mut workers = Vec::new();
        for byte in 1..=4 {
            let p = p.clone();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                let payload = vec![byte; 32 * 1024];
                barrier.wait();
                for _ in 0..20 { super::write(&p, &payload).unwrap(); }
            }));
        }
        for worker in workers { worker.join().unwrap(); }
        let saved = std::fs::read(&p).unwrap();
        assert_eq!(saved.len(), 32 * 1024);
        assert!(saved.iter().all(|b| *b == saved[0]));
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    }

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
        // A sync failure happens after writing the temp data, before replacing the old file.
        assert!(super::write_with_sync(&p, b"new", |file| {
            assert_eq!(file.metadata()?.len(), 3);
            assert_eq!(std::fs::read(&p)?, b"old");
            Err(std::io::Error::other("sync failed"))
        }).is_err());
        assert_eq!(std::fs::read(&p).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1, "failed temp is cleaned up");
    }
}
