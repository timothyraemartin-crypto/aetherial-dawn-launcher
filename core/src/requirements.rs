//! Mods every Aetherial Dawn player needs next to SKSE64 2.2.6: the Address
//! Library for SKSE Plugins (with the file for the server's Skyrim build) and
//! Crash Logger, so a crash report names the module that failed. Crash Logger
//! is free on GitHub, so the launcher installs it itself from the pinned
//! release. The Address Library is only on Nexus Mods, which doesn't allow
//! other sites to hand it out, so players download it there.

use std::path::{Path, PathBuf};

use crate::{Error, Result};

pub const CRASH_LOGGER_VERSION: &str = "1.25.0";
const CRASH_LOGGER_URL: &str = "https://github.com/alandtse/CrashLoggerSSE/releases/download/v1.25.0/CrashLogger_1.25.0.7z";
const CRASH_LOGGER_SHA256: &str = "ce8592d60a2394bc05874d4cc759d3513686c3e26f841fe831a98c9d66fa979c";
/// Crash Logger's files; tidying never moves them.
pub const CRASH_LOGGER_FILES: [&str; 3] = ["CrashLogger.dll", "CrashLogger.pdb", "msdia140.dll"];

/// Skyrim Souls RE (unpaused menus), Timothy's requirement 2026-09-26.
/// 2.4.0 is the newest release built for 1.6.1170; 3.x targets 1.7.x.
pub const SOULS_VERSION: &str = "2.4.0";
pub const SOULS_PAGE: &str = "https://www.nexusmods.com/skyrimspecialedition/mods/27859";
const SOULS_URL: &str = "https://github.com/Vermunds/SkyrimSoulsRE/releases/download/2.4.0/SkyrimSoulsRE.zip";
const SOULS_SHA256: &str = "a5295783c6cab3e766bdd6306896910f61fe5f97122997dea0bbb9bf76f735fc";
/// Skyrim Souls RE's files; tidying never moves them.
pub const SOULS_FILES: [&str; 4] = ["SkyrimSoulsRE.dll", "SkyrimSoulsRE.ini", "SkyrimSoulsRE.pdb", "CombatAlertOverlayMenu.swf"];

/// SSE Engine Fixes, required by Skyrim Souls RE (Timothy, 2026-09-26). He
/// chose the All-In-One package from Nexus (20:19), which holds the SKSE
/// plugin and the preloader files next to SkyrimSE.exe. The GitHub part 1
/// installer stays for reference but isn't run before Play any more.
pub const ENGINE_FIXES_VERSION: &str = "7.0.20";
const ENGINE_FIXES_URL: &str = "https://github.com/aers/EngineFixesSkyrim64/releases/download/7.0.20/EngineFixes.FOMOD.Installer.7z";
const ENGINE_FIXES_SHA256: &str = "21330c95011f41859139635b43ce95ffb5fbadd3cd375d2f4358abcc3cb99407";
/// Engine Fixes' files in Data/SKSE/Plugins; tidying never moves them.
pub const ENGINE_FIXES_FILES: [&str; 5] = ["EngineFixes.dll", "EngineFixes.pdb", "EngineFixes.toml", "EngineFixes_SNCT.ini", "EngineFixes_preload.txt"];
/// Engine Fixes' old part 2 files, next to SkyrimSE.exe. Engine Fixes 7 on
/// SKSE 2.2 loads early through SKSE's own EngineFixes_preload.txt, and the
/// All-In-One package doesn't ship all of these, so they're copied when the
/// package has them but never required.
pub const ENGINE_FIXES_PRELOAD: [&str; 3] = ["d3dx9_42.dll", "tbb.dll", "tbbmalloc.dll"];
/// SKSE Menu Framework's plugin; tidying never moves it.
pub const MENU_FRAMEWORK_DLL: &str = "SKSEMenuFramework.dll";
/// ImGui Icons' folder in Data/Interface; tidying never moves it.
pub const IMGUI_ICONS_DIR: &str = "ImGuiIcons";
/// The Unofficial Skyrim Special Edition Patch's plugin.
pub const USSEP_PLUGIN: &str = "Unofficial Skyrim Special Edition Patch.esp";
/// SkyUI SE's plugin and archive (Timothy, 2026-09-26: "lets add skyui").
pub const SKYUI_PLUGIN: &str = "SkyUI_SE.esp";
pub const SKYUI_ARCHIVE: &str = "SkyUI_SE.bsa";
/// SSE Display Tweaks and its settings file, which the Black Screen and
/// Startup Fix replaces (Timothy, 2026-09-26).
pub const DISPLAY_TWEAKS_DLL: &str = "SSEDisplayTweaks.dll";
pub const DISPLAY_TWEAKS_INI: &str = "SSEDisplayTweaks.ini";

/// A required mod players download from Nexus Mods themselves, because Nexus
/// doesn't let other sites hand its files out.
#[derive(Debug, Clone, serde::Serialize)]
pub struct NexusMod {
    pub id: &'static str,
    pub name: &'static str,
    pub page: &'static str,
    /// Which file on the Files tab to pick.
    pub pick: String,
    /// What the launcher looks for, in the player's words.
    pub looks_for: String,
}

/// Nexus Mods pages the launcher may open.
pub const NEXUS_PAGES: [(&str, &str); 6] = [
    ("address-library", "https://www.nexusmods.com/skyrimspecialedition/mods/32444?tab=files"),
    ("engine-fixes", "https://www.nexusmods.com/skyrimspecialedition/mods/17230?tab=files"),
    ("ussep", "https://www.nexusmods.com/skyrimspecialedition/mods/266?tab=files"),
    ("menu-framework", "https://www.nexusmods.com/skyrimspecialedition/mods/120352?tab=files"),
    ("imgui-icons", "https://www.nexusmods.com/skyrimspecialedition/mods/114790?tab=files"),
    ("skyui", "https://www.nexusmods.com/skyrimspecialedition/mods/12604?tab=files"),
];

fn page(id: &str) -> &'static str {
    NEXUS_PAGES.iter().find(|(i, _)| *i == id).map(|(_, p)| *p).unwrap_or("")
}

pub fn engine_fixes_ok(game_dir: &Path) -> bool {
    plugins_dir(game_dir).join("EngineFixes.dll").is_file()
}

pub fn engine_fixes_preload_ok(game_dir: &Path) -> bool {
    plugins_dir(game_dir).join("EngineFixes_preload.txt").is_file()
}

pub fn ussep_ok(game_dir: &Path) -> bool {
    game_dir.join("Data").join(USSEP_PLUGIN).is_file() && crate::ussep::too_new(game_dir).is_none()
}

/// A real SkyUI SE install: its plugin (not the empty stub an old setup left
/// behind in the first live test) and its archive.
pub fn skyui_ok(game_dir: &Path) -> bool {
    let data = game_dir.join("Data");
    let esp = data.join(SKYUI_PLUGIN);
    esp.is_file() && crate::loadorder::broken(&esp).is_none() && data.join(SKYUI_ARCHIVE).is_file()
}

pub fn menu_framework_ok(game_dir: &Path) -> bool {
    plugins_dir(game_dir).join(MENU_FRAMEWORK_DLL).is_file()
}

pub fn imgui_icons_ok(game_dir: &Path) -> bool {
    game_dir.join("Data").join("Interface").join(IMGUI_ICONS_DIR).is_dir()
}

/// Removes half-written copies (`<file>.part`) of the launcher's own mod
/// files left in Data/SKSE/Plugins by an install that was cut short, such
/// as EngineFixes.toml.part. Only names from the launcher's lists.
pub fn clean_partials(game_dir: &Path) -> Vec<String> {
    let dir = plugins_dir(game_dir);
    let mut out = Vec::new();
    for f in CRASH_LOGGER_FILES.iter().chain(SOULS_FILES.iter()).chain(ENGINE_FIXES_FILES.iter()) {
        let p = dir.join(format!("{f}.part"));
        if p.is_file() && std::fs::remove_file(&p).is_ok() {
            out.push(format!("{f}.part"));
        }
    }
    out
}

/// The Nexus-only required mods that aren't installed yet, in the order the
/// player should get them.
pub fn missing_nexus_mods(game_dir: &Path, game_version: Option<&str>) -> Vec<NexusMod> {
    let mut out = Vec::new();
    if let Some(v) = game_version {
        if !address_library_ok(game_dir, v) {
            out.push(NexusMod {
                id: "address-library",
                name: "Address Library for SKSE Plugins",
                page: page("address-library"),
                pick: "All in one (Anniversary Edition)".into(),
                looks_for: format!("{} in Data\\SKSE\\Plugins", address_library_file(v)),
            });
        }
    }
    if !engine_fixes_ok(game_dir) || !engine_fixes_preload_ok(game_dir) {
        out.push(NexusMod {
            id: "engine-fixes",
            name: "SSE Engine Fixes (All-In-One)",
            page: page("engine-fixes"),
            pick: "Engine Fixes (All-In-One) for 1.6.1170 and newer".into(),
            looks_for: "EngineFixes.dll and EngineFixes_preload.txt in Data\\SKSE\\Plugins".into(),
        });
    }
    if !ussep_ok(game_dir) {
        out.push(NexusMod {
            id: "ussep",
            name: "Unofficial Skyrim Special Edition Patch",
            page: page("ussep"),
            pick: "version 4.3.8a, the one for Skyrim 1.6.1170, in the archived files at the bottom of the Files tab (4.3.9 and newer need Skyrim 1.7.99)".into(),
            looks_for: format!("{USSEP_PLUGIN} in Data"),
        });
    }
    if !menu_framework_ok(game_dir) {
        out.push(NexusMod {
            id: "menu-framework",
            name: "SKSE Menu Framework",
            page: page("menu-framework"),
            pick: "the main file".into(),
            looks_for: format!("{MENU_FRAMEWORK_DLL} in Data\\SKSE\\Plugins"),
        });
    }
    if !imgui_icons_ok(game_dir) {
        out.push(NexusMod {
            id: "imgui-icons",
            name: "ImGui Icons",
            page: page("imgui-icons"),
            pick: "the main file".into(),
            looks_for: format!("the {IMGUI_ICONS_DIR} folder in Data\\Interface"),
        });
    }
    if !skyui_ok(game_dir) {
        out.push(NexusMod {
            id: "skyui",
            name: "SkyUI",
            page: page("skyui"),
            pick: "SkyUI 5.2SE (main file)".into(),
            looks_for: format!("{SKYUI_PLUGIN} and {SKYUI_ARCHIVE} in Data"),
        });
    }
    out
}

/// Downloads SSE Engine Fixes part 1 from its GitHub release, checks it, and
/// puts the AE plugin and its settings in Data/SKSE/Plugins, keeping settings
/// the player already has.
#[allow(dead_code)]
pub async fn install_engine_fixes(client: &reqwest::Client, game_dir: &Path) -> Result<()> {
    let bytes = client.get(ENGINE_FIXES_URL).header("User-Agent", "AetherialDawnLauncher").send().await?.error_for_status()?.bytes().await?;
    use sha2::{Digest, Sha256};
    let got = hex::encode(Sha256::digest(&bytes));
    if !got.eq_ignore_ascii_case(ENGINE_FIXES_SHA256) {
        return Err(Error::HashMismatch { path: "SSE Engine Fixes".into(), expected: ENGINE_FIXES_SHA256.into(), actual: got });
    }
    let dir = plugins_dir(game_dir);
    std::fs::create_dir_all(&dir)?;
    unpack_engine_fixes(&bytes, &dir)
}

fn unpack_engine_fixes(archive: &[u8], dir: &Path) -> Result<()> {
    let mut reader = sevenz_rust2::ArchiveReader::new(std::io::Cursor::new(archive), sevenz_rust2::Password::empty())
        .map_err(|e| Error::Game(format!("SSE Engine Fixes download is damaged: {e}")))?;
    let mut dll = false;
    reader
        .for_each_entries(|entry, data| {
            let name = entry.name().replace('\\', "/");
            let Some((_, rel)) = name.split_once('/') else { return Ok(true) };
            let file = rel.strip_prefix("AE/SKSE/Plugins/").or_else(|| rel.strip_prefix("Required/SKSE/Plugins/"));
            let Some(file) = file else { return Ok(true) };
            if entry.is_directory() || !ENGINE_FIXES_FILES.iter().any(|f| f.eq_ignore_ascii_case(file)) {
                return Ok(true);
            }
            let dest = dir.join(file);
            let settings = !file.to_ascii_lowercase().ends_with(".dll") && !file.to_ascii_lowercase().ends_with(".pdb");
            if settings && dest.is_file() {
                return Ok(true);
            }
            let tmp = dir.join(format!("{file}.part"));
            let mut out = std::fs::File::create(&tmp)?;
            std::io::copy(data, &mut out)?;
            drop(out);
            std::fs::rename(&tmp, &dest)?;
            dll |= file.eq_ignore_ascii_case("EngineFixes.dll");
            Ok(true)
        })
        .map_err(|e| Error::Game(format!("couldn't unpack SSE Engine Fixes: {e}")))?;
    if !dll {
        return Err(Error::Game("the SSE Engine Fixes download didn't contain EngineFixes.dll".into()));
    }
    Ok(())
}

pub const SKSE_VERSION: &str = "2.2.6";
const SKSE_URL: &str = "https://github.com/ianpatt/skse64/releases/download/v2.2.6/skse64_2_02_06.7z";
const SKSE_SHA256: &str = "d7297f1a1d613e5265e1af4dbbfe8bd37a32719c1ccef363fc6187fa6eba0848";

/// SKSE for the server's build: its loader, its DLL for 1.6.1170, and its
/// scripts in Data\Scripts.
pub fn skse_ok(game_dir: &Path) -> bool {
    game_dir.join("skse64_loader.exe").is_file() && game_dir.join("skse64_1_6_1170.dll").is_file()
}

/// Downloads SKSE 2.2.6 from its official GitHub release, checks it, and puts
/// the loader, DLL and scripts in the game folder (readme files are skipped).
pub async fn install_skse(client: &reqwest::Client, game_dir: &Path) -> Result<()> {
    install_skse_from(client, game_dir, &Sources::official().skse).await
}

async fn install_skse_from(client: &reqwest::Client, game_dir: &Path, src: &Source) -> Result<()> {
    let bytes = download_checked(client, src, "SKSE").await?;
    unpack_skse(&bytes, game_dir)
}

/// Where a required helper is downloaded from and the SHA-256 it must have.
#[derive(Debug, Clone)]
pub struct Source {
    pub url: String,
    pub sha256: String,
}

/// Where the three helpers Play installs come from (tests use a local server).
#[derive(Debug, Clone)]
pub struct Sources {
    pub skse: Source,
    pub crash_logger: Source,
    pub souls: Source,
}

impl Sources {
    /// The pinned official GitHub releases.
    pub fn official() -> Sources {
        let s = |url: &str, sha256: &str| Source { url: url.into(), sha256: sha256.into() };
        Sources { skse: s(SKSE_URL, SKSE_SHA256), crash_logger: s(CRASH_LOGGER_URL, CRASH_LOGGER_SHA256), souls: s(SOULS_URL, SOULS_SHA256) }
    }
}

async fn download_checked(client: &reqwest::Client, src: &Source, label: &str) -> Result<Vec<u8>> {
    let bytes = client.get(&src.url).header("User-Agent", "AetherialDawnLauncher").send().await?.error_for_status()?.bytes().await?;
    use sha2::{Digest, Sha256};
    let got = hex::encode(Sha256::digest(&bytes));
    if !got.eq_ignore_ascii_case(&src.sha256) {
        return Err(Error::HashMismatch { path: label.into(), expected: src.sha256.clone(), actual: got });
    }
    Ok(bytes.to_vec())
}

/// A required helper Play couldn't install, and why.
#[derive(Debug)]
pub struct HelperFailed {
    pub name: &'static str,
    pub version: &'static str,
    pub cause: Error,
}

impl HelperFailed {
    /// The warning Play shows when it starts the game without this helper.
    pub fn warning(&self) -> String {
        let (n, v) = (self.name, self.version);
        let why = match &self.cause {
            Error::Http(_) => "the download didn't get through".to_string(),
            Error::HashMismatch { .. } => "the download was damaged".to_string(),
            Error::Io(e) => format!("it couldn't be written into your Skyrim folder ({e})"),
            e => e.to_string(),
        };
        format!("{n} {v} isn't installed: {why}. Skyrim starts without it; Play tries again next time.")
    }

    /// The sentence Play shows, worded from what actually went wrong.
    pub fn message(&self) -> String {
        let (n, v) = (self.name, self.version);
        match &self.cause {
            Error::Http(e) => format!("Couldn't download {n} {v} ({}). Check your internet connection and try again.", crate::scrub(&e.to_string())),
            Error::HashMismatch { .. } => format!("The {n} {v} download was damaged, so it wasn't installed. Try again."),
            Error::Io(e) => format!("Couldn't write {n} {v} into your Skyrim folder ({e}). Close Skyrim and Vortex, then try again."),
            e => format!("Couldn't install {n} {v}: {e}."),
        }
    }
}

/// What Play does before anything else: installs SKSE 2.2.6, Crash Logger
/// and Skyrim Souls RE when they're missing or SKSE wouldn't load the copy
/// there, and checks each again afterwards. SKSE is needed to join at all,
/// so its failure is the Err that stops Play. Crash Logger and Souls RE that
/// can't be installed come back in the Ok list: Play warns and carries on,
/// and tries them again next time (the default chosen 2026-09-28 until
/// Timothy decides between blocking and warning). `log` gets a line per step.
pub async fn ensure_helpers(client: &reqwest::Client, game_dir: &Path, src: &Sources, log: &mut (dyn FnMut(&str) + Send)) -> std::result::Result<Vec<HelperFailed>, HelperFailed> {
    let cleaned = clean_partials(game_dir);
    if !cleaned.is_empty() {
        log(&format!("removed half-written mod files: {}", cleaned.join(", ")));
    }
    let fail = |name, version, cause| HelperFailed { name, version, cause };
    if !skse_ok(game_dir) {
        install_skse_from(client, game_dir, &src.skse).await.map_err(|e| fail("SKSE", SKSE_VERSION, e))?;
        if !skse_ok(game_dir) {
            return Err(fail("SKSE", SKSE_VERSION, Error::Game("its files weren't there after installing".into())));
        }
        log(&format!("installed SKSE {SKSE_VERSION}"));
    }
    if !crash_logger_ok(game_dir) {
        // A good copy set aside by an older launcher beats a download (and
        // works with GitHub unreachable).
        match restore_crash_logger(game_dir) {
            Ok(Some(stamp)) => log(&format!("put the crash logger back from {}", stamp.display())),
            Ok(None) => {}
            Err(e) => log(&format!("couldn't put the crash logger back: {e}")),
        }
    }
    let mut missing = Vec::new();
    if !crash_logger_ok(game_dir) {
        let why = crash_logger_state(game_dir);
        match install_crash_logger_from(client, game_dir, &src.crash_logger).await {
            Ok(()) if crash_logger_ok(game_dir) => log(&format!("installed Crash Logger {CRASH_LOGGER_VERSION} ({why})")),
            Ok(()) => missing.push(fail("Crash Logger", CRASH_LOGGER_VERSION, Error::Game("SKSE wouldn't load the copy it installed".into()))),
            Err(e) => missing.push(fail("Crash Logger", CRASH_LOGGER_VERSION, e)),
        }
    }
    if !souls_ok(game_dir) {
        let why = dll_state(&plugins_dir(game_dir).join("SkyrimSoulsRE.dll"));
        match install_souls_from(client, game_dir, &src.souls).await {
            Ok(()) if souls_ok(game_dir) => log(&format!("installed Skyrim Souls RE {SOULS_VERSION} ({why})")),
            Ok(()) => missing.push(fail("Skyrim Souls RE", SOULS_VERSION, Error::Game("SKSE wouldn't load the copy it installed".into()))),
            Err(e) => missing.push(fail("Skyrim Souls RE", SOULS_VERSION, e)),
        }
    }
    Ok(missing)
}

fn crash_logger_state(game_dir: &Path) -> String {
    dll_state(&plugins_dir(game_dir).join("CrashLogger.dll"))
}

/// Why a helper DLL is being installed, for the log.
fn dll_state(dll: &Path) -> String {
    if !dll.is_file() {
        return "it was missing".into();
    }
    match crate::skse::build_of(dll) {
        crate::skse::Build::Wrong(w) => format!("the copy there was refused: {w}"),
        _ => "the copy there couldn't be read as an SKSE plugin".into(),
    }
}

fn unpack_skse(archive: &[u8], game_dir: &Path) -> Result<()> {
    let mut reader = sevenz_rust2::ArchiveReader::new(std::io::Cursor::new(archive), sevenz_rust2::Password::empty())
        .map_err(|e| Error::Game(format!("SKSE download is damaged: {e}")))?;
    let mut wrote = 0;
    reader
        .for_each_entries(|entry, data| {
            let name = entry.name().replace('\\', "/");
            let Some((_, rel)) = name.split_once('/') else { return Ok(true) };
            let keep = !entry.is_directory()
                && !rel.contains("..")
                && (rel.starts_with("Data/Scripts/") || (!rel.contains('/') && (rel.ends_with(".exe") || rel.ends_with(".dll"))));
            if !keep {
                return Ok(true);
            }
            let dest = game_dir.join(rel);
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p)?;
            }
            let tmp = dest.with_extension("part");
            let mut out = std::fs::File::create(&tmp)?;
            std::io::copy(data, &mut out)?;
            drop(out);
            std::fs::rename(&tmp, &dest)?;
            wrote += 1;
            Ok(true)
        })
        .map_err(|e| Error::Game(format!("couldn't unpack SKSE: {e}")))?;
    if wrote == 0 {
        return Err(Error::Game("the SKSE download was empty".into()));
    }
    Ok(())
}

fn plugins_dir(game_dir: &Path) -> PathBuf {
    game_dir.join("Data").join("SKSE").join("Plugins")
}

/// The Address Library file SKSE plugins look for on this game version, such
/// as `versionlib-1-6-1170-0.bin`.
pub fn address_library_file(game_version: &str) -> String {
    format!("versionlib-{}.bin", game_version.trim().replace('.', "-"))
}

pub fn address_library_ok(game_dir: &Path, game_version: &str) -> bool {
    plugins_dir(game_dir).join(address_library_file(game_version)).is_file()
}

/// A required helper DLL counts as installed only when SKSE 2.2.6 would load
/// it on 1.6.1170. A copy built for another Skyrim, or one that can't be read
/// as an SKSE plugin (cut short, damaged), is installed again.
fn loads(dll: &Path) -> bool {
    dll.is_file() && crate::skse::build_of(dll) == crate::skse::Build::Fits
}

pub fn crash_logger_ok(game_dir: &Path) -> bool {
    loads(&plugins_dir(game_dir).join("CrashLogger.dll"))
}

/// Launchers before 0.1.20 moved crash loggers aside with other SKSE plugins.
/// Puts the newest one back when none is in the game, so the next crash
/// names the module that failed. Copies set aside because SKSE would refuse
/// them (`-wrong-build`, `-too-new`) and any copy SKSE wouldn't load are
/// left where they are. Returns the folder it came from.
pub fn restore_crash_logger(game_dir: &Path) -> Result<Option<PathBuf>> {
    let plugins = plugins_dir(game_dir);
    let current = plugins.join("CrashLogger.dll");
    // A working crash logger is in place; a CrashLogger.dll SKSE wouldn't
    // load doesn't count, and is set aside when a good copy replaces it.
    if loads(&current) || crate::strays::CRASH_LOGGERS[1..].iter().any(|n| plugins.join(n).is_file()) {
        return Ok(None);
    }
    let Ok(rd) = std::fs::read_dir(game_dir.join(crate::strays::DISABLED_DIR)) else { return Ok(None) };
    let mut stamps: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().is_some_and(|n| !n.to_string_lossy().contains("-wrong-build") && !n.to_string_lossy().contains("-too-new")))
        .collect();
    stamps.sort();
    for stamp in stamps.iter().rev() {
        let from = stamp.join("Data").join("SKSE").join("Plugins").join("CrashLogger.dll");
        if loads(&from) {
            std::fs::create_dir_all(&plugins)?;
            if current.is_file() {
                let why = match crate::skse::build_of(&current) {
                    crate::skse::Build::Wrong(w) => w,
                    _ => "it couldn't be read as an SKSE plugin".into(),
                };
                crate::modlist::set_aside_wrong_build(game_dir, "Data/SKSE/Plugins/CrashLogger.dll", &why)?;
            }
            std::fs::rename(&from, &current)?;
            return Ok(Some(stamp.clone()));
        }
    }
    Ok(None)
}

/// Downloads the pinned Crash Logger release from GitHub, checks it, and puts
/// its files in Data/SKSE/Plugins.
pub async fn install_crash_logger(client: &reqwest::Client, game_dir: &Path) -> Result<()> {
    install_crash_logger_from(client, game_dir, &Sources::official().crash_logger).await
}

async fn install_crash_logger_from(client: &reqwest::Client, game_dir: &Path, src: &Source) -> Result<()> {
    let bytes = download_checked(client, src, "Crash Logger").await?;
    let dir = plugins_dir(game_dir);
    std::fs::create_dir_all(&dir)?;
    unpack_crash_logger(&bytes, &dir)
}

fn unpack_crash_logger(archive: &[u8], dir: &Path) -> Result<()> {
    let mut reader = sevenz_rust2::ArchiveReader::new(std::io::Cursor::new(archive), sevenz_rust2::Password::empty())
        .map_err(|e| Error::Game(format!("Crash Logger download is damaged: {e}")))?;
    let mut staged: Vec<(PathBuf, PathBuf)> = Vec::new();
    // A failure writing to the game folder stays an IO error (disk full,
    // folder locked); a failure reading the archive is a damaged download.
    let mut write_err: Option<std::io::Error> = None;
    let unpacked = reader.for_each_entries(|entry, data| {
        let name = entry.name().replace('\\', "/");
        let Some(file) = name.strip_prefix("SKSE/Plugins/") else { return Ok(true) };
        if entry.is_directory() || file.contains('/') || !CRASH_LOGGER_FILES.iter().any(|f| f.eq_ignore_ascii_case(file)) {
            return Ok(true);
        }
        let mut bytes = Vec::new();
        data.read_to_end(&mut bytes)?;
        let tmp = dir.join(format!("{file}.part"));
        staged.push((tmp.clone(), dir.join(file)));
        if let Err(e) = std::fs::write(&tmp, &bytes) {
            write_err = Some(e);
            return Ok(false);
        }
        Ok(true)
    });
    if let Some(e) = write_err {
        discard(&staged);
        return Err(e.into());
    }
    if let Err(e) = unpacked {
        discard(&staged);
        return Err(Error::Game(format!("couldn't unpack Crash Logger: {e}")));
    }
    if !staged.iter().any(|(_, d)| is_named(d, "CrashLogger.dll")) {
        discard(&staged);
        return Err(Error::Game("the Crash Logger download didn't contain CrashLogger.dll".into()));
    }
    put_in_place(staged, "CrashLogger.dll")
}

fn is_named(p: &Path, name: &str) -> bool {
    p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))
}

/// Removes the `.part` files of an unpack that didn't finish.
fn discard(staged: &[(PathBuf, PathBuf)]) {
    for (tmp, _) in staged {
        let _ = std::fs::remove_file(tmp);
    }
}

/// Renames every unpacked `.part` file into place with the DLL last, so the
/// DLL (what the installed check reads) is only there when all its files are.
fn put_in_place(mut staged: Vec<(PathBuf, PathBuf)>, dll: &str) -> Result<()> {
    staged.sort_by_key(|(_, d)| is_named(d, dll));
    for (i, (tmp, dest)) in staged.iter().enumerate() {
        if let Err(e) = std::fs::rename(tmp, dest) {
            discard(&staged[i..]);
            return Err(e.into());
        }
    }
    Ok(())
}

pub fn souls_ok(game_dir: &Path) -> bool {
    loads(&plugins_dir(game_dir).join("SkyrimSoulsRE.dll"))
}

/// Downloads Skyrim Souls RE 2.4.0 from its GitHub release, checks it, and
/// puts its files in Data (SKSE plugin, its ini unless one exists, Interface
/// and Scripts). The Scripts/Source folder is skipped.
pub async fn install_souls(client: &reqwest::Client, game_dir: &Path) -> Result<()> {
    install_souls_from(client, game_dir, &Sources::official().souls).await
}

async fn install_souls_from(client: &reqwest::Client, game_dir: &Path, src: &Source) -> Result<()> {
    let bytes = download_checked(client, src, "Skyrim Souls RE").await?;
    unpack_souls(&bytes, &game_dir.join("Data"))
}

fn unpack_souls(archive: &[u8], data: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).map_err(|e| Error::Game(format!("Skyrim Souls RE download is damaged: {e}")))?;
    let mut staged: Vec<(PathBuf, PathBuf)> = Vec::new();
    let unpacked = (|| -> Result<()> {
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| Error::Game(e.to_string()))?;
        let name = f.name().replace('\\', "/");
        let keep = !f.is_dir()
            && !name.split('/').any(|c| c == ".." || c.is_empty())
            && !name.to_ascii_lowercase().starts_with("scripts/source/")
            && ["skse/plugins/", "interface/", "scripts/"].iter().any(|p| name.to_ascii_lowercase().starts_with(p));
        if !keep {
            continue;
        }
        let dest = data.join(&name);
        // Keep the player's own settings.
        if name.to_ascii_lowercase().ends_with(".ini") && dest.is_file() {
            continue;
        }
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p)?;
        }
        // `<file>.part`, the name clean_partials looks for (the dll, ini
        // and pdb share a stem, so the extension can't simply be swapped).
        let tmp = dest.with_file_name(format!("{}.part", dest.file_name().unwrap_or_default().to_string_lossy()));
        staged.push((tmp.clone(), dest));
        // Reading (a damaged archive) and writing (disk full, folder
        // locked) fail differently, so the player is told the right thing.
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut bytes).map_err(|e| Error::Game(format!("couldn't unpack {name}: {e}")))?;
        std::fs::write(&tmp, &bytes)?;
    }
    Ok(())
    })();
    if let Err(e) = unpacked {
        discard(&staged);
        return Err(e);
    }
    if !staged.iter().any(|(_, d)| is_named(d, "SkyrimSoulsRE.dll") && d.parent().is_some_and(|p| is_named(p, "Plugins"))) {
        discard(&staged);
        return Err(Error::Game("the Skyrim Souls RE download didn't contain SkyrimSoulsRE.dll".into()));
    }
    put_in_place(staged, "SkyrimSoulsRE.dll")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpacks_souls_when_available() {
        let Ok(path) = std::env::var("AD_SOULS_ZIP") else { return };
        let bytes = std::fs::read(path).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        unpack_souls(&bytes, &tmp.path().join("Data")).unwrap();
        assert!(souls_ok(tmp.path()));
        assert!(tmp.path().join("Data/Interface/CombatAlertOverlayMenu.swf").is_file());
        assert!(tmp.path().join("Data/Scripts/uimenubase.pex").is_file());
        assert!(!tmp.path().join("Data/Scripts/Source").exists());
    }

    #[test]
    fn unpacks_engine_fixes_when_available() {
        let Ok(p) = std::env::var("AD_EF_7Z") else { return };
        let tmp = tempfile::tempdir().unwrap();
        let dir = plugins_dir(tmp.path());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("EngineFixes.toml"), b"mine").unwrap();
        unpack_engine_fixes(&std::fs::read(p).unwrap(), &dir).unwrap();
        assert!(engine_fixes_ok(tmp.path()));
        assert!(dir.join("EngineFixes_preload.txt").is_file());
        assert_eq!(std::fs::read(dir.join("EngineFixes.toml")).unwrap(), b"mine");
        assert!(!tmp.path().join("SE").exists());
    }

    #[test]
    fn lists_missing_nexus_mods() {
        let tmp = tempfile::tempdir().unwrap();
        let ids: Vec<&str> = missing_nexus_mods(tmp.path(), Some("1.6.1170.0")).iter().map(|m| m.id).collect();
        assert_eq!(ids, ["address-library", "engine-fixes", "ussep", "menu-framework", "imgui-icons", "skyui"]);
        assert!(missing_nexus_mods(tmp.path(), Some("1.6.1170.0")).iter().all(|m| m.page.starts_with("https://www.nexusmods.com/")));
        let d = tmp.path();
        std::fs::create_dir_all(plugins_dir(d)).unwrap();
        std::fs::create_dir_all(d.join("Data/Interface/ImGuiIcons")).unwrap();
        for f in ["versionlib-1-6-1170-0.bin", MENU_FRAMEWORK_DLL, "EngineFixes.dll"] {
            std::fs::write(plugins_dir(d).join(f), b"x").unwrap();
        }
        std::fs::write(plugins_dir(d).join("EngineFixes_preload.txt"), b"x").unwrap();
        std::fs::write(d.join("Data").join(USSEP_PLUGIN), b"x").unwrap();
        // The 59-byte stub from the first live test isn't SkyUI.
        std::fs::write(d.join("Data").join(SKYUI_ARCHIVE), b"x").unwrap();
        std::fs::write(d.join("Data").join(SKYUI_PLUGIN), crate::loadorder::test_plugin(0.0, false)).unwrap();
        assert_eq!(missing_nexus_mods(d, Some("1.6.1170.0")).iter().map(|m| m.id).collect::<Vec<_>>(), ["skyui"]);
        std::fs::write(d.join("Data").join(SKYUI_PLUGIN), crate::loadorder::test_plugin(1.7, true)).unwrap();
        assert!(missing_nexus_mods(d, Some("1.6.1170.0")).is_empty());
    }

    #[test]
    fn cleans_only_its_own_partials() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = plugins_dir(tmp.path());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("EngineFixes.toml.part"), b"x").unwrap();
        std::fs::write(dir.join("Other.dll.part"), b"x").unwrap();
        assert_eq!(clean_partials(tmp.path()), ["EngineFixes.toml.part"]);
        assert!(dir.join("Other.dll.part").is_file());
    }

    #[test]
    fn names_the_address_library_file() {
        assert_eq!(address_library_file("1.6.1170.0"), "versionlib-1-6-1170-0.bin");
        let tmp = tempfile::tempdir().unwrap();
        assert!(!address_library_ok(tmp.path(), "1.6.1170.0"));
        std::fs::create_dir_all(plugins_dir(tmp.path())).unwrap();
        std::fs::write(plugins_dir(tmp.path()).join("versionlib-1-6-1170-0.bin"), b"x").unwrap();
        assert!(address_library_ok(tmp.path(), "1.6.1170.0"));
    }

    #[test]
    fn unpacks_skse() {
        let Ok(p) = std::env::var("AD_SKSE_7Z") else { return };
        let tmp = tempfile::tempdir().unwrap();
        unpack_skse(&std::fs::read(p).unwrap(), tmp.path()).unwrap();
        assert!(skse_ok(tmp.path()));
        assert!(tmp.path().join("Data/Scripts/actor.pex").is_file());
        assert!(!tmp.path().join("skse64_readme.txt").exists());
    }

    /// Runs against the real release when it's been downloaded to
    /// $AD_CRASHLOGGER_7Z (the network isn't used in tests).
    #[test]
    fn unpacks_the_release() {
        let Ok(p) = std::env::var("AD_CRASHLOGGER_7Z") else { return };
        let tmp = tempfile::tempdir().unwrap();
        unpack_crash_logger(&std::fs::read(p).unwrap(), tmp.path()).unwrap();
        for f in CRASH_LOGGER_FILES {
            assert!(tmp.path().join(f).is_file(), "{f}");
        }
    }

    /// Serves `routes` (path, status, body) over plain HTTP and counts requests per path.
    async fn serve(routes: Vec<(&'static str, u16, Vec<u8>)>) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let hits = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let h = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut c, _)) = l.accept().await else { return };
                let mut buf = vec![0u8; 4096];
                let n = c.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("").to_string();
                h.lock().unwrap().push(path.clone());
                let (status, body) = routes.iter().find(|r| r.0 == path).map(|r| (r.1, r.2.clone())).unwrap_or((404, Vec::new()));
                let head = format!("HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
                let _ = c.write_all(head.as_bytes()).await;
                let _ = c.write_all(&body).await;
            }
        });
        (base, hits)
    }

    fn sha(b: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(b))
    }

    /// A Souls-like zip, stored (not compressed) so a test can damage one file.
    fn souls_zip(dll: &[u8]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        w.start_file("SKSE/Plugins/SkyrimSoulsRE.dll", o).unwrap();
        std::io::Write::write_all(&mut w, dll).unwrap();
        w.start_file("Interface/CombatAlertOverlayMenu.swf", o).unwrap();
        std::io::Write::write_all(&mut w, b"SWF-MENU-CONTENT").unwrap();
        w.finish().unwrap().into_inner()
    }

    /// A game folder with SKSE and a Crash Logger SKSE would load.
    fn game_with_skse() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(plugins_dir(t.path())).unwrap();
        std::fs::write(t.path().join("skse64_loader.exe"), b"x").unwrap();
        std::fs::write(t.path().join("skse64_1_6_1170.dll"), b"x").unwrap();
        std::fs::write(plugins_dir(t.path()).join("CrashLogger.dll"), crate::skse::tests::good_dll()).unwrap();
        t
    }

    fn sources(base: &str, cl: &[u8], souls: &[u8]) -> Sources {
        let s = |p: &str, b: &[u8]| Source { url: format!("{base}{p}"), sha256: sha(b) };
        Sources { skse: s("/skse", b""), crash_logger: s("/cl", cl), souls: s("/souls", souls) }
    }

    #[tokio::test]
    async fn a_helper_that_cant_be_installed_stops_play_with_its_own_reason() {
        let http = reqwest::Client::new();
        let mut log = |_: &str| {};
        let good = souls_zip(&crate::skse::tests::good_dll());
        let dll = |t: &tempfile::TempDir| plugins_dir(t.path()).join("SkyrimSoulsRE.dll");

        // Download fails: Play stops and names the mod and the connection.
        let t = game_with_skse();
        let (base, _) = serve(vec![("/souls", 500, Vec::new())]).await;
        let e = ensure_helpers(&http, t.path(), &sources(&base, b"", &good), &mut log).await.unwrap().remove(0);
        assert_eq!(e.name, "Skyrim Souls RE");
        assert!(e.message().contains("internet"), "{}", e.message());
        // Play warns and carries on without it.
        assert!(e.warning().contains("Skyrim starts without it"), "{}", e.warning());

        // A damaged download: not blamed on the internet, nothing written.
        let (base, _) = serve(vec![("/souls", 200, b"not the zip".to_vec())]).await;
        let e = ensure_helpers(&http, t.path(), &sources(&base, b"", &good), &mut log).await.unwrap().remove(0);
        assert!(e.message().contains("damaged") && !e.message().contains("internet"), "{}", e.message());
        assert!(!dll(&t).exists());

        // Cut short part way through unpacking (the second file is damaged):
        // the DLL isn't left behind, so the next Play installs again.
        let mut broken = good.clone();
        let at = broken.windows(16).position(|w| w == b"SWF-MENU-CONTENT").unwrap();
        broken[at] ^= 0xff;
        let (base, _) = serve(vec![("/souls", 200, broken.clone())]).await;
        let e = ensure_helpers(&http, t.path(), &sources(&base, b"", &broken), &mut log).await.unwrap().remove(0);
        assert!(e.message().contains("Couldn't install Skyrim Souls RE"), "{}", e.message());
        assert!(!dll(&t).exists());
        assert!(std::fs::read_dir(plugins_dir(t.path())).unwrap().flatten().all(|f| !f.file_name().to_string_lossy().ends_with(".part")));

        // A copy SKSE wouldn't load after installing still stops Play.
        let old = souls_zip(&crate::skse::tests::old_dll());
        let (base, _) = serve(vec![("/souls", 200, old.clone())]).await;
        let e = ensure_helpers(&http, t.path(), &sources(&base, b"", &old), &mut log).await.unwrap().remove(0);
        assert!(e.message().contains("wouldn't load"), "{}", e.message());

        // That wrong copy is replaced by the right one on the next try.
        let (base, hits) = serve(vec![("/souls", 200, good.clone())]).await;
        assert!(ensure_helpers(&http, t.path(), &sources(&base, b"", &good), &mut log).await.unwrap().is_empty());
        assert!(souls_ok(t.path()));
        assert!(t.path().join("Data/Interface/CombatAlertOverlayMenu.swf").is_file());
        assert_eq!(hits.lock().unwrap().as_slice(), ["/souls"]);
        // With everything in place, nothing is downloaded.
        let (base, hits) = serve(vec![]).await;
        assert!(ensure_helpers(&http, t.path(), &sources(&base, b"", &good), &mut log).await.unwrap().is_empty());
        assert!(hits.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_wrong_build_crash_logger_is_not_put_back_or_counted() {
        let t = game_with_skse();
        let g = t.path();
        let set_aside = |stamp: &str, b: &[u8]| {
            let d = g.join(crate::strays::DISABLED_DIR).join(stamp).join("Data/SKSE/Plugins");
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("CrashLogger.dll"), b).unwrap();
            d.join("CrashLogger.dll")
        };
        std::fs::remove_file(plugins_dir(g).join("CrashLogger.dll")).unwrap();
        let good = set_aside("100-plugins", &crate::skse::tests::good_dll());
        let cut = set_aside("200-plugins", &crate::skse::tests::good_dll()[..300]);
        let wrong = set_aside("300-wrong-build", &crate::skse::tests::old_dll());
        let too_new = set_aside("400-too-new", &crate::skse::tests::good_dll());
        // The newest copy SKSE would load, from a folder that isn't a refusal.
        assert_eq!(restore_crash_logger(g).unwrap(), Some(g.join(crate::strays::DISABLED_DIR).join("100-plugins")));
        assert!(!good.exists() && cut.exists() && wrong.exists() && too_new.exists());
        assert!(crash_logger_ok(g));
        // A wrong-build or cut-short copy in the game isn't counted as installed,
        // so Play downloads it again (and stops when it can't).
        for bad in [crate::skse::tests::old_dll(), crate::skse::tests::good_dll()[..300].to_vec()] {
            std::fs::write(plugins_dir(g).join("CrashLogger.dll"), &bad).unwrap();
            assert!(!crash_logger_ok(g));
            let (base, hits) = serve(vec![("/cl", 503, Vec::new())]).await;
            let e = ensure_helpers(&reqwest::Client::new(), g, &sources(&base, b"cl", b""), &mut |_| {}).await.unwrap().remove(0);
            assert_eq!(e.name, "Crash Logger");
            assert_eq!(hits.lock().unwrap().first().map(String::as_str), Some("/cl"));
            // Nothing is put back over a copy that's there, even a wrong one.
            assert_eq!(restore_crash_logger(g).unwrap(), None);
        }
    }

    #[tokio::test]
    async fn a_good_set_aside_crash_logger_is_used_before_downloading() {
        let t = game_with_skse();
        let g = t.path();
        let d = g.join(crate::strays::DISABLED_DIR).join("100-plugins/Data/SKSE/Plugins");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("CrashLogger.dll"), crate::skse::tests::good_dll()).unwrap();
        // The copy in the game is the wrong build, and GitHub is unreachable.
        std::fs::write(plugins_dir(g).join("CrashLogger.dll"), crate::skse::tests::old_dll()).unwrap();
        let (base, hits) = serve(vec![("/cl", 503, Vec::new())]).await;
        ensure_helpers(&reqwest::Client::new(), g, &sources(&base, b"cl", b""), &mut |_| {}).await.unwrap().remove(0);
        // Souls RE is missing in this fixture, so Play still stops, but on
        // Souls RE: Crash Logger came back from the backup with no download.
        assert!(crash_logger_ok(g));
        assert!(!hits.lock().unwrap().iter().any(|p| p == "/cl"));
        // The wrong copy was set aside as a wrong build, not deleted.
        let aside: Vec<_> = std::fs::read_dir(g.join(crate::strays::DISABLED_DIR)).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert!(aside.iter().any(|n| n.ends_with("-wrong-build")), "{aside:?}");
    }

    #[tokio::test]
    async fn a_write_failure_is_told_as_one() {
        let t = game_with_skse();
        let good = souls_zip(&crate::skse::tests::good_dll());
        // Something the launcher can't write over where the file goes.
        std::fs::create_dir_all(t.path().join("Data/Interface/CombatAlertOverlayMenu.swf.part/x")).unwrap();
        let (base, _) = serve(vec![("/souls", 200, good.clone())]).await;
        let e = ensure_helpers(&reqwest::Client::new(), t.path(), &sources(&base, b"", &good), &mut |_| {}).await.unwrap().remove(0);
        assert!(matches!(e.cause, Error::Io(_)), "{e:?}");
        assert!(e.message().contains("Couldn't write Skyrim Souls RE"), "{}", e.message());
        assert!(!plugins_dir(t.path()).join("SkyrimSoulsRE.dll").exists());
    }

    /// The real pinned archives (AD_CRASH_LOGGER_7Z, AD_SOULS_ZIP), served
    /// locally: both install, and SKSE 2.2.6 would load both DLLs.
    #[tokio::test]
    async fn real_helpers_install_and_load_when_available() {
        let (Ok(cl), Ok(souls)) = (std::env::var("AD_CRASH_LOGGER_7Z"), std::env::var("AD_SOULS_ZIP")) else { return };
        let (cl, souls) = (std::fs::read(cl).unwrap(), std::fs::read(souls).unwrap());
        let t = game_with_skse();
        std::fs::remove_file(plugins_dir(t.path()).join("CrashLogger.dll")).unwrap();
        let (base, _) = serve(vec![("/cl", 200, cl.clone()), ("/souls", 200, souls.clone())]).await;
        let src = sources(&base, &cl, &souls);
        assert_eq!(src.crash_logger.sha256, CRASH_LOGGER_SHA256);
        assert_eq!(src.souls.sha256, SOULS_SHA256);
        assert!(ensure_helpers(&reqwest::Client::new(), t.path(), &src, &mut |_| {}).await.unwrap().is_empty());
        assert!(crash_logger_ok(t.path()) && souls_ok(t.path()));
    }

    #[tokio::test]
    async fn only_skse_stops_play() {
        let t = tempfile::tempdir().unwrap();
        let (base, _) = serve(vec![]).await;
        let e = ensure_helpers(&reqwest::Client::new(), t.path(), &sources(&base, b"", b""), &mut |_| {}).await.unwrap_err();
        assert_eq!(e.name, "SKSE");
    }
}
