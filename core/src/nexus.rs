//! The slice of the Nexus Mods API the staff server-lane export uses
//! (https://app.swaggerhub.com/apis-docs/NexusMods/nexus-mods_public_api_params_in_form_data/1.0):
//! check a personal API key and get a download link for a pinned file, which
//! needs a Premium account. Players never sign in to Nexus in the launcher;
//! mods come through Vortex. Nothing here gets around Nexus's rules for
//! free members.

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

const API: &str = "https://api.nexusmods.com/v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct User {
    pub name: String,
    #[serde(default)]
    pub is_premium: bool,
}

#[derive(Deserialize)]
struct Link {
    #[serde(rename = "URI")]
    uri: String,
}

/// A Nexus personal API key as pasted: one long token with no spaces.
/// Returns it trimmed, or says why it isn't one. The error never includes
/// the pasted text.
pub fn clean_key(pasted: &str) -> Result<String> {
    let k = pasted.trim();
    if k.len() < 20 || k.len() > 400 || !k.chars().all(|c| c.is_ascii_alphanumeric() || "+/=_-".contains(c)) {
        return Err(Error::Game("That doesn't look like a Nexus API key. Copy the whole key from your Nexus account's API keys page.".into()));
    }
    Ok(k.to_string())
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
            401 => Err(Error::Game("Nexus Mods didn't accept the API key".into())),
            403 => Err(Error::Game("Nexus Mods only gives direct downloads to Premium members".into())),
            404 => Err(Error::Game("Nexus Mods couldn't find that mod or file".into())),
            429 => Err(Error::Game("Nexus Mods says too many requests. Wait an hour and try again".into())),
            _ => Ok(r.error_for_status()?.json().await?),
        }
    }

    pub async fn validate(&self) -> Result<User> {
        self.json(&format!("{API}/users/validate.json")).await
    }

    /// A download address for a file (Premium accounts only).
    pub async fn download_link(&self, game: &str, mod_id: u64, file_id: u64) -> Result<String> {
        let url = format!("{API}/games/{game}/mods/{mod_id}/files/{file_id}/download_link.json");
        let links: Vec<Link> = self.json(&url).await?;
        links.into_iter().next().map(|l| l.uri).filter(|u| u.starts_with("https://")).ok_or_else(|| Error::Game("Nexus Mods gave no download address".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_pasted_key_with_surrounding_whitespace() {
        let key = "abcDEF123+/=abcDEF123+/=abcDEF123--xyz--QQ==";
        assert_eq!(clean_key(&format!("  {key}\r\n")).unwrap(), key);
    }

    #[test]
    fn refuses_text_that_is_not_a_key_without_echoing_it() {
        for bad in ["", "short", "hello world, this is not a key at all", "abcDEF123abcDEF123abcDEF123 with space", &"a".repeat(401)] {
            let e = clean_key(bad).unwrap_err().to_string();
            assert!(e.contains("doesn't look like a Nexus API key"), "{e}");
            assert!(bad.is_empty() || !e.contains(bad), "the error repeated the pasted text");
        }
    }
}
