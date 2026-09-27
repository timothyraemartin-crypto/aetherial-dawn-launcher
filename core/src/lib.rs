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
pub mod pristine;
pub mod requirements;
pub mod settings;
pub mod skse;
pub mod steamapp;
pub mod strays;
pub mod sync;
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
    if has(&["corrupted while downloading", "hash mismatch"]) {
        return "A download came through damaged. The launcher fetches it again on the next try.".into();
    }
    if has(&["timed out", "operation timed out", "deadline has elapsed"]) {
        return "The connection was too slow. The launcher tries again on the next try.".into();
    }
    if has(&["error sending request", "dns error", "connection refused", "connection reset", "network error", "error decoding response", "tcp connect"]) {
        return "The launcher couldn't reach the internet just now. Check your connection; it tries again on the next try.".into();
    }
    if has(&["invalid data:", "expected value at line", "eof while parsing"]) {
        return "The server sent something the launcher couldn't read. It tries again on the next try.".into();
    }
    if let Some(rest) = raw.strip_prefix("file error: ") {
        return format!("A game file couldn't be changed ({rest}). Close Skyrim and Vortex, then try again.");
    }
    raw.to_string()
}

#[cfg(test)]
mod plain_tests {
    #[test]
    fn words_errors_plainly() {
        assert!(super::plain("file error: Access is denied. (os error 5)").starts_with("Windows wouldn't let"));
        assert!(super::plain("network error: error sending request for url (https://x/y)").contains("couldn't reach"));
        assert!(!super::plain("Data/x.dll was corrupted while downloading (expected ab, got cd)").contains("ab"));
        assert_eq!(super::plain("Pick your Skyrim folder first."), "Pick your Skyrim folder first.");
    }
}
