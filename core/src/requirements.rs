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
/// The Unofficial Skyrim Special Edition Patch's plugin. No longer required
/// (2026-09-26): the server can't load it, and it crashed the game drawing
/// land it changes, so it's switched off before Play.
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
pub const NEXUS_PAGES: [(&str, &str); 5] = [
    ("address-library", "https://www.nexusmods.com/skyrimspecialedition/mods/32444?tab=files"),
    ("engine-fixes", "https://www.nexusmods.com/skyrimspecialedition/mods/17230?tab=files"),
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
    let bytes = client.get(SKSE_URL).header("User-Agent", "AetherialDawnLauncher").send().await?.error_for_status()?.bytes().await?;
    use sha2::{Digest, Sha256};
    let got = hex::encode(Sha256::digest(&bytes));
    if !got.eq_ignore_ascii_case(SKSE_SHA256) {
        return Err(Error::HashMismatch { path: "SKSE".into(), expected: SKSE_SHA256.into(), actual: got });
    }
    unpack_skse(&bytes, game_dir)
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

pub fn crash_logger_ok(game_dir: &Path) -> bool {
    plugins_dir(game_dir).join("CrashLogger.dll").is_file()
}

/// Downloads the pinned Crash Logger release from GitHub, checks it, and puts
/// its files in Data/SKSE/Plugins.
pub async fn install_crash_logger(client: &reqwest::Client, game_dir: &Path) -> Result<()> {
    let bytes = client.get(CRASH_LOGGER_URL).header("User-Agent", "AetherialDawnLauncher").send().await?.error_for_status()?.bytes().await?;
    use sha2::{Digest, Sha256};
    let got = hex::encode(Sha256::digest(&bytes));
    if !got.eq_ignore_ascii_case(CRASH_LOGGER_SHA256) {
        return Err(Error::HashMismatch { path: "Crash Logger".into(), expected: CRASH_LOGGER_SHA256.into(), actual: got });
    }
    let dir = plugins_dir(game_dir);
    std::fs::create_dir_all(&dir)?;
    unpack_crash_logger(&bytes, &dir)
}

fn unpack_crash_logger(archive: &[u8], dir: &Path) -> Result<()> {
    let mut reader = sevenz_rust2::ArchiveReader::new(std::io::Cursor::new(archive), sevenz_rust2::Password::empty())
        .map_err(|e| Error::Game(format!("Crash Logger download is damaged: {e}")))?;
    let mut wrote = 0;
    reader
        .for_each_entries(|entry, data| {
            let name = entry.name().replace('\\', "/");
            let Some(file) = name.strip_prefix("SKSE/Plugins/") else { return Ok(true) };
            if entry.is_directory() || file.contains('/') || !CRASH_LOGGER_FILES.iter().any(|f| f.eq_ignore_ascii_case(file)) {
                return Ok(true);
            }
            let tmp = dir.join(format!("{file}.part"));
            let mut out = std::fs::File::create(&tmp)?;
            std::io::copy(data, &mut out)?;
            drop(out);
            std::fs::rename(&tmp, dir.join(file))?;
            wrote += 1;
            Ok(true)
        })
        .map_err(|e| Error::Game(format!("couldn't unpack Crash Logger: {e}")))?;
    if wrote == 0 {
        return Err(Error::Game("the Crash Logger download didn't contain CrashLogger.dll".into()));
    }
    Ok(())
}

pub fn souls_ok(game_dir: &Path) -> bool {
    plugins_dir(game_dir).join("SkyrimSoulsRE.dll").is_file()
}

/// Downloads Skyrim Souls RE 2.4.0 from its GitHub release, checks it, and
/// puts its files in Data (SKSE plugin, its ini unless one exists, Interface
/// and Scripts). The Scripts/Source folder is skipped.
pub async fn install_souls(client: &reqwest::Client, game_dir: &Path) -> Result<()> {
    let bytes = client.get(SOULS_URL).header("User-Agent", "AetherialDawnLauncher").send().await?.error_for_status()?.bytes().await?;
    use sha2::{Digest, Sha256};
    let got = hex::encode(Sha256::digest(&bytes));
    if !got.eq_ignore_ascii_case(SOULS_SHA256) {
        return Err(Error::HashMismatch { path: "Skyrim Souls RE".into(), expected: SOULS_SHA256.into(), actual: got });
    }
    unpack_souls(&bytes, &game_dir.join("Data"))
}

fn unpack_souls(archive: &[u8], data: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).map_err(|e| Error::Game(format!("Skyrim Souls RE download is damaged: {e}")))?;
    let mut dll = false;
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
        let tmp = dest.with_extension("part");
        let mut out = std::fs::File::create(&tmp)?;
        std::io::copy(&mut f, &mut out)?;
        drop(out);
        std::fs::rename(&tmp, &dest)?;
        dll |= name.eq_ignore_ascii_case("SKSE/Plugins/SkyrimSoulsRE.dll");
    }
    if !dll {
        return Err(Error::Game("the Skyrim Souls RE download didn't contain SkyrimSoulsRE.dll".into()));
    }
    Ok(())
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
        assert_eq!(ids, ["address-library", "engine-fixes", "menu-framework", "imgui-icons", "skyui"]);
        assert!(missing_nexus_mods(tmp.path(), Some("1.6.1170.0")).iter().all(|m| m.page.starts_with("https://www.nexusmods.com/")));
        let d = tmp.path();
        std::fs::create_dir_all(plugins_dir(d)).unwrap();
        std::fs::create_dir_all(d.join("Data/Interface/ImGuiIcons")).unwrap();
        for f in ["versionlib-1-6-1170-0.bin", MENU_FRAMEWORK_DLL, "EngineFixes.dll"] {
            std::fs::write(plugins_dir(d).join(f), b"x").unwrap();
        }
        std::fs::write(plugins_dir(d).join("EngineFixes_preload.txt"), b"x").unwrap();
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
}
