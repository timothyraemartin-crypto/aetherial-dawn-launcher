//! The launcher's side of the Aetherial Dawn Vortex extension (Package C,
//! PR #7): pairing, signed jobs and the Ready check from Vortex's own state.
//!
//! Pairing is a persistent per-install token (32 random bytes, hex) in
//! `%LOCALAPPDATA%\gg.aetherialdawn.launcher\vortex\token`, which only this
//! Windows user can read. It is rotated only on an explicit re-pair or a
//! security failure (Codex 5875974143). Every job is HMAC-SHA256 signed with
//! it and carries a fresh timestamp and a one-use nonce; the extension
//! (vortex-extension/jobs.js) refuses anything else.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::modlist::ModEntry;
use crate::{Error, Result};

pub const TOKEN: &str = "token";
pub const PORT: &str = "port";

/// The launcher's Vortex folder under its own local data.
pub fn home(app_data: &Path) -> PathBuf {
    app_data.join("vortex")
}

/// HMAC-SHA256 of `body` keyed with the token's text, as hex (the same as
/// Node's `createHmac('sha256', token)` in jobs.js).
pub fn sign(token: &str, body: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut key = [0u8; 64];
    if token.len() > 64 {
        key[..32].copy_from_slice(&Sha256::digest(token.as_bytes()));
    } else {
        key[..token.len()].copy_from_slice(token.as_bytes());
    }
    let pad = |b: u8| key.iter().map(|k| k ^ b).collect::<Vec<u8>>();
    let inner = Sha256::new().chain_update(pad(0x36)).chain_update(body.as_bytes()).finalize();
    hex::encode(Sha256::new().chain_update(pad(0x5c)).chain_update(inner).finalize())
}

fn valid_token(t: &str) -> bool {
    t.len() == 64 && t.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// The pairing token, made once and kept. A token file that doesn't read as
/// one is replaced (the extension then needs Vortex restarted to pick it up).
pub fn pair(home: &Path) -> Result<String> {
    if let Ok(t) = std::fs::read_to_string(home.join(TOKEN)) {
        let t = t.trim();
        if valid_token(t) {
            return Ok(t.to_string());
        }
    }
    rotate(home)
}

/// A new token, replacing the old one atomically: for an explicit re-pair
/// or after a security failure.
pub fn rotate(home: &Path) -> Result<String> {
    use rand::RngCore;
    let mut b = [0u8; 32];
    rand::rng().fill_bytes(&mut b);
    let t = hex::encode(b);
    std::fs::create_dir_all(home)?;
    let tmp = home.join(format!("{TOKEN}.part"));
    std::fs::write(&tmp, &t)?;
    std::fs::rename(&tmp, home.join(TOKEN))?;
    Ok(t)
}

#[derive(Debug, Deserialize)]
struct PortFile {
    port: u16,
}

/// The port the extension listens on, when Vortex is running with it.
pub fn port(home: &Path) -> Option<u16> {
    let text = std::fs::read(home.join(PORT)).ok()?;
    serde_json::from_slice::<PortFile>(&text).ok().map(|p| p.port).filter(|p| *p != 0)
}

/// A signed job: its body and signature.
pub fn job(token: &str, verb: &str, args: &serde_json::Value, revision: &str, now_ms: u64) -> (String, String) {
    use rand::RngCore;
    let mut n = [0u8; 16];
    rand::rng().fill_bytes(&mut n);
    let body = serde_json::json!({ "verb": verb, "args": args, "revision": revision, "ts": now_ms, "nonce": hex::encode(n) }).to_string();
    let sig = sign(token, &body);
    (body, sig)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// Why a job didn't happen, as the extension or the connection said.
#[derive(Debug, Clone, PartialEq)]
pub enum JobError {
    /// No port file, or nothing answered: Vortex isn't running with the
    /// extension.
    NotRunning,
    /// The extension refused it, with its code and message.
    Refused { code: String, message: String },
    Other(String),
}

impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobError::NotRunning => write!(f, "Vortex isn't running with the Aetherial Dawn extension"),
            JobError::Refused { message, .. } => write!(f, "{message}"),
            JobError::Other(e) => write!(f, "{e}"),
        }
    }
}

/// Sends one signed job to the extension on 127.0.0.1 and returns its answer.
pub async fn call(http: &reqwest::Client, home: &Path, token: &str, verb: &str, args: &serde_json::Value, revision: &str) -> std::result::Result<serde_json::Value, JobError> {
    let port = port(home).ok_or(JobError::NotRunning)?;
    let (body, sig) = job(token, verb, args, revision, now_ms());
    let resp = http
        .post(format!("http://127.0.0.1:{port}/job"))
        .header("x-ad-sig", sig)
        .header("content-type", "application/json")
        .body(body)
        .timeout(std::time::Duration::from_secs(if verb == "install" || verb == "deploy" { 600 } else { 20 }))
        .send()
        .await
        .map_err(|e| if e.is_connect() { JobError::NotRunning } else { JobError::Other(e.to_string()) })?;
    let v: serde_json::Value = resp.json().await.map_err(|e| JobError::Other(e.to_string()))?;
    if v.get("ok").and_then(|o| o.as_bool()) == Some(true) {
        Ok(v)
    } else {
        Err(JobError::Refused {
            code: v.get("code").and_then(|c| c.as_str()).unwrap_or("failed").to_string(),
            message: v.get("error").and_then(|c| c.as_str()).unwrap_or("the extension refused the request").to_string(),
        })
    }
}

/// The extension's `status` answer.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Status {
    pub profile: Option<Profile>,
    #[serde(default)]
    pub mods: Vec<VortexMod>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub active: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct VortexMod {
    pub id: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default, rename = "nexusModId")]
    pub nexus_mod_id: Option<u64>,
    #[serde(default, rename = "nexusFileId")]
    pub nexus_file_id: Option<u64>,
    #[serde(default)]
    pub enabled: bool,
}

/// What the Ready gate knows from Vortex: per listed mod, whether the
/// "Aetherial Dawn" profile has that exact Nexus file installed and switched
/// on, and any other file of a listed mod still switched on there.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Membership {
    pub profile_active: bool,
    /// Listed mod ids (the list's own ids) not installed and on in the profile.
    pub missing: Vec<String>,
    /// Vortex mod ids of another file of a listed mod that is still on (such
    /// as the Unofficial Patch 4.3.9c beside the listed 4.3.8a).
    pub other_versions_on: Vec<String>,
}

impl Membership {
    pub fn ready(&self) -> bool {
        self.profile_active && self.missing.is_empty() && self.other_versions_on.is_empty()
    }
}

/// Checks the list against Vortex's own state, by Nexus mod and file id.
/// Only Nexus-pinned entries are Vortex's to hold; the others (GitHub and
/// built-in ones) are the launcher's.
pub fn membership(list: &[ModEntry], status: &Status) -> Membership {
    let profile_active = status.profile.as_ref().is_some_and(|p| p.active);
    let installed = |m: &VortexMod| m.state.as_deref().is_none_or(|s| s == "installed");
    let mut missing = Vec::new();
    let mut other_versions_on = Vec::new();
    for e in list {
        let Some(n) = &e.nexus else { continue };
        let Some(file) = n.file else { continue };
        let exact = status.mods.iter().any(|m| m.nexus_mod_id == Some(n.mod_id) && m.nexus_file_id == Some(file) && m.enabled && installed(m));
        if !exact {
            missing.push(e.id.clone());
        }
        for m in &status.mods {
            if m.nexus_mod_id == Some(n.mod_id) && m.nexus_file_id != Some(file) && m.enabled && !list.iter().any(|o| o.nexus.as_ref().is_some_and(|on| on.mod_id == n.mod_id && on.file == m.nexus_file_id)) && !other_versions_on.contains(&m.id) {
                other_versions_on.push(m.id.clone());
            }
        }
    }
    Membership { profile_active, missing, other_versions_on }
}

/// The Vortex packages in the profile that are the exact Nexus file of a
/// listed mod, for "Only the server's mods" to keep (allowlist::Approved).
pub fn approved(list: &[ModEntry], status: &Status) -> Vec<crate::allowlist::Approved> {
    status
        .mods
        .iter()
        .filter_map(|m| {
            let (mod_id, file_id) = (m.nexus_mod_id?, m.nexus_file_id?);
            list.iter()
                .any(|e| e.nexus.as_ref().is_some_and(|n| n.mod_id == mod_id && n.file == Some(file_id)))
                .then(|| crate::allowlist::Approved { vortex_id: m.id.clone(), nexus_mod_id: mod_id, nexus_file_id: file_id })
        })
        .collect()
}

/// The token check for callers that must not go on without a pairing.
pub fn token(home: &Path) -> Result<String> {
    let t = std::fs::read_to_string(home.join(TOKEN)).map_err(|_| Error::Game("the launcher isn't paired with Vortex yet".into()))?;
    let t = t.trim().to_string();
    if valid_token(&t) {
        Ok(t)
    } else {
        Err(Error::Game("the Vortex pairing token doesn't read; pair again".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modlist::NexusRef;

    #[test]
    fn signs_exactly_as_the_extension_checks() {
        // The same vector as vortex-extension/test/jobs.test.js.
        let body = r#"{"verb":"status","args":{},"revision":"r1","ts":1000000,"nonce":"0123456789abcdef0123456789abcdef"}"#;
        assert_eq!(sign(&"a".repeat(64), body), "3122895dea18179f37ba45ba8220fd633975ccaf40b2fb1a200c5422edc3ab00");
        // RFC 4231 test case 2.
        assert_eq!(sign("Jefe", "what do ya want for nothing?"), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    #[test]
    fn pairing_keeps_one_token_until_rotated() {
        let t = tempfile::tempdir().unwrap();
        let h = home(t.path());
        assert!(token(&h).is_err());
        let a = pair(&h).unwrap();
        assert!(valid_token(&a));
        assert_eq!(pair(&h).unwrap(), a, "kept across starts");
        assert_eq!(token(&h).unwrap(), a);
        let b = rotate(&h).unwrap();
        assert_ne!(a, b);
        assert_eq!(pair(&h).unwrap(), b);
        std::fs::write(h.join(TOKEN), "not a token").unwrap();
        assert!(token(&h).is_err());
        assert_ne!(pair(&h).unwrap(), "not a token");
    }

    #[test]
    fn a_job_is_signed_with_a_fresh_nonce_each_time() {
        let tok = "b".repeat(64);
        let (b1, s1) = job(&tok, "status", &serde_json::json!({}), "r1", 5);
        let (b2, _) = job(&tok, "status", &serde_json::json!({}), "r1", 5);
        assert_eq!(s1, sign(&tok, &b1));
        let v1: serde_json::Value = serde_json::from_str(&b1).unwrap();
        let v2: serde_json::Value = serde_json::from_str(&b2).unwrap();
        assert_ne!(v1["nonce"], v2["nonce"]);
        assert_eq!(v1["nonce"].as_str().unwrap().len(), 32);
        assert_eq!((v1["verb"].as_str(), v1["revision"].as_str(), v1["ts"].as_u64()), (Some("status"), Some("r1"), Some(5)));
    }

    #[test]
    fn port_file_reads_or_says_not_running() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(port(t.path()), None);
        std::fs::write(t.path().join(PORT), r#"{"port":51234,"pid":9}"#).unwrap();
        assert_eq!(port(t.path()), Some(51234));
        std::fs::write(t.path().join(PORT), "{").unwrap();
        assert_eq!(port(t.path()), None);
    }

    #[test]
    fn membership_is_by_exact_nexus_file_in_the_active_profile() {
        let e = |id: &str, m: u64, f: u64| ModEntry { id: id.into(), name: id.into(), nexus: Some(NexusRef { mod_id: m, file: Some(f), pick: None }), ..Default::default() };
        let list = vec![e("ussep", 266, 733846), e("skyui", 12604, 35407), ModEntry { id: "github-one".into(), ..Default::default() }];
        let vm = |id: &str, m: u64, f: u64, on: bool| VortexMod { id: id.into(), state: Some("installed".into()), nexus_mod_id: Some(m), nexus_file_id: Some(f), enabled: on };
        // Timothy's profile today: 4.3.9c on, SkyUI on.
        let mut st = Status { profile: Some(Profile { id: "p1".into(), name: "Aetherial Dawn".into(), active: true }), mods: vec![vm("u439c", 266, 999999, true), vm("skyui", 12604, 35407, true)] };
        let m = membership(&list, &st);
        assert_eq!((m.missing.clone(), m.other_versions_on.clone()), (vec!["ussep".to_string()], vec!["u439c".to_string()]));
        assert!(!m.ready());
        // 4.3.8a installed but still off: still missing.
        st.mods.push(vm("u438a", 266, 733846, false));
        assert_eq!(membership(&list, &st).missing, ["ussep"]);
        // On, and 4.3.9c off: ready.
        st.mods[2].enabled = true;
        st.mods[0].enabled = false;
        assert!(membership(&list, &st).ready());
        // Another profile active: not ready, whatever it holds.
        st.profile.as_mut().unwrap().active = false;
        assert!(!membership(&list, &st).ready());
        // A mod mid-install doesn't count.
        st.profile.as_mut().unwrap().active = true;
        st.mods[2].state = Some("installing".into());
        assert_eq!(membership(&list, &st).missing, ["ussep"]);
        // Approved for "Only the server's mods": the listed files only.
        let a = approved(&list, &st);
        assert_eq!(a.iter().map(|a| a.vortex_id.as_str()).collect::<Vec<_>>(), ["skyui", "u438a"]);
    }
}
