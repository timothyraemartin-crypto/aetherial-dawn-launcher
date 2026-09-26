//! Works out whether the player's Skyrim is the build the server needs.
//!
//! The executable's version alone isn't enough: Bethesda's 1.7.99 update
//! (August 2026) changed the game data but left SkyrimSE.exe at 1.6.1170.0.
//! So the check also looks at which depot manifests Steam says it installed,
//! and remembers the files the launcher's own downgrade put in place.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::game::GAME_EXE;
use crate::manifest::GameSpec;
use crate::Result;

const MARKER: &str = ".aetherial-dawn/game.json";

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameCheck {
    /// SkyrimSE.exe version, such as "1.6.1170.0".
    pub installed: Option<String>,
    pub target: Option<String>,
    /// True when the player must downgrade before playing.
    pub needed: bool,
    /// Why, in words a player understands.
    pub reason: Option<String>,
    /// True when the server listed the Steam depots, so the launcher can fix it.
    pub can_downgrade: bool,
    /// False when the SKSE build for the target version is missing.
    pub skse_ok: bool,
    pub skse_version: Option<String>,
    /// e.g. "skse64_1_6_1170.dll".
    pub skse_dll: Option<String>,
    /// Soft check: play is allowed, but the game files may be the wrong build.
    pub warning: Option<String>,
}

/// Reads the file version from a Windows executable's version resource.
pub fn exe_version(path: &Path) -> Option<[u16; 4]> {
    let bytes = std::fs::read(path).ok()?;
    // VS_FIXEDFILEINFO starts with the signature 0xFEEF04BD.
    let sig = [0xBD, 0x04, 0xEF, 0xFE];
    let at = bytes.windows(4).position(|w| w == sig)?;
    let word = |o: usize| bytes.get(at + o..at + o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let (ms, ls) = (word(8)?, word(12)?);
    Some([(ms >> 16) as u16, ms as u16, (ls >> 16) as u16, ls as u16])
}

/// "1.6.1170" or "1.6.1170.0" → [1, 6, 1170, 0].
pub fn parse_version(s: &str) -> Option<[u16; 4]> {
    let mut v = [0u16; 4];
    let parts: Vec<&str> = s.trim().split('.').collect();
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    for (i, p) in parts.iter().enumerate() {
        v[i] = p.parse().ok()?;
    }
    Some(v)
}

pub fn show(v: [u16; 4]) -> String {
    format!("{}.{}.{}.{}", v[0], v[1], v[2], v[3])
}

/// Short form players know, such as "1.6.1170".
pub fn short(v: [u16; 4]) -> String {
    format!("{}.{}.{}", v[0], v[1], v[2])
}

/// Steam's record of the game: `steamapps/appmanifest_<app>.acf` next to `common/`.
pub fn acf_path(game_dir: &Path, app: u32) -> Option<PathBuf> {
    Some(game_dir.parent()?.parent()?.join(format!("appmanifest_{app}.acf")))
}

/// Depot → manifest id from the `InstalledDepots` block of an .acf file.
pub fn installed_depots(acf: &str) -> HashMap<u32, u64> {
    let mut out = HashMap::new();
    let mut stack: Vec<String> = Vec::new();
    let mut pending: Option<String> = None;
    let mut chars = acf.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut tok = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => { if let Some(n) = chars.next() { tok.push(n); } }
                        '"' => break,
                        _ => tok.push(c),
                    }
                }
                match pending.take() {
                    None => pending = Some(tok),
                    Some(key) => {
                        // key/value pair
                        let n = stack.len();
                        if n >= 2 && stack[n - 2].eq_ignore_ascii_case("InstalledDepots") && key == "manifest" {
                            if let (Ok(d), Ok(m)) = (stack[n - 1].parse(), tok.parse()) {
                                out.insert(d, m);
                            }
                        }
                    }
                }
            }
            '{' => stack.push(pending.take().unwrap_or_default()),
            '}' => { stack.pop(); pending = None; }
            _ => {}
        }
    }
    out
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Marker {
    version: String,
    depots: Vec<(u32, String)>,
    files: Vec<(String, u64, u64)>,
    /// True when the player said "already on this version" rather than the
    /// launcher putting the files in place itself.
    /// None on marks from launchers before 0.1.13, which didn't say.
    #[serde(default)]
    manual: Option<bool>,
}

/// The files that change between builds: the executable and the base game data.
/// The files the version record watches: Steam's own game files, which a
/// Steam update replaces. Mod archives, Creation Club downloads (updated by
/// Steam separately) and plugins are left out, so installing or tidying mods
/// never reads as "Steam updated your game".
pub fn is_version_file(rel: &str) -> bool {
    let l = rel.to_ascii_lowercase();
    let Some(name) = l.strip_prefix("data/") else { return l == GAME_EXE.to_ascii_lowercase() };
    matches!(name, "skyrim.esm" | "update.esm" | "dawnguard.esm" | "hearthfires.esm" | "dragonborn.esm") || (name.starts_with("skyrim - ") && name.ends_with(".bsa"))
}

fn fingerprint(game_dir: &Path) -> Vec<(String, u64, u64)> {
    let mut names = vec![GAME_EXE.to_string()];
    if let Ok(rd) = std::fs::read_dir(game_dir.join("Data")) {
        let mut data: Vec<String> = rd
            .filter_map(|e| e.ok())
            .map(|e| format!("Data/{}", e.file_name().to_string_lossy()))
            .filter(|n| is_version_file(n))
            .collect();
        data.sort();
        names.extend(data);
    }
    names
        .into_iter()
        .filter_map(|n| {
            let md = std::fs::metadata(game_dir.join(&n)).ok()?;
            let mtime = md.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
            Some((n, md.len(), mtime))
        })
        .collect()
}

fn spec_depots(spec: &GameSpec) -> Vec<(u32, String)> {
    spec.depots.iter().map(|d| (d.depot, d.manifest.clone())).collect()
}

/// Notes that the game folder now holds the build the server wants, so later
/// checks can trust it even though Steam's own record says otherwise.
pub fn record(game_dir: &Path, spec: &GameSpec, manual: bool) -> Result<()> {
    let m = Marker {
        version: spec.version.clone().unwrap_or_default(),
        depots: spec_depots(spec),
        files: fingerprint(game_dir),
        manual: Some(manual),
    };
    let path = game_dir.join(MARKER);
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(path, serde_json::to_vec_pretty(&m)?)?;
    Ok(())
}

/// Updates the record after the launcher itself moved plugin files out of
/// Data, when the record held just before. Keeps who made it.
pub fn refresh(game_dir: &Path, spec: &GameSpec) -> Result<bool> {
    let Some(m) = read_marker(game_dir) else { return Ok(false) };
    record(game_dir, spec, m.manual != Some(false))?;
    Ok(true)
}

/// After a crash: drops a marker the player set by hand, so the version check
/// looks at Steam's record again. Returns true when one was removed.
pub fn forget_manual(game_dir: &Path) -> bool {
    let path = game_dir.join(MARKER);
    let trusted = read_marker(game_dir).map(|m| m.manual == Some(false)).unwrap_or(true);
    !trusted && std::fs::remove_file(path).is_ok()
}

/// Stops Steam updating Skyrim past the build the launcher just put in place:
/// sets "Only update this game when I launch it" in the app manifest and makes
/// the file read-only, as the SkyMP and modding downgrade guides do.
/// The launcher starts the game through SKSE, never through Steam.
pub fn hold_updates(game_dir: &Path, app: u32) -> Result<PathBuf> {
    let path = acf_path(game_dir, app).ok_or_else(|| crate::Error::Game("Couldn't find Steam's appmanifest file.".into()))?;
    let text = std::fs::read_to_string(&path)?;
    let text = set_auto_update(&text);
    let mut perm = std::fs::metadata(&path)?.permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perm.set_readonly(false);
    std::fs::set_permissions(&path, perm.clone())?;
    std::fs::write(&path, text)?;
    perm.set_readonly(true);
    std::fs::set_permissions(&path, perm)?;
    Ok(path)
}

/// Sets "AutoUpdateBehavior" to "1" in the top-level AppState block.
fn set_auto_update(acf: &str) -> String {
    let mut out = Vec::new();
    let mut done = false;
    for line in acf.lines() {
        if !done && line.trim_start().starts_with("\"AutoUpdateBehavior\"") {
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            out.push(format!("{indent}\"AutoUpdateBehavior\"\t\t\"1\""));
            done = true;
        } else {
            out.push(line.to_string());
        }
    }
    if !done {
        // Put it right after the opening brace of AppState.
        if let Some(i) = out.iter().position(|l| l.trim() == "{") {
            out.insert(i + 1, "\t\"AutoUpdateBehavior\"\t\t\"1\"".into());
        }
    }
    let mut s = out.join("\n");
    if acf.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn read_marker(game_dir: &Path) -> Option<Marker> {
    serde_json::from_slice(&std::fs::read(game_dir.join(MARKER)).ok()?).ok()
}

/// Whether the launcher itself put the current build in place (not a
/// player saying "already on this version").
pub fn made_by_launcher(game_dir: &Path) -> bool {
    read_marker(game_dir).is_some_and(|m| m.manual == Some(false))
}

fn marker_holds(game_dir: &Path, spec: &GameSpec) -> bool {
    let Some(m) = read_marker(game_dir) else { return false };
    // Records from launchers before 0.1.26 also listed mod and Creation Club
    // archives; only the files watched now are compared.
    let recorded: Vec<_> = m.files.into_iter().filter(|f| is_version_file(&f.0)).collect();
    m.version == spec.version.clone().unwrap_or_default() && m.depots == spec_depots(spec) && !recorded.is_empty() && recorded == fingerprint(game_dir)
}

pub fn check(game_dir: &Path, spec: Option<&GameSpec>) -> GameCheck {
    let installed = exe_version(&game_dir.join(GAME_EXE));
    let mut c = GameCheck { installed: installed.map(show), skse_ok: true, ..Default::default() };
    let Some(spec) = spec else { return c };
    let Some(target) = spec.version.as_deref().and_then(parse_version) else { return c };
    c.target = Some(show(target));
    c.can_downgrade = !spec.depots.is_empty();
    c.skse_version = spec.skse_version.clone();
    let dll = format!("skse64_{}_{}_{}.dll", target[0], target[1], target[2]);
    c.skse_ok = game_dir.join(&dll).is_file();
    c.skse_dll = Some(dll);

    match installed {
        None => {
            c.needed = true;
            c.reason = Some(format!("The launcher couldn't read your Skyrim version. Aetherial Dawn needs {}.", short(target)));
        }
        Some(v) if v != target => {
            c.needed = true;
            c.reason = Some(format!("Your Skyrim is {}. Aetherial Dawn needs {}.", short(v), short(target)));
        }
        Some(_) if spec.depots.is_empty() => {}
        Some(_) => {
            // Same executable; make sure Steam hasn't swapped the game data underneath it.
            let acf = acf_path(game_dir, spec.app).and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
            let have = installed_depots(&acf);
            let stale = spec.depots.iter().any(|d| matches!(have.get(&d.depot), Some(m) if m.to_string() != d.manifest));
            if marker_holds(game_dir, spec) {
                // Steam's record never changes after a downgrade, so only a
                // mark the launcher didn't make itself gets the soft warning.
                let by_launcher = read_marker(game_dir).and_then(|m| m.manual) == Some(false);
                if stale && !by_launcher {
                    c.warning = Some(format!(
                        "Your game was marked as {} by hand, but Steam says it has newer game files. If Skyrim crashes on start, click Fix version.",
                        short(target)
                    ));
                }
            } else if stale {
                c.needed = true;
                c.reason = Some(format!("Steam has updated your game data past {}. Aetherial Dawn needs {}.", short(target), short(target)));
            }
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Depot;

    #[test]
    fn holds_updates_and_forgets_manual_marker() {
        let d = game([1, 6, 1170, 0], ACF);
        let dir = d.path().join("steamapps/common/Skyrim Special Edition");
        let acf = hold_updates(&dir, 489830).unwrap();
        let text = std::fs::read_to_string(&acf).unwrap();
        assert!(text.contains("\"AutoUpdateBehavior\"\t\t\"1\""));
        assert_eq!(installed_depots(&text).len(), 2);
        assert!(std::fs::metadata(&acf).unwrap().permissions().readonly());
        // Running it again (file now read-only) replaces the value, no duplicate.
        hold_updates(&dir, 489830).unwrap();
        assert_eq!(std::fs::read_to_string(&acf).unwrap().matches("AutoUpdateBehavior").count(), 1);
        let edited = set_auto_update("\"AppState\"\n{\n\t\"AutoUpdateBehavior\"\t\t\"0\"\n}\n");
        assert!(edited.contains("\"1\"") && !edited.contains("\"0\""));

        record(&dir, &spec(), false).unwrap();
        assert!(check(&dir, Some(&spec())).warning.is_none());
        assert!(!forget_manual(&dir));
        record(&dir, &spec(), true).unwrap();
        assert!(forget_manual(&dir));
        assert!(!dir.join(MARKER).exists());
    }

    const ACF: &str = r#"
"AppState"
{
	"appid"		"489830"
	"InstalledDepots"
	{
		"489831"
		{
			"manifest"		"8442952117333549665"
			"size"		"7351706128"
		}
		"489833"
		{
			"manifest"		"1914580699073641964"
			"size"		"37568512"
		}
	}
	"UserConfig" { "language"		"english" }
}"#;

    fn spec() -> GameSpec {
        GameSpec {
            version: Some("1.6.1170.0".into()),
            skse_version: Some("2.2.6".into()),
            app: 489830,
            depots: vec![
                Depot { depot: 489831, manifest: "8442952117333549665".into() },
                Depot { depot: 489833, manifest: "1914580699073641964".into() },
            ],
            tool: None,
        }
    }

    /// Smallest file with a VS_FIXEDFILEINFO the reader will find.
    fn fake_exe(v: [u16; 4]) -> Vec<u8> {
        let mut b = vec![0u8; 64];
        b.extend([0xBD, 0x04, 0xEF, 0xFE, 0, 0, 1, 0]);
        b.extend((((v[0] as u32) << 16) | v[1] as u32).to_le_bytes());
        b.extend((((v[2] as u32) << 16) | v[3] as u32).to_le_bytes());
        b.extend([0u8; 40]);
        b
    }

    fn game(v: [u16; 4], acf: &str) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("steamapps/common/Skyrim Special Edition");
        std::fs::create_dir_all(dir.join("Data")).unwrap();
        std::fs::write(dir.join(GAME_EXE), fake_exe(v)).unwrap();
        std::fs::write(dir.join("Data/Skyrim.esm"), b"esm").unwrap();
        std::fs::write(root.path().join("steamapps/appmanifest_489830.acf"), acf).unwrap();
        root
    }
    fn dir(root: &tempfile::TempDir) -> PathBuf {
        root.path().join("steamapps/common/Skyrim Special Edition")
    }

    #[test]
    fn reads_versions_and_depots() {
        assert_eq!(parse_version("1.6.1170"), Some([1, 6, 1170, 0]));
        let d = installed_depots(ACF);
        assert_eq!(d.get(&489831), Some(&8442952117333549665));
        assert_eq!(d.get(&489833), Some(&1914580699073641964));
        assert_eq!(d.len(), 2);
    }

    #[test]
    fn matching_build_is_fine() {
        let g = game([1, 6, 1170, 0], ACF);
        let c = check(&dir(&g), Some(&spec()));
        assert!(!c.needed, "{c:?}");
        assert_eq!(c.installed.as_deref(), Some("1.6.1170.0"));
        assert!(!c.skse_ok);
        assert_eq!(c.skse_dll.as_deref(), Some("skse64_1_6_1170.dll"));
    }

    #[test]
    fn other_exe_needs_downgrade() {
        let g = game([1, 6, 1179, 0], ACF);
        let c = check(&dir(&g), Some(&spec()));
        assert!(c.needed && c.can_downgrade);
        assert!(c.reason.unwrap().contains("1.6.1179"));
    }

    #[test]
    fn newer_data_under_same_exe_needs_downgrade_until_recorded() {
        let g = game([1, 6, 1170, 0], &ACF.replace("8442952117333549665", "1111"));
        let d = dir(&g);
        assert!(check(&d, Some(&spec())).needed);
        record(&d, &spec(), true).unwrap();
        let c = check(&d, Some(&spec()));
        assert!(!c.needed && c.warning.is_some(), "a hand-set mark over stale depots gets the soft warning");
        // A mark from before 0.1.13 (no "manual" field) is treated the same way.
        let legacy = std::fs::read_to_string(d.join(MARKER)).unwrap().replace("\"manual\": true", "\"x\": 0");
        std::fs::write(d.join(MARKER), legacy).unwrap();
        assert!(check(&d, Some(&spec())).warning.is_some());
        record(&d, &spec(), false).unwrap();
        let c = check(&d, Some(&spec()));
        assert!(!c.needed && c.warning.is_none());
        // Mod and Creation Club archives coming and going don't count.
        std::fs::write(d.join("Data/SomeMod.bsa"), b"mod").unwrap();
        std::fs::write(d.join("Data/ccBGSSSE001-Fish.esm"), b"cc").unwrap();
        assert!(!check(&d, Some(&spec())).needed);
        // Steam rewrites the data afterwards: the record no longer holds.
        std::fs::write(d.join("Data/Skyrim.esm"), b"newer esm").unwrap();
        assert!(check(&d, Some(&spec())).needed);
    }

    #[test]
    fn old_records_listing_mod_archives_still_hold() {
        let g = game([1, 6, 1170, 0], &ACF.replace("8442952117333549665", "1111"));
        let d = dir(&g);
        std::fs::write(d.join("Data/OldMod.bsa"), b"mod").unwrap();
        record(&d, &spec(), false).unwrap();
        // Written the old way, with the mod archive in the list; then the mod goes.
        let mut m = read_marker(&d).unwrap();
        m.files.push(("Data/OldMod.bsa".into(), 3, 0));
        std::fs::write(d.join(MARKER), serde_json::to_vec(&m).unwrap()).unwrap();
        std::fs::remove_file(d.join("Data/OldMod.bsa")).unwrap();
        assert!(!check(&d, Some(&spec())).needed);
    }

    #[test]
    fn no_target_means_no_check() {
        let g = game([1, 7, 0, 0], ACF);
        assert!(!check(&dir(&g), None).needed);
    }
}
