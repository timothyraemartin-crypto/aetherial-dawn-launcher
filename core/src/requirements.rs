//! SKSE64 2.2.6, which SkyMP needs to start, and the Address Library file for
//! the server's Skyrim build that SkyrimPlatform looks for. SKSE is free on
//! GitHub, so the launcher installs it itself from the pinned release. No
//! other mod is installed by the launcher.

use std::path::{Path, PathBuf};

use crate::{Error, Result};

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

/// Where SKSE is downloaded from and the SHA-256 it must have.
#[derive(Debug, Clone)]
pub struct Source {
    pub url: String,
    pub sha256: String,
}

/// Where Play gets SKSE (tests use a local server).
#[derive(Debug, Clone)]
pub struct Sources {
    pub skse: Source,
}

impl Sources {
    /// The pinned official GitHub release.
    pub fn official() -> Sources {
        Sources { skse: Source { url: SKSE_URL.into(), sha256: SKSE_SHA256.into() } }
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

/// SKSE, which Play couldn't install, and why.
#[derive(Debug)]
pub struct HelperFailed {
    pub name: &'static str,
    pub version: &'static str,
    pub cause: Error,
}

impl HelperFailed {
    /// The sentence Play shows, worded from what actually went wrong.
    pub fn message(&self) -> String {
        let (n, v) = (self.name, self.version);
        match &self.cause {
            Error::Http(e) => format!("Couldn't download {n} {v} ({}). Check your internet connection and try again.", crate::scrub(&e.to_string())),
            Error::HashMismatch { .. } => format!("The {n} {v} download was damaged, so it wasn't installed. Try again."),
            Error::Io(e) => format!("Couldn't write {n} {v} into your Skyrim folder ({e}). Close Skyrim, then try again."),
            e => format!("Couldn't install {n} {v}: {e}."),
        }
    }
}

/// What Play does before anything else: installs SKSE when it is missing or
/// the copy there isn't the server's build, and checks again afterwards.
/// SKSE is needed to join at all, so its failure is the Err that stops Play.
/// `log` gets a line per step.
pub async fn ensure_skse(client: &reqwest::Client, game_dir: &Path, src: &Sources, log: &mut (dyn FnMut(&str) + Send)) -> std::result::Result<(), HelperFailed> {
    let fail = |cause| HelperFailed { name: "SKSE", version: SKSE_VERSION, cause };
    if !skse_ok(game_dir) {
        install_skse_from(client, game_dir, &src.skse).await.map_err(fail)?;
        if !skse_ok(game_dir) {
            return Err(fail(Error::Game("its files weren't there after installing".into())));
        }
        log(&format!("installed SKSE {SKSE_VERSION}"));
    }
    Ok(())
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

    #[test]
    fn unpacks_skse() {
        let Ok(p) = std::env::var("AD_SKSE_7Z") else { return };
        let tmp = tempfile::tempdir().unwrap();
        unpack_skse(&std::fs::read(p).unwrap(), tmp.path()).unwrap();
        assert!(skse_ok(tmp.path()));
        assert!(tmp.path().join("Data/Scripts/actor.pex").is_file());
        assert!(!tmp.path().join("skse64_readme.txt").exists());
    }

    /// A bad download stops Play with SKSE's own reason; nothing is written.
    #[tokio::test]
    async fn skse_that_cant_be_installed_stops_play() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let Ok((mut c, _)) = l.accept().await else { return };
                let mut buf = vec![0u8; 4096];
                let _ = c.read(&mut buf).await;
                let _ = c.write_all(b"HTTP/1.1 200 X\r\ncontent-length: 3\r\nconnection: close\r\n\r\nabc").await;
            }
        });
        let t = tempfile::tempdir().unwrap();
        let src = Sources { skse: Source { url: format!("{base}/skse"), sha256: "00".repeat(32) } };
        let e = ensure_skse(&reqwest::Client::new(), t.path(), &src, &mut |_| {}).await.unwrap_err();
        assert_eq!(e.name, "SKSE");
        assert!(matches!(e.cause, Error::HashMismatch { .. }));
        assert!(e.message().contains("damaged"));
        assert!(!skse_ok(t.path()));
    }
}
