//! The server's Souls-style camera (Timothy, 2026-09-26: "adjust the smooth
//! cam settings to feel more like darksouls"). Applied once per game folder:
//! SmoothCam's own settings file gets closer, right-shoulder framing, a
//! snappier follow and no crosshair in melee; True Directional Movement's
//! MCM file gets directional movement and a hard target lock. Only the keys
//! below change; the player's other settings stay, the old files are copied
//! to the backup folder first, and a marker stops it running again, so
//! later tweaks in Mod Configuration are never undone.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// Bump to apply a changed preset once more (after backing up again).
pub const PRESET_VERSION: u32 = 1;
const MARKER: &str = ".aetherial-dawn/mods/camera-preset.json";
pub const SMOOTHCAM_JSON: &str = "Data/SKSE/Plugins/SmoothCam.json";
pub const TDM_INI: &str = "Data/MCM/Settings/TrueDirectionalMovement.ini";

/// One SmoothCam state: over the right shoulder, a little lower; melee pulls
/// toward the middle so a locked target stays framed, aiming stays wide.
fn group(side: f64, up: f64) -> Value {
    json!({
        "sideOffset": side, "upOffset": up,
        "combatMeleeSideOffset": side * 0.6, "combatMeleeUpOffset": up,
        "combatMagicSideOffset": side + 10.0, "combatMagicUpOffset": up,
        "combatRangedSideOffset": side + 15.0, "combatRangedUpOffset": up,
    })
}

/// The SmoothCam keys the preset sets (names from SmoothCam 1.7's config).
pub fn smoothcam_preset() -> Value {
    json!({
        // Snappy follow: the camera catches up fast and lags only a little.
        "enableInterp": true,
        "minCameraFollowDistance": 40.0,
        "minCameraFollowRate": 0.45,
        "maxCameraFollowRate": 0.9,
        "zoomMaxSmoothingDistance": 400.0,
        "separateLocalInterp": true,
        "localMinFollowRate": 0.8,
        "localMaxFollowRate": 1.0,
        "separateZInterp": true,
        "separateZMinFollowRate": 0.6,
        "separateZMaxFollowRate": 1.0,
        // Quick moves between exploring and combat framing.
        "enableOffsetInterpolation": true,
        "offsetInterpDurationSecs": 0.45,
        "enablePitchZoom": false,
        // No crosshair outside ranged and magic aiming.
        "hideNonCombatCrosshair": true,
        "hideCrosshairMeleeCombat": true,
        "standing": group(30.0, -5.0),
        "walking": group(30.0, -5.0),
        "running": group(30.0, -5.0),
        "sprinting": group(22.0, -5.0),
        "sneaking": group(30.0, -10.0),
    })
}

/// The True Directional Movement keys the preset sets, by section.
pub const TDM_PRESET: [(&str, &str, &str); 9] = [
    ("DirectionalMovement", "uDirectionalMovementSheathed", "2"),
    ("DirectionalMovement", "uDirectionalMovementDrawn", "2"),
    // The camera swings behind you while you move, as in Dark Souls.
    ("DirectionalMovement", "uAdjustCameraYawDuringMovement", "2"),
    ("TargetLock", "uTargetLockMode", "0"),
    ("TargetLock", "bTargetLockHideCrosshair", "1"),
    ("TargetLock", "bResetCameraWithTargetLock", "1"),
    ("TargetLock", "fTargetLockYawAdjustSpeed", "10"),
    ("TargetLock", "fTargetLockPitchAdjustSpeed", "3"),
    ("TargetLock", "bAutoTargetNextOnDeath", "1"),
];

/// Overlays `add` onto `base`, object by object.
fn merge(base: &mut Value, add: &Value) {
    match (base, add) {
        (Value::Object(b), Value::Object(a)) => {
            for (k, v) in a {
                match b.get_mut(k) {
                    Some(bv) if bv.is_object() && v.is_object() => merge(bv, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, a) => *b = a.clone(),
    }
}

/// Sets `key = value` under `[section]` in ini text, keeping everything else.
pub fn ini_set(text: &str, section: &str, key: &str, value: &str) -> String {
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let head = format!("[{}]", section.to_ascii_lowercase());
    let start = lines.iter().position(|l| l.trim().to_ascii_lowercase() == head);
    match start {
        None => {
            if lines.last().map(|l| !l.trim().is_empty()).unwrap_or(false) {
                lines.push(String::new());
            }
            lines.push(format!("[{section}]"));
            lines.push(format!("{key} = {value}"));
        }
        Some(s) => {
            let end = lines.iter().skip(s + 1).position(|l| l.trim_start().starts_with('[')).map(|e| s + 1 + e).unwrap_or(lines.len());
            let found = (s + 1..end).find(|&i| lines[i].split('=').next().map(|k| k.trim().eq_ignore_ascii_case(key)).unwrap_or(false));
            match found {
                Some(i) => lines[i] = format!("{key} = {value}"),
                None => {
                    // After the section's last non-blank line.
                    let mut at = end;
                    while at > s + 1 && lines[at - 1].trim().is_empty() {
                        at -= 1;
                    }
                    lines.insert(at, format!("{key} = {value}"));
                }
            }
        }
    }
    let mut out = lines.join(nl);
    out.push_str(nl);
    out
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Marker {
    version: u32,
    when: u64,
    files: Vec<String>,
}

fn marker(game_dir: &Path) -> Option<Marker> {
    std::fs::read(game_dir.join(MARKER)).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// When the preset was applied (seconds since 1970), if it has been.
pub fn applied(game_dir: &Path) -> Option<u64> {
    marker(game_dir).filter(|m| m.version >= PRESET_VERSION).map(|m| m.when)
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

/// Applies the preset once. Returns the files it changed (empty when it
/// already ran).
pub fn apply_once(game_dir: &Path) -> std::io::Result<Vec<String>> {
    if applied(game_dir).is_some() {
        return Ok(Vec::new());
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let stamp = format!("{now}-camera-preset");
    back_up(game_dir, SMOOTHCAM_JSON, &stamp)?;
    back_up(game_dir, TDM_INI, &stamp)?;

    let sc = game_dir.join(SMOOTHCAM_JSON);
    // A file SmoothCam can't read would be replaced by its defaults anyway.
    let mut cfg: Value = std::fs::read(&sc).ok().and_then(|b| serde_json::from_slice(&b).ok()).filter(Value::is_object).unwrap_or_else(|| json!({}));
    merge(&mut cfg, &smoothcam_preset());
    if let Some(p) = sc.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(&sc, serde_json::to_string_pretty(&cfg)?)?;

    let tdm = game_dir.join(TDM_INI);
    let mut text = std::fs::read_to_string(&tdm).unwrap_or_default();
    for (s, k, v) in TDM_PRESET {
        text = ini_set(&text, s, k, v);
    }
    if let Some(p) = tdm.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(&tdm, text)?;

    let files = vec![SMOOTHCAM_JSON.to_string(), TDM_INI.to_string()];
    let m = Marker { version: PRESET_VERSION, when: now, files: files.clone() };
    let mp = game_dir.join(MARKER);
    if let Some(p) = mp.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(mp, serde_json::to_vec_pretty(&m)?)?;
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_once_and_keeps_other_settings() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        std::fs::create_dir_all(g.join("Data/SKSE/Plugins")).unwrap();
        std::fs::write(g.join(SMOOTHCAM_JSON), r#"{"zoomMul": 500.0, "standing": {"fovOffset": 5.0, "sideOffset": 25.0}, "minCameraFollowRate": 0.25}"#).unwrap();
        std::fs::create_dir_all(g.join("Data/MCM/Settings")).unwrap();
        std::fs::write(g.join(TDM_INI), "[TargetLock]\r\nuTargetLockMode = 1\r\nfTargetLockDistance = 3000\r\n\r\n[Keys]\r\nuTargetLockKey = 258\r\n").unwrap();

        assert_eq!(apply_once(g).unwrap().len(), 2);
        let v: Value = serde_json::from_slice(&std::fs::read(g.join(SMOOTHCAM_JSON)).unwrap()).unwrap();
        assert_eq!(v["zoomMul"], 500.0);
        assert_eq!(v["standing"]["fovOffset"], 5.0);
        assert_eq!(v["standing"]["sideOffset"], 30.0);
        assert_eq!(v["minCameraFollowRate"], 0.45);
        let ini = std::fs::read_to_string(g.join(TDM_INI)).unwrap();
        assert!(ini.contains("uTargetLockMode = 0\r\n"));
        assert!(ini.contains("fTargetLockDistance = 3000"));
        assert!(ini.contains("uTargetLockKey = 258"));
        assert!(ini.contains("[DirectionalMovement]\r\nuDirectionalMovementSheathed = 2"));
        // The old files are in the backup folder.
        let aside: Vec<_> = std::fs::read_dir(g.join(crate::strays::DISABLED_DIR)).unwrap().flatten().map(|e| e.path()).collect();
        assert!(aside[0].join(SMOOTHCAM_JSON).is_file() && aside[0].join(TDM_INI).is_file());

        // The player's own later change survives the next Play.
        std::fs::write(g.join(SMOOTHCAM_JSON), r#"{"minCameraFollowRate": 0.1}"#).unwrap();
        assert!(apply_once(g).unwrap().is_empty());
        assert!(std::fs::read_to_string(g.join(SMOOTHCAM_JSON)).unwrap().contains("0.1"));
        assert!(applied(g).is_some());
    }

    #[test]
    fn writes_fresh_files() {
        let t = tempfile::tempdir().unwrap();
        apply_once(t.path()).unwrap();
        let ini = std::fs::read_to_string(t.path().join(TDM_INI)).unwrap();
        assert!(ini.starts_with("[DirectionalMovement]\n"));
        assert!(ini.contains("[TargetLock]\nuTargetLockMode = 0\n"));
    }
}
