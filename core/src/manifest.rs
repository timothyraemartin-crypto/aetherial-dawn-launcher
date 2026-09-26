//! The client file list the server publishes at `<base>/client/manifest.json`.
//! Format: skymp-setup/launcher/launcher-spec.md in the project files.

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

use crate::{Error, Result};

pub const SUPPORTED_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub schema: u32,
    pub build: String,
    pub server: Server,
    #[serde(default)]
    pub master: String,
    #[serde(default)]
    pub files: Vec<FileEntry>,
    #[serde(default)]
    pub remove: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Server {
    pub name: String,
    pub ip: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let m: Manifest = serde_json::from_slice(bytes)?;
        if m.schema != SUPPORTED_SCHEMA {
            return Err(Error::UnsupportedSchema(m.schema));
        }
        for f in &m.files {
            safe_relative(&f.path)?;
            if f.sha256.len() != 64 || !f.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(Error::UnsafePath(format!("{} (bad hash)", f.path)));
            }
        }
        for r in &m.remove {
            safe_relative(r)?;
        }
        Ok(m)
    }

    pub async fn fetch(client: &reqwest::Client, base_url: &str) -> Result<Self> {
        let url = format!("{}/client/manifest.json", base_url.trim_end_matches('/'));
        let bytes = client.get(url).send().await?.error_for_status()?.bytes().await?;
        Self::parse(&bytes)
    }
}

/// Turns a manifest path into a relative path that cannot leave the game
/// folder. Rejects absolute paths, drive letters, `..`, and backslashes.
pub fn safe_relative(p: &str) -> Result<PathBuf> {
    let bad = || Error::UnsafePath(p.to_string());
    if p.is_empty() || p.contains('\\') || p.contains(':') || p.starts_with('/') {
        return Err(bad());
    }
    let mut out = PathBuf::new();
    for c in Path::new(p).components() {
        match c {
            Component::Normal(s) => out.push(s),
            _ => return Err(bad()),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0263829989b6fd954f72baaf2fc64bc2e2f01d692d4de72986ea808f6e99813f";

    fn manifest(path: &str) -> String {
        format!(
            r#"{{"schema":1,"build":"1","server":{{"name":"A","ip":"h","port":7777}},
               "files":[{{"path":"{path}","size":2,"sha256":"{HASH}"}}],"remove":[]}}"#
        )
    }

    #[test]
    fn parses_generator_output() {
        let m = Manifest::parse(manifest("Data/Platform/Plugins/skymp5-client.js").as_bytes()).unwrap();
        assert_eq!(m.server.port, 7777);
        assert_eq!(m.files.len(), 1);
    }

    #[test]
    fn rejects_escaping_paths() {
        for p in ["../evil.dll", "Data/../../x", "/etc/passwd", "C:/Windows/x", "Data\\\\x", "./Data/x", ""] {
            assert!(Manifest::parse(manifest(p).as_bytes()).is_err(), "{p} should be rejected");
        }
    }

    #[test]
    fn rejects_unknown_schema() {
        let s = manifest("Data/x").replace("\"schema\":1", "\"schema\":2");
        assert!(matches!(Manifest::parse(s.as_bytes()), Err(Error::UnsupportedSchema(2))));
    }
}
