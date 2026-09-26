//! Writes `Data/Platform/Plugins/skymp5-client-settings.txt`, the JSON file the
//! SkyMP client reads on startup (keys from skymp5-client/src in the skymp repo).
//! Keys the launcher doesn't manage are kept as they are.

use serde_json::{json, Map, Value};
use std::path::Path;

use crate::auth::Profile;
use crate::Result;

pub const SETTINGS_PATH: &str = "Data/Platform/Plugins/skymp5-client-settings.txt";

/// The server's public key for its signed gamemode scripts (SkyMP
/// serverJsVerificationService). The client skips the serverinfo request
/// because server-info-ignore is on, so the key has to be in the settings
/// file, or every signed server script (the F3 menus) is rejected. Public, so
/// it's fine to ship.
pub const SERVER_PUBLIC_KEYS: [(&str, &str); 1] = [(
    "CPPad1",
    "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAv/MvitoXZ+ISkm41BttNZEVf1bcw7xbq2NBAEtRn2cY=\n-----END PUBLIC KEY-----\n",
)];

pub struct ClientSettings<'a> {
    pub server_ip: &'a str,
    pub server_port: u16,
    /// The login service (SkyMP "master"), e.g. https://host/ad.
    pub master: &'a str,
    pub server_master_key: &'a str,
    /// Game session from the login service for this Play.
    pub session: &'a str,
}

pub fn merge(existing: Option<&str>, s: &ClientSettings) -> String {
    let mut root = existing
        .and_then(|t| serde_json::from_str::<Value>(t).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default();
    root.insert("server-ip".into(), json!(s.server_ip));
    root.insert("server-port".into(), json!(s.server_port));
    root.insert("master".into(), json!(s.master));
    root.insert("server-master-key".into(), json!(s.server_master_key));
    root.insert("server-info-ignore".into(), json!(true));
    let keys: Map<String, Value> = SERVER_PUBLIC_KEYS.iter().map(|(id, pem)| (id.to_string(), json!(pem))).collect();
    root.insert("server-public-keys".into(), Value::Object(keys));
    let game_data = root.entry("gameData").or_insert_with(|| Value::Object(Map::new()));
    if !game_data.is_object() {
        *game_data = Value::Object(Map::new());
    }
    let gd = game_data.as_object_mut().unwrap();
    // A numeric profileId would put the client in offline mode.
    gd.remove("profileId");
    gd.insert("session".into(), json!(s.session));
    serde_json::to_string_pretty(&Value::Object(root)).unwrap()
}

pub fn write(game_dir: &Path, s: &ClientSettings) -> Result<()> {
    let path = game_dir.join(SETTINGS_PATH);
    let existing = std::fs::read_to_string(&path).ok();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, merge(existing.as_deref(), s))?;
    Ok(())
}

/// The SkyMP client's remembered login (skymp5-client authService.ts reads it
/// from PluginsNoLoad). With it the in-game login box already shows the player
/// as signed in.
pub const AUTH_DATA_PATH: &str = "Data/Platform/PluginsNoLoad/auth-data-no-load.js";

pub fn write_auth_data(game_dir: &Path, session: &str, p: &Profile) -> Result<()> {
    let path = game_dir.join(AUTH_DATA_PATH);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = json!({
        "session": session,
        "masterApiId": p.master_api_id,
        "discordUsername": p.discord_username,
        "discordDiscriminator": p.discord_discriminator,
        "discordAvatar": p.discord_avatar,
    });
    std::fs::write(path, format!("//{body}"))?;
    Ok(())
}

/// Signing out also forgets the game's remembered login and session.
pub fn clear_login(game_dir: &Path) {
    let _ = std::fs::remove_file(game_dir.join(AUTH_DATA_PATH));
    let path = game_dir.join(SETTINGS_PATH);
    if let Some(Value::Object(mut root)) = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()) {
        if let Some(Value::Object(gd)) = root.get_mut("gameData") {
            gd.remove("session");
        }
        let _ = std::fs::write(&path, serde_json::to_string_pretty(&Value::Object(root)).unwrap());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: ClientSettings = ClientSettings {
        server_ip: "play.example.org",
        server_port: 7777,
        master: "https://h/ad",
        server_master_key: "aetherial-dawn",
        session: "abc",
    };

    #[test]
    fn writes_fresh_file() {
        let v: Value = serde_json::from_str(&merge(None, &S)).unwrap();
        assert_eq!(v["server-ip"], "play.example.org");
        assert_eq!(v["server-port"], 7777);
        assert_eq!(v["master"], "https://h/ad");
        assert_eq!(v["server-master-key"], "aetherial-dawn");
        assert_eq!(v["server-info-ignore"], true);
        assert!(v["server-public-keys"]["CPPad1"].as_str().unwrap().starts_with("-----BEGIN PUBLIC KEY-----\nMCow"));
        assert_eq!(v["gameData"]["session"], "abc");
    }

    #[test]
    fn keeps_unmanaged_keys_and_drops_profile_id() {
        let old = r#"{"show-net-info":true,"server-ip":"old","gameData":{"profileId":1,"other":"x"}}"#;
        let v: Value = serde_json::from_str(&merge(Some(old), &S)).unwrap();
        assert_eq!(v["show-net-info"], true);
        assert_eq!(v["server-ip"], "play.example.org");
        assert_eq!(v["gameData"]["other"], "x");
        assert!(v["gameData"].get("profileId").is_none());
    }

    #[test]
    fn replaces_garbage() {
        let v: Value = serde_json::from_str(&merge(Some("not json"), &S)).unwrap();
        assert_eq!(v["server-port"], 7777);
    }

    #[test]
    fn auth_data_and_sign_out() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), &S).unwrap();
        let p = Profile { master_api_id: Some(7), discord_username: Some("Lydia".into()), ..Default::default() };
        write_auth_data(d.path(), "abc", &p).unwrap();
        let text = std::fs::read_to_string(d.path().join(AUTH_DATA_PATH)).unwrap();
        let v: Value = serde_json::from_str(text.strip_prefix("//").unwrap()).unwrap();
        assert_eq!(v["session"], "abc");
        assert_eq!(v["masterApiId"], 7);
        assert!(v["discordDiscriminator"].is_null());
        clear_login(d.path());
        assert!(!d.path().join(AUTH_DATA_PATH).exists());
        let v: Value = serde_json::from_str(&std::fs::read_to_string(d.path().join(SETTINGS_PATH)).unwrap()).unwrap();
        assert!(v["gameData"].get("session").is_none());
        assert_eq!(v["server-ip"], "play.example.org");
    }
}
