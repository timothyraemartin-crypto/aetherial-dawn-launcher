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
        required_files(i),
        skse_builds(i),
        wanted_off(i),
        camera_preset(i),
        ussep_version(i),
        load_order(i),
        server_order(i),
        plugin_names(i),
        load_order_file(i),
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
            check("masters", "Base game files", Status::Info, "Present. The server's list of masters couldn't be loaded, so they weren't compared.", vec![])
        } else {
            check("masters", "Base game files", Status::Fail, "Missing from Data. Verify Skyrim in Steam, then click Fix version.", missing)
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
        check("masters", "Base game files", Status::Ok, format!("All {} match the server.", want.len()), vec![])
    } else {
        check("masters", "Base game files", Status::Fail, "Don't match the server's. Press Play and the launcher puts the right ones in place.", bad)
    }
}

fn load_order(i: &Inputs) -> Check {
    let Some(dir) = i.appdata else {
        return check("loadorder", "Load order", Status::Info, "Couldn't find the load order folder.", vec![]);
    };
    let mut items = Vec::new();
    if let Some(m) = i.manifest {
        for e in loadorder::extras(i.game_dir, &dir.join("plugins.txt"), m) {
            items.push(format!("switched on: {}", e.describe()));
        }
    }
    if items.is_empty() {
        check("loadorder", "Load order", Status::Ok, "Only the base game, Creation Club and Aetherial Dawn's plugins are switched on.", vec![])
    } else {
        check("loadorder", "Load order", Status::Warn, "The launcher switches extra plugins off before Play.", items)
    }
}

/// The game's plugins against the server's, position by position, as the
/// SkyMP client checks them (serverorder.rs).
fn server_order(i: &Inputs) -> Check {
    let order = i.masters.map(crate::serverorder::server_order).unwrap_or_default();
    if !crate::serverorder::beyond_base(&order) {
        return check("serverorder", "Server plugin order", Status::Ok, "The server loads only the five base masters.", vec![]);
    }
    let Some(dir) = i.appdata else {
        return check("serverorder", "Server plugin order", Status::Info, "Couldn't find the load order folder.", vec![]);
    };
    let bad = crate::serverorder::mismatches(i.game_dir, &dir.join("plugins.txt"), &order);
    if bad.is_empty() {
        check("serverorder", "Server plugin order", Status::Ok, format!("The game loads the server's {} plugins in the same order.", order.len()), vec![])
    } else {
        check("serverorder", "Server plugin order", Status::Fail, "The game's plugins don't match the server's order, so the game would refuse to connect. The launcher sets the order before Play.", bad)
    }
}

/// Plugins running under a dash-named copy because the SkyMP client can't
/// load their own names (aliases.rs). Information, not a fault.
fn plugin_names(i: &Inputs) -> Check {
    let mut items: Vec<String> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(i.game_dir.join("Data")) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            let l = n.to_ascii_lowercase();
            if (l.ends_with(".esp") || l.ends_with(".esm") || l.ends_with(".esl")) && !loadorder::client_can_load_name(&n) {
                items.push(format!("{n} loads as {} so the game accepts it", crate::aliases::run_as(i.game_dir, &n)));
            }
        }
    }
    items.sort();
    if items.is_empty() {
        check("pluginnames", "Plugin names", Status::Ok, "Every plugin's name is one the game client accepts.", vec![])
    } else {
        check("pluginnames", "Plugin names", Status::Info, "The game can't load these plugin names as they are, so before Play the launcher loads a copy under a name it accepts. Nothing is renamed.", items)
    }
}

/// loadorder.txt only records the order; the game loads the five masters
/// first whatever it says, so this is kept apart from plugins switched on.
fn load_order_file(i: &Inputs) -> Check {
    let Some(dir) = i.appdata else {
        return check("loadorderfile", "Load order file", Status::Info, "Couldn't find the load order folder.", vec![]);
    };
    let mut items = Vec::new();
    let data = i.game_dir.join("Data");
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
        check("loadorderfile", "Load order file", Status::Ok, "Your load order starts with the five base game files.", vec![])
    } else {
        check("loadorderfile", "Load order file", Status::Info, "Your load order is out of date. The launcher fixes it before Play; this doesn't crash the game.", items)
    }
}

/// Support files a required mod can't start without, and required mods'
/// files sitting in the launcher's backup folder (2026-09-26: 0.1.38 moved
/// SKSE Menu Framework's fonts and settings, and the game crashed 3 s in).
pub fn missing_required_files(game_dir: &Path) -> Vec<String> {
    let plugins = game_dir.join("Data/SKSE/Plugins");
    let mut items = Vec::new();
    let has_ext = |d: &Path, ext: &str| std::fs::read_dir(d).map(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().to_ascii_lowercase().ends_with(ext))).unwrap_or(false);
    if plugins.join(requirements::MENU_FRAMEWORK_DLL).is_file() {
        if !plugins.join("SKSEMenuFrameworkStrings_EN.json").is_file() && !plugins.join("SKSEMenuFrameworkStrings.json").is_file() {
            items.push("SKSE Menu Framework: its strings file (SKSEMenuFrameworkStrings_EN.json) is missing".to_string());
        }
        if !has_ext(&plugins.join("fonts"), ".ttf") {
            items.push("SKSE Menu Framework: its fonts (Data\\SKSE\\Plugins\\fonts) are missing".to_string());
        }
        if !has_ext(&plugins.join("SKSEMenuFrameworkThemes"), ".json") {
            items.push("SKSE Menu Framework: its themes (SKSEMenuFrameworkThemes) are missing".to_string());
        }
    }
    if plugins.join("EngineFixes.dll").is_file() && !plugins.join("EngineFixes.toml").is_file() {
        items.push("SSE Engine Fixes: EngineFixes.toml is missing".to_string());
    }
    let root = game_dir.join(strays::DISABLED_DIR);
    let mut aside = std::collections::BTreeSet::new();
    let mut stack = vec![root.clone()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                // A too-new or wrong build set aside on purpose (e.g. the
                // Unofficial Patch 4.3.9c) isn't a file to put back.
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                if d == root && (name.contains("-too-new") || name.contains("-wrong-build")) {
                    continue;
                }
                stack.push(p);
                continue;
            }
            // <stamp>/Data/...
            let Ok(rel) = p.strip_prefix(&root) else { continue };
            let rel: PathBuf = rel.components().skip(1).collect();
            let rel_s = rel.to_string_lossy().replace('\\', "/");
            if crate::allowlist::required_file(&rel_s) && !game_dir.join(&rel).exists() {
                aside.insert(rel_s);
            }
        }
    }
    items.extend(aside.into_iter().map(|r| format!("a required mod's file is in the launcher's backup folder: {r}")));
    items
}

fn required_files(i: &Inputs) -> Check {
    let items = missing_required_files(i.game_dir);
    if items.is_empty() {
        check("requiredfiles", "Required mods' files", Status::Ok, "Every required mod has its support files.", vec![])
    } else {
        check("requiredfiles", "Required mods' files", Status::Fail, "A required mod is missing files it needs to start, which crashes the game a few seconds in. Before Play the launcher puts back files it set aside and reinstalls the mod if anything is still missing.", items)
    }
}

/// SKSE DLLs built for another Skyrim, named by mod where the mod list
/// checks for them.
fn skse_builds(i: &Inputs) -> Check {
    let list = crate::modlist::builtin(None);
    let items: Vec<String> = crate::skse::wrong_builds(i.game_dir)
        .into_iter()
        .map(|(rel, why)| match list.iter().find(|m| m.check.iter().any(|c| c.eq_ignore_ascii_case(&rel))) {
            Some(m) => format!("{} ({}): {why}; the launcher sets it aside and installs the 1.6.1170 build", m.name, rel.rsplit('/').next().unwrap_or(&rel)),
            None => format!("{}: {why}; the launcher sets it aside before Play", rel.rsplit('/').next().unwrap_or(&rel)),
        })
        .collect();
    if items.is_empty() {
        check("sksebuilds", "SKSE mod builds", Status::Ok, "Every SKSE mod is the build for Skyrim 1.6.1170.", vec![])
    } else {
        check("sksebuilds", "SKSE mod builds", Status::Fail, "An SKSE mod is the build for another Skyrim, so SKSE stops the game with \"only compatible with versions earlier than 1.6.629\".", items)
    }
}

/// Required mods' plugins switched off in the load order (their menus, like
/// SmoothCam's settings page, don't show then).
fn wanted_off(i: &Inputs) -> Check {
    let off = i.appdata.map(|a| loadorder::wanted_but_off(i.game_dir, &a.join("plugins.txt"))).unwrap_or_default();
    if off.is_empty() {
        check("requiredoff", "Required mods switched on", Status::Ok, "Every required mod's plugin is switched on.", vec![])
    } else {
        check("requiredoff", "Required mods switched on", Status::Info, "A required mod's plugin is switched off in Vortex's Plugins tab. The launcher switches it on before every Play, since its settings page in Mod Configuration needs it.", off)
    }
}

/// The Unofficial Patch must be the one for Skyrim 1.6.1170 (4.3.8a).
fn ussep_version(i: &Inputs) -> Check {
    match crate::ussep::too_new(i.game_dir) {
        Some(why) => check("ussepversion", "Unofficial Patch version", Status::Fail, format!("{why}. It crashes the game while drawing land, so the launcher sets it aside and installs 4.3.8a."), vec![]),
        None => check("ussepversion", "Unofficial Patch version", Status::Ok, "The Unofficial Patch is the version for Skyrim 1.6.1170.", vec![]),
    }
}

/// Whether the Souls-style camera preset has been applied.
fn camera_preset(i: &Inputs) -> Check {
    let name = "SmoothCam's Modern Camera Preset";
    match (crate::camera::applied(i.game_dir), crate::camera::find_modern(i.game_dir)) {
        (Some(_), _) => check("camerapreset", "Camera preset", Status::Ok, format!("{name} is your camera. Fine-tune it in Esc > Mod Configuration > SmoothCam; the launcher won't undo your changes."), vec![]),
        (None, Some((f, _))) => check("camerapreset", "Camera preset", Status::Info, format!("{name} ({f}) becomes your camera on the next Play. Your current SmoothCam settings are backed up first."), vec![]),
        (None, None) => check("camerapreset", "Camera preset", Status::Info, format!("{name} isn't installed yet; the launcher installs it with the other required mods."), vec![]),
    }
}

/// A cause read from the crash log itself, which beats the checks: the
/// 23:50 reports blamed a parked MCMHelper.esp for a terrain crash.
pub fn crash_cause(crash: &str) -> Option<String> {
    let l = crash.to_ascii_lowercase();
    if l.contains("bgsterrainmanager") || (l.contains("tesobjectland") && l.contains("unofficial")) {
        return Some("The game crashed drawing land changed by the Unofficial Patch (terrain update)".into());
    }
    None
}

/// The launcher's own best guess at a crash's cause, from the checks, most
/// specific first. None when nothing points anywhere.
pub fn likely_cause(r: &Report) -> Option<String> {
    let failed = |id: &str, st: Status| r.checks.iter().find(|c| c.id == id && c.status >= st);
    if let Some(c) = failed("requiredfiles", Status::Fail) {
        return Some(format!("A required mod is missing its support files: {}", c.items.first().cloned().unwrap_or_default()));
    }
    if let Some(c) = failed("ussepversion", Status::Fail) {
        return Some(c.detail.split(". ").next().unwrap_or("Unofficial Patch made for a newer Skyrim").to_string());
    }
    if let Some(c) = failed("sksebuilds", Status::Fail) {
        return Some(format!("An SKSE mod is the build for another Skyrim: {}", c.items.first().cloned().unwrap_or_default()));
    }
    for (id, why) in [
        ("exe", "Wrong Skyrim version"),
        ("masters", "Base game files don't match the server"),
        ("newer", "Plugins made for a newer Skyrim"),
        ("strays", "SKSE plugins or loose menus from other mods"),
    ] {
        if failed(id, Status::Warn).is_some() {
            return Some(why.to_string());
        }
    }
    if failed("loadorder", Status::Warn).is_some() {
        return Some("Plugins from other mods are switched on".to_string());
    }
    None
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
        check("stubs", "Plugin files", Status::Ok, "No broken plugin files.", vec![])
    } else {
        check("stubs", "Plugin files", Status::Warn, "Some plugin files are broken. They're switched off, so they can't hurt.", items)
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
        return check("ini", "Game settings file", Status::Info, "Couldn't find the Documents folder.", vec![]);
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
        check("ini", "Game settings file", Status::Ok, "Every file Skyrim's settings file lists is there.", vec![])
    } else {
        check("ini", "Game settings file", Status::Warn, "Skyrim's settings file lists files that aren't there. The launcher cleans this before Play.", items)
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

/// ENB, ReShade and other injector files next to SkyrimSE.exe.
pub fn injector_files(game_dir: &Path) -> Vec<String> {
    let mut items = Vec::new();
    if let Ok(rd) = std::fs::read_dir(game_dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            let l = n.to_ascii_lowercase();
            if INJECTORS.contains(&l.as_str()) || l.ends_with(".asi") || l.starts_with("enbseries") || l.starts_with("reshade") || l == "enblocal.ini" {
                items.push(n);
            }
        }
    }
    items.sort();
    items
}

fn injectors(i: &Inputs) -> Check {
    let items = injector_files(i.game_dir);
    if items.is_empty() {
        check("injectors", "Injectors next to Skyrim", Status::Ok, "No ENB, ReShade or other injector files.", vec![])
    } else {
        check("injectors", "Injectors next to Skyrim", Status::Warn, "These load into Skyrim and can crash the game's menus. If the game crashes, the launcher sets them aside before the next Play.", items)
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
        check("steam", "Steam updates", Status::Warn, format!("{detail}. If Steam updates Skyrim, the launcher puts the right files back before you play."), vec![])
    }
}

/// The mods every player needs: SKSE64, the Address Library with the file for
/// the server's build, Crash Logger, Skyrim Souls RE and its dependencies.
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
    // The same test Play uses: a copy SKSE wouldn't load doesn't count.
    if requirements::crash_logger_ok(i.game_dir) {
        have.push("Crash Logger");
    } else if dir.join("CrashLogger.dll").is_file() {
        missing.push("Crash Logger: the copy there won't load on this Skyrim (the launcher replaces it before Play)".to_string());
    } else {
        missing.push("Crash Logger: not installed (the launcher installs it before Play)".to_string());
    }
    if requirements::souls_ok(i.game_dir) {
        have.push("Skyrim Souls RE");
    } else {
        missing.push("Skyrim Souls RE: not installed (the launcher installs it before Play)".to_string());
    }
    for m in requirements::missing_nexus_mods(i.game_dir, None) {
        missing.push(format!("{}: {} is missing", m.name, m.looks_for));
    }
    for (ok, name) in [
        (requirements::engine_fixes_ok(i.game_dir) && requirements::engine_fixes_preload_ok(i.game_dir), "SSE Engine Fixes"),
        (requirements::ussep_ok(i.game_dir), "Unofficial Skyrim Special Edition Patch"),
        (requirements::menu_framework_ok(i.game_dir), "SKSE Menu Framework"),
        (requirements::imgui_icons_ok(i.game_dir), "ImGui Icons"),
        (requirements::skyui_ok(i.game_dir), "SkyUI"),
    ] {
        if ok {
            have.push(name);
        }
    }
    if i.game_dir.join("skse64_loader.exe").is_file() {
        have.push("SKSE64");
    } else {
        missing.push("SKSE64: skse64_loader.exe is missing".to_string());
    }
    if missing.is_empty() {
        check("requirements", "Required mods", Status::Ok, format!("{} installed.", have.join(", ")), vec![])
    } else {
        let fail = missing.iter().any(|m| !m.contains("(the launcher installs it before Play)"));
        check(
            "requirements",
            "Required mods",
            if fail { Status::Fail } else { Status::Warn },
            "Aetherial Dawn needs SKSE64 2.2.6, the Address Library, Crash Logger, Skyrim Souls RE, SSE Engine Fixes (All-In-One), the Unofficial Skyrim Special Edition Patch, SKSE Menu Framework and ImGui Icons.",
            missing,
        )
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

    #[test]
    fn names_menu_framework_files_as_the_cause() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let pl = g.join("Data/SKSE/Plugins");
        std::fs::create_dir_all(pl.join("fonts")).unwrap();
        std::fs::write(pl.join("SKSEMenuFramework.dll"), b"x").unwrap();
        std::fs::write(pl.join("fonts/fa-solid-900.ttf"), b"x").unwrap();
        crate::strays::move_aside(g, &["Data/SKSE/Plugins/fonts/fa-solid-900.ttf".into()], "s-other-mods").unwrap();
        let items = missing_required_files(g);
        assert_eq!(items.len(), 4, "{items:?}");
        assert!(items[3].ends_with("Data/SKSE/Plugins/fonts/fa-solid-900.ttf"));
        let r = Report { checks: vec![check("loadorder", "Load order", Status::Warn, "", vec![]), check("requiredfiles", "Required mods' files", Status::Fail, "", items)], worst: Status::Fail };
        assert!(likely_cause(&r).unwrap().starts_with("A required mod is missing"));
        crate::allowlist::restore_kept(g).unwrap();
        std::fs::create_dir_all(pl.join("SKSEMenuFrameworkThemes")).unwrap();
        std::fs::write(pl.join("SKSEMenuFrameworkThemes/modern.json"), b"{}").unwrap();
        std::fs::write(pl.join("SKSEMenuFrameworkStrings_EN.json"), b"{}").unwrap();
        assert!(missing_required_files(g).is_empty());
        // A too-new Unofficial Patch parked on purpose isn't missing.
        std::fs::create_dir_all(g.join("Data/BashTags")).unwrap();
        std::fs::write(g.join("Data/BashTags/unofficial skyrim special edition patch.txt"), b"x").unwrap();
        crate::strays::move_aside(g, &["Data/BashTags/unofficial skyrim special edition patch.txt".into()], "1-too-new-ussep").unwrap();
        assert!(missing_required_files(g).is_empty());
    }
}
