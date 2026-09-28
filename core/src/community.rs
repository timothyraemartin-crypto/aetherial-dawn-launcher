//! Turns Steam's current Skyrim (1.7.104) into the server's 1.6.1170 with the
//! free community xdelta patches from MulderLoad
//! (github.com/Mulderland/MulderLoad, Skyrim SE Steam Downgrader). No Steam
//! sign-in: the patches only work on the player's own copy of the game.
//!
//! The patches are downloaded and checked against the SHA-1s MulderLoad's own
//! downgrader pins, unpacked into a work folder on the game's drive, and every
//! patched file is written there first. Only when all of them have been made
//! are they swapped into the game, so a failure leaves the game as it was.
//! xdelta3 itself is the official 3.0.11 build from jmacd/xdelta-gpl,
//! downloaded the same way and run with no window.

use sha1::{Digest, Sha1};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// SHA-1 of Steam's SkyrimSE.exe 1.7.104 (August 27, 2026), the only build
/// these patches start from.
pub const FROM_EXE_SHA1: &str = "2f784a183f884067a9a41338664b55f6dc198a48";
pub const FROM_VERSION: &str = "1.7.104";
pub const TARGET: &str = "1.6.1170.0";
pub const WORK_DIR: &str = ".aetherial-dawn/downgrade";

const CDN: &str = "https://cdn.mulderload.eu/games/the-elder-scrolls-5-skyrim-special-edition/steam-downgrader";
const XDELTA_URL: &str = "https://github.com/jmacd/xdelta-gpl/releases/download/v3.0.11/xdelta3-3.0.11-x86_64.exe.zip";
const XDELTA_MIRROR: &str = "https://cdn.mulderload.eu/dependencies/xdelta3/xdelta3-3.0.11-x86_64.exe.zip";
const XDELTA_SHA1: &str = "d280cca0a52ce7e6da03bc2d27035a7b46b39c77";

/// One of MulderLoad's patch archives: a path under the CDN, its pinned
/// SHA-1, and how many `.001`, `.002`… parts it is split into (1 = one file).
#[derive(Debug, Clone, PartialEq)]
pub struct Pack {
    pub path: &'static str,
    pub sha1: &'static str,
    pub parts: u8,
}

const fn pack(path: &'static str, sha1: &'static str, parts: u8) -> Pack {
    Pack { path, sha1, parts }
}

/// Game, English and shared files: the same for every language.
const BASE: [Pack; 3] = [
    pack("1.7.104_to_1.6.1170/489831.7z", "0adfa48883116c088f172f51c34f56070ffcdf80", 2),
    pack("1.7.104_to_1.6.1170/489832.7z", "c2e1c9e57ad823a59477f942c86575388a13dbff", 1),
    pack("1.7.104_to_1.6.1170/489833.7z", "486c9d908b8ab444e28d9d577f6fbde61d4992e9", 1),
];

/// Voice and text archives for other languages. MulderLoad's 1.6.1170 option
/// uses its 1.7.99-to-1.6.640 language packs, since those files didn't change.
const LANGUAGES: [(&str, &str, Pack); 7] = [
    ("fr", "French", pack("1.7.99_to_1.6.640/489834.7z", "537743eca56cbabecefd3fa5e3ce89f986e24754", 3)),
    ("it", "Italian", pack("1.7.99_to_1.6.640/489835.7z", "5931e49806c7d42a11865f88e31136df6daca9a6", 3)),
    ("de", "German", pack("1.7.99_to_1.6.640/489836.7z", "903b69bddcc4fe83676772fbf8c487e9d55a14e4", 3)),
    ("es", "Spanish", pack("1.7.99_to_1.6.640/489837.7z", "d348cde03f344cd770d001fedb5eaa10c061427a", 3)),
    ("ru", "Russian", pack("1.7.99_to_1.6.640/489838.7z", "f2fb33c7746ec220a88dbb3f8bc47c0ffb457cbd", 2)),
    ("pl", "Polish", pack("1.7.99_to_1.6.640/489839.7z", "331c499e35053f0b6183cbae8672f43598ab3eae", 2)),
    ("ja", "Japanese", pack("1.7.99_to_1.6.640/544861.7z", "b8a6754b383892b460dc4e4e5811ba688039c386", 3)),
];

pub fn sha1_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha1::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(hex::encode(h.finalize()))
}

/// Whether this game folder holds the Steam build the patches start from.
pub fn supported(game_dir: &Path) -> bool {
    sha1_file(&game_dir.join("SkyrimSE.exe")).is_ok_and(|h| h == FROM_EXE_SHA1)
}

/// The game's language, found the way MulderLoad does it: by the voice
/// archive Steam installed. English when none of the others is there.
pub fn language(game_dir: &Path) -> (&'static str, Option<Pack>) {
    let mut found = ("English", None);
    for (code, name, p) in LANGUAGES.iter() {
        if game_dir.join("Data").join(format!("Skyrim - Voices_{code}0.bsa")).is_file() {
            found = (*name, Some(p.clone()));
        }
    }
    found
}

pub fn packs(game_dir: &Path) -> Vec<Pack> {
    let mut v = BASE.to_vec();
    v.extend(language(game_dir).1);
    v
}

fn part_names(p: &Pack) -> Vec<String> {
    let file = p.path.rsplit('/').next().unwrap_or(p.path);
    if p.parts <= 1 {
        vec![file.to_string()]
    } else {
        (1..=p.parts).map(|i| format!("{file}.{i:03}")).collect()
    }
}

/// Progress: stage ("download", "unpack", "patch", "swap"), file, done, total.
pub type Report<'a> = &'a mut (dyn FnMut(&str, &str, u64, u64) + Send);

async fn fetch(client: &reqwest::Client, url: &str, dest: &Path, report: Report<'_>, label: &str) -> Result<()> {
    use futures_util::StreamExt;
    let resp = client.get(url).header("User-Agent", "AetherialDawnLauncher").send().await?.error_for_status()?;
    let total = resp.content_length().unwrap_or(0);
    let tmp = dest.with_extension("download");
    let mut out = std::fs::File::create(&tmp)?;
    let mut done = 0u64;
    let mut stream = resp.bytes_stream();
    let mut last = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        out.write_all(&chunk)?;
        done += chunk.len() as u64;
        if done - last >= 4 << 20 || done == total {
            report("download", label, done, total);
            last = done;
        }
    }
    out.flush()?;
    drop(out);
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

/// Downloads one archive's parts into `work`, reusing parts already there,
/// and returns the archive joined into one file. The pinned SHA-1 is checked
/// against the first part and against the joined file, since MulderLoad pins
/// one of them.
async fn download_pack(client: &reqwest::Client, p: &Pack, work: &Path, report: Report<'_>) -> Result<PathBuf> {
    let base = p.path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let mut parts = Vec::new();
    for name in part_names(p) {
        let dest = work.join(&name);
        if !dest.is_file() {
            fetch(client, &format!("{CDN}/{base}/{name}"), &dest, report, &name).await?;
        }
        parts.push(dest);
    }
    let joined = if parts.len() == 1 {
        parts[0].clone()
    } else {
        let j = work.join(p.path.rsplit('/').next().unwrap_or("pack.7z"));
        let mut out = std::fs::File::create(&j)?;
        for part in &parts {
            std::io::copy(&mut std::fs::File::open(part)?, &mut out)?;
        }
        j
    };
    let first = sha1_file(&parts[0])?;
    if first != p.sha1 && sha1_file(&joined)? != p.sha1 {
        for part in &parts {
            let _ = std::fs::remove_file(part);
        }
        let _ = std::fs::remove_file(&joined);
        return Err(Error::HashMismatch { path: p.path.into(), expected: p.sha1.into(), actual: first });
    }
    Ok(joined)
}

/// Unpacks an archive into `stage`, keeping its folder layout. Returns the
/// relative paths written.
pub fn unpack(archive: &Path, stage: &Path) -> Result<Vec<String>> {
    let file = std::fs::File::open(archive)?;
    let mut reader = sevenz_rust2::ArchiveReader::new(file, sevenz_rust2::Password::empty())
        .map_err(|e| Error::Game(format!("patch archive is damaged: {e}")))?;
    let mut wrote = Vec::new();
    reader
        .for_each_entries(|entry, data| {
            let rel = entry.name().replace('\\', "/");
            if entry.is_directory() || rel.split('/').any(|c| c == ".." || c.is_empty()) || rel.starts_with('/') || rel.contains(':') {
                return Ok(true);
            }
            let dest = stage.join(&rel);
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p)?;
            }
            let mut out = std::fs::File::create(&dest)?;
            std::io::copy(data, &mut out)?;
            wrote.push(rel);
            Ok(true)
        })
        .map_err(|e| Error::Game(format!("couldn't unpack the patches: {e}")))?;
    Ok(wrote)
}

/// Downloads xdelta3.exe into `work` (Windows). Elsewhere, `xdelta3` on PATH.
async fn xdelta(client: &reqwest::Client, work: &Path) -> Result<PathBuf> {
    if !cfg!(windows) {
        return Ok(PathBuf::from("xdelta3"));
    }
    let exe = work.join("xdelta3.exe");
    if exe.is_file() {
        return Ok(exe);
    }
    let mut bytes = None;
    for url in [XDELTA_URL, XDELTA_MIRROR] {
        let got = async { client.get(url).header("User-Agent", "AetherialDawnLauncher").send().await?.error_for_status()?.bytes().await }.await;
        if let Ok(b) = got {
            if hex::encode(Sha1::digest(&b)) == XDELTA_SHA1 {
                bytes = Some(b);
                break;
            }
        }
    }
    let bytes = bytes.ok_or_else(|| Error::Game("couldn't download xdelta3".into()))?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| Error::Game(format!("xdelta3 download is damaged: {e}")))?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| Error::Game(e.to_string()))?;
        if f.name().to_ascii_lowercase().ends_with(".exe") {
            let mut out = std::fs::File::create(&exe)?;
            std::io::copy(&mut f, &mut out)?;
            return Ok(exe);
        }
    }
    Err(Error::Game("xdelta3 download had no program in it".into()))
}

/// Runs `xdelta3 -d -s <old> <patch> <out>` with no window.
pub fn apply_xdelta(tool: &Path, old: &Path, patch: &Path, out: &Path) -> Result<()> {
    let _ = std::fs::remove_file(out);
    let mut cmd = std::process::Command::new(tool);
    cmd.arg("-d").arg("-f").arg("-s").arg(old).arg(patch).arg(out);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let o = cmd.output().map_err(|e| Error::Game(format!("couldn't run xdelta3 ({e})")))?;
    if !o.status.success() {
        let msg = String::from_utf8_lossy(&o.stderr).trim().to_string();
        return Err(Error::Game(format!("xdelta3 failed ({msg})")));
    }
    Ok(())
}

/// A file to put in the game: `from` in the work folder, `to` relative to the
/// game folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Swap {
    pub from: PathBuf,
    pub to: String,
}

/// Starts the error for a game file the patches need that isn't there.
pub const MISSING_FILE: &str = "NO_PATCH_FILES:";

/// Makes every new file in `stage`: applies each `<file>.xdelta` to the
/// game's `<file>` (the result goes to `<stage>/<file>`), and takes any other
/// file as a whole file shipped with the patches. Nothing in the game changes.
pub fn build_swaps(tool: &Path, game_dir: &Path, stage: &Path, files: &[String], report: Report<'_>) -> Result<Vec<Swap>> {
    let deltas: Vec<&String> = files.iter().filter(|f| f.to_ascii_lowercase().ends_with(".xdelta")).collect();
    let mut swaps = Vec::new();
    for (i, rel) in deltas.iter().enumerate() {
        let target = &rel[..rel.len() - ".xdelta".len()];
        report("patch", target, i as u64, deltas.len() as u64);
        let old = game_dir.join(target);
        if !old.is_file() {
            // The launcher has Steam repair it (text audit A9: NO_PATCH_FILES
            // runs the UI's Steam repair, then patching carries on).
            return Err(Error::Game(format!("{MISSING_FILE}{target} is missing from the game folder.")));
        }
        let out = stage.join(format!("{target}.new"));
        apply_xdelta(tool, &old, &stage.join(rel), &out).map_err(|e| Error::Game(format!("couldn't patch {target}: {e}")))?;
        let _ = std::fs::remove_file(stage.join(rel));
        swaps.push(Swap { from: out, to: target.to_string() });
    }
    for rel in files.iter().filter(|f| !f.to_ascii_lowercase().ends_with(".xdelta")) {
        swaps.push(Swap { from: stage.join(rel), to: rel.clone() });
    }
    Ok(swaps)
}

/// Puts the new files into the game. The work folder is on the game's drive,
/// so each one is a rename.
pub fn swap_in(game_dir: &Path, swaps: &[Swap], report: Report<'_>) -> Result<()> {
    for (i, s) in swaps.iter().enumerate() {
        report("swap", &s.to, i as u64, swaps.len() as u64);
        let dest = game_dir.join(&s.to);
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p)?;
        }
        let _ = std::fs::remove_file(&dest);
        std::fs::rename(&s.from, &dest)
            .or_else(|_| std::fs::copy(&s.from, &dest).map(|_| ()))
            .map_err(|e| Error::Game(format!("couldn't replace {} ({e}). Close Skyrim, Steam and Vortex, then try again.", s.to)))?;
    }
    Ok(())
}

/// The newer game writes a ContentCatalog.txt the old one can crash on, and
/// its shader cache doesn't fit the old exe. MulderLoad clears both.
pub fn after_downgrade(game_dir: &Path) {
    let _ = std::fs::remove_dir_all(game_dir.join("Data").join("ShaderCache"));
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let cat = PathBuf::from(local).join("Skyrim Special Edition").join("ContentCatalog.txt");
        if std::fs::read_to_string(&cat).is_ok_and(|s| s.contains("AchievementSafe")) {
            let _ = std::fs::rename(&cat, cat.with_extension("bak"));
        }
    }
}

/// The whole job, from Steam's 1.7.104 to 1.6.1170. Returns the files changed.
pub async fn downgrade(client: &reqwest::Client, game_dir: &Path, report: Report<'_>) -> Result<Vec<String>> {
    if !supported(game_dir) {
        return Err(Error::Game(format!("NOT_SUPPORTED:This Skyrim isn't Steam's {FROM_VERSION}.")));
    }
    let work = game_dir.join(WORK_DIR);
    let stage = work.join("stage");
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage)?;
    let mut files = Vec::new();
    for p in packs(game_dir) {
        let archive = download_pack(client, &p, &work, report).await?;
        report("unpack", p.path, 0, 0);
        let (a, s) = (archive.clone(), stage.clone());
        let got = tokio::task::spawn_blocking(move || unpack(&a, &s)).await.map_err(|e| Error::Game(e.to_string()))??;
        files.extend(got);
    }
    files.sort();
    files.dedup();
    let tool = xdelta(client, &work).await?;
    let swaps = build_swaps(&tool, game_dir, &stage, &files, report)?;
    swap_in(game_dir, &swaps, report)?;
    after_downgrade(game_dir);
    let _ = std::fs::remove_dir_all(&work);
    Ok(swaps.into_iter().map(|s| s.to).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_and_languages() {
        assert_eq!(part_names(&BASE[0]), ["489831.7z.001", "489831.7z.002"]);
        assert_eq!(part_names(&BASE[1]), ["489832.7z"]);
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("Data")).unwrap();
        assert_eq!(language(tmp.path()).0, "English");
        assert_eq!(packs(tmp.path()).len(), 3);
        std::fs::write(tmp.path().join("Data/Skyrim - Voices_de0.bsa"), b"").unwrap();
        assert_eq!(language(tmp.path()).0, "German");
        assert_eq!(packs(tmp.path()).len(), 4);
        assert!(!supported(tmp.path()));
    }

    /// Needs xdelta3 on PATH; skipped otherwise.
    #[test]
    fn patches_whole_folder_then_swaps() {
        if std::process::Command::new("xdelta3").arg("-V").output().is_err() {
            eprintln!("xdelta3 not installed, skipping");
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("game");
        let stage = tmp.path().join("stage");
        std::fs::create_dir_all(game.join("Data")).unwrap();
        std::fs::create_dir_all(stage.join("Data")).unwrap();
        let old: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let mut new = old.clone();
        new[1000..1010].copy_from_slice(b"1.6.1170!!");
        std::fs::write(game.join("Data/Skyrim.esm"), &old).unwrap();
        std::fs::write(tmp.path().join("new.esm"), &new).unwrap();
        let st = std::process::Command::new("xdelta3")
            .args(["-e", "-f", "-s"])
            .arg(game.join("Data/Skyrim.esm"))
            .arg(tmp.path().join("new.esm"))
            .arg(stage.join("Data/Skyrim.esm.xdelta"))
            .status()
            .unwrap();
        assert!(st.success());
        std::fs::write(stage.join("steam_api64.dll"), b"whole file").unwrap();
        let files = vec!["Data/Skyrim.esm.xdelta".to_string(), "steam_api64.dll".to_string()];
        let mut log = |_: &str, _: &str, _: u64, _: u64| {};
        let swaps = build_swaps(Path::new("xdelta3"), &game, &stage, &files, &mut log).unwrap();
        // Nothing in the game changed yet.
        assert_eq!(std::fs::read(game.join("Data/Skyrim.esm")).unwrap(), old);
        swap_in(&game, &swaps, &mut log).unwrap();
        assert_eq!(std::fs::read(game.join("Data/Skyrim.esm")).unwrap(), new);
        assert_eq!(std::fs::read(game.join("steam_api64.dll")).unwrap(), b"whole file");
        // A missing source file stops before anything is written.
        let files = vec!["Data/Gone.esm.xdelta".to_string()];
        let e = build_swaps(Path::new("xdelta3"), &game, &stage, &files, &mut log).unwrap_err().to_string();
        // With the prefix that has Steam repair the game (text audit A9).
        assert!(e.starts_with(MISSING_FILE), "{e}");
    }
}
