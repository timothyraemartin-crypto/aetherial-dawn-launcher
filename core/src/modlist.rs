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
    /// Files this mod replaces even when another mod put them there first
    /// (the old one is backed up), and that only count as installed when
    /// they came from this mod.
    #[serde(default)]
    pub owns: Vec<String>,
    /// Plugins the archive keeps in a folder of choices (Embers XD's
    /// "plugins/esp/Embers XD.esp"), by their path in the archive: they go
    /// to the top of Data. Other plugins below the top of Data never load
    /// and aren't copied.
    #[serde(default)]
    pub lift: Vec<String>,
    /// The server's preset for this mod's settings files, written once
    /// (presets.rs).
    #[serde(default)]
    pub settings: Vec<crate::presets::Setting>,
    /// Builds of the mod for CPU instruction sets, as folders in the archive
    /// ("avx512", "avx2", "avx", "plain" -> folder). The best one this CPU
    /// runs is installed and the other folders are left out (FSMP: an AVX-512
    /// build crashes the game on a CPU without it).
    #[serde(default)]
    pub cpu: BTreeMap<String, String>,
    /// A program the mod ships that the launcher runs itself, minimized,
    /// before the game starts (BodySlide's batch build; tools.rs).
    #[serde(default, deserialize_with = "crate::tools::one_or_many")]
    pub run: Vec<crate::tools::ToolRun>,
    /// The download's size, and its size unpacked (the Mod Curator's
    /// numbers), for the disk-space check and the time left.
    #[serde(default, alias = "archiveBytes", skip_serializing_if = "Option::is_none")]
    pub archive_bytes: Option<u64>,
    #[serde(default, alias = "unpackedBytes", skip_serializing_if = "Option::is_none")]
    pub unpacked_bytes: Option<u64>,
    /// The download is this one file, not an archive, and goes to this path
    /// relative to the game folder (a pinned settings file such as RaceMenu's
    /// skee64_custom.ini). It needs `sha256`: the copy in the game folder
    /// counts as installed only while it still has that hash, so a changed
    /// or missing copy is put back by the next mod check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Paths relative to the game folder this mod installs everything but
    /// (Alternate High Poly Head without its facegenmorphs morphs.ini).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skip: Vec<String>,
}

/// CPU levels a `cpu` map can name, best first.
pub const CPU_LEVELS: [&str; 4] = ["avx512", "avx2", "avx", "plain"];

/// The instruction sets this CPU (and Windows) can run, best first.
pub fn cpu_supports() -> Vec<&'static str> {
    let mut out = Vec::new();
    #[cfg(target_arch = "x86_64")]
    {
        // FSMP's AVX-512 build uses the BW and VL extensions too.
        if std::arch::is_x86_feature_detected!("avx512f") && std::arch::is_x86_feature_detected!("avx512bw") && std::arch::is_x86_feature_detected!("avx512vl") {
            out.push("avx512");
        }
        if std::arch::is_x86_feature_detected!("avx2") {
            out.push("avx2");
        }
        if std::arch::is_x86_feature_detected!("avx") {
            out.push("avx");
        }
    }
    out.push("plain");
    out
}

impl ModEntry {
    /// The `cpu` build to install on a CPU with these levels, as (level, folder).
    pub fn cpu_pick_for(&self, supports: &[&str]) -> Option<(String, String)> {
        CPU_LEVELS.iter().filter(|l| supports.contains(l)).find_map(|l| self.cpu.iter().find(|(k, _)| k.eq_ignore_ascii_case(l)).map(|(k, v)| (k.clone(), v.clone())))
    }

    pub fn cpu_pick(&self) -> Option<(String, String)> {
        self.cpu_pick_for(&cpu_supports())
    }
}

impl ModEntry {
    pub fn installed(&self, game_dir: &Path) -> bool {
        // An install that stopped part way (the launcher closed while files
        // were moving into Data) never counts, whatever files are there.
        if installing_path(game_dir, &self.id).exists() {
            return false;
        }
        if let Some(f) = &self.file {
            return self.single_file_matches(game_dir, f);
        }
        // The list now pins another file, or picks other installer options,
        // than the launcher installed: install it again.
        if let Some(r) = load_installed(game_dir).mods.get(&self.id) {
            let pin = self.nexus.as_ref().and_then(|n| n.file);
            if (pin.is_some() && r.file_id.is_some() && pin != r.file_id) || r.fomod.as_ref().is_some_and(|f| f != &self.fomod) {
                return false;
            }
            // The game folder moved to a PC with another CPU.
            if !self.cpu.is_empty() && r.cpu.is_some() && r.cpu != self.cpu_pick().map(|p| p.0) {
                return false;
            }
        }
        // No checks: the launcher's own record of what it installed (mods
        // whose FOMOD installer decides the file names, or textures only).
        if self.check.is_empty() {
            return load_installed(game_dir).mods.get(&self.id).is_some_and(|r| {
                let files: Vec<&String> = r.files.iter().chain(&r.skipped).collect();
                !files.is_empty() && files.iter().all(|f| safe_rel(f).is_some_and(|p| game_dir.join(p).exists()))
            }) && self.owns.iter().all(|o| self.owns_now(game_dir, o));
        }
        self.game_files_present(game_dir)
    }

    /// A one-file download: the file is in place with the pinned hash.
    fn single_file_matches(&self, game_dir: &Path, rel: &str) -> bool {
        let (Some(p), Some(want)) = (safe_rel(rel), &self.sha256) else { return false };
        crate::patcher::sha256_file(&game_dir.join(p)).is_ok_and(|h| h.eq_ignore_ascii_case(want))
    }

    /// Checks the listed files in Skyrim without trusting the launcher's old
    /// direct-install receipt. A Vortex package has its own profile and
    /// deployment proof, so a receipt for an older direct install must not
    /// make a newly deployed Vortex package look missing. Entries with no
    /// checks need Vortex deployment proof rather than an empty success.
    pub fn game_files_present(&self, game_dir: &Path) -> bool {
        !self.check.is_empty()
            && self.check.iter().all(|c| safe_rel(c).map(|r| present_like(&game_dir.join(r))).unwrap_or(false))
            && self.owns.iter().all(|o| self.owns_now(game_dir, o))
    }

    /// Whether an owned file in the game folder came from this mod: the
    /// launcher installed it, or Vortex deployed it from this mod's folder.
    fn owns_now(&self, game_dir: &Path, rel: &str) -> bool {
        if !game_dir.join(rel).is_file() {
            return false;
        }
        let rec = load_installed(game_dir);
        if rec.mods.get(&self.id).is_some_and(|m| m.files.iter().any(|f| f.eq_ignore_ascii_case(rel))) {
            return true;
        }
        let Some(n) = &self.nexus else { return false };
        let id = n.mod_id.to_string();
        // Vortex folder names carry the Nexus id, with dashes or spaces.
        crate::allowlist::vortex_files(game_dir)
            .iter()
            .any(|f| f.rel.eq_ignore_ascii_case(rel) && f.source.split(|c: char| !c.is_ascii_digit()).any(|t| t == id))
    }

    /// Whether Vortex has deployed files from a mod folder carrying this
    /// entry's Nexus id (Vortex names folders "<name>-<nexus id>-<version>-
    /// <time>"), with at least one of them in the game folder now.
    pub fn vortex_deployed(&self, game_dir: &Path) -> bool {
        let Some(n) = &self.nexus else { return false };
        let id = n.mod_id.to_string();
        crate::allowlist::vortex_files(game_dir)
            .iter()
            .any(|f| f.source.split(|c: char| !c.is_ascii_digit()).any(|t| t == id) && game_dir.join(&f.rel).is_file())
    }

    /// A copy that checks for one file only.
    /// (download, unpacked) bytes: the list's numbers, else a safe guess.
    /// Unpacked is 3 times the download when unknown (the Curator's median
    /// is 2.2), and an unknown download counts as 64 MB (the built-in mods
    /// without numbers are all small SKSE plugins).
    pub fn sizes(&self) -> (u64, u64) {
        let archive = self.archive_bytes.unwrap_or(UNKNOWN_ARCHIVE);
        (archive, self.unpacked_bytes.unwrap_or(archive * 3))
    }

    pub fn clone_with_check(&self, c: &str) -> ModEntry {
        ModEntry { check: vec![c.to_string()], ..self.clone() }
    }

    /// Where a free member downloads it: a pinned file's own download page
    /// (`nmm=1`), so the button there is for that exact file, archived ones
    /// included; otherwise the Files tab.
    pub fn download_page(&self) -> Option<String> {
        let n = self.nexus.as_ref()?;
        match n.file {
            Some(f) => Some(format!("https://www.nexusmods.com/{NEXUS_GAME}/mods/{}?tab=files&file_id={f}&nmm=1", n.mod_id)),
            None => self.page(),
        }
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
    /// The list's own revision ("3.4.6"), when the server names one; the
    /// launcher shows and logs it with the bytes' sha256 (inventory.rs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

/// Nexus file ids for built-ins whose newer or best-named files can be wrong
/// for Skyrim 1.6.1170. The other built-ins below are pinned to the exact
/// files selected for this manual Vortex profile too.
pub const ADDRESS_LIBRARY_FILE: u64 = 470707;
pub const TRUE_DIRECTIONAL_MOVEMENT_FILE: u64 = 798770;

/// The mods the launcher requires on its own (Skyrim Souls RE's
/// dependencies and the Address Library). The GitHub-hosted required mods
/// (SKSE, Crash Logger, Souls RE, Engine Fixes part 1) are installed by
/// `requirements` before Play and aren't repeated here.
/// A download of unknown size counts as this much (see `ModEntry::sizes`).
pub const UNKNOWN_ARCHIVE: u64 = 64 << 20;

/// (id, download bytes, unpacked bytes) of the launcher's own mods.
const BUILTIN_SIZES: [(&str, u64, u64); 13] = [
    ("address-library", 2_412_602, 9_283_200),
    ("engine-fixes", 5_607_605, 32_699_114),
    ("ussep", 168_852_028, 282_223_050),
    ("menu-framework", 10_715_519, 32_146_557),
    ("imgui-icons", 3_062_080, 6_392_717),
    ("skyui", 2_693_003, 2_902_400),
    ("display-tweaks", 187_137, 561_411),
    ("black-screen-fix", 1_800, 2_700),
    ("mcm-helper", 8_187_265, 24_561_795),
    ("smoothcam", 36_638_690, 104_909_236),
    ("smoothcam-modern-preset", 2_186, 51_900),
    ("true-directional-movement", 5_210_375, 15_631_125),
    ("truehud", 4_387_378, 13_162_134),
];

pub fn builtin(game_version: Option<&str>) -> Vec<ModEntry> {
    use crate::requirements as r;
    let mut out = Vec::new();
    if let Some(v) = game_version {
        out.push(ModEntry {
            id: "address-library".into(),
            name: "Address Library for SKSE Plugins".into(),
            // Pinned: the files named "All in one (Anniversary Edition)" are
            // all archived and the newest (v8, 2022) has no 1.6.1170
            // database, while today's main files are for newer Skyrim.
            // v11 "All in one (1.6.X)" carries versionlib-1-6-1170-0.bin
            // (Mod Curator's pin check, 2026-09-28).
            nexus: Some(NexusRef { mod_id: 32444, file: Some(ADDRESS_LIBRARY_FILE), pick: Some("All in one (1.6.X)".into()) }),
            check: vec![format!("Data/SKSE/Plugins/{}", r::address_library_file(v))],
            hint: Some("All in one (1.6.X), version 11".into()),
            ..Default::default()
        });
    }
    let mut ef_check = vec!["Data/SKSE/Plugins/EngineFixes.dll".to_string()];
    ef_check.push("Data/SKSE/Plugins/EngineFixes_preload.txt".to_string());
    // The settings file it can't start without (health.rs required files):
    // missing, Play installs the mod again (text audit A6).
    ef_check.push("Data/SKSE/Plugins/EngineFixes.toml".to_string());
    out.push(ModEntry {
        id: "engine-fixes".into(),
        name: "SSE Engine Fixes (All-In-One)".into(),
        nexus: Some(NexusRef { mod_id: 17230, file: Some(669326), pick: Some("All-In-One".into()) }),
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
        nexus: Some(NexusRef { mod_id: 266, file: Some(crate::ussep::NEXUS_FILE), pick: Some(crate::ussep::NEXUS_PICK.into()) }),
        check: vec![format!("Data/{}", r::USSEP_PLUGIN)],
        hint: Some("version 4.3.8a, the one for Skyrim 1.6.1170".into()),
        ..Default::default()
    });
    out.push(ModEntry {
        id: "menu-framework".into(),
        name: "SKSE Menu Framework".into(),
        nexus: Some(NexusRef { mod_id: 120352, file: Some(806684), pick: None }),
        // With the strings, fonts and themes it can't start without (health.rs
        // required files): any missing, Play installs it again (text audit A6).
        check: vec![
            format!("Data/SKSE/Plugins/{}", r::MENU_FRAMEWORK_DLL),
            "Data/SKSE/Plugins/SKSEMenuFrameworkStrings*.json".into(),
            "Data/SKSE/Plugins/fonts/*.ttf".into(),
            "Data/SKSE/Plugins/SKSEMenuFrameworkThemes/*.json".into(),
        ],
        hint: Some("the main file".into()),
        ..Default::default()
    });
    out.push(ModEntry {
        id: "imgui-icons".into(),
        name: "ImGui Icons".into(),
        nexus: Some(NexusRef { mod_id: 114790, file: Some(690123), pick: None }),
        check: vec![format!("Data/Interface/{}", r::IMGUI_ICONS_DIR)],
        hint: Some("the main file".into()),
        ..Default::default()
    });
    out.push(ModEntry {
        id: "skyui".into(),
        name: "SkyUI".into(),
        nexus: Some(NexusRef { mod_id: 12604, file: Some(749043), pick: Some("SkyUI".into()) }),
        check: vec![format!("Data/{}", r::SKYUI_PLUGIN), format!("Data/{}", r::SKYUI_ARCHIVE)],
        hint: Some("SkyUI 5.2SE (main file)".into()),
        ..Default::default()
    });
    // Timothy, 2026-09-26: "add these two mods" (against a black screen).
    // Display Tweaks first, so the fix's ini lands over its default one.
    out.push(ModEntry {
        id: "display-tweaks".into(),
        name: "SSE Display Tweaks".into(),
        nexus: Some(NexusRef { mod_id: 34705, file: Some(797175), pick: Some("AE".into()) }),
        check: vec![format!("Data/SKSE/Plugins/{}", r::DISPLAY_TWEAKS_DLL)],
        hint: Some("the main file for Anniversary Edition (1.6)".into()),
        ..Default::default()
    });
    out.push(ModEntry {
        id: "black-screen-fix".into(),
        name: "Black Screen and Startup Fix".into(),
        nexus: Some(NexusRef { mod_id: 176509, file: Some(738614), pick: Some("1080".into()) }),
        check: vec![format!("Data/SKSE/Plugins/{}", r::DISPLAY_TWEAKS_INI)],
        hint: Some("the file for your screen (1080p or 1440p)".into()),
        // Display Tweaks ships an ini of the same name; the fix's must win
        // (quality check 2026-09-27: it was skipped and still counted).
        owns: vec![format!("Data/SKSE/Plugins/{}", r::DISPLAY_TWEAKS_INI)],
        ..Default::default()
    });
    // Timothy, 2026-09-26: "These mods need added". Client-only camera,
    // movement and HUD mods, plus MCM Helper, which TDM and TrueHUD's menus
    // need. Checked by DLL; their plugins are switched on when present
    // (loadorder::COMPANION_PLUGINS).
    out.push(ModEntry {
        id: "mcm-helper".into(),
        name: "MCM Helper".into(),
        nexus: Some(NexusRef { mod_id: 53000, file: Some(795510), pick: None }),
        check: vec!["Data/SKSE/Plugins/MCMHelper.dll".into()],
        hint: Some("the main file".into()),
        ..Default::default()
    });
    out.push(ModEntry {
        id: "smoothcam".into(),
        name: "SmoothCam".into(),
        nexus: Some(NexusRef { mod_id: 41252, file: Some(729856), pick: Some("AE".into()) }),
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
        nexus: Some(NexusRef { mod_id: 41636, file: Some(220887), pick: None }),
        check: vec!["Data/SKSE/Plugins/SmoothCamPreset*.json".into()],
        hint: Some("the main file".into()),
        ..Default::default()
    });
    out.push(ModEntry {
        id: "true-directional-movement".into(),
        name: "True Directional Movement".into(),
        // Pinned: the "AE" pick matched seven 2022 builds SKSE rejects on
        // 1.6.1170 before reaching the one that works (2.3.1).
        nexus: Some(NexusRef { mod_id: 51614, file: Some(TRUE_DIRECTIONAL_MOVEMENT_FILE), pick: Some("2.3.1".into()) }),
        check: vec!["Data/SKSE/Plugins/TrueDirectionalMovement.dll".into()],
        hint: Some("the main file, version 2.3.1".into()),
        ..Default::default()
    });
    out.push(ModEntry {
        id: "truehud".into(),
        name: "TrueHUD".into(),
        nexus: Some(NexusRef { mod_id: 62775, file: Some(798218), pick: None }),
        check: vec!["Data/SKSE/Plugins/TrueHUD.dll".into()],
        hint: Some("the main file".into()),
        ..Default::default()
    });
    // The Mod Curator's sizes (world-mods/tools/mods-sizes.json, "builtin",
    // 2026-09-28), for the disk check and the time left.
    for m in &mut out {
        if let Some((_, a, u)) = BUILTIN_SIZES.iter().find(|(id, _, _)| *id == m.id) {
            m.archive_bytes.get_or_insert(*a);
            m.unpacked_bytes.get_or_insert(*u);
        }
    }
    out
}

/// The built-in list with the server's list laid over it: a server entry with
/// the same id replaces the built-in one, new ids are added after. Entries
/// with unsafe check paths or no source are dropped.
pub fn merged(game_version: Option<&str>, server: Option<&ModList>) -> Vec<ModEntry> {
    let mut out = builtin(game_version);
    if let Some(s) = server {
        for m in &s.mods {
            if m.id.is_empty() || (m.nexus.is_none() && m.url.is_none()) || m.check.iter().chain(&m.owns).chain(&m.skip).chain(&m.file).any(|c| safe_rel(c).is_none()) {
                continue;
            }
            // A one-file download is pinned or it isn't used, and it's never
            // one of the game's own files (base masters, Creation Club,
            // _ResourcePack): those only come from the player's own Skyrim.
            if m.file.iter().chain(&m.skip).any(|f| game_owned(f)) || (m.file.is_some() && m.sha256.as_ref().is_none_or(|h| h.len() != 64)) {
                continue;
            }
            if let Some(url) = &m.url {
                if !url.starts_with("https://") {
                    continue;
                }
            }
            match out.iter_mut().find(|e| e.id == m.id) {
                Some(e) => {
                    // The launcher's own knowledge of owned files stays.
                    let owns = std::mem::take(&mut e.owns);
                    *e = m.clone();
                    if e.owns.is_empty() {
                        e.owns = owns;
                    }
                }
                None => out.push(m.clone()),
            }
        }
    }
    out
}

/// The two Community Overlays face archives install to the same 25 paths.
/// For this manual Vortex playtest the male archive is the selected variant;
/// the female archive may remain installed, but is not a second deployment
/// requirement. Apply this only to the two exact, checkless feed pins.
const MALE_FACE_FILES: [&str; 25] = [
    "01 Head.dds", "02 Head.dds", "03 Head.dds", "04 Head.dds", "05 Head.dds",
    "06 Head.dds", "07 Head.dds", "07 Head Secondary.dds", "08 Head.dds",
    "08 Head Secondary.dds", "09 Head.dds", "09 Head Secondary.dds", "20 Head.dds",
    "21 Head.dds", "22 Head.dds", "26 Head.dds", "27 Head.dds", "28 Head.dds",
    "29 Head.dds", "30 Head Base.dds", "30 Head Base W.dds", "30 Head Secondary.dds",
    "30 Head Secondary W.dds", "Extra Head Gemstone.dds", "Extra Head Renegade.dds",
];

pub(crate) fn selected_male_face_checks() -> Vec<String> {
    MALE_FACE_FILES.iter().map(|name| format!("Data/textures/actors/character/Overlays/Community Overlays/{name}")).collect()
}

pub fn play_required(mut list: Vec<ModEntry>) -> Vec<ModEntry> {
    let face = |id: &str, file: u64| list.iter().any(|m| m.id == id
        && m.nexus.as_ref().is_some_and(|n| n.mod_id == 22487 && n.file == Some(file))
        && m.check.is_empty() && m.owns.is_empty());
    if face("community-overlays-1-female-face", 104828)
        && face("community-overlays-1-male-face", 104868)
    {
        list.retain(|m| m.id != "community-overlays-1-female-face");
        if let Some(male) = list.iter_mut().find(|m| m.id == "community-overlays-1-male-face") {
            male.check = selected_male_face_checks();
        }
    }
    list
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

/// Skyrim's own files, which only ever come from the player's own Steam
/// copy: the game's programs next to SkyrimSE.exe (any .exe but SKSE's
/// loader), Skyrim.ccc, and in Data the base and Creation Club plugins,
/// _ResourcePack, and their archives ("Skyrim - *.bsa", "cc*.bsa").
pub fn game_owned(rel: &str) -> bool {
    let l = rel.replace('\\', "/").to_ascii_lowercase();
    match l.strip_prefix("data/") {
        None => !l.contains('/') && (matches!(l.as_str(), "skyrim.ccc" | "steam_api64.dll" | "bink2w64.dll") || (l.ends_with(".exe") && l != "skse64_loader.exe")),
        Some(n) => {
            let stem = n.rsplit_once('.').map(|(s, _)| s).unwrap_or(n);
            !n.contains('/')
                && (n == "skyrim.ccc"
                    || n.starts_with("_resourcepack.")
                    || n.starts_with("marketplacetextures.")
                    || (n.starts_with("skyrim - ") && n.ends_with(".bsa"))
                    || crate::aliases::shipped_with_game(n)
                    || (n.ends_with(".bsa") && crate::aliases::shipped_with_game(&format!("{stem}.esm"))))
        }
    }
}

/// Files a listed mod must go without that are in the game folder anyway
/// (Vortex deploys a package whole): Play sets them aside each time.
pub fn skipped_present(list: &[ModEntry], game_dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = list
        .iter()
        .flat_map(|m| m.skip.iter().map(move |s| (m, s)))
        // Only a file this mod put there (Vortex deployed it from this
        // mod's folder, or the launcher installed it), never one of the
        // game's own.
        .filter(|(m, s)| safe_rel(s).is_some() && !game_owned(s) && m.owns_now(game_dir, s))
        .map(|(_, s)| s.replace('\\', "/"))
        .collect();
    out.sort();
    out.dedup();
    out
}

pub fn missing<'a>(list: &'a [ModEntry], game_dir: &Path) -> Vec<&'a ModEntry> {
    list.iter().filter(|m| !m.installed(game_dir)).collect()
}

/// Two entries that fetch the same pinned Nexus file, where the second
/// checks nothing the first doesn't: one download installs both (the
/// built-in Unofficial Patch and the server lane's copy of it, 2026-09-28).
pub fn same_download(first: &ModEntry, later: &ModEntry) -> bool {
    let pinned = |m: &ModEntry| m.nexus.as_ref().and_then(|n| n.file.map(|f| (n.mod_id, f)));
    pinned(first).is_some()
        && pinned(first) == pinned(later)
        && !later.check.is_empty()
        && later.check.iter().all(|c| first.check.iter().any(|f| f.eq_ignore_ascii_case(c)))
}

/// What Download all fetches, in order: the missing entries, each pinned
/// file once.
pub fn to_fetch<'a>(list: &'a [ModEntry], game_dir: &Path) -> Vec<&'a ModEntry> {
    let mut out: Vec<&ModEntry> = Vec::new();
    for m in missing(list, game_dir) {
        if !out.iter().any(|f| same_download(f, m)) {
            out.push(m);
        }
    }
    out
}

/// What a caught nxm:// link means for the entry waiting on it.
#[derive(Debug, PartialEq, Eq)]
pub enum LinkFits {
    Take,
    /// A link for another mod (another tab, a later mod's page).
    OtherMod,
    /// The right mod but not the pinned file (the newest file pressed
    /// instead): never installed, since its plugin wouldn't match the
    /// server's.
    OtherFile { pinned: u64 },
}

pub fn link_fits(n: &NexusRef, game: &str, mod_id: u64, file_id: u64) -> LinkFits {
    if mod_id != n.mod_id || !game.eq_ignore_ascii_case(NEXUS_GAME) {
        return LinkFits::OtherMod;
    }
    match n.file {
        Some(pinned) if pinned != file_id => LinkFits::OtherFile { pinned },
        _ => LinkFits::Take,
    }
}

/// Whether Vortex deploys mods into this game's Data folder.
pub fn vortex_manages(game_dir: &Path) -> bool {
    let data = game_dir.join("Data");
    data.join("vortex.deployment.json").is_file() || data.join("__folder_managed_by_vortex").exists()
}

/// A relative path with no parent steps or roots, using forward slashes.
pub fn safe_rel(p: &str) -> Option<PathBuf> {
    let p = p.replace('\\', "/");
    if p.is_empty() || p.starts_with('/') || p.contains(':') {
        return None;
    }
    let mut out = PathBuf::new();
    for c in p.split('/') {
        if c.is_empty() || c == "." {
            continue;
        }
        // Windows drops a trailing dot or space ("..." and ".. " name the
        // folder above), so a part ending in one is refused.
        if c == ".." || c.ends_with(['.', ' ']) {
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

/// Unpacks a zip or 7z archive into `dir`, skipping unsafe paths. All or
/// nothing: it unpacks into a folder beside `dir` and renames it into place
/// only when every entry came out, so a damaged or cut-short download never
/// leaves a half-unpacked folder (PR #7, Codex 5875867553).
pub fn extract(archive: &Path, dir: &Path) -> Result<()> {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.with_file_name(format!("{name}.unpacking"));
    let _ = std::fs::remove_dir_all(&tmp);
    if let Err(e) = extract_into(archive, &tmp) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    if let Err(e) = std::fs::rename(&tmp, dir) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e.into());
    }
    Ok(())
}

fn extract_into(archive: &Path, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut head = [0u8; 6];
    {
        use std::io::Read;
        let mut f = std::fs::File::open(archive)?;
        let _ = f.read(&mut head)?;
    }
    if head.starts_with(b"PK") {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?).map_err(|e| Error::Game(format!("the download isn't a readable zip: {e}")))?;
        let mut total = 0u64;
        for i in 0..zip.len() {
            // LZMA entries (method 14) go through lzma-rust2; the rest
            // through the zip crate (deflate, deflate64, bzip2, zstd).
            #[allow(deprecated)]
            let lzma = zip.by_index_raw(i).map_err(|e| Error::Game(e.to_string()))?.compression() == zip::CompressionMethod::Unsupported(14);
            let method = zip.by_index_raw(i).map(|f| f.compression()).ok();
            let mut f = if lzma { zip.by_index_raw(i) } else { zip.by_index(i) }.map_err(|e| Error::Game(format!("{e} (zip method {method:?})")))?;
            let Some(rel) = safe_rel(f.name()) else { continue };
            let dest = dir.join(rel);
            if f.is_dir() {
                std::fs::create_dir_all(&dest)?;
                continue;
            }
            // The declared size is checked before anything is created, and
            // the bytes really written are counted as they come.
            if f.size() > MAX_ENTRY || total + f.size() > MAX_TOTAL {
                return Err(too_big(f.name()));
            }
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p)?;
            }
            let mut out = std::fs::File::create(&dest)?;
            let name = f.name().to_string();
            let got = if lzma {
                let (size, crc) = (f.size(), f.crc32());
                let got = copy_capped(&mut zip_lzma(&mut f, size)?, &mut out, &dest, total, &name)?;
                drop(out);
                if got != size || crate::serverorder::crc32(&dest) != Some(crc) {
                    return Err(Error::Game(format!("couldn't unpack {name}: the download is damaged")));
                }
                got
            } else {
                copy_capped(&mut f, &mut out, &dest, total, &name)?
            };
            total += got;
        }
        Ok(())
    } else if head == [b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C] {
        let mut reader = sevenz_rust2::ArchiveReader::open(archive, sevenz_rust2::Password::empty()).map_err(|e| Error::Game(format!("the download isn't a readable 7z: {e}")))?;
        let mut total = 0u64;
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
                if entry.size() > MAX_ENTRY || total + entry.size() > MAX_TOTAL {
                    return Err(std::io::Error::other(too_big(entry.name()).to_string()).into());
                }
                let mut out = std::fs::File::create(&dest)?;
                total += copy_capped(data, &mut out, &dest, total, entry.name()).map_err(|e| std::io::Error::other(e.to_string()))?;
                Ok(true)
            })
            .map_err(|e| Error::Game(format!("couldn't unpack the download: {e}")))?;
        Ok(())
    } else if head.starts_with(b"Rar!\x1a\x07") {
        #[cfg(feature = "rar")]
        return extract_rar(archive, dir);
        #[cfg(not(feature = "rar"))]
        return Err(Error::Game("this build can't unpack RAR downloads".into()));
    } else {
        Err(Error::Game("the download isn't a zip, 7z or RAR archive".into()))
    }
}

/// RAR v4 and v5 through rarlab's own unrar library (the `unrar` crate,
/// PR #7, Codex 5875976813). Every entry goes to a path the launcher builds
/// with `safe_rel`, never to the name inside the archive; the sizes in each
/// header are checked against the caps before anything is written, and the
/// bytes that land are checked again (unrar stops at the header's size and
/// checks the CRC). Split, locked and password archives are refused, and so
/// is any entry that lands as a link.
#[cfg(feature = "rar")]
fn extract_rar(archive: &Path, dir: &Path) -> Result<()> {
    let damaged = |e: unrar::error::UnrarError| Error::Game(format!("couldn't unpack the download (RAR): {e}"));
    // Hard links, file copies and symlinks point an entry at another path;
    // unrar would make them, and a hard link lands as an ordinary file, so
    // they're refused from the headers before anything is written.
    if let Some((name, kind)) = rar5_redirect(archive)? {
        let what = match kind {
            4 => "a hard link",
            5 => "a copy of another file",
            _ => "a link",
        };
        return Err(Error::Game(format!("{name} in the download is {what}, not a file; the launcher won't unpack it")));
    }
    let listed = unrar::Archive::new(archive).open_for_listing().map_err(damaged)?;
    if listed.is_locked() || listed.has_encrypted_headers() {
        return Err(Error::Game("the download is a locked or password RAR the launcher can't install".into()));
    }
    let mut total = 0u64;
    let mut open = unrar::Archive::new(archive).open_for_processing().map_err(damaged)?;
    while let Some(entry) = open.read_header().map_err(damaged)? {
        let h = entry.entry();
        let name = h.filename.to_string_lossy().into_owned();
        if h.is_split() {
            return Err(Error::Game("the download is a RAR split into parts; the launcher takes one-part archives only".into()));
        }
        if h.is_encrypted() {
            return Err(Error::Game(format!("{name} is password protected in the download")));
        }
        let rel = safe_rel(&name);
        if h.is_directory() || rel.is_none() {
            if let (true, Some(rel)) = (h.is_directory(), &rel) {
                std::fs::create_dir_all(dir.join(rel))?;
            }
            open = entry.skip().map_err(damaged)?;
            continue;
        }
        let dest = dir.join(rel.unwrap());
        let size = h.unpacked_size;
        if size > MAX_ENTRY || total + size > MAX_TOTAL {
            return Err(too_big(&name));
        }
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p)?;
        }
        open = entry.extract_to(&dest).map_err(damaged)?;
        let meta = std::fs::symlink_metadata(&dest)?;
        if !meta.file_type().is_file() {
            return Err(Error::Game(format!("{name} in the download is a link, not a file")));
        }
        if meta.len() != size {
            return Err(Error::Game(format!("couldn't unpack {name}: the download is damaged")));
        }
        total += size;
    }
    Ok(())
}

/// The first RAR5 entry whose header carries a file system redirection
/// record (extra record 5: 1-3 symlink or junction, 4 hard link, 5 file
/// copy), as (its name, the redirection type). Reads headers only, seeking
/// past each data area. Not a RAR5 archive (RAR4 has neither hard links nor
/// file copies): None. A header that doesn't read is an error.
#[cfg(feature = "rar")]
fn rar5_redirect(archive: &Path) -> Result<Option<(String, u64)>> {
    use std::io::{Read, Seek, SeekFrom};
    let bad = || Error::Game("couldn't unpack the download (RAR): a header doesn't read".into());
    let mut f = std::io::BufReader::new(std::fs::File::open(archive)?);
    let mut sig = [0u8; 8];
    if f.read_exact(&mut sig).is_err() || &sig != b"Rar!\x1a\x07\x01\x00" {
        return Ok(None);
    }
    fn vint(b: &[u8], at: &mut usize) -> Option<u64> {
        let mut n = 0u64;
        for shift in (0..70).step_by(7) {
            let x = *b.get(*at)?;
            *at += 1;
            n |= u64::from(x & 0x7F).checked_shl(shift)?;
            if x & 0x80 == 0 {
                return Some(n);
            }
        }
        None
    }
    loop {
        // CRC32, then the header size as a vint of at most 3 bytes.
        let mut crc = [0u8; 4];
        if f.read_exact(&mut crc).is_err() {
            return Ok(None);
        }
        let mut size_bytes = Vec::new();
        let size = loop {
            let mut b = [0u8; 1];
            f.read_exact(&mut b).map_err(|_| bad())?;
            size_bytes.push(b[0]);
            if b[0] & 0x80 == 0 {
                break vint(&size_bytes, &mut 0).ok_or_else(bad)?;
            }
            if size_bytes.len() >= 3 {
                return Err(bad());
            }
        };
        if size == 0 || size > 2 << 20 {
            return Err(bad());
        }
        let mut h = vec![0u8; size as usize];
        f.read_exact(&mut h).map_err(|_| bad())?;
        let at = &mut 0usize;
        let kind = vint(&h, at).ok_or_else(bad)?;
        let flags = vint(&h, at).ok_or_else(bad)?;
        let extra = if flags & 0x01 != 0 { vint(&h, at).ok_or_else(bad)? } else { 0 };
        let data = if flags & 0x02 != 0 { vint(&h, at).ok_or_else(bad)? } else { 0 };
        if kind == 5 {
            return Ok(None);
        }
        if kind == 2 && extra > 0 {
            // File header: its name, then the extra area at the end.
            let file_flags = vint(&h, at).ok_or_else(bad)?;
            vint(&h, at).ok_or_else(bad)?; // unpacked size
            vint(&h, at).ok_or_else(bad)?; // attributes
            *at += if file_flags & 0x02 != 0 { 4 } else { 0 } + if file_flags & 0x04 != 0 { 4 } else { 0 };
            vint(&h, at).ok_or_else(bad)?; // compression
            vint(&h, at).ok_or_else(bad)?; // host OS
            let len = vint(&h, at).ok_or_else(bad)? as usize;
            let name = String::from_utf8_lossy(h.get(*at..*at + len).ok_or_else(bad)?).into_owned();
            let start = h.len().checked_sub(extra as usize).ok_or_else(bad)?;
            let x = &h[start..];
            let mut i = 0usize;
            while i < x.len() {
                let rec_size = vint(x, &mut i).ok_or_else(bad)? as usize;
                let end = i.checked_add(rec_size).filter(|e| *e <= x.len()).ok_or_else(bad)?;
                let mut j = i;
                if vint(x, &mut j).ok_or_else(bad)? == 5 {
                    let redir = vint(x, &mut j).ok_or_else(bad)?;
                    return Ok(Some((name, redir)));
                }
                i = end;
            }
        }
        f.seek(SeekFrom::Current(i64::try_from(data).map_err(|_| bad())?)).map_err(|_| bad())?;
    }
}

/// Limits on what one download may unpack to (a crafted archive can claim
/// anything): one file, the whole archive, and an LZMA dictionary.
const MAX_ENTRY: u64 = 8 << 30;
const MAX_TOTAL: u64 = 40 << 30;
const MAX_LZMA_DICT: u32 = 256 << 20;

fn too_big(name: &str) -> Error {
    Error::Game(format!("the download unpacks to more than the launcher allows ({name})"))
}

/// Copies at most what the caps leave (one file, and the archive so far),
/// counting the bytes really written; past that the file is removed.
fn copy_capped(src: &mut dyn std::io::Read, out: &mut std::fs::File, dest: &Path, total: u64, name: &str) -> Result<u64> {
    copy_within(src, out, dest, MAX_ENTRY.min(MAX_TOTAL.saturating_sub(total)), name)
}

fn copy_within(src: &mut dyn std::io::Read, out: &mut std::fs::File, dest: &Path, left: u64, name: &str) -> Result<u64> {
    let got = std::io::copy(&mut std::io::Read::take(src, left + 1), out)?;
    if got > left {
        let _ = std::fs::remove_file(dest);
        return Err(too_big(name));
    }
    Ok(got)
}

/// A zip entry's LZMA stream (APPNOTE 5.8.8: a 2-byte version, a 2-byte
/// properties length, the 5 properties bytes, then raw LZMA data).
fn zip_lzma<R: std::io::Read>(mut r: R, size: u64) -> Result<lzma_rust2::LzmaReader<R>> {
    let mut head = [0u8; 4];
    r.read_exact(&mut head)?;
    let len = u16::from_le_bytes([head[2], head[3]]) as usize;
    if len < 5 {
        return Err(Error::Game("a zip entry has damaged LZMA properties".into()));
    }
    let mut props = vec![0u8; len];
    r.read_exact(&mut props)?;
    let dict = u32::from_le_bytes([props[1], props[2], props[3], props[4]]);
    // The decoder allocates the dictionary up front; mods use 64 MB at most.
    if dict > MAX_LZMA_DICT {
        return Err(Error::Game(format!("a zip entry asks for a {} MB LZMA dictionary", dict >> 20)));
    }
    lzma_rust2::LzmaReader::new_with_props(r, size, props[0], dict, None).map_err(|e| Error::Game(format!("couldn't unpack an LZMA entry: {e}")))
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
    plan_for(entry, unpacked, &cpu_supports())
}

/// `plan` on a CPU with these instruction sets.
pub fn plan_for(entry: &ModEntry, unpacked: &Path, supports: &[&str]) -> Result<Vec<Copy>> {
    let mut out = plan_unskipped(entry, unpacked, supports)?;
    // Skyrim's own files only ever come from the player's own Steam copy,
    // never from a download.
    out.retain(|c| !game_owned(&c.to.to_string_lossy()));
    if !entry.skip.is_empty() {
        let norm = |p: &Path| p.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
        let skip: Vec<String> = entry.skip.iter().map(|s| s.replace('\\', "/").to_ascii_lowercase()).collect();
        out.retain(|c| !skip.contains(&norm(&c.to)));
        if out.is_empty() {
            return Err(Error::Game(format!("the download for {} had nothing to install", entry.name)));
        }
    }
    Ok(out)
}

fn plan_unskipped(entry: &ModEntry, unpacked: &Path, supports: &[&str]) -> Result<Vec<Copy>> {
    let mut out = Vec::new();
    if let Some(rel) = &entry.file {
        let to = safe_rel(rel).ok_or_else(|| Error::Game(format!("{} lists an unsafe path: {rel}", entry.name)))?;
        let name = to.file_name().ok_or_else(|| Error::Game(format!("{} lists no file name: {rel}", entry.name)))?;
        let from = unpacked.join(name);
        if !from.is_file() {
            return Err(Error::Game(format!("the download for {} is missing", entry.name)));
        }
        return Ok(vec![Copy { from, to }]);
    }
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
    if let Some(config) = fomod_config(unpacked) {
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
    } else if entry.cpu.is_empty() {
        return Err(Error::Game(format!("{} couldn't be installed automatically: the launcher couldn't tell where its files go. The launcher tries it again next time", entry.name)));
    }
    // One CPU build: nothing from the other builds' folders, and the chosen
    // folder's files over whatever the installer picked.
    if !entry.cpu.is_empty() {
        let (level, folder) = entry.cpu_pick_for(supports).ok_or_else(|| Error::Game(format!("{} has no build for this computer's processor", entry.name)))?;
        let find = |f: &str| {
            find_ci(unpacked, f).or_else(|| std::fs::read_dir(unpacked).ok()?.flatten().filter(|e| e.path().is_dir()).find_map(|e| find_ci(&e.path(), f)))
        };
        let dirs: Vec<PathBuf> = entry.cpu.values().filter_map(|f| find(f)).collect();
        out.retain(|c| !dirs.iter().any(|d| c.from.starts_with(d)));
        let Some(chosen) = find(&folder).filter(|d| d.is_dir()) else {
            return Err(Error::Game(format!("the download for {} doesn't have its {level} build ({folder})", entry.name)));
        };
        let mut picked = Vec::new();
        copy_tree(&data_root(&chosen).unwrap_or(chosen), Path::new("Data"), &mut picked);
        out.retain(|c| !picked.iter().any(|p| p.to.to_string_lossy().eq_ignore_ascii_case(&c.to.to_string_lossy())));
        out.extend(picked);
    }
    // A plugin below the top of Data never loads: leave it out, unless the
    // list lifts it to the top.
    let is_plugin = |p: &Path| p.extension().map(|x| ["esp", "esm", "esl"].contains(&x.to_string_lossy().to_ascii_lowercase().as_str())).unwrap_or(false);
    out.retain(|c| !(is_plugin(&c.to) && c.to.components().count() > 2));
    for l in &entry.lift {
        let Some(rel) = safe_rel(l) else { continue };
        let from = find_ci(unpacked, &rel.to_string_lossy()).or_else(|| {
            std::fs::read_dir(unpacked).ok()?.flatten().filter(|e| e.path().is_dir()).find_map(|e| find_ci(&e.path(), &rel.to_string_lossy()))
        });
        match from {
            Some(f) if f.is_file() => {
                let to = PathBuf::from("Data").join(f.file_name().unwrap());
                out.retain(|c| !c.to.to_string_lossy().eq_ignore_ascii_case(&to.to_string_lossy()));
                out.push(Copy { from: f, to });
            }
            _ => return Err(Error::Game(format!("the download for {} doesn't have {l}", entry.name))),
        }
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

/// Plugin names at the top of Data among game-relative paths.
pub fn top_plugins(files: &[String]) -> Vec<String> {
    files
        .iter()
        .filter_map(|f| f.replace('\\', "/").strip_prefix("Data/").map(str::to_string))
        .filter(|n| !n.contains('/') && [".esp", ".esm", ".esl"].iter().any(|x| n.to_ascii_lowercase().ends_with(x)))
        .collect()
}

/// Whether a FOMOD option's name allows Skyrim 1.6.1170: true when it names
/// no Skyrim version, names 1.6.1170, or names a lower version with "+" or
/// "and newer"/"and up"; false when it names only versions that exclude it
/// ("1.5.97", "v1.7.99+", "1.6.640 only").
pub fn option_fits_game(name: &str) -> bool {
    const GAME: (u32, u32, u32) = (1, 6, 1170);
    let l = name.to_ascii_lowercase();
    let b = l.as_bytes();
    let mut versions = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() && (i == 0 || !(b[i - 1].is_ascii_digit() || b[i - 1] == b'.')) {
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            let tok = l[start..i].trim_end_matches('.');
            let parts: Vec<u32> = tok.split('.').filter_map(|x| x.parse().ok()).collect();
            let before = l[..start].trim_end_matches(['v', ' ']);
            let upto = before.ends_with("pre-") || before.ends_with("pre") || before.ends_with("before") || before.ends_with('<') || before.ends_with("below");
            if parts.len() == 3 && parts[0] == 1 && (5..=7).contains(&parts[1]) {
                let rest = l[i..].trim_start_matches([' ', ')']);
                let open = ["+", "and newer", "and up", "or newer", "or later", "and later"].iter().any(|w| rest.starts_with(w));
                let v = (parts[0], parts[1], parts[2]);
                // "pre-1.6.1170" / "before 1.6.640": only below that version.
                versions.push(if upto { GAME < v } else { v == GAME || (open && v <= GAME) });
            } else if parts.len() == 2 && parts[0] == 1 && (5..=7).contains(&parts[1]) {
                // "1.6" names the whole AE line.
                versions.push(if upto { false } else { parts[1] == 6 });
            }
            continue;
        }
        i += 1;
    }
    versions.is_empty() || versions.iter().any(|&fits| fits)
}

/// FOMOD configs are UTF-8 or UTF-16 (with a byte order mark).
fn read_xml_text(path: &Path) -> Result<String> {
    let b = std::fs::read(path)?;
    if b.starts_with(&[0xFF, 0xFE]) || b.starts_with(&[0xFE, 0xFF]) {
        let le = b[0] == 0xFF;
        let units: Vec<u16> = b[2..].as_chunks::<2>().0.iter().map(|c| if le { u16::from_le_bytes(*c) } else { u16::from_be_bytes(*c) }).collect();
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
    Ok(fomod_pick(xml, choose)?.0)
}

/// The FOMOD config in an unpacked archive, as it `plan` finds it.
fn fomod_config(unpacked: &Path) -> Option<PathBuf> {
    find_ci(unpacked, "fomod/ModuleConfig.xml").or_else(|| {
        // The fomod folder can sit one level down.
        std::fs::read_dir(unpacked).ok()?.flatten().filter(|e| e.path().is_dir()).find_map(|e| find_ci(&e.path(), "fomod/ModuleConfig.xml"))
    })
}

/// For the log: each FOMOD group's options, with the ones picked marked
/// "[x]", so a test install shows the names to pick by.
pub fn fomod_report(entry: &ModEntry, unpacked: &Path) -> Option<Vec<String>> {
    let text = read_xml_text(&fomod_config(unpacked)?).ok()?;
    fomod_pick(&text, &entry.fomod).ok().map(|r| r.1)
}

/// A FOMOD file or folder: its source in the archive and its destination.
pub type FomodFile = (String, String);

/// `fomod_files` and a line per group saying what it offered and picked.
/// A choice starting with "!" never picks options whose names contain the
/// rest ("!Moss").
pub fn fomod_pick(xml: &str, choose: &[String]) -> Result<(Vec<FomodFile>, Vec<String>)> {
    let mut report = Vec::new();
    let doc = roxmltree::Document::parse(xml).map_err(|e| Error::Game(format!("the mod's FOMOD installer is unreadable: {e}")))?;
    let root = doc.root_element();
    let mut out = Vec::new();
    if let Some(req) = child(root, "requiredInstallFiles") {
        out.extend(file_list(req));
    }
    let wanted: Vec<String> = choose.iter().filter(|c| !c.starts_with('!')).map(|c| c.to_ascii_lowercase()).collect();
    let never: Vec<String> = choose.iter().filter_map(|c| c.strip_prefix('!')).map(|c| c.to_ascii_lowercase()).filter(|c| !c.is_empty()).collect();
    let name_of = |p: &roxmltree::Node| p.attribute("name").unwrap_or("").to_string();
    let banned = |p: &roxmltree::Node| {
        let n = name_of(p).to_ascii_lowercase();
        never.iter().any(|w| n.contains(w.as_str()))
    };
    let mut flags: BTreeMap<String, String> = BTreeMap::new();
    if let Some(steps) = child(root, "installSteps") {
        for step in children(steps, "installStep") {
            let Some(groups) = child(step, "optionalFileGroups") else { continue };
            for group in children(groups, "group") {
                let kind = group.attribute("type").unwrap_or("SelectAny");
                let Some(plugins) = child(group, "plugins") else { continue };
                let all: Vec<_> = children(plugins, "plugin").filter(|p| !banned(p)).collect();
                let mut named: Vec<_> = all.iter().filter(|p| wanted.iter().any(|w| p.attribute("name").unwrap_or("").to_ascii_lowercase().contains(w.as_str()))).copied().collect();
                // "AE" also matches "SSE/AE v1.7.99+" (Moons and Stars,
                // 2026-09-27): when any named option fits 1.6.1170, the ones
                // naming only other Skyrim versions are dropped (in a
                // SelectAny group they would all install, the last winning).
                if named.iter().any(|p| option_fits_game(p.attribute("name").unwrap_or(""))) {
                    named.retain(|p| option_fits_game(p.attribute("name").unwrap_or("")));
                }
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
                report.push(format!(
                    "{} ({kind}): {}",
                    group.attribute("name").unwrap_or("?"),
                    children(plugins, "plugin").map(|p| format!("{}{}", if picked.iter().any(|x| x == &p) { "[x] " } else { "" }, name_of(&p))).collect::<Vec<_>>().join(" | ")
                ));
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
    Ok((out, report))
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
    /// The FOMOD picks it was installed with (None before 0.1.68).
    #[serde(default)]
    pub fomod: Option<Vec<String>>,
    /// The CPU build it was installed with, for mods with a `cpu` map.
    #[serde(default)]
    pub cpu: Option<String>,
}

fn record_path(game_dir: &Path) -> PathBuf {
    game_dir.join(MODS_DIR).join("installed.json")
}

/// Written before an install moves anything into the game folder and
/// removed once its record is saved: while it's there, the mod isn't
/// installed (a half-installed Data folder never passes for a whole one).
pub fn installing_path(game_dir: &Path, id: &str) -> PathBuf {
    game_dir.join(MODS_DIR).join("installing").join(id)
}

/// Mods whose install stopped part way.
pub fn half_installed(game_dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(game_dir.join(MODS_DIR).join("installing")).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    out.sort();
    out
}

fn save_installed(game_dir: &Path, all: &Installed) -> Result<()> {
    let path = record_path(game_dir);
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = path.with_extension("json.part");
    std::fs::write(&tmp, serde_json::to_vec_pretty(all)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

/// Files a record names that no other record claims, for moving aside.
fn only_theirs(all: &Installed, id: &str, files: &[String], keep: &[String]) -> Vec<String> {
    files
        .iter()
        .filter(|f| !keep.iter().any(|k| k.eq_ignore_ascii_case(f)))
        .filter(|f| !all.mods.iter().any(|(other, r)| other != id && r.files.iter().chain(&r.skipped).any(|o| o.eq_ignore_ascii_case(f))))
        .filter(|f| !crate::allowlist::required_file(f))
        .cloned()
        .collect()
}

/// Every id the launcher's own list can hold, whatever the game version.
pub fn builtin_ids() -> HashSet<String> {
    builtin(None).into_iter().chain(builtin(Some(crate::community::TARGET))).map(|m| m.id).collect()
}

/// Mods the launcher installed that the list no longer has (an entry
/// removed from mods.json): their files that no listed mod uses are moved
/// to `.aetherial-dawn/disabled/<stamp>/` (never deleted), and their records
/// go. The launcher's own required mods are never touched. Returns
/// (mod name, files moved).
pub fn retire_unlisted(game_dir: &Path, list: &[ModEntry], stamp: &str) -> Result<Vec<(String, Vec<String>)>> {
    let mut all = load_installed(game_dir);
    let builtin = builtin_ids();
    let gone: Vec<String> = all.mods.keys().filter(|id| !builtin.contains(*id) && !list.iter().any(|m| &m.id == *id)).cloned().collect();
    // A list that lost most of what's installed (an empty or cut-short
    // mods.json from a misbehaving server) is never taken at its word.
    let theirs = all.mods.keys().filter(|id| !builtin.contains(*id)).count();
    if gone.is_empty() || list.iter().all(|m| builtin.contains(&m.id)) || gone.len() * 2 > theirs {
        return Ok(Vec::new());
    }
    let keep: Vec<String> = list.iter().flat_map(|m| m.check.iter().chain(&m.owns).map(|c| c.replace('\\', "/"))).collect();
    let mut out = Vec::new();
    for id in gone {
        let rec = all.mods.get(&id).cloned().unwrap_or_default();
        let files = only_theirs(&all, &id, &rec.files, &keep);
        crate::strays::move_aside(game_dir, &files, stamp)?;
        all.mods.remove(&id);
        save_installed(game_dir, &all)?;
        let _ = std::fs::remove_file(installing_path(game_dir, &id));
        out.push((rec.name, files));
    }
    Ok(out)
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
    // Files are moved from the unpacked folder (same drive, so no second
    // copy on disk and no time spent copying), except a file the installer
    // puts in two places, which is copied until its last use.
    let mut uses: std::collections::HashMap<&Path, usize> = std::collections::HashMap::new();
    for c in copies {
        *uses.entry(c.from.as_path()).or_default() += 1;
    }
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let marker = installing_path(game_dir, &entry.id);
    if let Some(p) = marker.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::write(&marker, entry.name.as_bytes())?;
    // A copy the launcher was closed during, from an earlier try.
    for c in copies {
        let _ = std::fs::remove_file(game_dir.join(&c.to).with_extension("aetherial-part"));
    }
    for c in copies {
        let dest = game_dir.join(&c.to);
        let rel = c.to.to_string_lossy().replace('\\', "/");
        // A one-file download is the pinned copy: it replaces the file there.
        let owned = entry.owns.iter().chain(&entry.file).any(|o| o.replace('\\', "/").eq_ignore_ascii_case(&rel));
        // Vortex's own copy of a file wins, unless it's a broken plugin or
        // a file this mod owns (backed up first).
        if dest.exists() && (vortex || is_settings(&c.to)) && present(&dest) && !owned {
            skipped.push(rel);
            continue;
        }
        if owned && dest.is_file() && std::fs::read(&dest).ok() != std::fs::read(&c.from).ok() {
            let to = game_dir.join(crate::strays::DISABLED_DIR).join(format!("{secs}-replaced")).join(&rel);
            if let Some(p) = to.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::copy(&dest, &to)?;
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
        let left = uses.get_mut(c.from.as_path()).map(|n| {
            *n -= 1;
            *n
        });
        if left != Some(0) || std::fs::rename(&c.from, &dest).is_err() {
            let tmp = dest.with_extension("aetherial-part");
            if let Err(e) = std::fs::copy(&c.from, &tmp).and_then(|_| std::fs::rename(&tmp, &dest)) {
                // No half-copied file stays in Data.
                let _ = std::fs::remove_file(&tmp);
                return Err(e.into());
            }
        }
        files.push(rel);
    }
    let rec = InstalledMod {
        name: entry.name.clone(),
        file_id,
        version,
        files,
        skipped,
        when: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        fomod: Some(entry.fomod.clone()),
        cpu: (!entry.cpu.is_empty()).then(|| entry.cpu_pick().map(|p| p.0)).flatten(),
    };
    let mut all = load_installed(game_dir);
    // Files the version installed before brought and this one doesn't (a
    // changed pin) go aside, so nothing of the old version is left in Data.
    if let Some(old) = all.mods.get(&entry.id) {
        let dropped: Vec<String> = old.files.iter().filter(|f| !rec.files.iter().chain(&rec.skipped).any(|n| n.eq_ignore_ascii_case(f))).cloned().collect();
        let dropped = only_theirs(&all, &entry.id, &dropped, &[]);
        if !dropped.is_empty() {
            crate::strays::move_aside(game_dir, &dropped, &format!("{secs}-old-version-{}", entry.id))?;
        }
    }
    all.mods.insert(entry.id.clone(), rec.clone());
    save_installed(game_dir, &all)?;
    std::fs::remove_file(&marker)?;
    Ok(rec)
}

/// What installing one archive came to.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Installed,
    /// Made for a newer Skyrim than the game's masters.
    TooNew(Vec<String>),
    /// Has an SKSE DLL built for another Skyrim.
    WrongBuild(Vec<String>),
}

/// Unpacks, checks and installs one downloaded archive: the launcher's
/// installer after the download (mods.rs), and the cutover rehearsal's.
/// `plugins_txt` gets the new plugins switched on, as Vortex would.
#[allow(clippy::too_many_arguments)]
pub fn install_archive(m: &ModEntry, archive: &Path, game_dir: &Path, file_id: Option<u64>, version: Option<String>, allow_too_new: bool, plugins_txt: Option<&Path>, log: &dyn Fn(&str)) -> Result<Outcome> {
    verify(m, archive)?;
    let work = game_dir.join(MODS_DIR).join("unpacked").join(&m.id);
    let _ = std::fs::remove_dir_all(&work);
    // A one-file download goes in as it is, under its own name.
    match m.file.as_deref().and_then(|f| Path::new(f).file_name().map(|n| n.to_owned())) {
        Some(name) => {
            std::fs::create_dir_all(&work)?;
            std::fs::copy(archive, work.join(name))?;
        }
        None => extract(archive, &work)?,
    }
    if let Some(r) = fomod_report(m, &work) {
        log(&format!("mods: {} installer options (picks {:?}): {}", m.name, m.fomod, r.join(" || ")));
    }
    let mut copies = plan(m, &work)?;
    if let Some((level, folder)) = m.cpu_pick().filter(|_| !m.cpu.is_empty()) {
        log(&format!("mods: {} takes the {level} build ({folder}) for this processor", m.name));
    }
    // An SKSE DLL for another Skyrim never goes in; the next file is tried.
    let mut wrong = fix_wrong_builds(&mut copies, &work);
    // The Unofficial Patch for Skyrim 1.7.99 crashes 1.6.1170.
    for c in &copies {
        if c.to.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case(crate::requirements::USSEP_PLUGIN)).unwrap_or(false) {
            if let Some(v) = crate::ussep::plugin_too_new(&c.from) {
                wrong.push((format!("Unofficial Patch {v}"), "made for Skyrim 1.7.99".into()));
            }
        }
    }
    if !wrong.is_empty() {
        // What was read from each refused DLL, and a copy kept aside, so
        // a wrong call can be checked (RaceMenu 0.4.20, 2026-09-27).
        // One folder per mod, replaced each time, so retries don't pile up.
        let keep = game_dir.join(crate::strays::DISABLED_DIR).join("refused-download").join(&m.id);
        for c in copies.iter().filter(|c| wrong.iter().any(|(n, _)| c.to.file_name().is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(n)))) {
            log(&format!("mods: {} {} read as: {}", m.name, c.to.display(), crate::skse::describe(&c.from)));
            if std::fs::create_dir_all(&keep).is_ok() {
                let _ = std::fs::copy(&c.from, keep.join(c.to.file_name().unwrap()));
            }
        }
        let _ = std::fs::remove_dir_all(&work);
        log(&format!("mods: {} download has the wrong build: {}", m.name, wrong.iter().map(|(n, w)| format!("{n} ({w})")).collect::<Vec<_>>().join(", ")));
        return Ok(Outcome::WrongBuild(wrong.into_iter().map(|(n, _)| n).collect()));
    }
    let newer = too_new_plugins(&copies, game_dir);
    if !newer.is_empty() && !allow_too_new {
        let _ = std::fs::remove_dir_all(&work);
        return Ok(Outcome::TooNew(newer));
    }
    let rec = apply(m, &copies, game_dir, file_id, version)?;
    // The keys a preset can set, named exactly as the mod defines them.
    for k in crate::presets::mcm_keys(game_dir, &rec.files) {
        log(&format!("mods: {} MCM keys in {k}", m.name));
    }
    let _ = std::fs::remove_dir_all(&work);
    // Only call it installed when the files really are in Data.
    if !m.installed(game_dir) {
        let gone: Vec<&str> = m.check.iter().map(String::as_str).filter(|c| !m.clone_with_check(c).installed(game_dir)).collect();
        log(&format!("mods: {} unpacked but {} isn't in the game folder (copied {}, left {})", m.name, gone.join(", "), rec.files.join(", "), rec.skipped.join(", ")));
        return Err(Error::Game(format!("{} downloaded, but {} didn't end up in your Skyrim folder", m.name, gone.join(" and "))));
    }
    let _ = std::fs::remove_file(archive);
    // And switch its plugins on, as Vortex would.
    if let Some(txt) = plugins_txt {
        let mut names: Vec<String> = m.check.iter().filter_map(|c| c.strip_prefix("Data/")).filter(|n| !n.contains('/') && [".esp", ".esm", ".esl"].iter().any(|x| n.to_ascii_lowercase().ends_with(x))).map(str::to_string).collect();
        names.retain(|n| {
            let ok = crate::loadorder::masters_present(game_dir, n);
            if !ok {
                log(&format!("mods: {} left {n} off: a master it needs isn't installed", m.name));
            }
            ok
        });
        // And the plugins it installed whose masters are all here (a
        // patch for a mod the player doesn't have stays off).
        for n in top_plugins(&rec.files) {
            if names.iter().any(|x| x.eq_ignore_ascii_case(&n)) {
                continue;
            }
            if crate::loadorder::masters_present(game_dir, &n) {
                names.push(n);
            } else {
                log(&format!("mods: {} left {n} off: a master it needs isn't installed", m.name));
            }
        }
        match crate::loadorder::switch_on(txt, &names) {
            Ok(on) if !on.is_empty() => log(&format!("mods: switched on in plugins.txt: {}", on.join(", "))),
            Ok(_) => {}
            Err(e) => log(&format!("mods: couldn't switch {} on in plugins.txt: {e}", names.join(", "))),
        }
    }
    log(&format!("mods: installed {} ({} files, {} left to Vortex or kept)", m.name, rec.files.len(), rec.skipped.len()));
    Ok(Outcome::Installed)
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
    #[test]
    fn play_requires_only_the_explicit_male_face_variant_for_exact_feed_pins() {
        use super::{play_required, ModEntry, NexusRef};
        let face = |id: &str, file| ModEntry { id: id.into(), name: id.into(),
            nexus: Some(NexusRef { mod_id: 22487, file: Some(file), pick: None }), ..Default::default() };
        let female = face("community-overlays-1-female-face", 104828);
        let male = face("community-overlays-1-male-face", 104868);
        let selected = play_required(vec![female.clone(), male.clone()]);
        assert_eq!(selected.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["community-overlays-1-male-face"]);
        assert_eq!(selected[0].check.len(), 25, "all selected male texture paths must be deployed");
        let wrong_file = face("community-overlays-1-female-face", 104829);
        assert_eq!(play_required(vec![wrong_file, male.clone()]).len(), 2, "a changed feed pin gets no exception");
        let changed_check = ModEntry { check: vec!["Data/Face.dds".into()], ..female };
        assert_eq!(play_required(vec![changed_check, male]).len(), 2, "a changed requirement gets no exception");
    }

    #[test]
    fn fomod_options_naming_another_skyrim_go_last() {
        assert!(!super::option_fits_game("SSE/AE v1.7.99+"));
        assert!(!super::option_fits_game("SE 1.5.97"));
        assert!(!super::option_fits_game("AE 1.6.640 only"));
        assert!(super::option_fits_game("AE 1.6.1170"));
        assert!(super::option_fits_game("AE (1.6.640+)"));
        assert!(super::option_fits_game("AE 1.6.1130 and newer"));
        assert!(super::option_fits_game("AE"));
        assert!(super::option_fits_game("Version 2.1.0"));
        assert!(super::option_fits_game("SE/AE"));
        assert!(super::option_fits_game("1.5.97 and 1.6"), "names the AE line too");
        assert!(!super::option_fits_game("pre-1.6.1170"));
        assert!(!super::option_fits_game("AE before 1.6.640"));
        assert!(super::option_fits_game("pre-1.7.99"));
        assert!(!super::option_fits_game("SE 1.5"));
        let xml = r#"<config><installSteps><installStep name="s"><optionalFileGroups><group name="Game version" type="SelectExactlyOne"><plugins>
            <plugin name="SSE/AE v1.7.99+"><files><file source="new/po3_MoonMod.dll" destination="SKSE/Plugins/po3_MoonMod.dll"/></files><typeDescriptor><type name="Optional"/></typeDescriptor></plugin>
            <plugin name="AE v1.6.1170"><files><file source="old/po3_MoonMod.dll" destination="SKSE/Plugins/po3_MoonMod.dll"/></files><typeDescriptor><type name="Optional"/></typeDescriptor></plugin>
            <plugin name="SE v1.5.97"><files><file source="se/po3_MoonMod.dll" destination="SKSE/Plugins/po3_MoonMod.dll"/></files><typeDescriptor><type name="Optional"/></typeDescriptor></plugin>
        </plugins></group></optionalFileGroups></installStep></installSteps></config>"#;
        let (files, report) = super::fomod_pick(xml, &["AE".into()]).unwrap();
        assert_eq!(files.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), vec!["old/po3_MoonMod.dll"], "{report:?}");
        // In a SelectAny group every named option installs: the one for
        // another Skyrim is dropped, not just moved last.
        let any = xml.replace("SelectExactlyOne", "SelectAny");
        let (files, report) = super::fomod_pick(&any, &["AE".into()]).unwrap();
        assert_eq!(files.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), vec!["old/po3_MoonMod.dll"], "{report:?}");
        // When none fits, the pick still stands (nothing better to take).
        let (files, _) = super::fomod_pick(&any, &["SSE/AE v1.7".into()]).unwrap();
        assert_eq!(files.iter().map(|f| f.0.as_str()).collect::<Vec<_>>(), vec!["new/po3_MoonMod.dll"]);
    }

    #[test]
    fn unpacks_zips_made_with_other_compression() {
        for (name, bytes) in [("lzma", &include_bytes!("../testdata/lzma.zip")[..]), ("bzip2", &include_bytes!("../testdata/bzip2.zip")[..])] {
            let t = tempfile::tempdir().unwrap();
            let a = t.path().join(format!("{name}.zip"));
            std::fs::write(&a, bytes).unwrap();
            super::extract(&a, &t.path().join("out")).unwrap();
            assert_eq!(std::fs::read(t.path().join("out/SKSE/Plugins/x.dll")).unwrap(), b"hello world".repeat(50), "{name}");
        }
        // A huge LZMA dictionary, or a changed checksum, is refused.
        let lz = include_bytes!("../testdata/lzma.zip").to_vec();
        let name_len = u16::from_le_bytes([lz[26], lz[27]]) as usize;
        let extra_len = u16::from_le_bytes([lz[28], lz[29]]) as usize;
        let props = 30 + name_len + extra_len + 4;
        let mut big = lz.clone();
        big[props + 1..props + 5].copy_from_slice(&(1u32 << 31).to_le_bytes());
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("big.zip"), &big).unwrap();
        let e = super::extract(&t.path().join("big.zip"), &t.path().join("out")).unwrap_err().to_string();
        assert!(e.contains("dictionary"), "{e}");
        let mut bad = lz.clone();
        let cd = bad.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
        bad[cd + 16] ^= 0xFF;
        std::fs::write(t.path().join("bad.zip"), &bad).unwrap();
        let e = super::extract(&t.path().join("bad.zip"), &t.path().join("out2")).unwrap_err().to_string();
        assert!(e.contains("damaged"), "{e}");
        // More bytes than an entry declared are cut off at the cap and the
        // file removed (the cap counts what's written, not the header).
        let t2 = tempfile::tempdir().unwrap();
        let dest = t2.path().join("x.dll");
        let mut out = std::fs::File::create(&dest).unwrap();
        assert!(super::copy_within(&mut &[7u8; 100][..], &mut out, &dest, 99, "x.dll").is_err());
        assert!(!dest.exists());
        let mut out = std::fs::File::create(&dest).unwrap();
        assert_eq!(super::copy_within(&mut &[7u8; 100][..], &mut out, &dest, 100, "x.dll").unwrap(), 100);
        // Zstandard, written here.
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("zstd.zip");
        {
            use std::io::Write;
            let mut z = zip::ZipWriter::new(std::fs::File::create(&a).unwrap());
            z.start_file("SKSE/Plugins/x.dll", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Zstd)).unwrap();
            z.write_all(b"zstd bytes").unwrap();
            z.finish().unwrap();
        }
        super::extract(&a, &t.path().join("out")).unwrap();
        assert_eq!(std::fs::read(t.path().join("out/SKSE/Plugins/x.dll")).unwrap(), b"zstd bytes");
    }


    #[test]
    fn installs_the_build_for_this_cpu() {
        let t = tempfile::tempdir().unwrap();
        let u = t.path().join("u");
        for d in ["raw-vs2022-windows", "raw-vs2022-windows-avx", "raw-vs2022-windows-avx2", "raw-vs2022-windows-avx512"] {
            let p = u.join(d).join("SKSE/Plugins");
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("hdtsmp64.dll"), d).unwrap();
        }
        std::fs::create_dir_all(u.join("common/SKSE/Plugins/hdtsmp64")).unwrap();
        std::fs::write(u.join("common/SKSE/Plugins/hdtsmp64/configs.xml"), "c").unwrap();
        let e: ModEntry = serde_json::from_value(serde_json::json!({"id": "fsmp", "name": "FSMP", "cpu": {
            "avx512": "raw-vs2022-windows-avx512", "avx2": "raw-vs2022-windows-avx2", "avx": "raw-vs2022-windows-avx", "plain": "raw-vs2022-windows"}})).unwrap();
        let dll = |sup: &[&str]| {
            let c = plan_for(&e, &u, sup).unwrap();
            let d: Vec<_> = c.iter().filter(|c| c.to.ends_with("hdtsmp64.dll")).collect();
            assert_eq!(d.len(), 1, "{c:?}");
            assert_eq!(d[0].to, PathBuf::from("Data/SKSE/Plugins/hdtsmp64.dll"));
            std::fs::read_to_string(&d[0].from).unwrap()
        };
        assert_eq!(dll(&["avx2", "avx", "plain"]), "raw-vs2022-windows-avx2");
        assert_eq!(dll(&["avx512", "avx2", "avx", "plain"]), "raw-vs2022-windows-avx512");
        assert_eq!(dll(&["plain"]), "raw-vs2022-windows");
        let only_avx = ModEntry { cpu: [("avx2".to_string(), "raw-vs2022-windows-avx2".to_string())].into_iter().collect(), ..e.clone() };
        assert!(plan_for(&only_avx, &u, &["avx", "plain"]).is_err(), "never a build the CPU can't run");
        assert!(cpu_supports().contains(&"plain"));
    }

    #[test]
    fn a_pinned_single_file_is_put_back_whenever_its_copy_changes() {
        let t = tempfile::tempdir().unwrap();
        let game = t.path().join("game");
        std::fs::create_dir_all(game.join("Data")).unwrap();
        let body = b"[Sliders]\r\nbEnableHeadSculpt=0\r\n";
        let rel = "Data/SKSE/Plugins/skee64_custom.ini";
        let e: ModEntry = serde_json::from_value(serde_json::json!({"id": "racemenu-patch", "name": "RaceMenu patch",
            "url": "https://example.invalid/skee64_custom.ini", "file": rel, "sha256": crate::patcher::sha256_bytes(body)})).unwrap();
        let got = |name: &str, bytes: &[u8]| {
            let a = t.path().join(name);
            std::fs::write(&a, bytes).unwrap();
            install_archive(&e, &a, &game, None, None, false, None, &|_| {})
        };
        assert!(!e.installed(&game));
        assert!(got("a.ini", b"changed on the way").is_err(), "the pin is checked before anything goes in");
        assert!(!game.join(rel).exists());
        assert!(matches!(got("b.ini", body).unwrap(), Outcome::Installed));
        assert_eq!(std::fs::read(game.join(rel)).unwrap(), body);
        assert!(e.installed(&game));
        // Edited by hand or by another mod: not installed, and the next
        // install replaces it (an .ini the player has is otherwise kept).
        std::fs::write(game.join(rel), b"[Sliders]\r\nbEnableHeadSculpt=1\r\n").unwrap();
        assert!(!e.installed(&game));
        assert!(matches!(got("c.ini", body).unwrap(), Outcome::Installed));
        assert_eq!(std::fs::read(game.join(rel)).unwrap(), body);
        assert!(e.installed(&game));
        // Only pinned one-file entries, at safe paths, are taken from a list.
        let list = |v: serde_json::Value| merged(None, Some(&serde_json::from_value(serde_json::json!({"mods": [v]})).unwrap()));
        let has = |l: Vec<ModEntry>| l.iter().any(|m| m.id == "racemenu-patch");
        assert!(has(list(serde_json::to_value(&e).unwrap())));
        assert!(!has(list(serde_json::json!({"id": "racemenu-patch", "name": "P", "url": "https://example.invalid/x.ini", "file": rel}))));
        assert!(!has(list(serde_json::json!({"id": "racemenu-patch", "name": "P", "url": "https://example.invalid/x.ini", "file": "../x.ini", "sha256": crate::patcher::sha256_bytes(body)}))));
        assert!(!has(list(serde_json::json!({"id": "racemenu-patch", "name": "P", "nexus": {"mod": 1}, "skip": ["../x"]}))));
        // The game's own files are never downloaded or skipped, pinned or not.
        for game_file in ["Data/ccBGSSSE001-Fish.esm", "Data/_ResourcePack.esl", "Data/Skyrim.esm", "Data/ccQDRSSE001-SurvivalMode.esm", "Data/ccBGSSSE001-Fish.bsa", "Data/Skyrim - Textures0.bsa", "SkyrimSE.exe"] {
            assert!(!has(list(serde_json::json!({"id": "racemenu-patch", "name": "P", "nexus": {"mod": 1}, "skip": [game_file]}))), "skip {game_file}");
            assert!(!has(list(serde_json::json!({"id": "racemenu-patch", "name": "P", "url": "https://example.invalid/x", "file": game_file, "sha256": crate::patcher::sha256_bytes(body)}))), "{game_file}");
        }
    }

    #[test]
    fn the_racemenu_guard_entries_are_taken_and_a_deployed_skip_is_found() {
        let list: ModList = serde_json::from_str(include_str!("../../docs/examples/racemenu-sync-guard.json")).unwrap();
        let all = merged(None, Some(&list));
        let guard = all.iter().find(|m| m.id == "racemenu-sync-guard").expect("pinned one-file entry kept");
        assert_eq!(guard.file.as_deref(), Some("Data/SKSE/Plugins/skee64_custom.ini"));
        let ahph = all.iter().find(|m| m.id == "alternate-high-poly-head").unwrap();
        let t = tempfile::tempdir().unwrap();
        assert!(skipped_present(&all, t.path()).is_empty());
        // Vortex deployed the whole package, morphs.ini included.
        let ini = t.path().join(&ahph.skip[0]);
        std::fs::create_dir_all(ini.parent().unwrap()).unwrap();
        std::fs::write(&ini, "[x]").unwrap();
        // Not this mod's copy (no Vortex record, not the launcher's): left alone.
        assert!(skipped_present(&all, t.path()).is_empty());
        // Vortex deployed it from this mod's folder.
        std::fs::write(t.path().join("Data/vortex.deployment.json"), serde_json::json!({"files": [
            {"relPath": "meshes/actors/character/facegenmorphs/AlternateHighPolyHead_SE.esp/morphs.ini", "source": "Alternate High Poly Head-148541-2-0-1"}]}).to_string()).unwrap();
        assert_eq!(skipped_present(&all, t.path()), ahph.skip);
    }

    #[test]
    fn a_skipped_file_is_left_out_of_an_install() {
        let t = tempfile::tempdir().unwrap();
        let u = t.path().join("u");
        for f in ["meshes/actors/character/FaceGenMorphs/morphs.ini", "meshes/actors/character/FaceGenMorphs/other.tri", "textures/head.dds"] {
            std::fs::create_dir_all(u.join(f).parent().unwrap()).unwrap();
            std::fs::write(u.join(f), f).unwrap();
        }
        let e: ModEntry = serde_json::from_value(serde_json::json!({"id": "ahph", "name": "Alternate High Poly Head",
            "skip": ["Data\\meshes\\actors\\character\\facegenmorphs\\morphs.ini"]})).unwrap();
        let mut to: Vec<String> = plan(&e, &u).unwrap().iter().map(|c| c.to.to_string_lossy().replace('\\', "/")).collect();
        to.sort();
        assert_eq!(to, ["Data/meshes/actors/character/FaceGenMorphs/other.tri", "Data/textures/head.dds"]);
        let all = ModEntry { skip: vec![], ..e.clone() };
        assert_eq!(plan(&all, &u).unwrap().len(), 3);
        // A download that carries a game file never installs it.
        for f in ["ccBGSSSE001-Fish.esm", "ccBGSSSE001-Fish.bsa", "_ResourcePack.esl", "_ResourcePack.bsa", "Skyrim - Textures0.bsa"] {
            std::fs::write(u.join(f), "game").unwrap();
        }
        assert_eq!(plan(&all, &u).unwrap().len(), 3);
        assert!(game_owned("SkyrimSE.exe") && game_owned("SkyrimSELauncher.exe") && !game_owned("skse64_loader.exe") && !game_owned("d3dx9_42.dll"));
        assert!(!game_owned("Data/meshes/actors/character/facegenmorphs/AlternateHighPolyHead_SE.esp/morphs.ini") && !game_owned("Data/SkyUI_SE.bsa"));
    }
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
    fn a_damaged_or_cut_short_download_leaves_no_half_unpacked_folder() {
        let t = tempfile::tempdir().unwrap();
        let good = t.path().join("good.zip");
        zip_with(&good, &[("a.esp", &[1u8; 4096][..]), ("b.esp", &[2u8; 4096][..])]);
        let bytes = std::fs::read(&good).unwrap();
        // Cut short in the second file's data, and with its bytes flipped.
        let cut = t.path().join("cut.zip");
        std::fs::write(&cut, &bytes[..bytes.len() / 2]).unwrap();
        let mut bad = bytes.clone();
        let at = bytes.windows(5).position(|w| w == b"b.esp").unwrap() + 64;
        bad[at] ^= 0xFF;
        let damaged = t.path().join("damaged.zip");
        std::fs::write(&damaged, &bad).unwrap();
        for a in [&cut, &damaged] {
            let out = t.path().join("out");
            assert!(super::extract(a, &out).is_err(), "{a:?}");
            assert!(!out.exists(), "no half-unpacked folder from {a:?}");
            assert!(!t.path().join("out.unpacking").exists(), "no temp folder from {a:?}");
        }
        // A good one after a bad one still lands whole, replacing an old folder.
        let out = t.path().join("out");
        std::fs::create_dir_all(out.join("stale")).unwrap();
        super::extract(&good, &out).unwrap();
        assert!(out.join("a.esp").exists() && out.join("b.esp").exists());
        assert!(!out.join("stale").exists());
        assert!(!t.path().join("out.unpacking").exists());
    }

    #[test]
    #[cfg(feature = "rar")]
    fn unpacks_rar_v4_and_v5() {
        let t = tempfile::tempdir().unwrap();
        for (v, bytes) in [("4", crate::testrar::rar4(&[("Main.esp", b"main"), ("Textures\\x.dds", b"dds bytes")])), ("5", crate::testrar::rar5(&[("Main.esp", b"main"), ("Textures/x.dds", b"dds bytes")]))] {
            let a = t.path().join(format!("m{v}.rar"));
            std::fs::write(&a, &bytes).unwrap();
            let out = t.path().join(format!("out{v}"));
            super::extract(&a, &out).unwrap_or_else(|e| panic!("RAR{v}: {e}"));
            assert_eq!(std::fs::read(out.join("Main.esp")).unwrap(), b"main", "RAR{v}");
            assert_eq!(std::fs::read(out.join("Textures/x.dds")).unwrap(), b"dds bytes", "RAR{v}");
        }
    }

    #[test]
    #[cfg(feature = "rar")]
    fn a_rar_never_writes_outside_its_folder() {
        let t = tempfile::tempdir().unwrap();
        let names = ["../evil.esp", "..\\evil2.esp", "/abs.esp", "\\abs2.esp", "C:/drive.esp", "C:\\drive2.esp", "Data/.../trick.esp", "ok.esp"];
        for (v, bytes) in [("4", crate::testrar::rar4(&names.map(|n| (n, &b"x"[..])))), ("5", crate::testrar::rar5(&names.map(|n| (n, &b"x"[..]))))] {
            let root = t.path().join(v);
            let a = root.join("a.rar");
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(&a, &bytes).unwrap();
            let out = root.join("deep/out");
            std::fs::create_dir_all(root.join("deep")).unwrap();
            super::extract(&a, &out).unwrap_or_else(|e| panic!("RAR{v}: {e}"));
            let mut all = Vec::new();
            let mut stack = vec![root.clone()];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(d).unwrap().flatten() {
                    if e.file_type().unwrap().is_dir() { stack.push(e.path()) } else { all.push(e.path().strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/")) }
                }
            }
            all.sort();
            // Nothing lands outside the folder. unrar itself turns some names
            // into harmless ones inside it, and differently per system ("C:"
            // becomes "C_" on Windows; a '\\' in a RAR5 name becomes '_' on
            // Linux), so only the invariant is checked, not those names.
            assert!(all.iter().all(|f| f == "a.rar" || f.starts_with("deep/out/")), "RAR{v}: {all:?}");
            assert!(all.iter().any(|f| f == "deep/out/ok.esp"), "RAR{v}: {all:?}");
            assert!(all.iter().all(|f| !f.split('/').any(|c| c == "..") && !f.contains(':')), "RAR{v}: {all:?}");
            for abs in ["/abs.esp", "/abs2.esp", "C:/drive.esp", "C:/drive2.esp"] {
                assert!(!Path::new(abs).exists(), "RAR{v}: {abs}");
            }
        }
    }

    #[test]
    #[cfg(feature = "rar")]
    fn a_damaged_or_cut_short_rar_leaves_nothing_behind() {
        let t = tempfile::tempdir().unwrap();
        let files: [(&str, &[u8]); 2] = [("a.esp", &[1u8; 4096]), ("b.esp", &[2u8; 4096])];
        for (v, bytes) in [("4", crate::testrar::rar4(&files)), ("5", crate::testrar::rar5(&files))] {
            let cut = bytes[..bytes.len() - 3000].to_vec();
            let mut flipped = bytes.clone();
            let n = flipped.len();
            flipped[n - 100] ^= 0xFF;
            for (what, b) in [("cut short", cut), ("damaged", flipped)] {
                let a = t.path().join("x.rar");
                std::fs::write(&a, &b).unwrap();
                let out = t.path().join("out");
                assert!(super::extract(&a, &out).is_err(), "RAR{v} {what}");
                assert!(!out.exists(), "RAR{v} {what}: no half-unpacked folder");
                assert!(!t.path().join("out.unpacking").exists(), "RAR{v} {what}: no temp folder");
            }
        }
    }

    #[test]
    #[cfg(feature = "rar")]
    fn a_rar_claiming_more_than_the_caps_is_refused_before_writing() {
        // A stored entry whose header claims 9 GiB: refused from the header,
        // nothing is written.
        let t = tempfile::tempdir().unwrap();
        let b = crate::testrar::rar5_claiming("big.esp", 9 << 30);
        let a = t.path().join("big.rar");
        std::fs::write(&a, &b).unwrap();
        let out = t.path().join("out");
        let e = super::extract(&a, &out).unwrap_err().to_string();
        assert!(e.contains("more than the launcher allows"), "{e}");
        assert!(!out.exists() && !t.path().join("out.unpacking").exists());
    }

    #[cfg(feature = "rar")]
    #[test]
    fn rar5_hard_links_file_copies_and_symlinks_are_refused_from_the_headers() {
        // Quality checks on PR #28: an entry pointing at another path must
        // never be made, even when it would land as an ordinary file.
        for (kind, what) in [(4, "a hard link"), (5, "a copy of another file"), (1, "a link"), (2, "a link"), (3, "a link")] {
            let t = tempfile::tempdir().unwrap();
            let a = t.path().join("x.rar");
            std::fs::write(&a, crate::testrar::rar5_redirect("Real.esp", "Copy.esp", kind)).unwrap();
            assert_eq!(super::rar5_redirect(&a).unwrap(), Some(("Copy.esp".to_string(), kind)));
            let out = t.path().join("out");
            let e = super::extract(&a, &out).unwrap_err().to_string();
            assert!(e.contains(&format!("Copy.esp in the download is {what}")), "{kind}: {e}");
            assert!(!out.exists() && !t.path().join("out.unpacking").exists());
        }
        // Ordinary RAR5 and RAR4 archives have no redirection.
        let t = tempfile::tempdir().unwrap();
        for (n, b) in [("5.rar", crate::testrar::rar5(&[("Main.esp", b"main")])), ("4.rar", crate::testrar::rar4(&[("Main.esp", b"main")]))] {
            std::fs::write(t.path().join(n), b).unwrap();
            assert_eq!(super::rar5_redirect(&t.path().join(n)).unwrap(), None);
        }
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
            revision: None,
        };
        let m = merged(Some("1.6.1170.0"), Some(&s));
        assert_eq!(m.iter().find(|e| e.id == "ussep").unwrap().nexus.as_ref().unwrap().file, Some(9));
        assert!(m.iter().all(|e| e.id != "bad" && e.id != "plain"));
        assert_eq!(m.last().unwrap().id, "new");
        assert_eq!(m.iter().find(|e| e.id == "ussep").unwrap().page().unwrap(), "https://www.nexusmods.com/skyrimspecialedition/mods/266?tab=files&file_id=9");
        // Built in: the Unofficial Patch is pinned to 4.3.8a (file 733846).
        let b = merged(Some("1.6.1170.0"), None);
        assert_eq!(b.iter().find(|e| e.id == "ussep").unwrap().page().unwrap(), "https://www.nexusmods.com/skyrimspecialedition/mods/266?tab=files&file_id=733846");
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
    fn address_library_and_tdm_are_pinned_to_files_that_run_on_1_6_1170() {
        let list = builtin(Some("1.6.1170.0"));
        let pin = |id: &str| list.iter().find(|e| e.id == id).and_then(|e| e.nexus.as_ref()).map(|n| (n.mod_id, n.file)).unwrap();
        assert_eq!(pin("address-library"), (32444, Some(470707)));
        assert_eq!(pin("true-directional-movement"), (51614, Some(798770)));
        let al = list.iter().find(|e| e.id == "address-library").unwrap();
        assert_eq!(al.check, ["Data/SKSE/Plugins/versionlib-1-6-1170-0.bin"]);
    }

    #[test]
    fn every_builtin_nexus_package_has_the_manual_vortex_file_pin() {
        // These are the 13 selected mod/file pairs in the 2026-09-29
        // migration inventory. A same-mod alternative must not become Ready.
        let expected = [
            ("address-library", 32444, 470707),
            ("engine-fixes", 17230, 669326),
            ("ussep", 266, 733846),
            ("menu-framework", 120352, 806684),
            ("imgui-icons", 114790, 690123),
            ("skyui", 12604, 749043),
            ("display-tweaks", 34705, 797175),
            ("black-screen-fix", 176509, 738614),
            ("mcm-helper", 53000, 795510),
            ("smoothcam", 41252, 729856),
            ("smoothcam-modern-preset", 41636, 220887),
            ("true-directional-movement", 51614, 798770),
            ("truehud", 62775, 798218),
        ];
        let builtins = builtin(Some("1.6.1170.0"));
        assert_eq!(builtins.len(), expected.len());
        for (id, mod_id, file_id) in expected {
            let entry = builtins.iter().find(|entry| entry.id == id).unwrap();
            let nexus = entry.nexus.as_ref().unwrap();
            assert_eq!((nexus.mod_id, nexus.file), (mod_id, Some(file_id)), "{id}");
        }
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
    fn fomod_never_picks_excluded_options_and_reports_groups() {
        let xml = r#"<config><installSteps><installStep name="s"><optionalFileGroups>
<group name="Extras" type="SelectAny"><plugins>
<plugin name="Main"><files><file source="main.esp"/></files><typeDescriptor><type name="Recommended"/></typeDescriptor></plugin>
<plugin name="Moss ESL"><files><file source="moss.esp"/></files><typeDescriptor><type name="Recommended"/></typeDescriptor></plugin>
</plugins></group></optionalFileGroups></installStep></installSteps></config>"#;
        let (files, report) = fomod_pick(xml, &["!moss".into()]).unwrap();
        assert_eq!(files.iter().map(|(s, _)| s.as_str()).collect::<Vec<_>>(), ["main.esp"]);
        assert_eq!(report, ["Extras (SelectAny): [x] Main | Moss ESL"]);
    }

    #[test]
    fn lifts_listed_plugins_and_drops_nested_ones() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("m.zip");
        zip_with(&a, &[("meshes/fire.nif", b"m"), ("plugins/esm/Embers XD.esm", b"e"), ("plugins/esp/Embers XD.esp", b"p"), ("patches/JK/Embers XD - Patch - JK.esp", b"j")]);
        let u = t.path().join("u");
        extract(&a, &u).unwrap();
        let e = ModEntry { id: "e".into(), name: "E".into(), lift: vec!["plugins/esp/Embers XD.esp".into()], ..Default::default() };
        let mut to: Vec<String> = plan(&e, &u).unwrap().iter().map(|c| c.to.to_string_lossy().replace('\\', "/")).collect();
        to.sort();
        assert_eq!(to, ["Data/Embers XD.esp", "Data/meshes/fire.nif"]);
        let bad = ModEntry { lift: vec!["plugins/none.esp".into()], ..e };
        assert!(plan(&bad, &u).is_err());
    }

    #[test]
    fn an_empty_or_cut_short_list_retires_nothing() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let mut all = Installed::default();
        for i in 0..6 {
            std::fs::create_dir_all(g.join("Data")).unwrap();
            std::fs::write(g.join(format!("Data/m{i}.esp")), b"x").unwrap();
            all.mods.insert(format!("m{i}"), InstalledMod { name: format!("m{i}"), files: vec![format!("Data/m{i}.esp")], ..Default::default() });
        }
        save_installed(g, &all).unwrap();
        let entry = |i: usize| ModEntry { id: format!("m{i}"), ..Default::default() };
        assert!(retire_unlisted(g, &builtin(None), "a").unwrap().is_empty());
        assert!(retire_unlisted(g, &[entry(0), entry(1)], "b").unwrap().is_empty());
        let gone = retire_unlisted(g, &[entry(0), entry(1), entry(2), entry(3), entry(4)], "c").unwrap();
        assert_eq!(gone, vec![("m5".to_string(), vec!["Data/m5.esp".to_string()])]);
        assert!(!g.join("Data/m5.esp").exists() && g.join(crate::strays::DISABLED_DIR).join("c/Data/m5.esp").is_file());
        assert!(!load_installed(g).mods.contains_key("m5"));
    }

    #[test]
    fn every_builtin_mod_has_the_curators_sizes() {
        for m in builtin(Some("1.6.1170.0")) {
            assert!(m.archive_bytes.is_some() && m.unpacked_bytes.is_some(), "{}", m.id);
        }
        assert_eq!(builtin_ids().len(), BUILTIN_SIZES.len());
    }

    #[test]
    fn files_are_moved_into_data_and_a_file_used_twice_lands_in_both_places() {
        let t = tempfile::tempdir().unwrap();
        let game = t.path().join("game");
        std::fs::create_dir_all(game.join("Data")).unwrap();
        let (a, b) = (t.path().join("a.dds"), t.path().join("b.dds"));
        std::fs::write(&a, b"a").unwrap();
        std::fs::write(&b, b"b").unwrap();
        let e = ModEntry { id: "tex".into(), name: "Tex".into(), ..Default::default() };
        let copies = [
            Copy { from: a.clone(), to: "Data/textures/a.dds".into() },
            Copy { from: b.clone(), to: "Data/textures/b1.dds".into() },
            Copy { from: b.clone(), to: "Data/textures/b2.dds".into() },
        ];
        apply(&e, &copies, &game, None, None).unwrap();
        for (f, want) in [("a.dds", "a"), ("b1.dds", "b"), ("b2.dds", "b")] {
            assert_eq!(std::fs::read_to_string(game.join("Data/textures").join(f)).unwrap(), want);
        }
        // Moved, not copied: nothing left behind to take space twice.
        assert!(!a.exists() && !b.exists());
    }

    #[test]
    fn no_checks_means_the_launchers_own_record() {
        let t = tempfile::tempdir().unwrap();
        let game = t.path().join("game");
        std::fs::create_dir_all(game.join("Data")).unwrap();
        let src = t.path().join("a.dds");
        std::fs::write(&src, b"t").unwrap();
        let e = ModEntry { id: "tex".into(), name: "Tex".into(), ..Default::default() };
        assert!(!e.installed(&game));
        apply(&e, &[Copy { from: src, to: "Data/textures/a.dds".into() }], &game, Some(1), None).unwrap();
        assert!(e.installed(&game));
        std::fs::remove_file(game.join("Data/textures/a.dds")).unwrap();
        assert!(!e.installed(&game));
        // Changed picks or pin: not installed any more.
        std::fs::write(game.join("Data/textures/a.dds"), b"t").unwrap();
        assert!(e.installed(&game));
        let picks = ModEntry { fomod: vec!["2K".into()], ..e.clone() };
        assert!(!picks.installed(&game));
        let pinned = ModEntry { nexus: Some(NexusRef { mod_id: 5, file: Some(2), pick: None }), ..e.clone() };
        assert!(!pinned.installed(&game));
        assert_eq!(top_plugins(&["Data/A.esp".into(), "Data/x/B.esp".into(), "Data/c.dds".into()]), ["A.esp"]);
    }

    #[test]
    fn a_vortex_file_check_is_independent_of_an_old_direct_install_pin() {
        let t = tempfile::tempdir().unwrap();
        let game = t.path().join("game");
        std::fs::create_dir_all(game.join("Data")).unwrap();
        let src = t.path().join("a.dds");
        std::fs::write(&src, b"texture").unwrap();
        let old = ModEntry {
            id: "texture".into(),
            name: "Texture".into(),
            nexus: Some(NexusRef { mod_id: 5, file: Some(1), pick: None }),
            check: vec!["Data/textures/a.dds".into()],
            ..Default::default()
        };
        apply(&old, &[Copy { from: src, to: old.check[0].clone().into() }], &game, Some(1), None).unwrap();
        let current = ModEntry { nexus: Some(NexusRef { mod_id: 5, file: Some(2), pick: None }), ..old };
        assert!(!current.installed(&game), "the direct-install receipt has the wrong file ID");
        assert!(current.game_files_present(&game), "the Vortex gate may validate the current package separately");
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
    fn black_screen_fix_owns_its_ini() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let e = builtin(None).into_iter().find(|m| m.id == "black-screen-fix").unwrap();
        let ini = format!("Data/SKSE/Plugins/{}", crate::requirements::DISPLAY_TWEAKS_INI);
        std::fs::create_dir_all(g.join("Data/SKSE/Plugins")).unwrap();
        // Display Tweaks' own ini doesn't count.
        std::fs::write(g.join(&ini), "display tweaks default").unwrap();
        assert!(!e.installed(g));
        // Vortex deploying it from the fix's folder does.
        std::fs::write(
            g.join("Data/vortex.deployment.json"),
            format!(r#"{{"files":[{{"relPath":"SKSE/Plugins/{}","source":"Black Screen and Startup Fix 176509 1 0 1750000000"}}]}}"#, crate::requirements::DISPLAY_TWEAKS_INI),
        )
        .unwrap();
        assert!(e.installed(g));
        std::fs::remove_file(g.join("Data/vortex.deployment.json")).unwrap();
        // The launcher's install replaces Display Tweaks' ini, keeping a copy.
        let src = g.join("fix.ini");
        std::fs::write(&src, "fix preset").unwrap();
        let rec = apply(&e, &[Copy { from: src, to: PathBuf::from(&ini) }], g, None, None).unwrap();
        assert_eq!(rec.files.as_slice(), std::slice::from_ref(&ini));
        assert_eq!(std::fs::read_to_string(g.join(&ini)).unwrap(), "fix preset");
        let aside: Vec<_> = std::fs::read_dir(g.join(crate::strays::DISABLED_DIR)).unwrap().flatten().collect();
        assert_eq!(std::fs::read_to_string(aside[0].path().join(&ini)).unwrap(), "display tweaks default");
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

#[cfg(test)]
mod served_list_check {
    /// AD_MODS_JSON=path: the whole served list parses and nothing is dropped.
    #[test]
    fn served_list_parses() {
        let Ok(p) = std::env::var("AD_MODS_JSON") else { return };
        let list: super::ModList = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
        let n = list.mods.len();
        let merged = super::merged(Some("1.6.1170.0"), Some(&list));
        let builtin = super::builtin(Some("1.6.1170.0")).len();
        assert_eq!(merged.len(), builtin + n);
    }

    #[test]
    fn safe_rel_refuses_parts_windows_would_rename() {
        for bad in ["...", "Data/.../x.esp", "Data/.. /x.esp", "Data/x.esp.", "Data/x.esp ", "Data/folder./x.esp", "../x", "C:/x", "/x"] {
            assert!(super::safe_rel(bad).is_none(), "{bad}");
        }
        for ok in ["Data/x.esp", "Data/./x.esp", "Data\\a b\\.hidden", "Data/v1.2/x.esp"] {
            assert!(super::safe_rel(ok).is_some(), "{ok}");
        }
    }
}

#[cfg(test)]
mod free_account_tests {
    use super::*;

    /// The server lane's own copy of the Unofficial Patch, as the cutover's
    /// mods.json carries it.
    fn lane_ussep() -> ModEntry {
        serde_json::from_str(r#"{"id":"unofficial-skyrim-special-edition-patch-733846","name":"Unofficial Skyrim Special Edition Patch","nexus":{"mod":266,"file":733846},"check":["Data/Unofficial Skyrim Special Edition Patch.esp"]}"#).unwrap()
    }

    #[test]
    fn the_unofficial_patch_is_fetched_once_after_the_cutover() {
        let t = tempfile::tempdir().unwrap();
        let list = merged(None, Some(&ModList { mods: vec![lane_ussep()], ..Default::default() }));
        assert_eq!(list.iter().filter(|m| m.nexus.as_ref().is_some_and(|n| n.mod_id == 266)).count(), 2);
        let fetch = to_fetch(&list, t.path());
        let ussep: Vec<&str> = fetch.iter().filter(|m| m.nexus.as_ref().is_some_and(|n| n.mod_id == 266)).map(|m| m.id.as_str()).collect();
        assert_eq!(ussep, ["ussep"]);
        // Everything else is still fetched.
        assert_eq!(fetch.len(), missing(&list, t.path()).len() - 1);
    }

    #[test]
    fn only_the_same_pinned_file_with_nothing_more_to_check_is_one_download() {
        let b = builtin(None);
        let ussep = b.iter().find(|m| m.id == "ussep").unwrap();
        assert!(same_download(ussep, &lane_ussep()));
        // Another file of the same mod, or more to check, is its own download.
        let mut other = lane_ussep();
        other.nexus.as_mut().unwrap().file = Some(1);
        assert!(!same_download(ussep, &other));
        let mut more = lane_ussep();
        more.check.push("Data/Unofficial Skyrim Special Edition Patch.bsa".into());
        assert!(!same_download(ussep, &more));
        // Unpinned entries are never merged.
        let mut loose = lane_ussep();
        loose.nexus.as_mut().unwrap().file = None;
        assert!(!same_download(&loose, &loose.clone()));
    }

    #[test]
    fn a_link_for_any_file_but_the_pinned_one_is_refused() {
        let pinned = NexusRef { mod_id: 266, file: Some(733846), pick: None };
        assert_eq!(link_fits(&pinned, NEXUS_GAME, 266, 733846), LinkFits::Take);
        assert_eq!(link_fits(&pinned, NEXUS_GAME, 266, 900001), LinkFits::OtherFile { pinned: 733846 });
        assert_eq!(link_fits(&pinned, NEXUS_GAME, 32444, 733846), LinkFits::OtherMod);
        assert_eq!(link_fits(&pinned, "skyrim", 266, 733846), LinkFits::OtherMod);
        // Unpinned: any file of the mod (checked for the game's build after).
        let loose = NexusRef { mod_id: 266, file: None, pick: Some("AE".into()) };
        assert_eq!(link_fits(&loose, NEXUS_GAME, 266, 900001), LinkFits::Take);
    }

    #[test]
    fn a_pinned_file_opens_its_own_download_page() {
        let e = ModEntry { nexus: Some(NexusRef { mod_id: 266, file: Some(733846), pick: None }), ..Default::default() };
        assert_eq!(e.download_page().unwrap(), "https://www.nexusmods.com/skyrimspecialedition/mods/266?tab=files&file_id=733846&nmm=1");
        let loose = ModEntry { nexus: Some(NexusRef { mod_id: 266, file: None, pick: None }), ..Default::default() };
        assert_eq!(loose.download_page(), loose.page());
    }

    #[test]
    fn a_required_mod_missing_its_support_files_is_installed_again() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path();
        let b = builtin(None);
        let mf = b.iter().find(|m| m.id == "menu-framework").unwrap();
        let p = g.join("Data/SKSE/Plugins");
        std::fs::create_dir_all(p.join("fonts")).unwrap();
        std::fs::create_dir_all(p.join("SKSEMenuFrameworkThemes")).unwrap();
        std::fs::write(p.join(crate::requirements::MENU_FRAMEWORK_DLL), b"dll").unwrap();
        // The DLL alone (Vortex left the fonts out, 0.1.38) isn't installed.
        assert!(!mf.installed(g));
        std::fs::write(p.join("SKSEMenuFrameworkStrings_EN.json"), b"{}").unwrap();
        std::fs::write(p.join("fonts/a.ttf"), b"f").unwrap();
        assert!(!mf.installed(g));
        std::fs::write(p.join("SKSEMenuFrameworkThemes/dark.json"), b"{}").unwrap();
        assert!(mf.installed(g));
        // The strings file under its other name counts too.
        std::fs::remove_file(p.join("SKSEMenuFrameworkStrings_EN.json")).unwrap();
        std::fs::write(p.join("SKSEMenuFrameworkStrings.json"), b"{}").unwrap();
        assert!(mf.installed(g));
        let ef = b.iter().find(|m| m.id == "engine-fixes").unwrap();
        assert!(ef.check.iter().any(|c| c.ends_with("EngineFixes.toml")));
    }
}
