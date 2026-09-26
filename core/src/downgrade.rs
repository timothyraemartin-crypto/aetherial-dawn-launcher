//! Puts the Skyrim build the server needs into the player's game folder, the
//! way the community downgrade guides do: the player's own Steam account
//! downloads the exact depot manifests with DepotDownloader
//! (github.com/SteamRE/DepotDownloader). The player signs in inside
//! DepotDownloader's own window, by QR code in the Steam mobile app or by
//! typing their password there, so the launcher never sees any credential.
//! No Bethesda files ever come from the Aetherial Dawn server.

use std::path::{Path, PathBuf};

use crate::manifest::{GameSpec, Tool};
use crate::{Error, Result};

const RELEASES: &str = "https://api.github.com/repos/SteamRE/DepotDownloader/releases/latest";
#[cfg(windows)]
const TOOL_EXE: &str = "DepotDownloader.exe";
#[cfg(not(windows))]
const TOOL_EXE: &str = "DepotDownloader";

#[derive(Debug, Clone)]
pub enum Login {
    /// Scan a QR code with the Steam mobile app.
    Qr,
    /// Steam account name; the password is typed into DepotDownloader's window.
    User(String),
}

fn find_exe(dir: &Path) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = find_exe(&p) {
                return Some(f);
            }
        } else if p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(TOOL_EXE)) {
            return Some(p);
        }
    }
    None
}

async fn latest_release_url(client: &reqwest::Client) -> Result<String> {
    let v: serde_json::Value = client
        .get(RELEASES)
        .header("User-Agent", "AetherialDawnLauncher")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let assets = v["assets"].as_array().cloned().unwrap_or_default();
    let name = |a: &serde_json::Value| a["name"].as_str().unwrap_or("").to_ascii_lowercase();
    let pick = assets
        .iter()
        .find(|a| { let n = name(a); n.contains("windows") && n.contains("x64") && n.ends_with(".zip") })
        .or_else(|| assets.iter().find(|a| { let n = name(a); n.contains("windows") && n.ends_with(".zip") }))
        .ok_or_else(|| Error::Game("couldn't find the Windows download of DepotDownloader".into()))?;
    Ok(pick["browser_download_url"].as_str().unwrap_or_default().to_string())
}

/// Makes sure DepotDownloader is unpacked in `tools_dir`, downloading it when
/// missing or when the server pins a different build.
pub async fn ensure_tool(client: &reqwest::Client, tools_dir: &Path, pinned: Option<&Tool>) -> Result<PathBuf> {
    let dir = tools_dir.join("DepotDownloader");
    let source_file = dir.join("source.txt");
    let have_source = std::fs::read_to_string(&source_file).unwrap_or_default();
    if let Some(exe) = find_exe(&dir) {
        if pinned.is_none_or(|t| t.url == have_source.trim()) {
            return Ok(exe);
        }
    }
    let url = match pinned {
        Some(t) => t.url.clone(),
        None => latest_release_url(client).await?,
    };
    let bytes = client.get(&url).header("User-Agent", "AetherialDawnLauncher").send().await?.error_for_status()?.bytes().await?;
    if let Some(want) = pinned.and_then(|t| t.sha256.as_deref()) {
        use sha2::{Digest, Sha256};
        let got = hex::encode(Sha256::digest(&bytes));
        if !got.eq_ignore_ascii_case(want) {
            return Err(Error::HashMismatch { path: "DepotDownloader".into(), expected: want.into(), actual: got });
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| Error::Game(format!("DepotDownloader download is damaged: {e}")))?;
    zip.extract(&dir).map_err(|e| Error::Game(format!("couldn't unpack DepotDownloader: {e}")))?;
    std::fs::write(&source_file, &url)?;
    let exe = find_exe(&dir).ok_or_else(|| Error::Game("DepotDownloader download didn't contain the program".into()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(exe)
}

/// Steam account names are letters, digits and underscores.
pub fn valid_username(u: &str) -> bool {
    (2..=64).contains(&u.len()) && u.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

pub fn args(spec: &GameSpec, game_dir: &Path, login: &Login) -> Result<Vec<String>> {
    if spec.depots.is_empty() {
        return Err(Error::Game("the server hasn't listed which Steam files make up its Skyrim version".into()));
    }
    let mut a = vec!["-app".to_string(), spec.app.to_string(), "-depot".into()];
    a.extend(spec.depots.iter().map(|d| d.depot.to_string()));
    a.push("-manifest".into());
    a.extend(spec.depots.iter().map(|d| d.manifest.clone()));
    a.extend(["-os".into(), "windows".into(), "-validate".into(), "-dir".into(), game_dir.to_string_lossy().into_owned()]);
    match login {
        Login::Qr => a.push("-qr".into()),
        Login::User(u) => {
            if !valid_username(u) {
                return Err(Error::Game("that doesn't look like a Steam account name".into()));
            }
            a.extend(["-username".into(), u.clone(), "-remember-password".into()]);
        }
    }
    Ok(a)
}

/// Runs DepotDownloader in its own console window (Windows) and waits for it.
/// The player signs in and watches the download there.
pub async fn run(tool: &Path, args: &[String], work_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(work_dir)?;
    let status = spawn(tool, args, work_dir)?;
    let status = tokio::task::spawn_blocking(move || status.wait_with_output())
        .await
        .map_err(|e| Error::Game(e.to_string()))??
        .status;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Game("the Steam download didn't finish. Nothing was changed that a second try won't fix.".into()))
    }
}

#[cfg(windows)]
fn spawn(tool: &Path, args: &[String], work_dir: &Path) -> Result<std::process::Child> {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
    // A small batch file gives the window a title and plain instructions, and
    // keeps it open if something goes wrong so the player can read why.
    let q = |s: &str| format!("\"{}\"", s.replace('%', "%%").replace('"', ""));
    let line = std::iter::once(q(&tool.to_string_lossy())).chain(args.iter().map(|a| q(a))).collect::<Vec<_>>().join(" ");
    let script = format!(
        "@echo off\r\ntitle Aetherial Dawn - Skyrim version download\r\n\
echo.\r\necho  Aetherial Dawn is downloading the Skyrim version the server needs from Steam.\r\n\
echo  Sign in below. Your password or QR code goes only to Steam; the launcher never sees it.\r\n\
echo  If Steam Guard asks for a code, type it here.\r\necho.\r\n\
{line}\r\nset rc=%errorlevel%\r\n\
if not \"%rc%\"==\"0\" (echo. & echo  The download stopped. Press any key to close this window. & pause >nul)\r\n\
exit /b %rc%\r\n"
    );
    let bat = work_dir.join("downgrade.cmd");
    std::fs::write(&bat, script)?;
    Ok(std::process::Command::new("cmd")
        .arg("/c")
        .arg(&bat)
        .current_dir(work_dir)
        .creation_flags(CREATE_NEW_CONSOLE)
        .spawn()?)
}

#[cfg(not(windows))]
fn spawn(tool: &Path, args: &[String], work_dir: &Path) -> Result<std::process::Child> {
    Ok(std::process::Command::new(tool).args(args).current_dir(work_dir).spawn()?)
}

/// Runs DepotDownloader with no window of its own, for signing in inside the
/// launcher: its output is read through [`Scanner`] and the player's answers
/// (password, Steam Guard code) are written to its input. Nothing typed is
/// ever echoed, logged or kept by the launcher; with `-remember-password`
/// Steam hands DepotDownloader a sign-in token (kept in `work_dir`), so later
/// downloads usually need no password at all.
pub fn spawn_piped(tool: &Path, args: &[String], work_dir: &Path) -> Result<std::process::Child> {
    use std::process::Stdio;
    std::fs::create_dir_all(work_dir)?;
    let mut cmd = std::process::Command::new(tool);
    cmd.args(args).current_dir(work_dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    Ok(cmd.spawn()?)
}

/// What DepotDownloader is asking for or doing, for the launcher's window.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Event {
    /// Wants the Steam password.
    Password,
    /// Wants the code from the Steam mobile app's authenticator.
    GuardApp,
    /// Wants the code Steam emailed to this (partly hidden) address.
    GuardEmail { email: String },
    /// Waiting for the player to approve the sign-in in the Steam mobile app.
    ConfirmPhone,
    SigningIn,
    /// Steam turned the sign-in down (wrong password, too many tries...).
    LoginFailed { message: String },
    Progress { percent: f32 },
    Line { text: String },
}

/// Turns DepotDownloader's output into [`Event`]s. Prompts end without a
/// newline, so the unfinished tail of the output is checked for them too.
#[derive(Default)]
pub struct Scanner {
    tail: String,
}

fn prompt(text: &str) -> Option<Event> {
    let l = text.to_ascii_lowercase();
    if l.contains("enter account password") {
        Some(Event::Password)
    } else if l.contains("2-factor auth code") || l.contains("2 factor auth code") {
        Some(Event::GuardApp)
    } else if l.contains("auth code sent to the email") {
        let email = text
            .rsplit_once("email at ")
            .or_else(|| text.rsplit_once("email address at "))
            .map(|(_, e)| e.trim().trim_end_matches(':').trim().to_string())
            .unwrap_or_default();
        Some(Event::GuardEmail { email })
    } else {
        None
    }
}

fn classify(line: &str) -> Option<Event> {
    let t = line.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(e) = prompt(t) {
        return Some(e);
    }
    let l = t.to_ascii_lowercase();
    if l.contains("steam mobile app to confirm") {
        return Some(Event::ConfirmPhone);
    }
    if l.starts_with("logging '") && l.contains("into steam") {
        return Some(Event::SigningIn);
    }
    if l.contains("failed to authenticate") || l.contains("invalidpassword") || l.contains("unable to login") || l.contains("ratelimitexceeded") {
        return Some(Event::LoginFailed { message: t.to_string() });
    }
    if let Some((num, _)) = t.split_once('%') {
        if let Ok(p) = num.trim().parse::<f32>() {
            return Some(Event::Progress { percent: p });
        }
    }
    Some(Event::Line { text: t.to_string() })
}

impl Scanner {
    pub fn feed(&mut self, text: &str) -> Vec<Event> {
        self.tail.push_str(&text.replace('\r', "\n"));
        let mut out = Vec::new();
        while let Some(i) = self.tail.find('\n') {
            let line: String = self.tail.drain(..=i).collect();
            out.extend(classify(&line));
        }
        if self.tail.trim_end().ends_with(':') {
            if let Some(e) = prompt(&self.tail) {
                out.push(e);
                self.tail.clear();
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::Depot;

    #[test]
    fn builds_depotdownloader_arguments() {
        let spec = GameSpec {
            version: Some("1.6.1170.0".into()),
            app: 489830,
            depots: vec![Depot { depot: 489831, manifest: "1".into() }, Depot { depot: 489833, manifest: "3".into() }],
            ..Default::default()
        };
        let a = args(&spec, Path::new("/g"), &Login::Qr).unwrap().join(" ");
        assert_eq!(a, "-app 489830 -depot 489831 489833 -manifest 1 3 -os windows -validate -dir /g -qr");
        let a = args(&spec, Path::new("/g"), &Login::User("dovah_kiin".into())).unwrap().join(" ");
        assert!(a.ends_with("-username dovah_kiin -remember-password"));
        assert!(args(&spec, Path::new("/g"), &Login::User("x & del".into())).is_err());
    }

    #[test]
    fn reads_depotdownloader_prompts_and_progress() {
        let mut s = Scanner::default();
        assert_eq!(s.feed("Logging 'dovah' into Steam3...\r\nEnter account pass"), vec![Event::SigningIn]);
        assert_eq!(s.feed("word for \"dovah\": "), vec![Event::Password]);
        assert_eq!(s.feed("STEAM GUARD! Please enter your 2-factor auth code from your authenticator app: "), vec![Event::GuardApp]);
        assert_eq!(
            s.feed("STEAM GUARD! Please enter the auth code sent to the email at d***@mail.com: "),
            vec![Event::GuardEmail { email: "d***@mail.com".into() }]
        );
        assert_eq!(s.feed("STEAM GUARD! Use the Steam Mobile App to confirm your sign in...\n"), vec![Event::ConfirmPhone]);
        assert_eq!(s.feed(" 12.50% A:\\Skyrim\\Data\\Skyrim.esm\n"), vec![Event::Progress { percent: 12.5 }]);
        assert_eq!(
            s.feed("Failed to authenticate with Steam: InvalidPassword\n"),
            vec![Event::LoginFailed { message: "Failed to authenticate with Steam: InvalidPassword".into() }]
        );
        assert_eq!(s.feed("Depot 489831 - Downloaded 10 bytes\n"), vec![Event::Line { text: "Depot 489831 - Downloaded 10 bytes".into() }]);
    }
}
