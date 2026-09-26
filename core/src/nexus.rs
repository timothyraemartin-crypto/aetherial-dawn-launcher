//! Nexus Mods downloads the way Nexus allows them for mod managers
//! (https://app.swaggerhub.com/apis-docs/NexusMods/nexus-mods_public_api_params_in_form_data/1.0).
//!
//! Every player signs in with their own Nexus API key, kept encrypted on
//! their PC like the Discord sign-in. Premium members get download links
//! straight from the API, so the launcher fetches every mod with one click.
//! Free members have to press "Mod manager download" on each mod's page:
//! Nexus then opens an nxm:// link, which the launcher catches (it registers
//! itself for nxm:// while it's waiting and gives the link back to Vortex or
//! whoever had it after) and turns into a download. Nothing here gets around
//! Nexus's rules for free members.

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

const API: &str = "https://api.nexusmods.com/v1";
/// Where players make their personal API key.
pub const API_KEY_PAGE: &str = "https://next.nexusmods.com/settings/api-keys";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct User {
    pub name: String,
    #[serde(default)]
    pub is_premium: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct NexusFile {
    pub file_id: u64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub category_name: Option<String>,
    #[serde(default)]
    pub uploaded_timestamp: u64,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub size_in_bytes: Option<u64>,
}

#[derive(Deserialize)]
struct FilesAnswer {
    files: Vec<NexusFile>,
}

#[derive(Deserialize)]
struct Link {
    #[serde(rename = "URI")]
    uri: String,
}

pub struct Client<'a> {
    pub http: &'a reqwest::Client,
    pub key: &'a str,
    pub app_version: &'a str,
}

impl Client<'_> {
    fn get(&self, url: &str) -> reqwest::RequestBuilder {
        self.http
            .get(url)
            .header("apikey", self.key)
            .header("Application-Name", "Aetherial Dawn Launcher")
            .header("Application-Version", self.app_version)
            .header("User-Agent", format!("AetherialDawnLauncher/{}", self.app_version))
    }

    async fn json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T> {
        let r = self.get(url).send().await?;
        match r.status().as_u16() {
            401 => Err(Error::Game("Nexus Mods didn't accept the API key. Sign in to Nexus again".into())),
            403 => Err(Error::Game("Nexus Mods only gives direct downloads to Premium members".into())),
            404 => Err(Error::Game("Nexus Mods couldn't find that mod or file".into())),
            429 => Err(Error::Game("Nexus Mods says too many requests. Wait an hour and try again".into())),
            _ => Ok(r.error_for_status()?.json().await?),
        }
    }

    pub async fn validate(&self) -> Result<User> {
        self.json(&format!("{API}/users/validate.json")).await
    }

    pub async fn files(&self, game: &str, mod_id: u64) -> Result<Vec<NexusFile>> {
        let a: FilesAnswer = self.json(&format!("{API}/games/{game}/mods/{mod_id}/files.json")).await?;
        Ok(a.files)
    }

    /// A download address for a file. Premium members need nothing else; free
    /// members pass the key and expiry from the nxm:// link Nexus gave them.
    pub async fn download_link(&self, game: &str, mod_id: u64, file_id: u64, nxm: Option<&Nxm>) -> Result<String> {
        let mut url = format!("{API}/games/{game}/mods/{mod_id}/files/{file_id}/download_link.json");
        if let Some(n) = nxm {
            url.push_str(&format!("?key={}&expires={}", enc(&n.key), n.expires));
        }
        let links: Vec<Link> = self.json(&url).await?;
        links.into_iter().next().map(|l| l.uri).filter(|u| u.starts_with("https://")).ok_or_else(|| Error::Game("Nexus Mods gave no download address".into()))
    }
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// The files to try for a mod, best first: the list's file id when it pins
/// one; otherwise main files (then updates and optional ones), newest first,
/// preferring names that contain `pick`. Old versions come last so a newer
/// file made for a newer Skyrim can fall back to an older one.
pub fn candidates(files: &[NexusFile], file: Option<u64>, pick: Option<&str>) -> Vec<NexusFile> {
    if let Some(id) = file {
        return files.iter().filter(|f| f.file_id == id).cloned().collect();
    }
    let pick = pick.map(|p| p.to_ascii_lowercase());
    let rank = |f: &NexusFile| {
        let cat = f.category_name.as_deref().unwrap_or("").to_ascii_uppercase();
        let named = pick.as_ref().map(|p| f.name.to_ascii_lowercase().contains(p.as_str()) || f.file_name.to_ascii_lowercase().contains(p.as_str())).unwrap_or(false);
        let cat_rank = match cat.as_str() {
            "MAIN" => 0,
            "UPDATE" => 1,
            "OPTIONAL" => 2,
            "OLD_VERSION" => 3,
            "MISCELLANEOUS" => 4,
            _ => 5,
        };
        (if named { 0 } else { 1 }, if cat_rank == 3 { 1 } else { 0 }, cat_rank, std::cmp::Reverse(f.uploaded_timestamp))
    };
    let mut out: Vec<NexusFile> = files.iter().filter(|f| !matches!(f.category_name.as_deref(), Some("DELETED") | Some("ARCHIVED"))).cloned().collect();
    out.sort_by_key(|f| rank(f));
    if pick.is_some() && out.first().map(|f| rank(f).0 == 0).unwrap_or(false) {
        out.retain(|f| rank(f).0 == 0);
    }
    out
}

/// A parsed nxm:// link, e.g.
/// nxm://skyrimspecialedition/mods/266/files/12345?key=abc&expires=1700000000&user_id=5
#[derive(Debug, Clone, PartialEq)]
pub struct Nxm {
    pub game: String,
    pub mod_id: u64,
    pub file_id: u64,
    pub key: String,
    pub expires: u64,
}

pub fn parse_nxm(url: &str) -> Option<Nxm> {
    let rest = url.trim().trim_matches('"').strip_prefix("nxm://")?;
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    if parts.len() != 5 || !parts[1].eq_ignore_ascii_case("mods") || !parts[3].eq_ignore_ascii_case("files") {
        return None;
    }
    let mut key = None;
    let mut expires = None;
    for kv in query.split('&') {
        match kv.split_once('=') {
            Some(("key", v)) => key = Some(v.to_string()),
            Some(("expires", v)) => expires = v.parse().ok(),
            _ => {}
        }
    }
    Some(Nxm { game: parts[0].to_ascii_lowercase(), mod_id: parts[2].parse().ok()?, file_id: parts[4].parse().ok()?, key: key.filter(|k| !k.is_empty())?, expires: expires? })
}

// ---------- the nxm:// handler ----------

/// Registers the launcher as the nxm:// handler for this Windows user and
/// returns the command that was there before (Vortex's, usually), so it can
/// be put back with `restore_nxm_handler`.
#[cfg(windows)]
pub fn claim_nxm_handler(exe: &std::path::Path) -> Result<Option<String>> {
    use winreg::enums::HKEY_CURRENT_USER;
    let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
    let previous: Option<String> = hkcu.open_subkey(r"Software\Classes\nxm\shell\open\command").ok().and_then(|k| k.get_value::<String, _>("").ok());
    let (root, _) = hkcu.create_subkey(r"Software\Classes\nxm")?;
    root.set_value("", &"URL:NXM Protocol")?;
    root.set_value("URL Protocol", &"")?;
    let (cmd, _) = hkcu.create_subkey(r"Software\Classes\nxm\shell\open\command")?;
    cmd.set_value("", &format!("\"{}\" \"%1\"", exe.display()))?;
    Ok(previous)
}

#[cfg(windows)]
pub fn restore_nxm_handler(previous: Option<&str>) -> Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
    match previous {
        Some(p) => {
            let (cmd, _) = hkcu.create_subkey(r"Software\Classes\nxm\shell\open\command")?;
            cmd.set_value("", &p)?;
        }
        None => {
            let _ = hkcu.delete_subkey_all(r"Software\Classes\nxm");
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn claim_nxm_handler(_exe: &std::path::Path) -> Result<Option<String>> {
    Ok(None)
}

#[cfg(not(windows))]
pub fn restore_nxm_handler(_previous: Option<&str>) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(id: u64, name: &str, cat: &str, t: u64) -> NexusFile {
        NexusFile { file_id: id, name: name.into(), version: None, category_name: Some(cat.into()), uploaded_timestamp: t, file_name: format!("{name}.7z"), size_in_bytes: None }
    }

    #[test]
    fn parses_nxm_links() {
        let n = parse_nxm("nxm://SkyrimSpecialEdition/mods/266/files/512345?key=a-b_c&expires=1700000000&user_id=9").unwrap();
        assert_eq!(n, Nxm { game: "skyrimspecialedition".into(), mod_id: 266, file_id: 512345, key: "a-b_c".into(), expires: 1700000000 });
        assert!(parse_nxm("nxm://skyrimspecialedition/mods/266/files/1").is_none());
        assert!(parse_nxm("https://example.com").is_none());
        assert!(parse_nxm("nxm://skyrimspecialedition/collections/x/revisions/1").is_none());
    }

    #[test]
    fn picks_files() {
        let files = vec![
            f(1, "Engine Fixes - Part 1", "MAIN", 10),
            f(2, "Engine Fixes (All-In-One) for 1.6.1170 and newer", "MAIN", 20),
            f(3, "Old", "OLD_VERSION", 30),
            f(4, "Newest main", "MAIN", 40),
            f(5, "Gone", "DELETED", 50),
        ];
        assert_eq!(candidates(&files, None, Some("All-In-One")).iter().map(|f| f.file_id).collect::<Vec<_>>(), [2]);
        assert_eq!(candidates(&files, None, None).iter().map(|f| f.file_id).collect::<Vec<_>>(), [4, 2, 1, 3]);
        assert_eq!(candidates(&files, Some(3), None)[0].file_id, 3);
        assert!(candidates(&files, Some(99), None).is_empty());
    }
}
