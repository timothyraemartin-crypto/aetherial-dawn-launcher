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
pub const ADDRESS_LIBRARY_PAGE: &str = "https://www.nexusmods.com/skyrimspecialedition/mods/32444?tab=files";

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_address_library_file() {
        assert_eq!(address_library_file("1.6.1170.0"), "versionlib-1-6-1170-0.bin");
        let tmp = tempfile::tempdir().unwrap();
        assert!(!address_library_ok(tmp.path(), "1.6.1170.0"));
        std::fs::create_dir_all(plugins_dir(tmp.path())).unwrap();
        std::fs::write(plugins_dir(tmp.path()).join("versionlib-1-6-1170-0.bin"), b"x").unwrap();
        assert!(address_library_ok(tmp.path(), "1.6.1170.0"));
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
