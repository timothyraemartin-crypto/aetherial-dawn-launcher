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
use crate::aside::DISABLED_DIR;
use crate::{gameini, loadorder, requirements, version, watch};

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
        creation_club_light(i),
        load_order(i),
        server_order(i),
        load_order_file(i),
        stub_plugins(i),
        newer_plugins(i),
        ini_archives(i),
        injectors(i),
        overlays(),
        steam_updates(i),
        requirements_check(i),
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
        // A master the PC runs as a converted copy (a listed .esl as
        // "<stem>.esm") is checked by its canonical bytes, what every PC's
        // copy holds; `sha256` and `size` there are the game's own file.
        let canon = ["canonical_sha256", "canonicalSha256"].iter().find_map(|k| e.get(*k)).and_then(|s| s.as_str()).filter(|s| !s.is_empty());
        let size = match canon {
            Some(_) => ["canonical_size", "canonicalSize"].iter().find_map(|k| e.get(*k)).and_then(|s| s.as_u64()),
            None => e.get("size").and_then(|s| s.as_u64()),
        };
        let sha = canon.or_else(|| e.get("sha256").and_then(|s| s.as_str())).map(|s| s.to_ascii_lowercase());
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

/// Creation Club .esm/.esp files on the server's list are run as they are
/// (aliases.rs `shipped_with_game`), so one with the light (ESL) flag would
/// load as a light plugin on PCs while the server gives it a full index, and
/// everything after it would be numbered differently. Steam's files don't
/// carry the flag today (world-mods/TONIGHT-MODS.md step 0); this catches it
/// if that ever changes (references audit M15).
fn creation_club_light(i: &Inputs) -> Check {
    let data = i.game_dir.join("Data");
    let want = i.masters.map(parse_masters).unwrap_or_default();
    let flagged: Vec<String> = want
        .iter()
        .map(|(name, _, _)| name)
        .filter(|n| {
            let l = n.to_ascii_lowercase();
            (l.ends_with(".esm") || l.ends_with(".esp"))
                && crate::aliases::shipped_with_game(n)
                && !MASTERS.iter().any(|m| m.eq_ignore_ascii_case(n))
        })
        .filter(|n| light_flag_set(&data.join(n)))
        .cloned()
        .collect();
    if flagged.is_empty() {
        check("cclight", "Creation Club plugins", Status::Ok, "No Creation Club plugin the server uses is marked as a light plugin.", vec![])
    } else {
        check("cclight", "Creation Club plugins", Status::Fail, "A Creation Club file the server uses is marked as a light plugin, so your game and the server would number its contents differently. Tell staff, and don't play until they say it's fixed.", flagged)
    }
}

/// The TES4 header's light (ESL) flag, 0x200.
fn light_flag_set(path: &Path) -> bool {
    let mut b = [0u8; 12];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut b)).is_ok()
        && &b[..4] == b"TES4"
        && u32::from_le_bytes([b[8], b[9], b[10], b[11]]) & 0x200 != 0
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
    let Some(masters) = i.masters else {
        return check("serverorder", "Server plugin order", Status::Warn, "Couldn't load the server's plugin list, so the order wasn't checked.", vec![]);
    };
    let order = crate::serverorder::server_order(masters);
    if !crate::serverorder::beyond_base(&order) {
        return check("serverorder", "Server plugin order", Status::Ok, "The server loads only the five base masters.", vec![]);
    }
    let Some(dir) = i.appdata else {
        return check("serverorder", "Server plugin order", Status::Info, "Couldn't find the load order folder.", vec![]);
    };
    // As the game will start: Play empties Skyrim.ccc for the session, so the
    // Creation Club masters it lists (restored when the game closes) don't count.
    let bad = crate::serverorder::mismatches_with(i.game_dir, &dir.join("plugins.txt"), &order, false);
    if bad.is_empty() {
        check("serverorder", "Server plugin order", Status::Ok, format!("The game loads the server's {} plugins in the same order.", order.len()), vec![])
    } else {
        check("serverorder", "Server plugin order", Status::Fail, "The game's plugins don't match the server's order, so the game would refuse to connect. The launcher sets the order before Play.", bad)
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

/// The launcher's own best guess at a crash's cause, from the checks, most
/// specific first. None when nothing points anywhere.
/// What Play says when the server's masters don't match. Steam's own files
/// (the base masters, the free Creation Club masters, _ResourcePack) never
/// come from a mod: one that's missing comes back through Steam's Verify,
/// then Fix version; one that's there but a different build (or that the
/// launcher set aside as made for a newer Skyrim) needs Fix version only.
pub fn masters_fix_message(game_dir: &Path, items: &[String], exe_is_servers: bool) -> String {
    let split = |i: &String| {
        let (n, why) = i.split_once(": ").unwrap_or((i.as_str(), ""));
        (n.to_string(), why == "missing")
    };
    let steam = |n: &str| crate::patcher::patchable(&format!("Data/{n}")) || crate::aliases::shipped_with_game(n) || n.to_ascii_lowercase().starts_with("_resourcepack.");
    let set_aside = |n: &str| {
        std::fs::read_dir(game_dir.join(DISABLED_DIR)).into_iter().flatten().flatten()
            .any(|e| e.file_name().to_string_lossy().ends_with("-plugins") && e.path().join("Data").join(n).is_file())
    };
    let which = items.iter().take(3).cloned().collect::<Vec<_>>().join("; ");
    let all: Vec<(String, bool)> = items.iter().map(split).collect();
    let join = |v: Vec<&String>| v.iter().map(|n| n.as_str()).collect::<Vec<_>>().join(", ");
    let newer: Vec<&String> = all.iter().filter(|(n, missing)| *missing && steam(n) && set_aside(n)).map(|(n, _)| n).collect();
    if !newer.is_empty() {
        let fix = if exe_is_servers {
            "In Steam, right-click Skyrim Special Edition, Properties, Installed Files, Verify integrity of game files; then click Fix version"
        } else {
            "Click Fix version to change your game to the server's version"
        };
        return format!("Steam updated {} for a newer Skyrim, so the launcher set it aside. {fix}, then try Play: {which}", join(newer));
    }
    if all.is_empty() || !all.iter().all(|(n, _)| steam(n)) {
        return format!("Some files the server needs don't match yours. For a file the server sends, check for updates; for one of Skyrim's own files, click Fix version. Then try Play: {which}");
    }
    let gone: Vec<&String> = all.iter().filter(|(_, missing)| *missing).map(|(n, _)| n).collect();
    // With SkyrimSE.exe already the server's build, Fix version has nothing
    // to change ("already the server's version"): only Steam's Verify puts
    // the other files back, and then Fix version brings them all down.
    if gone.is_empty() && exe_is_servers {
        return format!("Some of your game files are a different Skyrim version from the server's. In Steam, right-click Skyrim Special Edition, Properties, Installed Files, Verify integrity of game files; then click Fix version and try Play: {which}");
    }
    if gone.is_empty() {
        return format!("Your game files are a different Skyrim version from the server's. Click Fix version, then try Play: {which}");
    }
    format!("{} is missing from your Skyrim. In Steam, right-click Skyrim Special Edition, Properties, Installed Files, Verify integrity of game files; then click Fix version and try Play: {which}", join(gone))
}

pub fn likely_cause(r: &Report) -> Option<String> {
    let failed = |id: &str, st: Status| r.checks.iter().find(|c| c.id == id && c.status >= st);
    for (id, why) in [
        ("exe", "Wrong Skyrim version"),
        ("masters", "Base game files don't match the server"),
        ("newer", "Plugins made for a newer Skyrim"),
    ] {
        if failed(id, Status::Warn).is_some() {
            return Some(why.to_string());
        }
    }
    if failed("loadorder", Status::Warn).is_some() {
        return Some("Plugins the game can't load are switched on".to_string());
    }
    for (id, why) in [
        ("firewall", "Windows Firewall may be blocking Skyrim"),
        ("udp", "The game server didn't answer on UDP (firewall, router or VPN)"),
        ("connect", "Skyrim ran but never connected to the server"),
    ] {
        if failed(id, Status::Warn).is_some() {
            return Some(why.to_string());
        }
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

/// What SkyMP needs from the PC: SKSE64 and the Address Library file for the
/// server's build.
fn requirements_check(i: &Inputs) -> Check {
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
    if requirements::skse_ok(i.game_dir) {
        have.push("SKSE64");
    } else {
        missing.push("SKSE64: not installed (the launcher installs it before Play)".to_string());
    }
    if missing.is_empty() {
        check("requirements", "Required files", Status::Ok, format!("{} installed.", have.join(", ")), vec![])
    } else {
        let fail = missing.iter().any(|m| !m.contains("(the launcher installs it before Play)"));
        check(
            "requirements",
            "Required files",
            if fail { Status::Fail } else { Status::Warn },
            "Aetherial Dawn needs SKSE64 2.2.6 and the Address Library for SKSE Plugins.",
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
    fn a_light_flagged_creation_club_master_on_the_server_list_fails() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("Data");
        std::fs::create_dir_all(&data).unwrap();
        let header = |flags: u32| { let mut b = b"TES4".to_vec(); b.extend(0u32.to_le_bytes()); b.extend(flags.to_le_bytes()); b.extend([0u8; 12]); b };
        std::fs::write(data.join("ccBGSSSE001-Fish.esm"), header(0x1)).unwrap();
        std::fs::write(data.join("ccBGSSSE025-AdvDSGS.esm"), header(0x1)).unwrap();
        std::fs::write(data.join("Some Mod.esp"), header(0x200)).unwrap();
        let masters = serde_json::json!([{"name": "ccBGSSSE001-Fish.esm"}, {"name": "ccBGSSSE025-AdvDSGS.esm"}, {"name": "Some Mod.esp"}, {"name": "ccQDRSSE001-SurvivalMode.esl"}]);
        let run = || creation_club_light(&Inputs { game_dir: t.path(), manifest: None, masters: Some(&masters), appdata: None, documents: None, hash_cache: None, home: None });
        assert_eq!(run().status, Status::Ok);
        std::fs::write(data.join("ccBGSSSE025-AdvDSGS.esm"), header(0x201)).unwrap();
        let c = run();
        assert_eq!((c.status, c.items), (Status::Fail, vec!["ccBGSSSE025-AdvDSGS.esm".to_string()]));
    }

    #[test]
    fn a_steam_master_mismatch_points_to_verify_and_fix_version_not_a_mod() {
        let tmp = tempfile::tempdir().unwrap();
        let g = tmp.path();
        let cc = vec!["ccBGSSSE037-Curios.esl: missing".to_string(), "_ResourcePack.esl: same size, different contents".to_string()];
        let m = masters_fix_message(g, &cc, false);
        assert!(m.starts_with("ccBGSSSE037-Curios.esl is missing") && m.contains("Verify integrity") && m.contains("Fix version") && !m.contains("server mod"), "{m}");
        // Present but another build: Fix version only, no Steam Verify.
        let changed = ["_ResourcePack.esl: same size, different contents".to_string(), "Skyrim.esm: 10 bytes, server has 12".to_string()];
        let m = masters_fix_message(g, &changed, false);
        assert!(m.contains("Click Fix version") && !m.contains("Verify"), "{m}");
        // The exe is already the server's: Fix version would say so and stop,
        // so Steam's Verify comes first.
        let m = masters_fix_message(g, &changed, true);
        assert!(m.contains("Verify integrity") && m.contains("then click Fix version"), "{m}");
        let modded = vec!["ccBGSSSE001-Fish.esm: missing".to_string(), "JKs-Skyrim.esp: missing".to_string()];
        assert!(masters_fix_message(g, &modded, false).contains("check for updates"));
        let aside = g.join(DISABLED_DIR).join("2026-09-30-19-40-00-plugins/Data");
        std::fs::create_dir_all(&aside).unwrap();
        std::fs::write(aside.join("ccBGSSSE001-Fish.esm"), b"newer").unwrap();
        let m = masters_fix_message(g, &["ccBGSSSE001-Fish.esm: missing".to_string()], false);
        assert!(m.starts_with("Steam updated ccBGSSSE001-Fish.esm for a newer Skyrim") && m.contains("Fix version"), "{m}");
        // Set aside while the exe is already the server's: Verify first.
        let m = masters_fix_message(g, &["ccBGSSSE001-Fish.esm: missing".to_string()], true);
        assert!(m.starts_with("Steam updated ccBGSSSE001-Fish.esm") && m.contains("Verify integrity") && m.contains("then click Fix version"), "{m}");
    }

    #[test]
    fn parses_master_list_shapes() {
        let a = serde_json::json!({"masters":[{"name":"Skyrim.esm","size":3,"sha256":"AB"}]});
        assert_eq!(parse_masters(&a), vec![("Skyrim.esm".into(), Some(3), Some("ab".into()))]);
        let b = serde_json::json!([{"path":"Data/Update.esm","size":1}]);
        assert_eq!(parse_masters(&b)[0].0, "Update.esm");
        let c = serde_json::json!({"Dawnguard.esm":{"size":2}});
        assert_eq!(parse_masters(&c)[0], ("Dawnguard.esm".into(), Some(2), None));
        // A converted master is checked by its canonical hash, and by the
        // canonical size only when the list gives one.
        let d = serde_json::json!({"masters": [{"name": "_ResourcePack.esm", "size": 9, "sha256": "AA", "canonical_sha256": "BB"},
            {"name": "ccQDRSSE001-SurvivalMode.esm", "size": 9, "sha256": "aa", "canonicalSha256": "cc", "canonicalSize": 8}]});
        assert_eq!(parse_masters(&d), vec![("_ResourcePack.esm".into(), None, Some("bb".into())), ("ccQDRSSE001-SurvivalMode.esm".into(), Some(8), Some("cc".into()))]);
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
    fn no_server_plugin_list_is_a_warning_not_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let i = Inputs { game_dir: tmp.path(), manifest: None, masters: None, appdata: Some(tmp.path()), documents: None, hash_cache: None, home: None };
        let c = server_order(&i);
        assert_eq!(c.status, Status::Warn, "{}", c.detail);
        let base = serde_json::json!({"masters":[{"name":"Skyrim.esm","size":1}]});
        let c = server_order(&Inputs { masters: Some(&base), ..i });
        assert_eq!(c.status, Status::Ok);
    }
}
