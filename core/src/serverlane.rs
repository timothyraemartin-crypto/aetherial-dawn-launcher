//! The server-mods export (staging runbook A1, 2026-09-27): the plugins the
//! test server loads come from Nexus through the owner's own Premium, the
//! same files every player's launcher installs. The launcher downloads the
//! server's list into its own folder (never the game folder), keeps the
//! plugins and zips them to server-lane.zip. Nothing here uploads anything:
//! the zip stays on the PC until someone carries it over.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::modlist::{self, ModEntry};
use crate::{Error, Result};

/// The served list, `<base_url>/server-lane.json`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ServerLane {
    /// Only the launcher signed in with this Discord account exports.
    pub for_discord_id: String,
    pub mods: Vec<LaneMod>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct LaneMod {
    #[serde(flatten)]
    pub entry: ModEntry,
    /// The plugins to take from this download, by file name. Empty takes
    /// every plugin the install would put at the top of Data. Named ones
    /// are found anywhere in the archive (a patch collection's options),
    /// the installer's own pick first.
    #[serde(default)]
    pub plugins: Vec<String>,
}

/// What an export left, `server-lane/export.json`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Record {
    /// `list_hash` of the list it was made from.
    pub list: String,
    /// Plugin name -> the mod it came from.
    pub plugins: BTreeMap<String, String>,
    /// SHA-256 of server-lane.zip.
    pub zip_sha256: String,
    pub zip_bytes: u64,
}

pub const LANE_DIR: &str = "server-lane";
pub const ZIP_NAME: &str = "server-lane.zip";
const RECORD: &str = "export.json";

/// A stable fingerprint of the list: a changed list exports again.
pub fn list_hash(lane: &ServerLane) -> String {
    use sha2::{Digest, Sha256};
    let text = serde_json::to_vec(lane).unwrap_or_default();
    format!("{:x}", Sha256::digest(&text))
}

/// Checks the list before anything downloads: every mod pinned to one Nexus
/// file (the server needs exactly the bytes players get), ids usable as
/// file names, no plugin taken twice.
pub fn check(lane: &ServerLane) -> Result<()> {
    let mut ids = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for m in &lane.mods {
        let e = &m.entry;
        if e.id.is_empty() || !e.id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return Err(Error::Game(format!("server lane: mod id {:?} isn't a plain name", e.id)));
        }
        if !ids.insert(e.id.clone()) {
            return Err(Error::Game(format!("server lane: {} is listed twice", e.id)));
        }
        if e.nexus.as_ref().and_then(|n| n.file).is_none() {
            return Err(Error::Game(format!("server lane: {} isn't pinned to one Nexus file", e.id)));
        }
        for p in &m.plugins {
            if !is_plugin(p) || p.contains(['/', '\\']) {
                return Err(Error::Game(format!("server lane: {} names {p:?}, which isn't a plugin file name", e.id)));
            }
            if !names.insert(p.to_ascii_lowercase()) {
                return Err(Error::Game(format!("server lane: {p} is taken from two mods")));
            }
        }
    }
    Ok(())
}

fn is_plugin(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    [".esp", ".esm", ".esl"].iter().any(|x| l.ends_with(x))
}

/// The export's folders under the launcher's own data folder.
pub fn lane_dir(app_data: &Path) -> PathBuf {
    app_data.join(LANE_DIR)
}

/// True when this list was already exported and its zip is still there.
pub fn done(root: &Path, hash: &str) -> bool {
    let Ok(text) = std::fs::read(root.join(RECORD)) else { return false };
    let Ok(rec) = serde_json::from_slice::<Record>(&text) else { return false };
    rec.list == hash && std::fs::metadata(root.join(ZIP_NAME)).map(|m| m.len() == rec.zip_bytes).unwrap_or(false)
}

/// Picks the plugins to keep from one unpacked download, as (source, plugin
/// name). `planned` is what the normal install would copy.
pub fn pick(m: &LaneMod, unpacked: &Path) -> Result<Vec<(PathBuf, String)>> {
    // With named plugins the archive is searched anyway, so an installer
    // the launcher can't follow doesn't stop it.
    let planned = match modlist::plan(&m.entry, unpacked) {
        Ok(p) => p,
        Err(_) if !m.plugins.is_empty() => Vec::new(),
        Err(e) => return Err(e),
    };
    let top: Vec<(PathBuf, String)> = planned
        .iter()
        .filter_map(|c| {
            let rel = c.to.to_string_lossy().replace('\\', "/");
            let name = rel.strip_prefix("Data/")?.to_string();
            (!name.contains('/') && is_plugin(&name)).then(|| (c.from.clone(), name))
        })
        .collect();
    if m.plugins.is_empty() {
        if top.is_empty() {
            return Err(Error::Game(format!("{} has no plugin to take", m.entry.name)));
        }
        return Ok(top);
    }
    let mut out = Vec::new();
    for want in &m.plugins {
        if let Some(t) = top.iter().find(|(_, n)| n.eq_ignore_ascii_case(want)) {
            out.push((t.0.clone(), want.clone()));
            continue;
        }
        let found: Vec<PathBuf> = files_named(unpacked, want);
        match found.len() {
            1 => out.push((found[0].clone(), want.clone())),
            0 => return Err(Error::Game(format!("{} has no {want} in its download", m.entry.name))),
            _ => {
                // Several copies with the same bytes are one file.
                let first = std::fs::read(&found[0])?;
                if found[1..].iter().all(|f| std::fs::read(f).map(|b| b == first).unwrap_or(false)) {
                    out.push((found[0].clone(), want.clone()));
                } else {
                    let rel: Vec<String> = found.iter().map(|f| f.strip_prefix(unpacked).unwrap_or(f).to_string_lossy().replace('\\', "/")).collect();
                    return Err(Error::Game(format!("{} has different files named {want} ({}); name one with the installer options", m.entry.name, rel.join(", "))));
                }
            }
        }
    }
    Ok(out)
}

fn files_named(dir: &Path, name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(e.path());
            } else if ft.is_file() && e.file_name().to_string_lossy().eq_ignore_ascii_case(name) {
                out.push(e.path());
            }
        }
    }
    out.sort();
    out
}

/// Copies the picked plugins into `<root>/Data`, refusing two different
/// files under one name.
pub fn collect(root: &Path, mod_id: &str, picked: &[(PathBuf, String)], seen: &mut BTreeMap<String, String>) -> Result<()> {
    let data = root.join("Data");
    std::fs::create_dir_all(&data)?;
    for (from, name) in picked {
        if let Some((prev_name, prev)) = seen.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            if prev != mod_id {
                return Err(Error::Game(format!("{name} comes from both {prev} and {mod_id} (as {prev_name})")));
            }
        }
        let tmp = data.join(format!("{name}.part"));
        std::fs::copy(from, &tmp)?;
        std::fs::rename(&tmp, data.join(name))?;
        seen.insert(name.clone(), mod_id.to_string());
    }
    Ok(())
}

/// Zips `<root>/Data` to `<root>/server-lane.zip` (entries "Data/<name>",
/// stored: plugins barely compress and the VPS unzips quicker) and writes the
/// record. Returns it.
pub fn finish(root: &Path, hash: &str, plugins: BTreeMap<String, String>) -> Result<Record> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    let data = root.join("Data");
    let tmp = root.join(format!("{ZIP_NAME}.part"));
    {
        let f = std::fs::File::create(&tmp)?;
        let mut z = zip::ZipWriter::new(std::io::BufWriter::new(f));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).large_file(true);
        for name in plugins.keys() {
            z.start_file(format!("Data/{name}"), opts).map_err(|e| Error::Game(format!("zip: {e}")))?;
            let mut src = std::fs::File::open(data.join(name))?;
            std::io::copy(&mut src, &mut z)?;
        }
        z.finish().map_err(|e| Error::Game(format!("zip: {e}")))?.flush()?;
    }
    let zip = root.join(ZIP_NAME);
    std::fs::rename(&tmp, &zip)?;
    let mut h = Sha256::new();
    let mut f = std::fs::File::open(&zip)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut bytes = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        h.update(&buf[..n]);
    }
    let rec = Record { list: hash.to_string(), plugins, zip_sha256: format!("{:x}", h.finalize()), zip_bytes: bytes };
    std::fs::write(root.join(RECORD), serde_json::to_vec_pretty(&rec)?)?;
    // The sha256sum line the VPS checks the carried zip against.
    std::fs::write(root.join(format!("{ZIP_NAME}.sha256")), format!("{}  {ZIP_NAME}\n", rec.zip_sha256))?;
    Ok(rec)
}

/// Clears what a previous export left (a changed list starts over), keeping
/// the downloads folder so finished downloads aren't fetched again.
pub fn start_over(root: &Path) -> Result<()> {
    for p in ["Data", "unpacked"] {
        let d = root.join(p);
        if d.exists() {
            std::fs::remove_dir_all(&d)?;
        }
    }
    for f in [RECORD, ZIP_NAME] {
        let _ = std::fs::remove_file(root.join(f));
    }
    let _ = std::fs::remove_file(root.join(format!("{ZIP_NAME}.sha256")));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(json: &str) -> ServerLane {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn reads_the_served_list_and_checks_pins() {
        let l = lane(r#"{"_note":"draft","for_discord_id":"1","mods":[{"id":"jks","name":"JK's Skyrim","nexus":{"mod":6289,"file":1},"plugins":["JKs Skyrim.esp"]}]}"#);
        assert_eq!(l.mods[0].plugins, vec!["JKs Skyrim.esp"]);
        assert_eq!(l.mods[0].entry.nexus.as_ref().unwrap().file, Some(1));
        check(&l).unwrap();
        let unpinned = lane(r#"{"for_discord_id":"1","mods":[{"id":"x","name":"X","nexus":{"mod":2}}]}"#);
        assert!(check(&unpinned).unwrap_err().to_string().contains("pinned"));
        let twice = lane(r#"{"for_discord_id":"1","mods":[{"id":"a","name":"A","nexus":{"mod":2,"file":3},"plugins":["P.esp"]},{"id":"b","name":"B","nexus":{"mod":4,"file":5},"plugins":["p.ESP"]}]}"#);
        assert!(check(&twice).is_err());
        let path = lane(r#"{"for_discord_id":"1","mods":[{"id":"a","name":"A","nexus":{"mod":2,"file":3},"plugins":["../x.esp"]}]}"#);
        assert!(check(&path).is_err());
        let bad_id = lane(r#"{"for_discord_id":"1","mods":[{"id":"../a","name":"A","nexus":{"mod":2,"file":3}}]}"#);
        assert!(check(&bad_id).is_err());
    }

    #[test]
    fn named_plugins_come_from_anywhere_in_the_archive() {
        let t = tempfile::tempdir().unwrap();
        let u = t.path().join("u");
        std::fs::create_dir_all(u.join("Patches/COTN")).unwrap();
        std::fs::create_dir_all(u.join("Patches/Other")).unwrap();
        std::fs::write(u.join("Main.esp"), b"main").unwrap();
        std::fs::write(u.join("Patches/COTN/Patch A.esp"), b"a").unwrap();
        std::fs::write(u.join("Patches/Other/Patch B.esp"), b"b1").unwrap();
        std::fs::create_dir_all(u.join("Patches/More")).unwrap();
        std::fs::write(u.join("Patches/More/Patch B.esp"), b"b2").unwrap();
        let mut m = lane(r#"{"for_discord_id":"1","mods":[{"id":"ocw","name":"OCW","nexus":{"mod":1,"file":2}}]}"#).mods.remove(0);
        let all = pick(&m, &u).unwrap();
        assert_eq!(all.iter().map(|p| p.1.as_str()).collect::<Vec<_>>(), vec!["Main.esp"]);
        m.plugins = vec!["patch a.esp".into()];
        let a = pick(&m, &u).unwrap();
        assert!(a[0].0.ends_with("Patches/COTN/Patch A.esp"));
        // The list's spelling names the file on the server.
        assert_eq!(a[0].1, "patch a.esp");
        m.plugins = vec!["Patch B.esp".into()];
        assert!(pick(&m, &u).unwrap_err().to_string().contains("different files"));
        m.plugins = vec!["Missing.esp".into()];
        assert!(pick(&m, &u).is_err());
    }

    #[test]
    fn zips_the_plugins_and_remembers_the_list() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join(LANE_DIR);
        let src = t.path().join("a.esp");
        std::fs::write(&src, b"plugin bytes").unwrap();
        let mut seen = BTreeMap::new();
        collect(&root, "jks", &[(src.clone(), "JKs Skyrim.esp".into())], &mut seen).unwrap();
        assert!(collect(&root, "other", &[(src, "jks skyrim.esp".into())], &mut seen).is_err());
        let l = lane(r#"{"for_discord_id":"1","mods":[]}"#);
        let h = list_hash(&l);
        assert!(!done(&root, &h));
        let rec = finish(&root, &h, seen).unwrap();
        assert!(done(&root, &h));
        assert!(!done(&root, "other list"));
        let z = zip::ZipArchive::new(std::fs::File::open(root.join(ZIP_NAME)).unwrap()).unwrap();
        assert_eq!(z.file_names().collect::<Vec<_>>(), vec!["Data/JKs Skyrim.esp"]);
        assert_eq!(std::fs::read_to_string(root.join("server-lane.zip.sha256")).unwrap(), format!("{}  server-lane.zip\n", rec.zip_sha256));
        start_over(&root).unwrap();
        assert!(!done(&root, &h) && !root.join("Data").exists());
    }
}
