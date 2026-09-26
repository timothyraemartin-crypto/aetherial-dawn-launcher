//! Game health: the checks Claude ran by hand on the first live tester's PC,
//! run by the launcher for every player before Play and after a crash. Each
//! check says ok, info, warn or fail with plain-words detail. The report holds
//! nothing secret: no settings file contents, tokens or sessions, and the
//! player's home folder is written as %USERPROFILE%.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::game::GAME_EXE;
use crate::manifest::Manifest;
use crate::{gameini, loadorder, requirements, strays, version, watch};

pub const MASTERS: [&str; 5] = ["Skyrim.esm", "Update.esm", "Dawnguard.esm", "HearthFires.esm", "Dragonborn.esm"];

/// DLLs Windows loads into Skyrim from its own folder: ENB, ReShade and other
/// injectors. They hook the renderer next to SkyrimPlatform's browser.
const INJECTORS: [&str; 7] = ["d3d11.dll", "dxgi.dll", "dinput8.dll", "d3d9.dll", "opengl32.dll", "version.dll", "winmm.dll"];

/// Overlays and capture tools that hook the game's rendering.
const OVERLAYS: [(&str, &str); 9] = [
    ("RTSS.exe", "RivaTuner Statistics Server"),
    ("MSIAfterburner.exe", "MSI Afterburner"),
    ("Overwolf.exe", "Overwolf"),
    ("Medal.exe", "Medal"),
    ("obs64.exe", "OBS Studio"),
    ("Discord.exe", "Discord (its in-game overlay can be on)"),
    ("GameBar.exe", "Xbox Game Bar"),
    ("NVIDIA Overlay.exe", "NVIDIA overlay"),
    ("ReShade.exe", "ReShade"),
];

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Info,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub title: &'static str,
    pub status: Status,
    pub detail: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub checks: Vec<Check>,
    /// The worst status of any check.
    pub worst: Status,
}

pub struct Inputs<'a> {
    pub game_dir: &'a Path,
    pub manifest: Option<&'a Manifest>,
    /// The server's masters.json, when it could be fetched.
    pub masters: Option<&'a serde_json::Value>,
    /// %LOCALAPPDATA%\Skyrim Special Edition (plugins.txt, loadorder.txt).
    pub appdata: Option<&'a Path>,
    /// The Documents folder (My Games\Skyrim Special Edition\Skyrim.ini).
    pub documents: Option<&'a Path>,
    /// Where hashes of large files are remembered between runs.
    pub hash_cache: Option<&'a Path>,
    /// Folder written as %USERPROFILE% in the report.
    pub home: Option<&'a Path>,
}

fn check(id: &'static str, title: &'static str, status: Status, detail: impl Into<String>, items: Vec<String>) -> Check {
    Check { id, title, status, detail: detail.into(), items }
}

pub fn run(i: &Inputs) -> Report {
    let mut checks = vec![
        exe(i),
        masters(i),
        load_order(i),
        stub_plugins(i),
        newer_plugins(i),
        ini_archives(i),
        stray_plugins(i),
        injectors(i),
        overlays(),
        steam_updates(i),
        crash_logger(i),
    ];
    if let Some(home) = i.home {
        let h = home.display().to_string();
        for c in &mut checks {
            c.detail = c.detail.replace(&h, "%USERPROFILE%");
            for it in &mut c.items {
                *it = it.replace(&h, "%USERPROFILE%");
            }
        }
    }
    let worst = checks.iter().map(|c| c.status).max().unwrap_or(Status::Ok);
    Report { checks, worst }
}

fn exe(i: &Inputs) -> Check {
    let have = version::exe_version(&i.game_dir.join(GAME_EXE));
    let want = i.manifest.and_then(|m| m.game.as_ref()).and_then(|g| g.version.as_deref()).and_then(version::parse_version);
    match (have, want) {
        (None, _) => check("exe", "Skyrim version", Status::Fail, "SkyrimSE.exe is missing or unreadable.", vec![]),
        (Some(h), Some(w)) if h != w => check(
            "exe",
            "Skyrim version",
            Status::Fail,
            format!("SkyrimSE.exe is {}; the server needs {}. Click Fix version.", version::show(h), version::show(w)),
            vec![],
        ),
        (Some(h), _) => check("exe", "Skyrim version", Status::Ok, format!("SkyrimSE.exe {}", version::show(h)), vec![]),
    }
}

/// Reads masters.json in any of the shapes the server might publish:
/// {"masters":[{name,size,sha256}]}, a bare list, or {"Skyrim.esm":{size,sha256}}.
pub fn parse_masters(v: &serde_json::Value) -> Vec<(String, Option<u64>, Option<String>)> {
    let list = v.get("masters").or_else(|| v.get("files")).unwrap_or(v);
    let entry = |name: String, e: &serde_json::Value| {
        let size = e.get("size").and_then(|s| s.as_u64());
        let sha = e.get("sha256").and_then(|s| s.as_str()).map(|s| s.to_ascii_lowercase());
        (name, size, sha)
    };
    match list {
        serde_json::Value::Array(a) => a
            .iter()
            .filter_map(|e| {
                let n = e.get("name").or_else(|| e.get("file")).or_else(|| e.get("path")).and_then(|n| n.as_str())?;
                Some(entry(n.rsplit(['/', '\\']).next().unwrap_or(n).to_string(), e))
            })
            .collect(),
        serde_json::Value::Object(o) => o.iter().filter(|(_, e)| e.is_object()).map(|(n, e)| entry(n.clone(), e)).collect(),
        _ => Vec::new(),
    }
}

#[derive(Default, Serialize, Deserialize)]
struct HashCache(HashMap<String, (u64, u64, String)>);

/// SHA-256 of a file, remembered by path, size and modified time.
fn sha256_cached(path: &Path, cache: &mut HashCache) -> Option<String> {
    let md = std::fs::metadata(path).ok()?;
    let mtime = md.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let key = path.display().to_string();
    if let Some((s, t, h)) = cache.0.get(&key) {
        if *s == md.len() && *t == mtime {
            return Some(h.clone());
        }
    }
    let mut f = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let h = hex::encode(hasher.finalize());
    cache.0.insert(key, (md.len(), mtime, h.clone()));
    Some(h)
}

fn masters(i: &Inputs) -> Check {
    let data = i.game_dir.join("Data");
    let Some(want) = i.masters.map(parse_masters).filter(|w| !w.is_empty()) else {
        let missing: Vec<String> = MASTERS.iter().filter(|m| !data.join(m).is_file()).map(|m| m.to_string()).collect();
        return if missing.is_empty() {
            check("masters", "Game masters", Status::Info, "Present. The server's list of masters couldn't be loaded, so they weren't compared.", vec![])
        } else {
            check("masters", "Game masters", Status::Fail, "Missing from Data. Verify Skyrim in Steam, then click Fix version.", missing)
        };
    };
    let mut cache: HashCache = i.hash_cache.and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut bad = Vec::new();
    for (name, size, sha) in &want {
        let p = data.join(name);
        let Ok(md) = std::fs::metadata(&p) else {
            bad.push(format!("{name}: missing"));
            continue;
        };
        if let Some(s) = size {
            if md.len() != *s {
                bad.push(format!("{name}: {} bytes, server has {s}", md.len()));
                continue;
            }
        }
        if let Some(sha) = sha {
            match sha256_cached(&p, &mut cache) {
                Some(h) if &h == sha => {}
                Some(_) => bad.push(format!("{name}: same size, different contents")),
                None => bad.push(format!("{name}: couldn't be read")),
            }
        }
    }
    if let Some(p) = i.hash_cache {
        if let Ok(b) = serde_json::to_vec(&cache) {
            let _ = std::fs::write(p, b);
        }
    }
    if bad.is_empty() {
        check("masters", "Game masters", Status::Ok, format!("All {} match the server.", want.len()), vec![])
    } else {
        check("masters", "Game masters", Status::Fail, "Don't match the server's. Click Fix version to download the right build.", bad)
    }
}

fn load_order(i: &Inputs) -> Check {
    let Some(dir) = i.appdata else {
        return check("loadorder", "Load order", Status::Info, "Couldn't find the load order folder.", vec![]);
    };
    let mut items = Vec::new();
    let data = i.game_dir.join("Data");
    if let Some(m) = i.manifest {
        for e in loadorder::extras(i.game_dir, &dir.join("plugins.txt"), m) {
            items.push(format!("switched on: {}", e.describe()));
        }
    }
    if let Ok(t) = std::fs::read_to_string(dir.join("loadorder.txt")) {
        let names: Vec<&str> = t.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
        let lower: Vec<String> = names.iter().map(|n| n.to_ascii_lowercase()).collect();
        let first: Vec<String> = MASTERS.iter().map(|m| m.to_ascii_lowercase()).collect();
        if lower.len() >= 5 && lower[..5] != first[..] {
            items.push(format!("loadorder.txt doesn't start with the five base masters: {}", names.iter().take(5).copied().collect::<Vec<_>>().join(", ")));
        }
        for n in &names {
            if !data.join(n).is_file() {
                items.push(format!("loadorder.txt lists {n}, which isn't in Data"));
            }
        }
    }
    if items.is_empty() {
        check("loadorder", "Load order", Status::Ok, "Only the base game, Creation Club and Aetherial Dawn's plugins are switched on.", vec![])
    } else {
        check("loadorder", "Load order", Status::Warn, "The launcher switches extra plugins off before Play.", items)
    }
}

fn stub_plugins(i: &Inputs) -> Check {
    let mut items = Vec::new();
    if let Ok(rd) = std::fs::read_dir(i.game_dir.join("Data")) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            let l = n.to_ascii_lowercase();
            if l.ends_with(".esp") || l.ends_with(".esm") || l.ends_with(".esl") {
                if let Some(why) = loadorder::broken(&e.path()) {
                    items.push(format!("{n}: {why}"));
                }
            }
        }
    }
    items.sort();
    if items.is_empty() {
        check("stubs", "Plugin files", Status::Ok, "No broken plugin files in Data.", vec![])
    } else {
        check("stubs", "Plugin files", Status::Warn, "Broken plugin files in Data. Switched off, they're harmless.", items)
    }
}

fn newer_plugins(i: &Inputs) -> Check {
    let items: Vec<String> = loadorder::too_new(i.game_dir).into_iter().map(|(n, why)| format!("{n}: {why}")).collect();
    if items.is_empty() {
        check("newer", "Plugins for a newer Skyrim", Status::Ok, "Every plugin in Data fits this Skyrim version.", vec![])
    } else {
        check("newer", "Plugins for a newer Skyrim", Status::Fail, "Plugins made for a newer Skyrim than this one. The launcher takes them out of play before each Play.", items)
    }
}

fn ini_archives(i: &Inputs) -> Check {
    let Some(docs) = i.documents else {
        return check("ini", "Skyrim.ini archives", Status::Info, "Couldn't find the Documents folder.", vec![]);
    };
    let mut items = Vec::new();
    for ini in gameini::ini_paths(docs) {
        let Ok(text) = std::fs::read_to_string(&ini) else { continue };
        let (_, r) = gameini::clean(&text, &i.game_dir.join("Data"));
        let name = ini.file_name().unwrap().to_string_lossy().into_owned();
        if !r.missing.is_empty() {
            items.push(format!("{name}: {} missing: {}", r.missing.len(), r.missing.join(", ")));
        }
        if !r.duplicates.is_empty() {
            items.push(format!("{name}: {} listed twice: {}", r.duplicates.len(), r.duplicates.join(", ")));
        }
    }
    if items.is_empty() {
        check("ini", "Skyrim.ini archives", Status::Ok, "Every archive the ini names is in Data.", vec![])
    } else {
        check("ini", "Skyrim.ini archives", Status::Warn, "The ini names archives that aren't there. The launcher cleans this before Play.", items)
    }
}

fn stray_plugins(i: &Inputs) -> Check {
    let Some(m) = i.manifest else {
        return check("strays", "Other mods' files", Status::Info, "Checked after the server file list loads.", vec![]);
    };
    let list = strays::find(i.game_dir, m);
    if list.is_empty() {
        check("strays", "Other mods' files", Status::Ok, "No SKSE plugins, Platform scripts or loose menus from other mods.", vec![])
    } else {
        check("strays", "Other mods' files", Status::Warn, "The launcher moves these to a backup folder before Play.", list)
    }
}

fn injectors(i: &Inputs) -> Check {
    let mut items = Vec::new();
    if let Ok(rd) = std::fs::read_dir(i.game_dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            let l = n.to_ascii_lowercase();
            if INJECTORS.contains(&l.as_str()) || l.ends_with(".asi") || l.starts_with("enbseries") || l.starts_with("reshade") || l == "enblocal.ini" {
                items.push(n);
            }
        }
    }
    items.sort();
    if items.is_empty() {
        check("injectors", "Injectors next to Skyrim", Status::Ok, "No ENB, ReShade or other injector files.", vec![])
    } else {
        check("injectors", "Injectors next to Skyrim", Status::Warn, "These load into Skyrim and can crash SkyrimPlatform's browser. Move them out if the game crashes.", items)
    }
}

fn overlays() -> Check {
    let running = watch::process_names();
    let items: Vec<String> = OVERLAYS
        .iter()
        .filter(|(exe, _)| running.iter().any(|r| r.eq_ignore_ascii_case(exe)))
        .map(|(exe, what)| format!("{what} ({exe})"))
        .collect();
    if items.is_empty() {
        check("overlays", "Overlays", Status::Ok, "No known overlays running.", vec![])
    } else {
        check("overlays", "Overlays", Status::Info, "Running now. If Skyrim crashes on start, try closing them or turning their in-game overlay off.", items)
    }
}

fn steam_updates(i: &Inputs) -> Check {
    let app = i.manifest.and_then(|m| m.game.as_ref()).map(|g| g.app).unwrap_or(489830);
    let Some(acf) = version::acf_path(i.game_dir, app) else {
        return check("steam", "Steam updates", Status::Info, "Couldn't find Steam's record for Skyrim.", vec![]);
    };
    let Ok(text) = std::fs::read_to_string(&acf) else {
        return check("steam", "Steam updates", Status::Info, "Couldn't read Steam's record for Skyrim.", vec![]);
    };
    let behavior = text
        .lines()
        .find(|l| l.trim_start().starts_with("\"AutoUpdateBehavior\""))
        .and_then(|l| l.split('"').nth(3))
        .unwrap_or("0")
        .to_string();
    let readonly = std::fs::metadata(&acf).map(|m| m.permissions().readonly()).unwrap_or(false);
    let detail = format!(
        "Update setting {} ({}), record {}",
        behavior,
        match behavior.as_str() {
            "1" => "only when launched",
            "2" => "high priority",
            _ => "always keep updated",
        },
        if readonly { "held (read-only)" } else { "not held" }
    );
    if behavior == "1" && readonly {
        check("steam", "Steam updates", Status::Ok, detail, vec![])
    } else {
        check("steam", "Steam updates", Status::Warn, format!("{detail}. Steam can update Skyrim past the server's version. Fix version holds it."), vec![])
    }
}

/// The mods every player needs: SKSE64, the Address Library with the file for
/// the server's build, and Crash Logger.
fn crash_logger(i: &Inputs) -> Check {
    let dir = i.game_dir.join("Data").join("SKSE").join("Plugins");
    let target = i.manifest.and_then(|m| m.game.as_ref()).and_then(|g| g.version.clone());
    let mut missing = Vec::new();
    let mut have = Vec::new();
    match &target {
        Some(v) if !requirements::address_library_ok(i.game_dir, v) => {
            missing.push(format!("Address Library for SKSE Plugins: {} is missing from Data\\SKSE\\Plugins", requirements::address_library_file(v)));
        }
        Some(_) => have.push("Address Library"),
        None => {}
    }
    if strays::CRASH_LOGGERS.iter().any(|n| dir.join(n).is_file()) {
        have.push("Crash Logger");
    } else {
        missing.push("Crash Logger: not installed (the launcher installs it before Play)".to_string());
    }
    if i.game_dir.join("skse64_loader.exe").is_file() {
        have.push("SKSE64");
    } else {
        missing.push("SKSE64: skse64_loader.exe is missing".to_string());
    }
    if missing.is_empty() {
        check("requirements", "Required mods", Status::Ok, format!("{} installed.", have.join(", ")), vec![])
    } else {
        let fail = missing.iter().any(|m| !m.starts_with("Crash Logger"));
        check("requirements", "Required mods", if fail { Status::Fail } else { Status::Warn }, "Aetherial Dawn needs SKSE64 2.2.6, the Address Library and Crash Logger.", missing)
    }
}

impl Report {
    /// Plain text, the same text the player sees and staff receive.
    pub fn text(&self) -> String {
        let mut o = String::new();
        for c in &self.checks {
            let tag = match c.status {
                Status::Ok => "OK  ",
                Status::Info => "INFO",
                Status::Warn => "WARN",
                Status::Fail => "FAIL",
            };
            o.push_str(&format!("[{tag}] {}: {}\n", c.title, c.detail));
            for it in &c.items {
                o.push_str(&format!("       - {it}\n"));
            }
        }
        o
    }
}

/// Where the hash cache lives, next to the launcher's other data.
pub fn cache_path(app_data: &Path) -> PathBuf {
    app_data.join("master-hashes.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_master_list_shapes() {
        let a = serde_json::json!({"masters":[{"name":"Skyrim.esm","size":3,"sha256":"AB"}]});
        assert_eq!(parse_masters(&a), vec![("Skyrim.esm".into(), Some(3), Some("ab".into()))]);
        let b = serde_json::json!([{"path":"Data/Update.esm","size":1}]);
        assert_eq!(parse_masters(&b)[0].0, "Update.esm");
        let c = serde_json::json!({"Dawnguard.esm":{"size":2}});
        assert_eq!(parse_masters(&c)[0], ("Dawnguard.esm".into(), Some(2), None));
    }

    #[test]
    fn flags_master_mismatch_and_hides_home() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("Skyrim.esm"), b"abc").unwrap();
        std::fs::write(data.join("Update.esm"), b"xyz").unwrap();
        let sha = hex::encode(Sha256::digest(b"abc"));
        let m = serde_json::json!({"masters":[
            {"name":"Skyrim.esm","size":3,"sha256":sha},
            {"name":"Update.esm","size":3,"sha256":sha},
            {"name":"Dawnguard.esm","size":1}
        ]});
        let cache = tmp.path().join("cache.json");
        let i = Inputs { game_dir: tmp.path(), manifest: None, masters: Some(&m), appdata: None, documents: None, hash_cache: Some(&cache), home: Some(tmp.path()) };
        let r = run(&i);
        let c = r.checks.iter().find(|c| c.id == "masters").unwrap();
        assert_eq!(c.status, Status::Fail);
        assert_eq!(c.items, ["Update.esm: same size, different contents", "Dawnguard.esm: missing"]);
        assert!(cache.exists());
        assert_eq!(r.worst, Status::Fail);
        assert!(!r.text().contains(&tmp.path().display().to_string()));
    }
}
