//! Brings the SkyMP client files in the Skyrim folder in line with the manifest.
//! Only files whose size or SHA-256 differ are downloaded. Each download goes
//! to a temporary file next to its target, is verified, then renamed over it,
//! so an interrupted update never leaves a half-written DLL behind.
//!
//! Every file is compared by SHA-256. To keep startup fast, hashes are cached
//! in `.aetherial-dawn/files.json` in the game folder, keyed by size and
//! modified time, so a file is only re-hashed when it changes on disk.

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::manifest::{safe_relative, FileEntry, Manifest};
use crate::settings::SETTINGS_PATH;
use crate::{Error, Result};

const PART_SUFFIX: &str = ".adl-part";
const CACHE_PATH: &str = ".aetherial-dawn/files.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct HashCache(HashMap<String, Cached>);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Cached {
    size: u64,
    mtime_ns: u128,
    sha256: String,
}

impl HashCache {
    fn load(game_dir: &Path) -> Self {
        std::fs::read(game_dir.join(CACHE_PATH))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save(&self, game_dir: &Path) -> Result<()> {
        let path = game_dir.join(CACHE_PATH);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::atomicfile::write(&path, &serde_json::to_vec(self)?)?;
        Ok(())
    }

    /// The file's current SHA-256, from the cache when size and modified time
    /// are unchanged. `None` if the file doesn't exist.
    async fn hash(&mut self, game_dir: &Path, rel: &str, force: bool) -> Result<Option<String>> {
        let path = game_dir.join(safe_relative(rel)?);
        let meta = match tokio::fs::metadata(&path).await {
            Ok(m) if m.is_file() => m,
            _ => {
                self.0.remove(rel);
                return Ok(None);
            }
        };
        let size = meta.len();
        let mtime_ns = meta.modified()?.duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        if !force {
            if let Some(c) = self.0.get(rel) {
                if c.size == size && c.mtime_ns == mtime_ns {
                    return Ok(Some(c.sha256.clone()));
                }
            }
        }
        let sha256 = sha256_file(&path).await?;
        self.0.insert(rel.to_string(), Cached { size, mtime_ns, sha256: sha256.clone() });
        Ok(Some(sha256))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub download: Vec<FileEntry>,
    pub remove: Vec<String>,
    /// Listed to remove, but not the launcher's to delete.
    pub kept: Vec<String>,
    pub download_bytes: u64,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.download.is_empty() && self.remove.is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub file: String,
    pub files_done: usize,
    pub files_total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

pub async fn sha256_file(path: &Path) -> Result<String> {
    let mut f = tokio::fs::File::open(path).await?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Works out what needs to change. `verify_all` ignores the hash cache and
/// re-reads every file.
pub async fn plan(game_dir: &Path, manifest: &Manifest, verify_all: bool) -> Result<Plan> {
    let mut cache = HashCache::load(game_dir);
    let mut download = Vec::new();
    for f in &manifest.files {
        if f.path == SETTINGS_PATH {
            continue;
        }
        let local = cache.hash(game_dir, &f.path, verify_all).await?;
        if !local.is_some_and(|h| h.eq_ignore_ascii_case(&f.sha256)) {
            download.push(f.clone());
        }
    }
    cache.save(game_dir)?;
    let mut remove = Vec::new();
    let mut kept = Vec::new();
    for r in &manifest.remove {
        if r == SETTINGS_PATH || !game_dir.join(safe_relative(r)?).exists() {
            continue;
        }
        if removable(&cache, r) {
            remove.push(r.clone());
        } else {
            kept.push(r.clone());
        }
    }
    let download_bytes = download.iter().map(|f| f.size).sum();
    Ok(Plan { download, remove, kept, download_bytes })
}

/// A file the server's list asks to remove goes only if the launcher put it
/// there (an earlier file list had it, so it's in the hash cache) and it isn't
/// one of Skyrim's own files. The remove list can't be used to delete a
/// player's saves, other mods or the game.
fn removable(cache: &HashCache, rel: &str) -> bool {
    cache.0.contains_key(rel) && !crate::modlist::game_owned(rel)
}

pub async fn apply(
    client: &reqwest::Client,
    base_url: &str,
    game_dir: &Path,
    plan: &Plan,
    mut on_progress: impl FnMut(&Progress),
) -> Result<()> {
    let base = base_url.trim_end_matches('/');
    let mut p = Progress {
        file: String::new(),
        files_done: 0,
        files_total: plan.download.len(),
        bytes_done: 0,
        bytes_total: plan.download_bytes,
    };
    for f in &plan.download {
        p.file = f.path.clone();
        on_progress(&p);
        let dest = game_dir.join(safe_relative(&f.path)?);
        let url = format!("{base}/client/files/{}", f.sha256.to_ascii_lowercase());
        download_verified(client, &url, &dest, f, |n| {
            p.bytes_done += n;
            on_progress(&p);
        })
        .await?;
        p.files_done += 1;
    }
    let known = HashCache::load(game_dir);
    for r in plan.remove.iter().filter(|r| removable(&known, r)) {
        let path = game_dir.join(safe_relative(r)?);
        match tokio::fs::remove_file(&path).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    // Record the new files so the next start doesn't re-hash them.
    let mut cache = HashCache::load(game_dir);
    for f in &plan.download {
        cache.hash(game_dir, &f.path, true).await?;
    }
    for r in &plan.remove {
        cache.0.remove(r);
    }
    cache.save(game_dir)?;
    p.file = String::new();
    on_progress(&p);
    Ok(())
}

async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    f: &FileEntry,
    mut on_bytes: impl FnMut(u64),
) -> Result<()> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let part = part_path(dest);
    let result = async {
        let resp = client.get(url).send().await?.error_for_status()?;
        let mut out = tokio::fs::File::create(&part).await?;
        let mut h = Sha256::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            h.update(&chunk);
            out.write_all(&chunk).await?;
            on_bytes(chunk.len() as u64);
        }
        out.flush().await?;
        drop(out);
        let actual = hex::encode(h.finalize());
        if !actual.eq_ignore_ascii_case(&f.sha256) {
            return Err(Error::HashMismatch { path: f.path.clone(), expected: f.sha256.clone(), actual });
        }
        replace(&part, dest).await
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&part).await;
    }
    result
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(PART_SUFFIX);
    PathBuf::from(s)
}

async fn replace(from: &Path, to: &Path) -> Result<()> {
    // std::fs::rename replaces an existing file on Windows as well (MoveFileEx
    // with MOVEFILE_REPLACE_EXISTING), but fails if the game has it open.
    tokio::fs::rename(from, to).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            Error::Game(format!("{} is in use. Close Skyrim and try again.", to.display()))
        } else {
            e.into()
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Server;

    fn entry(path: &str, body: &[u8]) -> FileEntry {
        FileEntry { path: path.into(), size: body.len() as u64, sha256: hex::encode(Sha256::digest(body)) }
    }

    fn manifest(files: Vec<FileEntry>, remove: Vec<String>) -> Manifest {
        Manifest {
            schema: 1,
            build: "t".into(),
            server: Server { name: "t".into(), ip: "h".into(), port: 7777 },
            master: String::new(),
            files,
            remove,
            game: None,
        }
    }

    #[tokio::test]
    async fn plans_only_changed_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Data")).unwrap();
        std::fs::write(dir.path().join("Data/same.js"), b"same").unwrap();
        std::fs::write(dir.path().join("Data/edited.js"), b"old!").unwrap();
        std::fs::write(dir.path().join("Data/old.dll"), b"x").unwrap();
        // old.dll came from an earlier file list.
        plan(dir.path(), &manifest(vec![entry("Data/old.dll", b"x")], vec![]), false).await.unwrap();
        let m = manifest(
            vec![
                entry("Data/same.js", b"same"),
                entry("Data/edited.js", b"new!"),
                entry("Data/missing.js", b"hello"),
                entry(SETTINGS_PATH, b"{}"),
            ],
            vec!["Data/old.dll".into(), "Data/gone.dll".into()],
        );

        let first = plan(dir.path(), &m, false).await.unwrap();
        let names: Vec<_> = first.download.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(names, ["Data/edited.js", "Data/missing.js"], "same-size edits are caught by hash");
        assert_eq!(first.remove, ["Data/old.dll"]);

        // A second plan uses the cache and gives the same answer.
        let cached = plan(dir.path(), &m, false).await.unwrap();
        assert_eq!(cached.download.len(), 2);
        assert!(dir.path().join(CACHE_PATH).exists());
    }

    #[tokio::test]
    async fn remove_only_deletes_files_the_launcher_put_there() {
        let dir = tempfile::tempdir().unwrap();
        let game = dir.path();
        std::fs::create_dir_all(game.join("Data/SKSE/Plugins")).unwrap();
        std::fs::write(game.join("Data/SKSE/Plugins/served.dll"), b"served").unwrap();
        std::fs::write(game.join("Data/players-own.esp"), b"mine").unwrap();
        std::fs::write(game.join("SkyrimSE.exe"), b"game").unwrap();
        std::fs::write(game.join("Data/Skyrim.esm"), b"game").unwrap();
        // The launcher synced served.dll earlier, so it knows the file.
        plan(game, &manifest(vec![entry("Data/SKSE/Plugins/served.dll", b"served")], vec![]), false).await.unwrap();

        let m = manifest(
            vec![],
            ["Data/SKSE/Plugins/served.dll", "Data/players-own.esp", "SkyrimSE.exe", "Data/Skyrim.esm", "Saves/x.ess"].map(String::from).to_vec(),
        );
        let p = plan(game, &m, false).await.unwrap();
        assert_eq!(p.remove, ["Data/SKSE/Plugins/served.dll"]);
        assert_eq!(p.kept, ["Data/players-own.esp", "SkyrimSE.exe", "Data/Skyrim.esm"], "listed but not the launcher's to delete");

        // apply() holds the same line even for a plan made by hand.
        let hand = Plan { download: vec![], remove: vec!["Data/players-own.esp".into(), "Data/SKSE/Plugins/served.dll".into()], kept: vec![], download_bytes: 0 };
        apply(&reqwest::Client::new(), "http://127.0.0.1:1", game, &hand, |_| {}).await.unwrap();
        assert!(game.join("Data/players-own.esp").exists());
        assert!(!game.join("Data/SKSE/Plugins/served.dll").exists());
    }

    #[test]
    fn part_file_sits_next_to_target() {
        assert_eq!(part_path(Path::new("a/b.dll")), PathBuf::from("a/b.dll.adl-part"));
    }
}
