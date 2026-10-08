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
    /// The Skyrim build the server needs, and where Steam keeps it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<GameSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct GameSpec {
    /// SkyrimSE.exe file version, such as "1.6.1170.0".
    #[serde(default)]
    pub version: Option<String>,
    /// The SKSE release for that game version, such as "2.2.6". Shown to players.
    #[serde(default)]
    pub skse_version: Option<String>,
    #[serde(default = "default_app")]
    pub app: u32,
    /// Steam depot manifests that make up that build. Manifest ids are strings
    /// because they don't fit in a JSON number.
    #[serde(default)]
    pub depots: Vec<Depot>,
    /// Pinned DepotDownloader build for Windows. Without it the launcher uses
    /// the latest release from github.com/SteamRE/DepotDownloader.
    #[serde(default)]
    pub tool: Option<Tool>,
}

fn default_app() -> u32 {
    489830
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Depot {
    pub depot: u32,
    pub manifest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tool {
    pub url: String,
    #[serde(default)]
    pub sha256: Option<String>,
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
        if let Some(g) = &m.game {
            for d in &g.depots {
                if d.manifest.parse::<u64>().is_err() {
                    return Err(Error::Game(format!("the server lists a bad Steam manifest id for depot {}", d.depot)));
                }
            }
        }
        for r in &m.remove {
            safe_relative(r)?;
        }
        Ok(m)
    }

    /// The server's file list, once its signature has been checked
    /// (feedsig.rs). Every caller goes through here.
    pub async fn fetch(client: &reqwest::Client, base_url: &str) -> Result<Self> {
        Self::fetch_with(crate::feedsig::trust(), client, base_url).await
    }

    pub async fn fetch_with(trust: &crate::feedsig::Trust, client: &reqwest::Client, base_url: &str) -> Result<Self> {
        let bytes = trust.fetch(client, base_url, crate::feedsig::Feed::Manifest, None).await?;
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

    #[tokio::test]
    async fn fetch_refuses_a_file_list_that_was_changed_after_signing() {
        use crate::feedsig::tests::{key, serve, sign, trust_for};
        let k = key(9);
        let good = manifest("Data/Platform/Plugins/skymp5-client.js");
        let evil = manifest("Data/SKSE/Plugins/evil.dll");
        let sig = sign(&k, crate::feedsig::Feed::Manifest, good.as_bytes());
        let http = reqwest::Client::new();

        let signed = serve(vec![("/l/client/manifest.json", 200, good.clone().into_bytes()), ("/l/client/manifest.json.sig", 200, sig.clone())]).await.replace("/launcher", "/l");
        assert_eq!(Manifest::fetch_with(&trust_for(&k, true, None), &http, &signed).await.unwrap().files[0].path, "Data/Platform/Plugins/skymp5-client.js");

        let tampered = serve(vec![("/l/client/manifest.json", 200, evil.into_bytes()), ("/l/client/manifest.json.sig", 200, sig)]).await.replace("/launcher", "/l");
        assert!(matches!(Manifest::fetch_with(&trust_for(&k, false, None), &http, &tampered).await, Err(Error::FeedSignature(_))));

        // Not signed yet: still used by a launcher that never saw a signature, never when required.
        let unsigned = serve(vec![("/l/client/manifest.json", 200, good.into_bytes())]).await.replace("/launcher", "/l");
        assert!(Manifest::fetch_with(&trust_for(&k, false, None), &http, &unsigned).await.is_ok());
        assert!(Manifest::fetch_with(&trust_for(&k, true, None), &http, &unsigned).await.is_err());
    }
}
