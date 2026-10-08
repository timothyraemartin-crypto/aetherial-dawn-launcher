//! The server's camera: SmoothCam's "Modern Camera Preset" (Nexus 41636,
//! Timothy 2026-09-26: "this is the mod I'm going to use"). The preset mod
//! only drops a preset file in one of SmoothCam's slots; loading it in the
//! menu replaces SmoothCam.json with the preset's settings. The launcher does
//! that once, the same way, so every player starts with it: the old
//! SmoothCam.json is copied to the backup folder first, and a marker stops it
//! running again, so later tweaks in Mod Configuration are never undone.
//!
//! 0.1.53 briefly applied a hand-made Souls-style preset instead (SmoothCam
//! and True Directional Movement); where it ran, its TDM change is undone.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Bump to apply a changed preset once more (after backing up again).
/// Version 1 was 0.1.53's hand-made preset.
pub const PRESET_VERSION: u32 = 2;
const MARKER: &str = ".aetherial-dawn/mods/camera-preset.json";
pub const SMOOTHCAM_JSON: &str = "Data/SKSE/Plugins/SmoothCam.json";
pub const TDM_INI: &str = "Data/MCM/Settings/TrueDirectionalMovement.ini";

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Marker {
    version: u32,
    when: u64,
    files: Vec<String>,
    #[serde(default)]
    preset: Option<String>,
}

fn marker(game_dir: &Path) -> Option<Marker> {
    std::fs::read(game_dir.join(MARKER)).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// When the preset was applied (seconds since 1970), if it has been.
pub fn applied(game_dir: &Path) -> Option<u64> {
    marker(game_dir).filter(|m| m.version >= PRESET_VERSION).map(|m| m.when)
}

/// The Modern Camera Preset's file and its settings, in whichever of
/// SmoothCam's preset slots it sits.
pub fn find_modern(game_dir: &Path) -> Option<(String, Value)> {
    let dir = game_dir.join("Data").join("SKSE").join("Plugins");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            n.starts_with("smoothcampreset") && n.ends_with(".json")
        })
        .collect();
    files.sort();
    files.into_iter().find_map(|p| {
        let v: Value = serde_json::from_slice(&std::fs::read(&p).ok()?).ok()?;
        let name = v.get("name")?.as_str()?.to_ascii_lowercase();
        let config = v.get("config").filter(|c| c.is_object())?.clone();
        name.contains("modern").then(|| (p.file_name().unwrap().to_string_lossy().into_owned(), config))
    })
}

/// Copies an existing settings file into `.aetherial-dawn/disabled/<stamp>/`.
fn back_up(game_dir: &Path, rel: &str, stamp: &str) -> std::io::Result<Option<PathBuf>> {
    let from = game_dir.join(rel);
    if !from.is_file() {
        return Ok(None);
    }
    let to = game_dir.join(crate::strays::DISABLED_DIR).join(stamp).join(rel);
    if let Some(p) = to.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::copy(&from, &to)?;
    Ok(Some(to))
}

/// Undoes 0.1.53's True Directional Movement change: puts back the file it
/// backed up, or removes the one it created.
fn undo_v1(game_dir: &Path, m: &Marker) -> std::io::Result<()> {
    let backup = game_dir.join(crate::strays::DISABLED_DIR).join(format!("{}-camera-preset", m.when)).join(TDM_INI);
    let tdm = game_dir.join(TDM_INI);
    if backup.is_file() {
        std::fs::copy(&backup, &tdm)?;
    } else if tdm.is_file() {
        std::fs::remove_file(&tdm)?;
    }
    Ok(())
}

/// Makes the Modern Camera Preset SmoothCam's settings, once. Returns the
/// preset file it used, or None when it already ran or the preset isn't
/// installed yet.
pub fn apply_once(game_dir: &Path) -> std::io::Result<Option<String>> {
    let old = marker(game_dir);
    if old.as_ref().map(|m| m.version >= PRESET_VERSION).unwrap_or(false) {
        return Ok(None);
    }
    let Some((file, config)) = find_modern(game_dir) else { return Ok(None) };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let stamp = format!("{now}-camera-preset");
    back_up(game_dir, SMOOTHCAM_JSON, &stamp)?;
    if let Some(m) = old.as_ref().filter(|m| m.version == 1) {
        back_up(game_dir, TDM_INI, &stamp)?;
        undo_v1(game_dir, m)?;
    }
    // Loading a preset in SmoothCam's menu replaces all its settings.
    let sc = game_dir.join(SMOOTHCAM_JSON);
    if let Some(p) = sc.parent() {
        std::fs::create_dir_all(p)?;
    }
    crate::atomicfile::safe_write(&sc, serde_json::to_string_pretty(&config)?.as_bytes())?;

    let m = Marker { version: PRESET_VERSION, when: now, files: vec![SMOOTHCAM_JSON.to_string()], preset: Some(file.clone()) };
    let mp = game_dir.join(MARKER);
    if let Some(p) = mp.parent() {
        std::fs::create_dir_all(p)?;
    }
    crate::atomicfile::write(&mp, &serde_json::to_vec_pretty(&m)?)?;
    Ok(Some(file))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset(g: &Path, slot: u32, name: &str, follow: f64) {
        let p = g.join(format!("Data/SKSE/Plugins/SmoothCamPreset{slot}.json"));
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, serde_json::json!({"name": name, "config": {"minCameraFollowRate": follow, "standing": {"sideOffset": 40.0}}}).to_string()).unwrap();
    }

    #[test]
    fn loads_the_modern_preset_once() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        assert_eq!(apply_once(g).unwrap(), None, "waits until the preset is installed");
        preset(g, 1, "My Own", 0.1);
        preset(g, 3, "ModernPreset", 0.5);
        std::fs::write(g.join(SMOOTHCAM_JSON), r#"{"minCameraFollowRate": 0.25}"#).unwrap();

        assert_eq!(apply_once(g).unwrap().as_deref(), Some("SmoothCamPreset3.json"));
        let v: Value = serde_json::from_slice(&std::fs::read(g.join(SMOOTHCAM_JSON)).unwrap()).unwrap();
        assert_eq!(v["minCameraFollowRate"], 0.5);
        assert_eq!(v["standing"]["sideOffset"], 40.0);
        let aside: Vec<_> = std::fs::read_dir(g.join(crate::strays::DISABLED_DIR)).unwrap().flatten().map(|e| e.path()).collect();
        assert!(std::fs::read_to_string(aside[0].join(SMOOTHCAM_JSON)).unwrap().contains("0.25"));

        // The player's later change survives the next Play.
        std::fs::write(g.join(SMOOTHCAM_JSON), r#"{"minCameraFollowRate": 0.1}"#).unwrap();
        assert_eq!(apply_once(g).unwrap(), None);
        assert!(std::fs::read_to_string(g.join(SMOOTHCAM_JSON)).unwrap().contains("0.1"));
        assert!(applied(g).is_some());
    }

    #[test]
    fn undoes_the_hand_made_preset() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        preset(g, 3, "ModernPreset", 0.5);
        std::fs::create_dir_all(g.join("Data/MCM/Settings")).unwrap();
        std::fs::write(g.join(TDM_INI), "[TargetLock]\nuTargetLockMode = 0\n").unwrap();
        std::fs::create_dir_all(g.join(".aetherial-dawn/mods")).unwrap();
        std::fs::write(g.join(MARKER), r#"{"version":1,"when":5,"files":[]}"#).unwrap();
        apply_once(g).unwrap();
        assert!(!g.join(TDM_INI).exists(), "0.1.53 created it, so it goes");
    }
}
