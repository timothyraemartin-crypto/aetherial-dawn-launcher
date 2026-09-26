//! Platform-independent launcher logic: reading the server manifest, syncing
//! client files into the Skyrim folder, writing SkyMP client settings, and
//! finding the game. The Tauri app in `src-tauri` is a thin shell over this.

pub mod auth;
pub mod downgrade;
pub mod game;
pub mod gameini;
pub mod health;
pub mod loadorder;
pub mod manifest;
pub mod pristine;
pub mod settings;
pub mod steamapp;
pub mod strays;
pub mod sync;
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
