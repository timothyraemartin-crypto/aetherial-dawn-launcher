//! Cleans the archive lists in the player's Skyrim.ini. Mod managers add their
//! mods' BSAs to sResourceArchiveList2 and often leave them behind when the mod
//! goes. In the first live test the list named 43 archives that no longer
//! existed, plus the vanilla ones twice, and Skyrim crashed while loading data.

use std::path::{Path, PathBuf};

use crate::Result;

const KEYS: [&str; 2] = ["sresourcearchivelist", "sresourcearchivelist2"];

/// What a repair changed.
#[derive(Debug, Default, PartialEq)]
pub struct Repair {
    pub missing: Vec<String>,
    pub duplicates: Vec<String>,
}

impl Repair {
    pub fn is_empty(&self) -> bool {
        self.missing.is_empty() && self.duplicates.is_empty()
    }
}

/// Skyrim.ini and SkyrimPrefs.ini in Documents\My Games\Skyrim Special Edition.
pub fn ini_paths(documents: &Path) -> Vec<PathBuf> {
    let dir = documents.join("My Games").join("Skyrim Special Edition");
    ["Skyrim.ini", "SkyrimPrefs.ini", "SkyrimCustom.ini"].iter().map(|n| dir.join(n)).collect()
}

/// Rewrites the text, dropping archives that aren't in `data_dir` and repeats
/// (across both lists, first one wins).
pub fn clean(text: &str, data_dir: &Path) -> (String, Repair) {
    let present: Vec<String> = std::fs::read_dir(data_dir)
        .map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().to_ascii_lowercase()).collect())
        .unwrap_or_default();
    let mut seen: Vec<String> = Vec::new();
    let mut rep = Repair::default();
    let mut out = Vec::new();
    for line in text.lines() {
        let key = line.split('=').next().unwrap_or("").trim().to_ascii_lowercase();
        let Some((k, v)) = line.split_once('=').filter(|_| KEYS.contains(&key.as_str())) else {
            out.push(line.to_string());
            continue;
        };
        let mut keep = Vec::new();
        for name in v.split(',').map(str::trim).filter(|n| !n.is_empty()) {
            let l = name.to_ascii_lowercase();
            if !present.contains(&l) {
                rep.missing.push(name.to_string());
            } else if seen.contains(&l) {
                rep.duplicates.push(name.to_string());
            } else {
                seen.push(l);
                keep.push(name);
            }
        }
        out.push(format!("{k}={}", keep.join(", ")));
    }
    let mut s = out.join(if text.contains("\r\n") { "\r\n" } else { "\n" });
    if text.ends_with('\n') {
        s.push_str(if text.contains("\r\n") { "\r\n" } else { "\n" });
    }
    (s, rep)
}

/// Cleans one ini file in place. The first time it changes a file it keeps the
/// original next to it as `<name>.aetherial-dawn-backup`.
pub fn repair(ini: &Path, data_dir: &Path) -> Result<Repair> {
    let Ok(text) = std::fs::read_to_string(ini) else { return Ok(Repair::default()) };
    let (new, rep) = clean(&text, data_dir);
    if !rep.is_empty() {
        let backup = ini.with_file_name(format!("{}.aetherial-dawn-backup", ini.file_name().unwrap().to_string_lossy()));
        if !backup.exists() {
            std::fs::write(&backup, &text)?;
        }
        std::fs::write(ini, new)?;
    }
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_missing_and_repeated_archives() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("Data");
        std::fs::create_dir_all(&data).unwrap();
        for f in ["Skyrim - Misc.bsa", "Skyrim - Textures0.bsa", "ccBGSSSE001-Fish.bsa"] {
            std::fs::write(data.join(f), b"").unwrap();
        }
        let ini = tmp.path().join("Skyrim.ini");
        std::fs::write(
            &ini,
            "[General]\r\nsLanguage=ENGLISH\r\n[Archive]\r\nsResourceArchiveList=Skyrim - Misc.bsa\r\nsResourceArchiveList2=Skyrim - Textures0.bsa, ccbgssse001-fish.bsa, Skyrim - Misc.bsa, SkyUI_SE.bsa, JK's Riften Outskirts.bsa\r\nbInvalidateOlderFiles=1\r\n",
        )
        .unwrap();
        let rep = repair(&ini, &data).unwrap();
        assert_eq!(rep.missing, ["SkyUI_SE.bsa", "JK's Riften Outskirts.bsa"]);
        assert_eq!(rep.duplicates, ["Skyrim - Misc.bsa"]);
        assert_eq!(
            std::fs::read_to_string(&ini).unwrap(),
            "[General]\r\nsLanguage=ENGLISH\r\n[Archive]\r\nsResourceArchiveList=Skyrim - Misc.bsa\r\nsResourceArchiveList2=Skyrim - Textures0.bsa, ccbgssse001-fish.bsa\r\nbInvalidateOlderFiles=1\r\n"
        );
        assert!(tmp.path().join("Skyrim.ini.aetherial-dawn-backup").exists());
        assert!(repair(&ini, &data).unwrap().is_empty());
    }
}
