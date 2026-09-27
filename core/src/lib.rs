//! Platform-independent launcher logic: reading the server manifest, syncing
//! client files into the Skyrim folder, writing SkyMP client settings, and
//! finding the game. The Tauri app in `src-tauri` is a thin shell over this.

pub mod aliases;
pub mod allowlist;
pub mod auth;
pub mod bsa;
pub mod camera;
pub mod community;
pub mod downgrade;
pub mod game;
pub mod gameini;
pub mod health;
pub mod loadorder;
pub mod manifest;
pub mod modlist;
pub mod nexus;
pub mod patcher;
pub mod presets;
pub mod pristine;
pub mod requirements;
pub mod serverlane;
pub mod serverorder;
pub mod settings;
pub mod skse;
pub mod steamapp;
pub mod strays;
pub mod sync;
pub mod tools;
pub mod ussep;
pub mod version;
pub mod watch;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid data: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the server's file list has an unsafe path: {0}")]
    UnsafePath(String),
    #[error("the server's file list uses format {0}, which this launcher doesn't understand. Update the launcher.")]
    UnsupportedSchema(u32),
    #[error("{path} was corrupted while downloading (expected {expected}, got {actual})")]
    HashMismatch { path: String, expected: String, actual: String },
    #[error("The server hasn't published its game files yet.")]
    NotPublished,
    #[error("{0}")]
    Game(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// One plain sentence for a player in place of a technical error; the raw
/// text stays in the launcher's log. Text that is already plain comes back
/// as it is.
pub fn plain(raw: &str) -> String {
    let l = raw.to_ascii_lowercase();
    let has = |k: &[&str]| k.iter().any(|k| l.contains(k));
    if has(&["os error 5)", "access is denied", "permission denied"]) {
        return "Windows wouldn't let the launcher change a game file. Close Skyrim and Vortex, then try again.".into();
    }
    if has(&["os error 32)", "being used by another process"]) {
        return "A game file is in use by another program. Close Skyrim and Vortex, then try again.".into();
    }
    if has(&["os error 112)", "not enough space", "no space left"]) {
        return "The drive with Skyrim is full. Free some space, then try again.".into();
    }
    if has(&["os error 740)", "requires elevation"]) {
        return "Windows wouldn't start it without administrator rights. Make sure SkyrimSE.exe and skse64_loader.exe aren't set to run as administrator, then try again.".into();
    }
    if has(&["os error 2)", "os error 3)", "cannot find the path", "cannot find the file", "no such file or directory"]) {
        return "A file or folder the launcher needs is missing. Check your Skyrim folder in Settings, then try again.".into();
    }
    if has(&["corrupted while downloading", "hash mismatch", "invalid zip", "could not find eocd", "isn't a readable zip", "couldn't unpack", "download is damaged"]) {
        return "A download came through damaged. The launcher fetches it again the next time you press Play.".into();
    }
    if has(&["fomod"]) {
        return "This mod's installer couldn't be read. The launcher tries again the next time you press Play; if it repeats, send Copy diagnostics to staff.".into();
    }
    // Before the general network case: these carry signed download addresses.
    if has(&["http status client error", "http status server error"]) {
        return "The server refused the download just now. The launcher tries again the next time you press Play.".into();
    }
    if has(&["timed out", "deadline has elapsed"]) {
        return "The connection was too slow. The launcher tries again on the next try.".into();
    }
    if has(&["websocket", "nexus sign-in:"]) {
        return "Couldn't reach Nexus to sign in. Check your internet and try again.".into();
    }
    if has(&["error sending request", "dns error", "connection refused", "connection reset", "network error", "error decoding response", "tcp connect", "couldn't reach"]) {
        return "The launcher couldn't reach the internet just now. Check your connection; it tries again on the next try.".into();
    }
    if has(&["invalid data:", "expected value at line", "eof while parsing", "missing field", "sent something unexpected"]) {
        return "The server sent something the launcher couldn't read. It tries again on the next try.".into();
    }
    if has(&["failed to open url", "failed to open path", "program not found"]) {
        return "Couldn't open that in Windows. Set a default web browser in Windows Settings, then try again.".into();
    }
    if has(&["panicked"]) {
        return "That stopped unexpectedly. Try again; if it repeats, send Copy diagnostics to staff.".into();
    }
    if raw.starts_with("file error: ") {
        return "A game file couldn't be changed. Close Skyrim and Vortex, then try again.".into();
    }
    scrub(raw)
}

/// Removes web addresses and "(os error N)" leftovers from otherwise plain
/// text, so signed download links never reach a screenshot.
fn scrub(raw: &str) -> String {
    let mut out = String::new();
    for w in raw.split(' ') {
        let t = w.trim_start_matches('(');
        // Links are hidden, except Discord invites staff send on purpose
        // (the temp-ban appeal invite).
        let invite = ["https://discord.gg/", "http://discord.gg/", "discord.gg/"].iter().any(|p| t.starts_with(p));
        if (t.starts_with("http://") || t.starts_with("https://")) && !invite {
            out.push_str("(link hidden)");
        } else {
            out.push_str(w);
        }
        out.push(' ');
    }
    out.pop();
    out
}

/// `plain` for a command's error as the UI gets it: coded answers
/// ("NEEDS_NEXUS_MODS:[...]") pass through; for "CODE:text" only the text is
/// reworded.
pub fn plain_ui(raw: &str) -> String {
    if raw.starts_with("NEEDS_NEXUS_MODS:") || raw == "NEEDS_NEXUS_SIGN_IN" {
        return raw.to_string();
    }
    if let Some((code, rest)) = raw.split_once(':') {
        if !code.is_empty() && code.bytes().all(|b| b.is_ascii_uppercase() || b == b'_') {
            return format!("{code}:{}", plain(rest));
        }
    }
    plain(raw)
}

#[cfg(test)]
mod plain_tests {
    #[test]
    fn words_errors_plainly() {
        assert!(super::plain("file error: Access is denied. (os error 5)").starts_with("Windows wouldn't let"));
        assert!(super::plain("network error: error sending request for url (https://x/y)").contains("couldn't reach"));
        assert!(!super::plain("Data/x.dll was corrupted while downloading (expected ab, got cd)").contains("ab"));
        assert_eq!(super::plain("Pick your Skyrim folder first."), "Pick your Skyrim folder first.");
        let cdn = super::plain("HTTP status client error (403 Forbidden) for url (https://cf-files.nexusmods.com/x?md5=a&expires=1)");
        assert!(cdn.contains("refused") && !cdn.contains("nexus-cdn") && !cdn.contains("md5"));
        assert!(super::plain("file error: The system cannot find the path specified. (os error 3)").contains("missing"));
        assert!(super::plain("The requested operation requires elevation. (os error 740)").contains("administrator"));
        assert!(!super::plain("file error: something odd").contains("odd"));
        assert_eq!(super::plain("see https://x.y/z now"), "see (link hidden) now");
        assert_eq!(super::plain("appeal at https://discord.gg/abc now"), "appeal at https://discord.gg/abc now");
        assert_eq!(super::plain_ui("NO_PATCH:network error: error sending request"), "NO_PATCH:The launcher couldn't reach the internet just now. Check your connection; it tries again on the next try.");
        assert_eq!(super::plain_ui("NEEDS_NEXUS_MODS:[{\"a\":1}]"), "NEEDS_NEXUS_MODS:[{\"a\":1}]");
        assert!(super::plain_ui("SIGNED_OUT:Please sign in again.").starts_with("SIGNED_OUT:Please"));
    }
}
