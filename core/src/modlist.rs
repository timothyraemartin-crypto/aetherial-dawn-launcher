//! The server's mod list and the launcher's own mod installer (Timothy,
//! 2026-09-26: "a download all mods for the mod list", one click for Nexus
//! Premium and free players alike).
//!
//! The list is built into the launcher (the required mods it already knows)
//! and extended or overridden by `<base>/mods.json` on the server, so staff can
//! add mods without a launcher release. Each mod comes from Nexus Mods or from
//! a direct link (GitHub). Archives (zip or 7z) are unpacked by the launcher:
//! a FOMOD installer is answered from the list's choices or its own defaults,
//! otherwise the folder that holds the game data is found by itself. What was
//! installed is recorded in `.aetherial-dawn/mods/installed.json`.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

pub const NEXUS_GAME: &str = "skyrimspecialedition";
/// Where the launcher keeps downloads and its record, inside the game folder.
pub const MODS_DIR: &str = ".aetherial-dawn/mods";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct NexusRef {
    /// Nexus Mods mod id (the number in the page address).
    #[serde(rename = "mod")]
    pub mod_id: u64,
    /// A specific file id. Without one the newest main file is used (or the
    /// newest one whose name contains `pick`).
    #[serde(default)]
    pub file: Option<u64>,
    /// Text in the file's name that picks it on the Files tab.
    #[serde(default)]
    pub pick: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Target {
    /// Into Data (the usual place for mods).
    #[default]
    Data,
    /// Next to SkyrimSE.exe (preloaders and runtime libraries).
    Game,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ModEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub nexus: Option<NexusRef>,
    /// A direct download (GitHub release) when the mod isn't on Nexus.
    #[serde(default)]
    pub url: Option<String>,
    /// SHA-256 of the archive, when the list pins one.
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub target: Target,
    /// With target "game": the file names to take from the archive.
    #[serde(default)]
    pub include: Vec<String>,
    /// With target "data": file names that go next to SkyrimSE.exe instead,
    /// when the package has them (an all-in-one package with a preloader).
    #[serde(default)]
    pub game_files: Vec<String>,
    /// FOMOD options to pick, by (part of) their name.
    #[serde(default)]
    pub fomod: Vec<String>,
    /// Paths relative to the game folder that must exist when it's installed.
    #[serde(default)]
    pub check: Vec<String>,
    /// Shown under the mod's name: which file to pick on Nexus.
    #[serde(default)]
    pub hint: Option<String>,
}

impl ModEntry {
    pub fn installed(&self, game_dir: &Path) -> bool {
        !self.check.is_empty() && self.check.iter().all(|c| safe_rel(c).map(|r| present_like(&game_dir.join(r))).unwrap_or(false))
    }

    /// A copy that checks for one file only.
    pub fn clone_with_check(&self, c: &str) -> ModEntry {
        ModEntry { check: vec![c.to_string()], ..self.clone() }
    }

    pub fn page(&self) -> Option<String> {
        let n = self.nexus.as_ref()?;
        Some(match n.file {
            Some(f) => format!("https://www.nexusmods.com/{NEXUS_GAME}/mods/{}?tab=files&file_id={f}", n.mod_id),
            None => format!("https://www.nexusmods.com/{NEXUS_GAME}/mods/{}?tab=files", n.mod_id),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModList {
    #[serde(default)]
    pub mods: Vec<ModEntry>,
    /// The application name Nexus Mods registered for the launcher's
    /// "Sign in with Nexus" (SSO). Without one players paste an API key.
    #[serde(default)]
    pub nexus_app: Option<String>,
}

/// The mods the launcher requires on its own (Skyrim Souls RE's
/// dependencies and the Address Library). The GitHub-hosted required mods
/// (SKSE, Crash Logger, Souls RE, Engine Fixes part 1) are installed by
/// `requirements` before Play and aren't repeated here.
pub fn builtin(game_version: Option<&str>) -> Vec<ModEntry> {
    use crate::requirements as r;
    let mut out = Vec::new();
    if let Some(v) = game_version {
        out.push(ModEntry {
            id: "address-library".into(),
            name: "Address Library for SKSE Plugins".into(),
            nexus: Some(NexusRef { mod_id: 32444, file: None, pick: Some("All in one (Anniversary Edition)".into()) }),
            check: vec![format!("Data/SKSE/Plugins/{}", r::address_library_file(v))],
            hint: Some("All in one (Anniversary Edition)".into()),
            ..Default::default()
        });
    }
    let mut ef_check = vec!["Data/SKSE/Plugins/EngineFixes.dll".to_string()];
    ef_check.push("Data/SKSE/Plugins/EngineFixes_preload.txt".to_string());
    out.push(ModEntry {
        id: "engine-fixes".into(),
        name: "SSE Engine Fixes (All-In-One)".into(),
        nexus: Some(NexusRef { mod_id: 17230, file: None, pick: Some("All-In-One".into()) }),
        game_files: r::ENGINE_FIXES_PRELOAD.iter().map(|s| s.to_string()).collect(),
        fomod: vec!["AE".into(), "1.6.1170".into()],
        check: ef_check,
        hint: Some("Engine Fixes (All-In-One) for 1.6.1170 and newer".into()),
        ..Default::default()
    });
    out.push(ModEntry {
        id: "ussep".into(),
        name: "Unofficial Skyrim Special Edition Patch".into(),
        // 4.3.9 and later need Skyrim 1.7.99 and crash 1.6.1170 (crate::ussep).
        nexus: Some(NexusRef { mod_id: 266, file: None, pick: Some(crate::ussep::NEXUS_PICK.into()) }),
        check: vec![format!("Data/{}", r::USSEP_PLUGIN)],
        hint: Some("version 4.3.8a under Old files, the one for Skyrim 1.6.1170 (not 4.3.9 or newer)".into()),
        ..Default::default()
    });
    out.push(ModEntry {
        id: "menu-framework".into(),
        name: "SKSE Menu Framework".into(),
        nexus: Some(NexusRef { mod_id: 120352, file: None, pick: None }),
        check: vec![format!("Data/SKSE/Plugins/{}", r::MENU_FRAMEWORK_DLL)],
        hint: Some("the main file".into()),
        ..Default::default()
    });
    out.push(ModEntry {
        id: "imgui-icons".into(),
        name: "ImGui Icons".into(),
        nexus: Some(NexusRef { mod_id: 114790, file: None, pick: None }),
        check: vec![format!("Data/Interface/{}", r::IMGUI_ICONS_DIR)],
        hint: Some("the main file".into()),
        ..Default::default()
    });
    out.push(ModEntry {
        id: "skyui".into(),
        name: "SkyUI".into(),
        nexus: Some(NexusRef { mod_id: 12604, file: None, pick: Some("SkyUI".into()) }),
        check: vec![format!("Data/{}", r::SKYUI_PLUGIN), format!("Data/{}", r::SKYUI_ARCHIVE)],
        hint: Some("SkyUI 5.2SE (main file)".into()),
        ..Default::default()
    });
    // Timothy, 2026-09-26: "add these two mods" (against a black screen).
    // Display Tweaks first, so the fix's ini lands over its default one.
    out.push(ModEntry {
        id: "display-tweaks".into(),
        name: "SSE Display Tweaks".into(),
        nexus: Some(NexusRef { mod_id: 34705, file: None, pick: Some("AE".into()) }),
        check: vec![format!("Data/SKSE/Plugins/{}", r::DISPLAY_TWEAKS_DLL)],
        hint: Some("the main file for Anniversary Edition (1.6)".into()),
        ..Default::default()
    });
    out.push(ModEntry {
        id: "black-screen-fix".into(),
        name: "Black Screen and Startup Fix".into(),
        nexus: Some(NexusRef { mod_id: 176509, file: None, pick: Some("1080".into()) }),
        check: vec![format!("Data/SKSE/Plugins/{}", r::DISPLAY_TWEAKS_INI)],
        hint: Some("the file for your screen (1080p or 1440p)".into()),
        ..Default::default()
    });
    // Timothy, 2026-09-26: "These mods need added". Client-only camera,
    // movement and HUD mods, plus MCM Helper, which TDM and TrueHUD's menus
    // need. Checked by DLL; their plugins are switched on when present
    // (loadorder::COMPANION_PLUGINS).
    out.push(ModEntry {
        id: "mcm-helper".into(),
        name: "MCM Helper".into(),
        nexus: Some(NexusRef { mod_id: 53000, file: None, pick: None }),
        check: vec!["Data/SKSE/Plugins/MCMHelper.dll".into()],
        hint: Some("the main file".into()),
        ..Default::default()
    });
    out.push(ModEntry {
        id: "smoothcam".into(),
        name: "SmoothCam".into(),
        nexus: Some(NexusRef { mod_id: 41252, file: None, pick: Some("AE".into()) }),
        check: vec!["Data/SKSE/Plugins/SmoothCam.dll".into()],
        hint: Some("the main file for Anniversary Edition (1.6)".into()),
        ..Default::default()
    });
    // Timothy, 2026-09-26: "this is the mod I'm going to use". A SmoothCam
    // preset file (SmoothCamPreset<slot>.json); `camera` makes it the
    // default camera once.
    out.push(ModEntry {
        id: "smoothcam-modern-preset".into(),
        name: "SmoothCam - Modern Camera Preset".into(),
        nexus: Some(NexusRef { mod_id: 41636, file: None, pick: None }),
        check: vec!["Data/SKSE/Plugins/SmoothCamPreset*.json".into()],
        hint: Some("the main file".into()),
        ..Default::default()
    });
    out.push(ModEntry {
        id: "true-directional-movement".into(),
        name: "True Directional Movement".into(),
        nexus: Some(NexusRef { mod_id: 51614, file: None, pick: Some("AE".into()) }),
        check: vec!["Data/SKSE/Plugins/TrueDirectionalMovement.dll".into()],
        hint: Some("the main file for Anniversary Edition (1.6)".into()),
        ..Default::default()
    });
    out.push(ModEntry {
        id: "truehud".into(),
        name: "TrueHUD".into(),
        nexus: Some(NexusRef { mod_id: 62775, file: None, pick: None }),
        check: vec!["Data/SKSE/Plugins/TrueHUD.dll".into()],
        hint: Some("the main file".into()),
        ..Default::default()
    });
    out
}

/// The built-in list with the server's list laid over it: a server entry with
/// the same id replaces the built-in one, new ids are added after. Entries
/// with unsafe check paths or no source are dropped.
pub fn merged(game_version: Option<&str>, server: Option<&ModList>) -> Vec<ModEntry> {
    let mut out = builtin(game_version);
    if let Some(s) = server {
        for m in &s.mods {
            if m.id.is_empty() || (m.nexus.is_none() && m.url.is_none()) || m.check.iter().any(|c| safe_rel(c).is_none()) {
                continue;
            }
            if let Some(url) = &m.url {
                if !url.starts_with("https://") {
                    continue;
                }
            }
            match out.iter_mut().find(|e| e.id == m.id) {
                Some(e) => *e = m.clone(),
                None => out.push(m.clone()),
            }
        }
    }
    out
}

/// A file or folder that's really there; a plugin must also be a sound one,
/// not an empty stub like the SkyUI_SE.esp from the first live test, and an
/// SKSE DLL must be the build SKSE loads on 1.6.1170 (not the old-Skyrim
/// True Directional Movement Vortex deployed, 2026-09-26).
fn present(p: &Path) -> bool {
    if !p.exists() || ussep_too_new(p) {
        return false;
    }
    if is_plugin(p) && crate::loadorder::broken(p).is_some() {
        return false;
    }
    !(is_skse_dll(p) && crate::skse::wrong_build(p).is_some())
}

/// The Unofficial Patch made for Skyrim 1.7.99 doesn't count on 1.6.1170.
fn ussep_too_new(p: &Path) -> bool {
    let is_ussep = p.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case(crate::requirements::USSEP_PLUGIN)).unwrap_or(false);
    is_ussep && p.parent().and_then(Path::parent).map(|g| crate::ussep::too_new(g).is_some()).unwrap_or(false)
}

/// Like `present`, with one `*` allowed in the file name (a SmoothCam preset
/// can sit in any of its preset slots).
fn present_like(p: &Path) -> bool {
    let name = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let Some((head, tail)) = name.split_once('*') else { return present(p) };
    let Some(dir) = p.parent() else { return false };
    std::fs::read_dir(dir)
        .map(|r| {
            r.flatten().any(|e| {
                let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                n.len() >= head.len() + tail.len() && n.starts_with(head) && n.ends_with(tail) && present(&e.path())
            })
        })
        .unwrap_or(false)
}

fn is_skse_dll(p: &Path) -> bool {
    let l = p.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    l.ends_with(".dll") && l.contains("/skse/plugins/")
}

/// SKSE DLLs in the plan that SKSE would refuse, with the reason. Where the
/// archive also has a same-named DLL that fits (an installer with an SE and
/// an AE folder), the plan is switched to that one first.
pub fn fix_wrong_builds(copies: &mut [Copy], unpacked: &Path) -> Vec<(String, String)> {
    let mut wrong = Vec::new();
    for c in copies.iter_mut() {
        if !crate::skse::is_skse_plugin(&c.to.to_string_lossy()) {
            continue;
        }
        let Some(why) = crate::skse::wrong_build(&c.from) else { continue };
        let name = c.to.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let better = files_under(unpacked).into_iter().find(|f| f.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase() == name).unwrap_or(false) && crate::skse::build_of(f) == crate::skse::Build::Fits);
        match better {
            Some(f) => c.from = f,
            None => wrong.push((c.to.file_name().unwrap().to_string_lossy().into_owned(), why)),
        }
    }
    wrong
}

fn is_plugin(p: &Path) -> bool {
    let l = p.to_string_lossy().to_ascii_lowercase();
    l.ends_with(".esp") || l.ends_with(".esm") || l.ends_with(".esl")
}

pub fn missing<'a>(list: &'a [ModEntry], game_dir: &Path) -> Vec<&'a ModEntry> {
    list.iter().filter(|m| !m.installed(game_dir)).collect()
}

/// Whether Vortex deploys mods into this game's Data folder.
pub fn vortex_manages(game_dir: &Path) -> bool {
    let data = game_dir.join("Data");
    data.join("vortex.deployment.json").is_file() || data.join("__folder_managed_by_vortex").exists()
}

/// A relative path with no parent steps or roots, using forward slashes.
fn safe_rel(p: &str) -> Option<PathBuf> {
    let p = p.replace('\\', "/");
    if p.is_empty() || p.starts_with('/') || p.contains(':') {
        return None;
    }
    let mut out = PathBuf::new();
    for c in p.split('/') {
        if c.is_empty() || c == "." {
            continue;
        }
        if c == ".." {
            return None;
        }
        out.push(c);
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

// ---------- unpacking ----------

/// Unpacks a zip or 7z archive into `dir`, skipping unsafe paths.
pub fn extract(archive: &Path, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut head = [0u8; 6];
    {
        use std::io::Read;
        let mut f = std::fs::File::open(archive)?;
        let _ = f.read(&mut head)?;
    }
    if head.starts_with(b"PK") {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?).map_err(|e| Error::Game(format!("the download isn't a readable zip: {e}")))?;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).map_err(|e| Error::Game(e.to_string()))?;
            let Some(rel) = safe_rel(f.name()) else { continue };
            let dest = dir.join(rel);
            if f.is_dir() {
                std::fs::create_dir_all(&dest)?;
                continue;
            }
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p)?;
            }
            let mut out = std::fs::File::create(&dest)?;
            std::io::copy(&mut f, &mut out)?;
        }
        Ok(())
    } else if head == [b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C] {
        let mut reader = sevenz_rust2::ArchiveReader::open(archive, sevenz_rust2::Password::empty()).map_err(|e| Error::Game(format!("the download isn't a readable 7z: {e}")))?;
        reader
            .for_each_entries(|entry, data| {
                let Some(rel) = safe_rel(entry.name()) else { return Ok(true) };
                let dest = dir.join(rel);
                if entry.is_directory() {
                    std::fs::create_dir_all(&dest)?;
                    return Ok(true);
                }
                if let Some(p) = dest.parent() {
                    std::fs::create_dir_all(p)?;
                }
                let mut out = std::fs::File::create(&dest)?;
                std::io::copy(data, &mut out)?;
                Ok(true)
            })
            .map_err(|e| Error::Game(format!("couldn't unpack the download: {e}")))?;
        Ok(())
    } else if head.starts_with(b"Rar!") {
        Err(Error::Game("this mod is packed as RAR, which the launcher can't unpack yet. Install it with Vortex".into()))
    } else {
        Err(Error::Game("the download isn't a zip or 7z archive".into()))
    }
}

// ---------- planning what goes where ----------

/// One file to copy: from the unpacked archive to a path relative to the game folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Copy {
    pub from: PathBuf,
    pub to: PathBuf,
}

const DATA_DIRS: [&str; 14] = ["skse", "interface", "meshes", "textures", "scripts", "sound", "seq", "platform", "strings", "music", "shaders", "lodsettings", "grass", "calientetools"];
const DATA_EXTS: [&str; 4] = ["esp", "esm", "esl", "bsa"];

fn looks_like_data(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    rd.flatten().any(|e| {
        let n = e.file_name().to_string_lossy().to_ascii_lowercase();
        if e.path().is_dir() {
            DATA_DIRS.contains(&n.as_str())
        } else {
            n.rsplit_once('.').map(|(_, x)| DATA_EXTS.contains(&x)).unwrap_or(false)
        }
    })
}

/// The shallowest folder (breadth first) that holds game data.
fn data_root(root: &Path) -> Option<PathBuf> {
    let mut level = vec![root.to_path_buf()];
    for _ in 0..4 {
        let mut next = Vec::new();
        for d in &level {
            if looks_like_data(d) {
                return Some(d.clone());
            }
            if let Ok(rd) = std::fs::read_dir(d) {
                let mut subs: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
                subs.sort();
                // "Data" inside an archive is the Data folder itself.
                if let Some(data) = subs.iter().find(|p| p.file_name().map(|n| n.eq_ignore_ascii_case("data")).unwrap_or(false)) {
                    return Some(data.clone());
                }
                next.extend(subs.into_iter().filter(|p| !p.file_name().map(|n| n.eq_ignore_ascii_case("fomod")).unwrap_or(false)));
            }
        }
        level = next;
    }
    None
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out
}

fn copy_tree(src: &Path, dest_rel: &Path, out: &mut Vec<Copy>) {
    if src.is_file() {
        out.push(Copy { from: src.to_path_buf(), to: dest_rel.to_path_buf() });
        return;
    }
    for f in files_under(src) {
        if let Ok(rel) = f.strip_prefix(src) {
            out.push(Copy { from: f.clone(), to: dest_rel.join(rel) });
        }
    }
}

/// A path inside the unpacked archive, ignoring case (FOMOD sources are often
/// written with different case than the files).
fn find_ci(root: &Path, rel: &str) -> Option<PathBuf> {
    let mut cur = root.to_path_buf();
    for c in safe_rel(rel)?.iter() {
        let want = c.to_string_lossy().to_ascii_lowercase();
        let hit = std::fs::read_dir(&cur).ok()?.flatten().find(|e| e.file_name().to_string_lossy().to_ascii_lowercase() == want)?;
        cur = hit.path();
    }
    Some(cur)
}

/// Decides which unpacked files go where, relative to the game folder.
pub fn plan(entry: &ModEntry, unpacked: &Path) -> Result<Vec<Copy>> {
    let mut out = Vec::new();
    if entry.target == Target::Game {
        let want: Vec<String> = entry.include.iter().map(|s| s.to_ascii_lowercase()).collect();
        let mut seen = HashSet::new();
        for f in files_under(unpacked) {
            let name = f.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            if (want.is_empty() || want.contains(&name)) && seen.insert(name.clone()) {
                out.push(Copy { from: f.clone(), to: PathBuf::from(f.file_name().unwrap()) });
            }
        }
        if !want.is_empty() && seen.len() < want.len() {
            return Err(Error::Game(format!("the download for {} is missing some of {}", entry.name, entry.include.join(", "))));
        }
        return Ok(out);
    }
    if let Some(config) = find_ci(unpacked, "fomod/ModuleConfig.xml").or_else(|| {
        // The fomod folder can sit one level down.
        std::fs::read_dir(unpacked).ok()?.flatten().filter(|e| e.path().is_dir()).find_map(|e| find_ci(&e.path(), "fomod/ModuleConfig.xml"))
    }) {
        let root = config.parent().and_then(|p| p.parent()).unwrap_or(unpacked).to_path_buf();
        let text = read_xml_text(&config)?;
        for (src, dest) in fomod_files(&text, &entry.fomod)? {
            let Some(from) = find_ci(&root, &src) else { continue };
            let dest = if dest.trim().is_empty() {
                // A file with no destination keeps its name at the top of Data.
                if from.is_file() { PathBuf::from(from.file_name().unwrap()) } else { PathBuf::new() }
            } else {
                safe_rel(&dest).ok_or_else(|| Error::Game(format!("the FOMOD installer has an unsafe path: {dest}")))?
            };
            copy_tree(&from, &PathBuf::from("Data").join(dest), &mut out);
        }
    } else if let Some(root) = data_root(unpacked) {
        copy_tree(&root, Path::new("Data"), &mut out);
    } else {
        return Err(Error::Game(format!("couldn't tell where {}'s files go. Install it with Vortex", entry.name)));
    }
    // Preloader files go next to SkyrimSE.exe, wherever the archive keeps them.
    if !entry.game_files.is_empty() {
        let want: Vec<String> = entry.game_files.iter().map(|s| s.to_ascii_lowercase()).collect();
        let is_game = |p: &Path| p.file_name().map(|n| want.contains(&n.to_string_lossy().to_ascii_lowercase())).unwrap_or(false);
        out.retain(|c| !is_game(&c.to));
        let mut seen = HashSet::new();
        for f in files_under(unpacked) {
            if is_game(&f) && seen.insert(f.file_name().unwrap().to_string_lossy().to_ascii_lowercase()) {
                out.push(Copy { from: f.clone(), to: PathBuf::from(f.file_name().unwrap()) });
            }
        }
    }
    // Readmes and FOMOD pictures don't belong in Data.
    out.retain(|c| {
        let l = c.to.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
        !(l.starts_with("data/fomod/") || (l.matches('/').count() == 1 && (l.ends_with(".txt") || l.ends_with(".md") || l.ends_with(".pdf") || l.ends_with(".png") || l.ends_with(".jpg"))))
    });
    if out.is_empty() {
        return Err(Error::Game(format!("the download for {} had nothing to install", entry.name)));
    }
    Ok(out)
}

/// FOMOD configs are UTF-8 or UTF-16 (with a byte order mark).
fn read_xml_text(path: &Path) -> Result<String> {
    let b = std::fs::read(path)?;
    if b.starts_with(&[0xFF, 0xFE]) || b.starts_with(&[0xFE, 0xFF]) {
        let le = b[0] == 0xFF;
        let units: Vec<u16> = b[2..].chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        return Ok(String::from_utf16_lossy(&units));
    }
    let s = String::from_utf8_lossy(&b).into_owned();
    Ok(s.trim_start_matches('\u{feff}').to_string())
}

fn child<'a, 'i>(n: roxmltree::Node<'a, 'i>, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    n.children().find(|c| c.is_element() && c.tag_name().name().eq_ignore_ascii_case(name))
}

fn children<'a, 'i>(n: roxmltree::Node<'a, 'i>, name: &'static str) -> impl Iterator<Item = roxmltree::Node<'a, 'i>> {
    n.children().filter(move |c| c.is_element() && c.tag_name().name().eq_ignore_ascii_case(name))
}

fn file_list(files: roxmltree::Node) -> Vec<(String, String)> {
    files
        .children()
        .filter(|c| c.is_element() && (c.tag_name().name() == "file" || c.tag_name().name() == "folder"))
        .map(|c| (c.attribute("source").unwrap_or("").to_string(), c.attribute("destination").unwrap_or("").to_string()))
        .filter(|(s, _)| !s.is_empty())
        .collect()
}

fn plugin_type(p: roxmltree::Node) -> String {
    let Some(td) = child(p, "typeDescriptor") else { return "Optional".into() };
    if let Some(t) = child(td, "type") {
        return t.attribute("name").unwrap_or("Optional").to_string();
    }
    child(td, "dependencyType").and_then(|d| child(d, "defaultType")).and_then(|t| t.attribute("name")).unwrap_or("Optional").to_string()
}

/// The files a FOMOD installer would install: its required files, the options
/// the mod list names (`choose`), otherwise Required and Recommended options,
/// and the first usable option of groups that need one. Conditional installs
/// that depend only on flags set by the chosen options are included.
pub fn fomod_files(xml: &str, choose: &[String]) -> Result<Vec<(String, String)>> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| Error::Game(format!("the mod's FOMOD installer is unreadable: {e}")))?;
    let root = doc.root_element();
    let mut out = Vec::new();
    if let Some(req) = child(root, "requiredInstallFiles") {
        out.extend(file_list(req));
    }
    let wanted: Vec<String> = choose.iter().map(|c| c.to_ascii_lowercase()).collect();
    let mut flags: BTreeMap<String, String> = BTreeMap::new();
    if let Some(steps) = child(root, "installSteps") {
        for step in children(steps, "installStep") {
            let Some(groups) = child(step, "optionalFileGroups") else { continue };
            for group in children(groups, "group") {
                let kind = group.attribute("type").unwrap_or("SelectAny");
                let Some(plugins) = child(group, "plugins") else { continue };
                let all: Vec<_> = children(plugins, "plugin").collect();
                let named: Vec<_> = all.iter().filter(|p| wanted.iter().any(|w| p.attribute("name").unwrap_or("").to_ascii_lowercase().contains(w.as_str()))).copied().collect();
                let mut picked: Vec<_> = if !named.is_empty() {
                    named
                } else {
                    all.iter().filter(|p| matches!(plugin_type(**p).as_str(), "Required" | "Recommended")).copied().collect()
                };
                if matches!(kind, "SelectExactlyOne" | "SelectAtMostOne") {
                    picked.truncate(1);
                }
                if picked.is_empty() && matches!(kind, "SelectExactlyOne" | "SelectAtLeastOne" | "SelectAll") {
                    picked = all.iter().filter(|p| plugin_type(**p) != "NotUsable").take(if kind == "SelectAll" { usize::MAX } else { 1 }).copied().collect();
                }
                for p in picked {
                    if let Some(files) = child(p, "files") {
                        out.extend(file_list(files));
                    }
                    if let Some(cf) = child(p, "conditionFlags") {
                        for f in children(cf, "flag") {
                            flags.insert(f.attribute("name").unwrap_or("").to_string(), f.text().unwrap_or("").trim().to_string());
                        }
                    }
                }
            }
        }
    }
    if let Some(cond) = child(root, "conditionalFileInstalls").and_then(|c| child(c, "patterns")) {
        for pat in children(cond, "pattern") {
            let Some(deps) = child(pat, "dependencies") else { continue };
            let or = deps.attribute("operator") == Some("Or");
            let tests: Vec<bool> = deps
                .children()
                .filter(|c| c.is_element())
                .map(|c| c.tag_name().name() == "flagDependency" && flags.get(c.attribute("flag").unwrap_or("")).map(|v| v == c.attribute("value").unwrap_or("")).unwrap_or(false))
                .collect();
            let ok = if or { tests.iter().any(|t| *t) } else { !tests.is_empty() && tests.iter().all(|t| *t) };
            if ok {
                if let Some(files) = child(pat, "files") {
                    out.extend(file_list(files));
                }
            }
        }
    }
    Ok(out)
}

// ---------- installing ----------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Installed {
    #[serde(default)]
    pub mods: BTreeMap<String, InstalledMod>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstalledMod {
    pub name: String,
    #[serde(default)]
    pub file_id: Option<u64>,
    #[serde(default)]
    pub version: Option<String>,
    pub files: Vec<String>,
    /// Files left alone because another tool (Vortex) already had them there.
    #[serde(default)]
    pub skipped: Vec<String>,
    pub when: u64,
}

fn record_path(game_dir: &Path) -> PathBuf {
    game_dir.join(MODS_DIR).join("installed.json")
}

pub fn load_installed(game_dir: &Path) -> Installed {
    std::fs::read(record_path(game_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Settings files the player may have changed; an existing one is kept.
fn is_settings(p: &Path) -> bool {
    let l = p.to_string_lossy().to_ascii_lowercase();
    l.ends_with(".ini") || l.ends_with(".toml") || l.ends_with(".json")
}

/// Copies the planned files into the game folder and records them. When
/// Vortex manages Data, files already there are left as they are (they're
/// Vortex's). Settings files the player already has are kept.
pub fn apply(entry: &ModEntry, copies: &[Copy], game_dir: &Path, file_id: Option<u64>, version: Option<String>) -> Result<InstalledMod> {
    let vortex = vortex_manages(game_dir);
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    for c in copies {
        let dest = game_dir.join(&c.to);
        let rel = c.to.to_string_lossy().replace('\\', "/");
        // Vortex's own copy of a file wins, unless it's a broken plugin.
        if dest.exists() && (vortex || is_settings(&c.to)) && present(&dest) {
            skipped.push(rel);
            continue;
        }
        // A DLL of the wrong build is set aside with the reason, never deleted.
        if dest.is_file() && is_skse_dll(&dest) {
            if let Some(why) = crate::skse::wrong_build(&dest) {
                set_aside_wrong_build(game_dir, &rel, &why)?;
            }
        }
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p)?;
        }
        let tmp = dest.with_extension("aetherial-part");
        std::fs::copy(&c.from, &tmp)?;
        std::fs::rename(&tmp, &dest)?;
        files.push(rel);
    }
    let rec = InstalledMod {
        name: entry.name.clone(),
        file_id,
        version,
        files,
        skipped,
        when: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    };
    let mut all = load_installed(game_dir);
    all.mods.insert(entry.id.clone(), rec.clone());
    let path = record_path(game_dir);
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(&all)?)?;
    Ok(rec)
}

/// Moves a DLL SKSE would refuse to `.aetherial-dawn/disabled/<time>-wrong-build/`
/// and notes why in `why.txt` there.
pub fn set_aside_wrong_build(game_dir: &Path, rel: &str, why: &str) -> Result<PathBuf> {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let stamp = format!("{secs}-wrong-build");
    let dest = crate::strays::move_aside(game_dir, &[rel.to_string()], &stamp)?;
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(dest.join("why.txt"))?;
    writeln!(f, "{rel}: {why}; SKSE won't load it on Skyrim 1.6.1170")?;
    Ok(dest)
}

/// Plugins in the plan made for a newer Skyrim than the game's masters
/// (a mod version for 1.7 on a 1.6.1170 game).
pub fn too_new_plugins(copies: &[Copy], game_dir: &Path) -> Vec<String> {
    let data = game_dir.join("Data");
    let base: Vec<(f32, u16)> = ["Skyrim.esm", "Update.esm", "Dawnguard.esm", "HearthFires.esm", "Dragonborn.esm"]
        .iter()
        .filter_map(|n| crate::loadorder::header(&data.join(n)))
        .collect();
    if base.len() < 5 {
        return Vec::new();
    }
    let max_v = base.iter().map(|b| b.0).fold(0.0f32, f32::max);
    let max_f = base.iter().map(|b| b.1).max().unwrap_or(0);
    copies
        .iter()
        .filter(|c| {
            let l = c.to.to_string_lossy().to_ascii_lowercase();
            l.ends_with(".esp") || l.ends_with(".esm") || l.ends_with(".esl")
        })
        .filter_map(|c| {
            let (v, f) = crate::loadorder::header(&c.from)?;
            (v > max_v + 0.001 || f > max_f).then(|| c.to.file_name().unwrap().to_string_lossy().into_owned())
        })
        .collect()
}

/// Checks a downloaded archive against the list's SHA-256, when it has one.
pub fn verify(entry: &ModEntry, archive: &Path) -> Result<()> {
    let Some(want) = &entry.sha256 else { return Ok(()) };
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(archive)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    let got = hex::encode(h.finalize());
    if got.eq_ignore_ascii_case(want) {
        Ok(())
    } else {
        Err(Error::HashMismatch { path: entry.name.clone(), expected: want.clone(), actual: got })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_with(path: &Path, files: &[(&str, &[u8])]) {
        use std::io::Write;
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (n, b) in files {
            z.start_file(*n, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
            z.write_all(b).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn server_list_overrides_and_extends() {
        let s = ModList {
            mods: vec![
                ModEntry { id: "ussep".into(), name: "USSEP pinned".into(), nexus: Some(NexusRef { mod_id: 266, file: Some(9), pick: None }), check: vec!["Data/x.esp".into()], ..Default::default() },
                ModEntry { id: "bad".into(), name: "Bad".into(), url: Some("https://x/y.zip".into()), check: vec!["../evil".into()], ..Default::default() },
                ModEntry { id: "plain".into(), name: "Plain http".into(), url: Some("http://x/y.zip".into()), ..Default::default() },
                ModEntry { id: "new".into(), name: "New".into(), url: Some("https://github.com/a/b.zip".into()), check: vec!["Data/new.esp".into()], ..Default::default() },
            ],
            nexus_app: None,
        };
        let m = merged(Some("1.6.1170.0"), Some(&s));
        assert_eq!(m.iter().find(|e| e.id == "ussep").unwrap().nexus.as_ref().unwrap().file, Some(9));
        assert!(m.iter().all(|e| e.id != "bad" && e.id != "plain"));
        assert_eq!(m.last().unwrap().id, "new");
        assert_eq!(m.iter().find(|e| e.id == "ussep").unwrap().page().unwrap(), "https://www.nexusmods.com/skyrimspecialedition/mods/266?tab=files&file_id=9");
    }

    #[test]
    fn finds_the_data_folder_and_skips_readmes() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("m.zip");
        zip_with(&a, &[("My Mod/Readme.txt", b"r"), ("My Mod/SKSE/Plugins/My.dll", b"d"), ("My Mod/My.esp", b"e")]);
        let u = t.path().join("u");
        extract(&a, &u).unwrap();
        let e = ModEntry { id: "m".into(), name: "M".into(), ..Default::default() };
        let mut to: Vec<String> = plan(&e, &u).unwrap().iter().map(|c| c.to.to_string_lossy().replace('\\', "/")).collect();
        to.sort();
        assert_eq!(to, ["Data/My.esp", "Data/SKSE/Plugins/My.dll"]);
    }

    #[test]
    fn game_target_takes_named_files() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("m.zip");
        zip_with(&a, &[("Part 2/d3dx9_42.dll", b"1"), ("Part 2/tbb.dll", b"2"), ("Part 2/tbbmalloc.dll", b"3"), ("Part 2/readme.txt", b"r")]);
        let u = t.path().join("u");
        extract(&a, &u).unwrap();
        let e = ModEntry { id: "p".into(), name: "P".into(), target: Target::Game, include: vec!["d3dx9_42.dll".into(), "tbb.dll".into(), "tbbmalloc.dll".into()], check: vec!["tbb.dll".into()], ..Default::default() };
        let copies = plan(&e, &u).unwrap();
        assert_eq!(copies.len(), 3);
        let game = t.path().join("game");
        std::fs::create_dir_all(game.join("Data")).unwrap();
        apply(&e, &copies, &game, Some(1), None).unwrap();
        assert!(e.installed(&game));
        assert_eq!(load_installed(&game).mods["p"].files.len(), 3);
    }

    #[test]
    fn all_in_one_splits_data_and_game_files() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("m.zip");
        zip_with(&a, &[("SKSE/Plugins/EngineFixes.dll", b"1"), ("SKSE/Plugins/EngineFixes_preload.txt", b"2"), ("d3dx9_42.dll", b"3"), ("tbbmalloc.dll", b"5")]);
        let u = t.path().join("u");
        extract(&a, &u).unwrap();
        let e = builtin(None).into_iter().find(|e| e.id == "engine-fixes").unwrap();
        let mut to: Vec<String> = plan(&e, &u).unwrap().iter().map(|c| c.to.to_string_lossy().replace('\\', "/")).collect();
        to.sort();
        assert_eq!(to, ["Data/SKSE/Plugins/EngineFixes.dll", "Data/SKSE/Plugins/EngineFixes_preload.txt", "d3dx9_42.dll", "tbbmalloc.dll"]);
        // A package with only the SKSE plugin installs too.
        let b = t.path().join("n.zip");
        zip_with(&b, &[("SKSE/Plugins/EngineFixes.dll", b"1"), ("SKSE/Plugins/EngineFixes_preload.txt", b"2")]);
        let v = t.path().join("v");
        extract(&b, &v).unwrap();
        assert_eq!(plan(&e, &v).unwrap().len(), 2);
    }

    #[test]
    fn answers_a_fomod() {
        let xml = r#"<?xml version="1.0"?><config><moduleName>X</moduleName>
<requiredInstallFiles><folder source="Core" destination=""/></requiredInstallFiles>
<installSteps><installStep name="s"><optionalFileGroups>
<group name="Game" type="SelectExactlyOne"><plugins>
<plugin name="SE"><files><folder source="SE" destination="SKSE/Plugins"/></files><typeDescriptor><type name="Optional"/></typeDescriptor></plugin>
<plugin name="AE"><files><folder source="AE" destination="SKSE/Plugins"/></files><conditionFlags><flag name="ae">On</flag></conditionFlags><typeDescriptor><type name="Optional"/></typeDescriptor></plugin>
</plugins></group>
<group name="Extras" type="SelectAny"><plugins>
<plugin name="Nice"><files><file source="nice.esp"/></files><typeDescriptor><type name="Recommended"/></typeDescriptor></plugin>
<plugin name="Meh"><files><file source="meh.esp"/></files><typeDescriptor><type name="Optional"/></typeDescriptor></plugin>
</plugins></group></optionalFileGroups></installStep></installSteps>
<conditionalFileInstalls><patterns><pattern><dependencies operator="And"><flagDependency flag="ae" value="On"/></dependencies><files><file source="ae.ini" destination="SKSE/Plugins/ae.ini"/></files></pattern></patterns></conditionalFileInstalls>
</config>"#;
        let got = fomod_files(xml, &["AE".into()]).unwrap();
        let srcs: Vec<&str> = got.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(srcs, ["Core", "AE", "nice.esp", "ae.ini"]);
        let first = fomod_files(xml, &[]).unwrap();
        assert_eq!(first[1].0, "SE");

        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("m.zip");
        zip_with(&a, &[("fomod/ModuleConfig.xml", xml.as_bytes()), ("core/a.txt", b"a"), ("ae/X.dll", b"x"), ("nice.esp", b"n"), ("ae.ini", b"i")]);
        let u = t.path().join("u");
        extract(&a, &u).unwrap();
        let e = ModEntry { id: "x".into(), name: "X".into(), fomod: vec!["AE".into()], ..Default::default() };
        let mut to: Vec<String> = plan(&e, &u).unwrap().iter().map(|c| c.to.to_string_lossy().replace('\\', "/")).collect();
        to.sort();
        assert_eq!(to, ["Data/SKSE/Plugins/X.dll", "Data/SKSE/Plugins/ae.ini", "Data/nice.esp"]);
    }

    #[test]
    fn keeps_vortex_files_and_player_settings() {
        let t = tempfile::tempdir().unwrap();
        let game = t.path();
        std::fs::create_dir_all(game.join("Data")).unwrap();
        let src = t.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.ini"), b"new").unwrap();
        std::fs::write(src.join("a.dll"), b"new").unwrap();
        std::fs::write(game.join("Data/a.ini"), b"mine").unwrap();
        let e = ModEntry { id: "a".into(), name: "A".into(), ..Default::default() };
        let copies = vec![Copy { from: src.join("a.ini"), to: "Data/a.ini".into() }, Copy { from: src.join("a.dll"), to: "Data/a.dll".into() }];
        let r = apply(&e, &copies, game, None, None).unwrap();
        assert_eq!(std::fs::read(game.join("Data/a.ini")).unwrap(), b"mine");
        assert_eq!(r.files, ["Data/a.dll"]);
        std::fs::write(game.join("Data/__folder_managed_by_vortex"), b"").unwrap();
        std::fs::write(game.join("Data/a.dll"), b"vortex").unwrap();
        apply(&e, &copies, game, None, None).unwrap();
        assert_eq!(std::fs::read(game.join("Data/a.dll")).unwrap(), b"vortex");
    }

    #[test]
    fn unpacks_engine_fixes_7z_with_bcj2() {
        let Ok(p) = std::env::var("AD_EF_7Z") else { return };
        let t = tempfile::tempdir().unwrap();
        extract(Path::new(&p), &t.path().join("u")).unwrap();
        let e = ModEntry { id: "ef".into(), name: "EF".into(), fomod: vec!["AE".into()], ..Default::default() };
        let to: Vec<String> = plan(&e, &t.path().join("u")).unwrap().iter().map(|c| c.to.to_string_lossy().replace('\\', "/")).collect();
        assert!(to.contains(&"Data/SKSE/Plugins/EngineFixes.dll".to_string()), "{to:?}");
        assert!(to.contains(&"Data/SKSE/Plugins/EngineFixes.toml".to_string()), "{to:?}");
    }

    #[test]
    fn takes_the_right_skse_build_and_sets_the_old_one_aside() {
        use crate::skse::tests::{good_dll, old_dll};
        let t = tempfile::tempdir().unwrap();
        let g = t.path().join("game");
        let pl = g.join("Data/SKSE/Plugins");
        std::fs::create_dir_all(&pl).unwrap();
        std::fs::write(g.join("Data/vortex.deployment.json"), b"{}").unwrap();
        std::fs::write(pl.join("TrueDirectionalMovement.dll"), old_dll()).unwrap();
        let e = ModEntry { id: "tdm".into(), name: "True Directional Movement".into(), check: vec!["Data/SKSE/Plugins/TrueDirectionalMovement.dll".into()], ..Default::default() };
        assert!(!e.installed(&g), "the old build doesn't count as installed");
        // An installer with an SE and an AE folder: the plan takes the AE DLL.
        let u = t.path().join("unpacked");
        for (dir, b) in [("SE", old_dll()), ("AE", good_dll())] {
            std::fs::create_dir_all(u.join(dir).join("SKSE/Plugins")).unwrap();
            std::fs::write(u.join(dir).join("SKSE/Plugins/TrueDirectionalMovement.dll"), b).unwrap();
        }
        let mut copies = vec![Copy { from: u.join("SE/SKSE/Plugins/TrueDirectionalMovement.dll"), to: PathBuf::from("Data/SKSE/Plugins/TrueDirectionalMovement.dll") }];
        assert!(fix_wrong_builds(&mut copies, &u).is_empty());
        assert!(copies[0].from.to_string_lossy().contains("AE"));
        apply(&e, &copies, &g, None, None).unwrap();
        assert!(e.installed(&g));
        // The old one was set aside with the reason, not deleted.
        let aside: Vec<_> = std::fs::read_dir(g.join(crate::strays::DISABLED_DIR)).unwrap().flatten().map(|d| d.path()).collect();
        assert_eq!(aside.len(), 1);
        assert!(aside[0].join("Data/SKSE/Plugins/TrueDirectionalMovement.dll").is_file());
        assert!(std::fs::read_to_string(aside[0].join("why.txt")).unwrap().contains("1.6.629"));
        // An archive with only the old build is refused.
        std::fs::remove_dir_all(u.join("AE")).unwrap();
        let mut copies = vec![Copy { from: u.join("SE/SKSE/Plugins/TrueDirectionalMovement.dll"), to: PathBuf::from("Data/SKSE/Plugins/TrueDirectionalMovement.dll") }];
        assert_eq!(fix_wrong_builds(&mut copies, &u).len(), 1);
    }

    #[test]
    fn replaces_a_broken_stub_even_under_vortex() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        std::fs::create_dir_all(g.join("Data")).unwrap();
        std::fs::write(g.join("Data/__folder_managed_by_vortex"), b"").unwrap();
        std::fs::write(g.join("Data/SkyUI_SE.esp"), crate::loadorder::test_plugin(0.0, false)).unwrap();
        std::fs::write(g.join("Data/SkyUI_SE.bsa"), b"old").unwrap();
        let m = builtin(None).into_iter().find(|m| m.id == "skyui").unwrap();
        assert!(!m.installed(g));
        let src = g.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("SkyUI_SE.esp"), crate::loadorder::test_plugin(1.7, true)).unwrap();
        std::fs::write(src.join("SkyUI_SE.bsa"), b"new").unwrap();
        let copies: Vec<Copy> = ["SkyUI_SE.esp", "SkyUI_SE.bsa"].iter().map(|n| Copy { from: src.join(n), to: PathBuf::from("Data").join(n) }).collect();
        let rec = apply(&m, &copies, g, None, None).unwrap();
        assert_eq!(rec.files, ["Data/SkyUI_SE.esp"]);
        assert_eq!(rec.skipped, ["Data/SkyUI_SE.bsa"]);
        assert!(m.installed(g));
    }
}
