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

/// The game's screen height from SkyrimPrefs.ini ([Display] iSize H).
pub fn screen_height(documents: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(documents.join("My Games").join("Skyrim Special Edition").join("SkyrimPrefs.ini")).ok()?;
    let mut display = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            display = l.eq_ignore_ascii_case("[display]");
        } else if display {
            if let Some((k, v)) = l.split_once('=') {
                if k.trim().eq_ignore_ascii_case("iSize H") {
                    return v.trim().parse().ok();
                }
            }
        }
    }
    None
}

/// Which Black Screen and Startup Fix preset fits a screen: its 1440p file
/// from 1440 pixels high up, else the 1080p one.
pub fn preset_for(height: Option<u32>) -> &'static str {
    if height.unwrap_or(0) >= 1440 {
        "1440"
    } else {
        "1080"
    }
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

/// Skyrim.ini and its `.base` copy, where `bAlwaysActive` belongs.
pub fn skyrim_ini_paths(documents: &Path) -> Vec<PathBuf> {
    let dir = documents.join("My Games").join("Skyrim Special Edition");
    ["Skyrim.ini", "Skyrim.ini.base"].iter().map(|n| dir.join(n)).collect()
}

/// The text with `bAlwaysActive=1` under `[General]`: the game keeps running
/// when its window loses focus, so a shared world never stops for one player
/// alt-tabbing. None when the text already says so.
pub fn with_always_active(text: &str) -> Option<String> {
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut general: Option<usize> = None;
    let mut in_general = false;
    for i in 0..lines.len() {
        let l = lines[i].trim().to_string();
        if l.starts_with('[') {
            in_general = l.eq_ignore_ascii_case("[general]");
            if in_general {
                general = Some(i);
            }
        } else if in_general {
            if let Some((k, v)) = l.split_once('=') {
                if k.trim().eq_ignore_ascii_case("balwaysactive") {
                    if v.trim() == "1" {
                        return None;
                    }
                    lines[i] = "bAlwaysActive=1".to_string();
                    return Some(joined(&lines, nl, text));
                }
            }
        }
    }
    match general {
        Some(i) => lines.insert(i + 1, "bAlwaysActive=1".to_string()),
        None => {
            lines.push("[General]".to_string());
            lines.push("bAlwaysActive=1".to_string());
        }
    }
    Some(joined(&lines, nl, text))
}

fn joined(lines: &[String], nl: &str, original: &str) -> String {
    let mut s = lines.join(nl);
    if original.is_empty() || original.ends_with('\n') || !original.contains('\n') {
        s.push_str(nl);
    }
    s
}

/// Writes `bAlwaysActive=1` into one ini file; the first time it changes a
/// file it keeps the original next to it as `<name>.aetherial-dawn-backup`.
pub fn ensure_always_active(ini: &Path) -> Result<bool> {
    let Ok(text) = std::fs::read_to_string(ini) else { return Ok(false) };
    let Some(new) = with_always_active(&text) else { return Ok(false) };
    let backup = ini.with_file_name(format!("{}.aetherial-dawn-backup", ini.file_name().unwrap().to_string_lossy()));
    if !backup.exists() {
        crate::loadorder::atomic_write(&backup, text.as_bytes())?;
    }
    crate::loadorder::atomic_write(ini, new.as_bytes())?;
    Ok(true)
}

/// One hot-fixable setting from launcher-rules.json (`tune.settings`).
#[derive(Debug, Clone, PartialEq)]
pub struct Tune {
    pub file: String,
    pub section: String,
    pub key: String,
    pub mode: TuneMode,
    pub value: f64,
}

/// `Set` writes the value (adding the line when missing); `Max` only lowers a
/// number above the value and `Min` only raises one below it. Max and Min
/// leave a missing line alone, so a setting the player never had stays unset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuneMode {
    Set,
    Max,
    Min,
    /// Sets the value to 1 / the player's own `FramerateLimit` (SSEDisplayTweaks.ini);
    /// skipped when no limit is set. The rules file's `value` is ignored.
    Framerate,
}

/// The settings the rules file may touch: (file, section, key, integer, lowest, highest).
/// Anything else, and any value outside its range, is ignored.
pub const TUNABLE: [(&str, &str, &str, bool, f64, f64); 6] = [
    ("SkyrimPrefs.ini", "Display", "fShadowDistance", false, 1000.0, 20000.0),
    ("SkyrimPrefs.ini", "Display", "iShadowMapResolution", true, 512.0, 4096.0),
    ("Skyrim.ini", "General", "fPostLoadUpdateTimeMS", false, 100.0, 2000.0),
    ("Skyrim.ini", "General", "uGridsToLoad", true, 3.0, 7.0),
    // Havok's longest physics step; 1 / frame cap (Framerate mode), 30 to 240 fps.
    ("Skyrim.ini", "HAVOK", "fMaxTime", false, 0.004, 0.034),
    // SSE Display Tweaks' frame cap, in Data/SKSE/Plugins (the game folder, not Documents).
    ("SSEDisplayTweaks.ini", "Render", "FramerateLimit", true, 30.0, 240.0),
];

impl Tune {
    /// A setting from the rules file when it names a tunable key, in range, with a known mode.
    pub fn checked(file: &str, section: &str, key: &str, mode: &str, value: f64) -> Option<Tune> {
        let (f, sec, k, int, lo, hi) = TUNABLE.iter().find(|t| t.0.eq_ignore_ascii_case(file) && t.1.eq_ignore_ascii_case(section) && t.2.eq_ignore_ascii_case(key))?;
        let mode = match mode {
            "set" => TuneMode::Set,
            "max" => TuneMode::Max,
            "min" => TuneMode::Min,
            "framerate" if key.eq_ignore_ascii_case("fMaxTime") => TuneMode::Framerate,
            _ => return None,
        };
        let value = if mode == TuneMode::Framerate { *lo } else { value };
        let ok = value.is_finite() && value >= *lo && value <= *hi && (!*int || value.fract() == 0.0);
        ok.then(|| Tune { file: f.to_string(), section: sec.to_string(), key: k.to_string(), mode, value })
    }

    fn text(&self) -> String {
        if self.value.fract() == 0.0 {
            format!("{}", self.value as i64)
        } else {
            format!("{}", self.value)
        }
    }
}

/// The text with the tunes for `file` applied, plus one line per change.
/// None when nothing changes.
pub fn with_tunes(text: &str, file: &str, tunes: &[Tune]) -> Option<(String, Vec<String>)> {
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut changes = Vec::new();
    for t in tunes.iter().filter(|t| t.file.eq_ignore_ascii_case(file)) {
        let (mut section_at, mut line_at, mut in_sec) = (None, None, false);
        for (i, l) in lines.iter().enumerate() {
            let l = l.trim();
            if l.starts_with('[') {
                in_sec = l.trim_matches(|c| c == '[' || c == ']').eq_ignore_ascii_case(&t.section);
                if in_sec && section_at.is_none() {
                    section_at = Some(i);
                }
            } else if in_sec && line_at.is_none() {
                if let Some((k, _)) = l.split_once('=') {
                    if k.trim().eq_ignore_ascii_case(&t.key) {
                        line_at = Some(i);
                    }
                }
            }
        }
        let new_line = format!("{}={}", t.key, t.text());
        match line_at {
            Some(i) => {
                let old = lines[i].split_once('=').map(|(_, v)| v.trim().to_string()).unwrap_or_default();
                let cur: Option<f64> = old.parse().ok();
                let write = match (t.mode, cur) {
                    (TuneMode::Set, Some(c)) => (c - t.value).abs() > f64::EPSILON,
                    (TuneMode::Set, None) => true,
                    (TuneMode::Max, Some(c)) => c > t.value,
                    (TuneMode::Min, Some(c)) => c < t.value,
                    (TuneMode::Framerate, _) => false,
                    // A value that isn't a number is left alone by Max and Min.
                    _ => false,
                };
                if write {
                    lines[i] = new_line;
                    changes.push(format!("{}: {} -> {}", t.key, old, t.text()));
                }
            }
            None if t.mode == TuneMode::Set => {
                match section_at {
                    Some(i) => lines.insert(i + 1, new_line),
                    None => {
                        lines.push(format!("[{}]", t.section));
                        lines.push(new_line);
                    }
                }
                changes.push(format!("{}: (not set) -> {}", t.key, t.text()));
            }
            None => {}
        }
    }
    (!changes.is_empty()).then(|| (joined(&lines, nl, text), changes))
}

/// The player's `FramerateLimit` from SSEDisplayTweaks.ini ([Render]); None when
/// unset, commented out, zero or outside 30 to 240.
pub fn framerate_limit(text: &str) -> Option<f64> {
    let mut render = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            render = l.eq_ignore_ascii_case("[render]");
        } else if render && !l.starts_with(';') && !l.starts_with('#') {
            if let Some((k, v)) = l.split_once('=') {
                if k.trim().eq_ignore_ascii_case("FramerateLimit") {
                    let n: f64 = v.split([';', '#']).next()?.trim().parse().ok()?;
                    return (30.0..=240.0).contains(&n).then_some(n);
                }
            }
        }
    }
    None
}

/// Turns Framerate-mode tunes into plain `Set` ones using the player's frame
/// cap (1 / limit, 7 decimals); with no usable cap they are dropped.
pub fn resolve_framerate(tunes: &[Tune], limit: Option<f64>) -> Vec<Tune> {
    tunes
        .iter()
        .filter_map(|t| {
            if t.mode != TuneMode::Framerate {
                return Some(t.clone());
            }
            let v = (1.0 / limit?) * 1e7;
            Some(Tune { mode: TuneMode::Set, value: v.round() / 1e7, ..t.clone() })
        })
        .collect()
}

/// Applies the tunes to one ini file in the Skyrim documents folder, keeping
/// the first original as `<name>.aetherial-dawn-backup`. Returns the changes.
pub fn apply_tunes(ini: &Path, tunes: &[Tune]) -> Result<Vec<String>> {
    let Ok(text) = std::fs::read_to_string(ini) else { return Ok(Vec::new()) };
    let name = ini.file_name().unwrap().to_string_lossy().to_string();
    let Some((new, changes)) = with_tunes(&text, &name, tunes) else { return Ok(Vec::new()) };
    let backup = ini.with_file_name(format!("{name}.aetherial-dawn-backup"));
    if !backup.exists() {
        crate::loadorder::atomic_write(&backup, text.as_bytes())?;
    }
    crate::loadorder::atomic_write(ini, new.as_bytes())?;
    Ok(changes)
}

/// Cleans one ini file in place. The first time it changes a file it keeps the
/// original next to it as `<name>.aetherial-dawn-backup`.
pub fn repair(ini: &Path, data_dir: &Path) -> Result<Repair> {
    let Ok(text) = std::fs::read_to_string(ini) else { return Ok(Repair::default()) };
    let (new, rep) = clean(&text, data_dir);
    if !rep.is_empty() {
        let backup = ini.with_file_name(format!("{}.aetherial-dawn-backup", ini.file_name().unwrap().to_string_lossy()));
        if !backup.exists() {
            crate::loadorder::atomic_write(&backup, text.as_bytes())?;
        }
        crate::loadorder::atomic_write(ini, new.as_bytes())?;
    }
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_active_is_added_under_general_without_touching_the_rest() {
        let text = "[Archive]\r\nsResourceArchiveList=a.bsa\r\n[General]\r\nsLanguage=ENGLISH\r\n[Display]\r\niSize H=1080\r\n";
        let new = with_always_active(text).unwrap();
        assert_eq!(new, "[Archive]\r\nsResourceArchiveList=a.bsa\r\n[General]\r\nbAlwaysActive=1\r\nsLanguage=ENGLISH\r\n[Display]\r\niSize H=1080\r\n");
        assert_eq!(with_always_active(&new), None, "already set: nothing to write");
    }

    #[test]
    fn always_active_is_corrected_or_the_section_made() {
        assert_eq!(with_always_active("[General]\nbAlwaysActive=0\nx=1\n").unwrap(), "[General]\nbAlwaysActive=1\nx=1\n");
        assert_eq!(with_always_active("[Display]\na=1\n").unwrap(), "[Display]\na=1\n[General]\nbAlwaysActive=1\n");
        assert_eq!(with_always_active("").unwrap(), "[General]\nbAlwaysActive=1\n");
        // the same key in another section does not count
        assert!(with_always_active("[Other]\nbAlwaysActive=1\n[General]\n").is_some());
    }

    #[test]
    fn always_active_is_written_to_the_file_and_the_original_kept() {
        let d = tempfile::tempdir().unwrap();
        let ini = d.path().join("Skyrim.ini");
        std::fs::write(&ini, "[General]\nsLanguage=ENGLISH\n").unwrap();
        assert!(ensure_always_active(&ini).unwrap());
        assert!(std::fs::read_to_string(&ini).unwrap().contains("bAlwaysActive=1"));
        assert_eq!(std::fs::read_to_string(d.path().join("Skyrim.ini.aetherial-dawn-backup")).unwrap(), "[General]\nsLanguage=ENGLISH\n");
        assert!(!ensure_always_active(&ini).unwrap());
        assert!(!ensure_always_active(&d.path().join("missing.ini")).unwrap());
    }

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

    #[test]
    fn tunes_only_lower_raise_or_set_inside_their_range() {
        let t = |k, m, v| {
            let f = if k == "fPostLoadUpdateTimeMS" { "Skyrim.ini" } else { "SkyrimPrefs.ini" };
            let sec = if f == "Skyrim.ini" { "General" } else { "Display" };
            Tune::checked(f, sec, k, m, v).unwrap()
        };
        assert!(Tune::checked("SkyrimPrefs.ini", "Display", "bFull Screen", "set", 0.0).is_none());
        assert!(Tune::checked("SkyrimPrefs.ini", "Display", "fShadowDistance", "set", 50.0).is_none());
        assert!(Tune::checked("SkyrimPrefs.ini", "Display", "iShadowMapResolution", "set", 1000.5).is_none());
        assert!(Tune::checked("SkyrimPrefs.ini", "Display", "fShadowDistance", "wild", 8000.0).is_none());
        let prefs = "[Display]\r\nfShadowDistance=10000.0000\r\niShadowMapResolution=1024\r\n";
        let tunes = [t("fShadowDistance", "max", 8000.0), t("iShadowMapResolution", "max", 2048.0)];
        let (out, ch) = with_tunes(prefs, "SkyrimPrefs.ini", &tunes).unwrap();
        assert_eq!(out, "[Display]\r\nfShadowDistance=8000\r\niShadowMapResolution=1024\r\n");
        assert_eq!(ch.len(), 1);
        assert!(with_tunes(&out, "SkyrimPrefs.ini", &tunes).is_none());
        // Max and Min leave a missing line alone; Set adds it, and the section if needed.
        assert!(with_tunes("[Display]\n", "SkyrimPrefs.ini", &tunes).is_none());
        let (out, _) = with_tunes("[Other]\nx=1\n", "Skyrim.ini", &[t("fPostLoadUpdateTimeMS", "set", 500.0)]).unwrap();
        assert_eq!(out, "[Other]\nx=1\n[General]\nfPostLoadUpdateTimeMS=500\n");
        // Another file's tunes do nothing here.
        assert!(with_tunes(prefs, "Skyrim.ini", &tunes).is_none());
    }

    #[test]
    fn tunes_are_written_to_the_file_and_the_original_kept() {
        let d = tempfile::tempdir().unwrap();
        let ini = d.path().join("SkyrimPrefs.ini");
        std::fs::write(&ini, "[Display]\nfShadowDistance=10000\n").unwrap();
        let tunes = [Tune::checked("SkyrimPrefs.ini", "Display", "fShadowDistance", "max", 8000.0).unwrap()];
        assert_eq!(apply_tunes(&ini, &tunes).unwrap().len(), 1);
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), "[Display]\nfShadowDistance=8000\n");
        assert_eq!(std::fs::read_to_string(d.path().join("SkyrimPrefs.ini.aetherial-dawn-backup")).unwrap(), "[Display]\nfShadowDistance=10000\n");
        assert!(apply_tunes(&d.path().join("missing.ini"), &tunes).unwrap().is_empty());
    }

    #[test]
    fn picks_the_preset_for_the_screen() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("My Games/Skyrim Special Edition");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SkyrimPrefs.ini"), "[Display]\r\niSize W=2560\r\niSize H=1440\r\n[Grass]\r\niSize H=5\r\n").unwrap();
        assert_eq!(screen_height(t.path()), Some(1440));
        assert_eq!(preset_for(screen_height(t.path())), "1440");
        assert_eq!(preset_for(Some(1080)), "1080");
        assert_eq!(preset_for(None), "1080");
    }

    #[test]
    fn havok_step_follows_the_players_frame_cap() {
        assert_eq!(framerate_limit("[Render]\r\nEnableVSync=true\r\nFramerateLimit=119 ; cap\r\n"), Some(119.0));
        assert_eq!(framerate_limit("[Render]\n;FramerateLimit=119\n"), None);
        assert_eq!(framerate_limit("[Render]\nFramerateLimit=0\n"), None);
        assert_eq!(framerate_limit("[Other]\nFramerateLimit=60\n"), None);
        let t = Tune::checked("Skyrim.ini", "HAVOK", "fMaxTime", "framerate", 0.0).unwrap();
        assert!(Tune::checked("Skyrim.ini", "General", "uGridsToLoad", "framerate", 5.0).is_none());
        assert!(resolve_framerate(std::slice::from_ref(&t), None).is_empty());
        let r = resolve_framerate(&[t], Some(119.0));
        assert_eq!(r[0].mode, TuneMode::Set);
        let (out, ch) = with_tunes("[General]\r\nbAlwaysActive=1\r\n", "Skyrim.ini", &r).unwrap();
        assert!(out.contains("[HAVOK]\r\nfMaxTime=0.0084034"), "{out}");
        assert_eq!(ch.len(), 1);
        assert!(with_tunes(&out, "Skyrim.ini", &r).is_none());
    }

    #[test]
    fn frame_cap_is_a_tunable_display_tweaks_key() {
        let t = Tune::checked("SSEDisplayTweaks.ini", "Render", "FramerateLimit", "set", 60.0).unwrap();
        assert!(Tune::checked("SSEDisplayTweaks.ini", "Render", "FramerateLimit", "set", 500.0).is_none());
        let (out, _) = with_tunes("[Render]\nEnableVSync=true\n;FramerateLimit=0\n", "SSEDisplayTweaks.ini", std::slice::from_ref(&t)).unwrap();
        assert!(out.contains("[Render]\nFramerateLimit=60\n"), "{out}");
        assert_eq!(framerate_limit(&out), Some(60.0));
        assert!(with_tunes(&out, "SSEDisplayTweaks.ini", std::slice::from_ref(&t)).is_none());
        assert!(with_tunes("[General]\n", "Skyrim.ini", std::slice::from_ref(&t)).is_none());
    }
}
