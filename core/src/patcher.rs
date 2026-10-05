//! Turns a player's Skyrim files into the server's build on their own PC, with
//! no Steam download. The server publishes binary patches (zstd "patch from"
//! deltas) that only work on top of the player's own copy of each file, found
//! by its SHA-256, plus the SHA-256 of the result. Files that already match are
//! left alone. Nothing is replaced until the patched file checks out.
//!
//! Patches are made with `AetherialDawn.exe --make-patches <from> <to> <out>`
//! from a folder with the newer Steam build and one with the server's build
//! (see [`build`]).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::{Error, Result};

pub const INDEX: &str = "patches/index.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Index {
    /// The game version the patches produce, such as "1.6.1170.0".
    pub target: String,
    pub files: Vec<PFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PFile {
    /// Game-relative path with forward slashes, such as "Data/Skyrim.esm".
    pub path: String,
    pub size: u64,
    pub sha256: String,
    #[serde(default)]
    pub patches: Vec<Patch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Patch {
    pub from_sha256: String,
    pub from_size: u64,
    /// Relative to the patches folder.
    pub file: String,
    pub size: u64,
    pub sha256: String,
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

pub fn sha256_bytes(b: &[u8]) -> String {
    hex::encode(Sha256::digest(b))
}

/// zstd needs a window that covers the whole old file.
fn window_log(len: u64) -> u32 {
    let mut w = 10;
    while w < 31 && (1u64 << w) < len {
        w += 1;
    }
    w
}

/// Writes a patch that turns `old` into `new`.
pub fn make_patch(old: &Path, new: &Path, out: &Path) -> Result<()> {
    let prefix = std::fs::read(old)?;
    let new_len = std::fs::metadata(new)?.len();
    if prefix.len() as u64 >= 1 << 31 || new_len >= 1 << 31 {
        return Err(Error::Game(format!("{} is too large to patch", new.display())));
    }
    let file = std::io::BufWriter::new(std::fs::File::create(out)?);
    let mut enc = zstd::stream::write::Encoder::with_ref_prefix(file, 12, &prefix)?;
    enc.window_log(window_log(prefix.len() as u64 + new_len))?;
    enc.long_distance_matching(true)?;
    enc.include_checksum(true)?;
    std::io::copy(&mut std::io::BufReader::new(std::fs::File::open(new)?), &mut enc)?;
    enc.finish()?.flush()?;
    Ok(())
}

/// Applies a patch made by [`make_patch`] to `old`, writing `out`.
pub fn apply_patch(old: &Path, patch: &Path, out: &Path) -> Result<()> {
    let prefix = std::fs::read(old)?;
    let reader = std::io::BufReader::new(std::fs::File::open(patch)?);
    let mut dec = zstd::stream::read::Decoder::with_ref_prefix(reader, &prefix)?;
    dec.window_log_max(31)?;
    let mut w = std::io::BufWriter::new(std::fs::File::create(out)?);
    std::io::copy(&mut dec, &mut w)?;
    w.flush()?;
    Ok(())
}

/// What the player's game needs: each file that differs, with the patch that
/// fits their copy, or none when no patch matches it.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub file: PFile,
    pub patch: Option<Patch>,
}

/// Compares the game folder with the index. `hash` gives a file's SHA-256
/// (callers can cache it by size and date).
pub fn plan(game_dir: &Path, index: &Index, mut hash: impl FnMut(&Path) -> Option<String>) -> Vec<Step> {
    let mut steps = Vec::new();
    for f in index.files.iter().filter(|f| patchable(&f.path)) {
        let p = game_dir.join(&f.path);
        let size = std::fs::metadata(&p).map(|m| m.len()).ok();
        if size == Some(f.size) && hash(&p).is_some_and(|h| h.eq_ignore_ascii_case(&f.sha256)) {
            continue;
        }
        let have = size.and_then(|s| hash(&p).map(|h| (s, h)));
        let patch = have.and_then(|(s, h)| f.patches.iter()
            .find(|x| x.from_size == s && x.from_sha256.eq_ignore_ascii_case(&h) && plain_file_name(&x.file)).cloned());
        steps.push(Step { file: f.clone(), patch });
    }
    steps
}

/// A patch's `file` must be one plain file name, as `make_patches` writes
/// them ("<from>-<to>.zst"): the launcher joins it to its own folders, so a
/// path ("..\x", "C:\x", "a/b") from a changed index could write anywhere
/// (triple check B-launcher-1). A patch with any other name never matches.
pub fn plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['/', '\\', ':'])
        && !name.ends_with(['.', ' '])
        && name.chars().all(|c| !c.is_control())
}

/// Where the server's patches write their results: the downgrade work
/// folder, on the game's drive, so putting them in is a rename.
pub fn stage_dir(game_dir: &Path) -> PathBuf {
    game_dir.join(crate::community::WORK_DIR).join("server-stage")
}

/// Makes one patched file in `stage` and checks it; the game isn't
/// touched. `patch_file` is the downloaded patch. The result goes into the
/// game with every other file in one journalled swap
/// (`community::swap_in`), so a failure part way leaves the game as it was.
pub fn make_patched(game_dir: &Path, stage: &Path, step: &Step, patch_file: &Path) -> Result<crate::community::Swap> {
    let live = game_dir.join(&step.file.path);
    let out = stage.join(&step.file.path);
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p)?;
    }
    apply_patch(&live, patch_file, &out)?;
    let got = sha256_file(&out)?;
    if !got.eq_ignore_ascii_case(&step.file.sha256) {
        let _ = std::fs::remove_file(&out);
        return Err(Error::HashMismatch { path: step.file.path.clone(), expected: step.file.sha256.clone(), actual: got });
    }
    Ok(crate::community::Swap { from: out, to: step.file.path.clone() })
}

/// Patches one file: made in the stage folder, checked, then swapped in.
/// The old copy is moved, never deleted, so a hard link to it (the kept
/// copy) keeps the old build.
pub fn apply_step(game_dir: &Path, step: &Step, patch_file: &Path) -> Result<()> {
    let swap = make_patched(game_dir, &stage_dir(game_dir), step, patch_file)?;
    crate::community::swap_in(game_dir, &[swap], &mut |_: &str, _: &str, _: u64, _: u64| {})
}

/// The files patches are made for: Steam's own game files. Never the
/// Creation Club files or _ResourcePack (QA 2026-09-30: no bytes of them,
/// not even as a diff, are ever served), so `--make-patches` leaves them
/// out and a served index that lists them is ignored.
pub fn patchable(rel: &str) -> bool {
    let l = rel.to_ascii_lowercase();
    match l.strip_prefix("data/") {
        None => matches!(l.as_str(), "skyrimse.exe" | "skyrimselauncher.exe" | "steam_api64.dll" | "bink2w64.dll"),
        Some(n) => {
            matches!(n, "skyrim.esm" | "update.esm" | "dawnguard.esm" | "hearthfires.esm" | "dragonborn.esm" | "skyrim.ccc")
                || (n.starts_with("skyrim - ") && n.ends_with(".bsa"))
                || n.starts_with("marketplacetextures.")
        }
    }
}

fn list(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for (sub, prefix) in [(dir.to_path_buf(), ""), (dir.join("Data"), "Data/")] {
        if let Ok(rd) = std::fs::read_dir(&sub) {
            for e in rd.flatten() {
                let rel = format!("{prefix}{}", e.file_name().to_string_lossy());
                if e.path().is_file() && patchable(&rel) {
                    out.push(rel);
                }
            }
        }
    }
    out.sort();
    out
}

fn find_ci(dir: &Path, rel: &str) -> Option<PathBuf> {
    let (sub, name) = match rel.split_once('/') {
        Some((a, b)) => (dir.join(a), b),
        None => (dir.to_path_buf(), rel),
    };
    std::fs::read_dir(&sub).ok()?.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name)).map(|e| e.path())
}

/// Makes patches from the game folder `from` (the newer Steam build) to `to`
/// (the server's build) in `out`, merging with an index already there so one
/// folder can hold patches from several Steam builds. Returns the index.
/// Files only the target has are listed with no patch: players need them
/// from Steam.
pub fn build(from: &Path, to: &Path, target: &str, out: &Path, mut log: impl FnMut(&str)) -> Result<Index> {
    std::fs::create_dir_all(out)?;
    let index_path = out.join("index.json");
    let mut index: Index = std::fs::read(&index_path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .filter(|i: &Index| i.target == target)
        .unwrap_or(Index { target: target.to_string(), files: Vec::new() });
    for rel in list(to) {
        let new = find_ci(to, &rel).expect("listed");
        let size = std::fs::metadata(&new)?.len();
        let sha = sha256_file(&new)?;
        let pos = match index.files.iter().position(|f| f.path.eq_ignore_ascii_case(&rel)) {
            Some(i) if index.files[i].sha256 == sha => i,
            Some(i) => {
                index.files[i] = PFile { path: rel.clone(), size, sha256: sha.clone(), patches: Vec::new() };
                i
            }
            None => {
                index.files.push(PFile { path: rel.clone(), size, sha256: sha.clone(), patches: Vec::new() });
                index.files.len() - 1
            }
        };
        let Some(old) = find_ci(from, &rel) else {
            log(&format!("{rel}: not in the source folder, no patch"));
            continue;
        };
        let old_size = std::fs::metadata(&old)?.len();
        let old_sha = sha256_file(&old)?;
        if old_sha == sha {
            log(&format!("{rel}: already the same"));
            continue;
        }
        if index.files[pos].patches.iter().any(|p| p.from_sha256 == old_sha) {
            log(&format!("{rel}: patch already made"));
            continue;
        }
        let name = format!("{}-{}.zst", &old_sha[..16], &sha[..16]);
        let file = out.join(&name);
        log(&format!("{rel}: making patch {name}"));
        make_patch(&old, &new, &file)?;
        // Prove it round-trips before listing it.
        let check = out.join(format!("{name}.check"));
        apply_patch(&old, &file, &check)?;
        let ok = sha256_file(&check)? == sha;
        let _ = std::fs::remove_file(&check);
        if !ok {
            return Err(Error::Game(format!("the patch for {rel} didn't reproduce the file")));
        }
        let p = Patch { from_sha256: old_sha, from_size: old_size, file: name, size: std::fs::metadata(&file)?.len(), sha256: sha256_file(&file)? };
        log(&format!("{rel}: patch is {} bytes", p.size));
        index.files[pos].patches.push(p);
    }
    index.files.sort_by(|a, b| a.path.cmp(&b.path));
    std::fs::write(&index_path, serde_json::to_vec_pretty(&index)?)?;
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(dir: &Path, exe: &[u8], esm: &[u8]) {
        std::fs::create_dir_all(dir.join("Data")).unwrap();
        std::fs::write(dir.join("SkyrimSE.exe"), exe).unwrap();
        std::fs::write(dir.join("Data/Skyrim.esm"), esm).unwrap();
        std::fs::write(dir.join("Data/Skyrim - Textures0.bsa"), b"same textures").unwrap();
        std::fs::write(dir.join("Data/SomeMod.esp"), b"not ours").unwrap();
        std::fs::write(dir.join("Data/ccBGSSSE001-Fish.esm"), [esm, b"cc"].concat()).unwrap();
        std::fs::write(dir.join("Data/_ResourcePack.esl"), [esm, b"rp"].concat()).unwrap();
    }

    #[test]
    fn a_patch_named_as_a_path_is_never_used() {
        for bad in ["", "..", ".", "..\\..\\x.zst", "../x.zst", "C:\\Users\\x.dll", "a/b.zst", "x.zst.", "x.zst ", "d:x"] {
            assert!(!plain_file_name(bad), "{bad:?}");
        }
        assert!(plain_file_name("0123456789abcdef-fedcba9876543210.zst"));
        let t = tempfile::tempdir().unwrap();
        game(t.path(), b"old exe", b"old esm");
        let have = sha256_file(&t.path().join("SkyrimSE.exe")).unwrap();
        let patch = |file: &str| Patch { from_sha256: have.clone(), from_size: 7, file: file.into(), size: 1, sha256: "00".into() };
        let index = Index { target: "1.6.1170.0".into(), files: vec![PFile { path: "SkyrimSE.exe".into(), size: 7, sha256: "ff".into(), patches: vec![patch("..\\..\\evil.dll")] }] };
        let steps = plan(t.path(), &index, |p| sha256_file(p).ok());
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].patch, None);
        let index = Index { files: vec![PFile { patches: vec![patch("aa-bb.zst")], ..index.files[0].clone() }], ..index };
        assert_eq!(plan(t.path(), &index, |p| sha256_file(p).ok())[0].patch.as_ref().map(|p| p.file.as_str()), Some("aa-bb.zst"));
    }

    #[test]
    fn builds_plans_and_applies_patches() {
        let t = tempfile::tempdir().unwrap();
        let (newer, server, out, player) = (t.path().join("newer"), t.path().join("server"), t.path().join("out"), t.path().join("player"));
        let big_new: Vec<u8> = (0..200_000u32).flat_map(|i| (i * 7).to_le_bytes()).collect();
        let mut big_old = big_new.clone();
        big_old[1234] ^= 0xFF;
        big_old.extend(b"extra from the newer build");
        game(&newer, b"exe 1.7", &big_old);
        game(&server, b"exe 1.6.1170", &big_new);
        let index = build(&newer, &server, "1.6.1170.0", &out, |_| {}).unwrap();
        assert_eq!(index.files.len(), 3);
        let esm = index.files.iter().find(|f| f.path == "Data/Skyrim.esm").unwrap();
        assert_eq!(esm.patches.len(), 1);
        assert!(esm.patches[0].size < 4000, "a small change makes a small patch");
        assert!(index.files.iter().find(|f| f.path.ends_with("Textures0.bsa")).unwrap().patches.is_empty());
        // Building again reuses what's there.
        assert_eq!(build(&newer, &server, "1.6.1170.0", &out, |_| {}).unwrap(), index);

        game(&player, b"exe 1.7", &big_old);
        let steps = plan(&player, &index, |p| sha256_file(p).ok());
        assert_eq!(steps.len(), 2);
        assert!(steps.iter().all(|s| s.patch.is_some()));
        for s in &steps {
            apply_step(&player, s, &out.join(&s.patch.as_ref().unwrap().file)).unwrap();
        }
        assert_eq!(std::fs::read(player.join("Data/Skyrim.esm")).unwrap(), big_new);
        assert_eq!(std::fs::read(player.join("SkyrimSE.exe")).unwrap(), b"exe 1.6.1170");
        assert!(plan(&player, &index, |p| sha256_file(p).ok()).is_empty());
        // Creation Club and _ResourcePack: never a patch, and a served list
        // that names one is ignored.
        assert!(index.files.iter().all(|f| !f.path.to_ascii_lowercase().contains("ccbgssse") && !f.path.to_ascii_lowercase().contains("_resourcepack")));
        let mut sneaky = index.clone();
        sneaky.files.push(PFile { path: "Data/ccBGSSSE001-Fish.esm".into(), size: 1, sha256: "00".repeat(32), patches: vec![] });
        assert!(plan(&player, &sneaky, |p| sha256_file(p).ok()).is_empty());
        // A copy no patch was made from can't be patched.
        std::fs::write(player.join("Data/Skyrim.esm"), b"some other build").unwrap();
        let steps = plan(&player, &index, |p| sha256_file(p).ok());
        assert_eq!(steps.len(), 1);
        assert!(steps[0].patch.is_none());
        // A failure making any file changes nothing in the game: every file
        // is made in the stage first, then all go in in one swap.
        game(&player, b"exe 1.7", &big_old);
        let steps = plan(&player, &index, |p| sha256_file(p).ok());
        let stage = stage_dir(&player);
        let first = make_patched(&player, &stage, &steps[0], &out.join(&steps[0].patch.as_ref().unwrap().file)).unwrap();
        let wrong_patch = out.join(&steps[0].patch.as_ref().unwrap().file);
        assert!(make_patched(&player, &stage, &steps[1], &wrong_patch).is_err());
        assert_eq!(std::fs::read(player.join("Data/Skyrim.esm")).unwrap(), big_old);
        assert_eq!(std::fs::read(player.join("SkyrimSE.exe")).unwrap(), b"exe 1.7");
        assert!(first.from.starts_with(&stage));
        let _ = std::fs::remove_dir_all(&stage);
    }
}
