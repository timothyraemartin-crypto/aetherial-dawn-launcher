//! Writes `Data/Platform/Plugins/skymp5-client-settings.txt`, the JSON file the
//! SkyMP client reads on startup (keys from skymp5-client/src in the skymp repo).
//! Keys the launcher doesn't manage are kept as they are.

use serde_json::{json, Map, Value};
use std::path::Path;

use crate::Result;

pub const SETTINGS_PATH: &str = "Data/Platform/Plugins/skymp5-client-settings.txt";

pub struct ClientSettings<'a> {
    pub server_ip: &'a str,
    pub server_port: u16,
    pub master: &'a str,
    pub profile_id: i64,
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
    let game_data = root.entry("gameData").or_insert_with(|| Value::Object(Map::new()));
    if !game_data.is_object() {
        *game_data = Value::Object(Map::new());
    }
    game_data.as_object_mut().unwrap().insert("profileId".into(), json!(s.profile_id));
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

#[cfg(test)]
mod tests {
    use super::*;

    const S: ClientSettings = ClientSettings { server_ip: "play.example.org", server_port: 7777, master: "", profile_id: 42 };

    #[test]
    fn writes_fresh_file() {
        let v: Value = serde_json::from_str(&merge(None, &S)).unwrap();
        assert_eq!(v["server-ip"], "play.example.org");
        assert_eq!(v["server-port"], 7777);
        assert_eq!(v["gameData"]["profileId"], 42);
    }

    #[test]
    fn keeps_unmanaged_keys() {
        let old = r#"{"show-net-info":true,"server-ip":"old","gameData":{"profileId":1,"other":"x"}}"#;
        let v: Value = serde_json::from_str(&merge(Some(old), &S)).unwrap();
        assert_eq!(v["show-net-info"], true);
        assert_eq!(v["server-ip"], "play.example.org");
        assert_eq!(v["gameData"]["other"], "x");
        assert_eq!(v["gameData"]["profileId"], 42);
    }

    #[test]
    fn replaces_garbage() {
        let v: Value = serde_json::from_str(&merge(Some("not json"), &S)).unwrap();
        assert_eq!(v["server-port"], 7777);
    }
}
