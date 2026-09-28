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
    /// The one Skyrim SE profile named "Aetherial Dawn", when there is
    /// exactly one.
    pub profile: Option<Profile>,
    /// How many Skyrim SE profiles are named "Aetherial Dawn".
    #[serde(default, rename = "aetherialProfiles")]
    pub aetherial_profiles: usize,
    /// The active Skyrim SE profile.
    #[serde(default, rename = "activeProfile")]
    pub active_profile: Option<ActiveProfile>,
    #[serde(default)]
    pub mods: Vec<VortexMod>,
    #[serde(default)]
    pub collections: Vec<Collection>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub active: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct ActiveProfile {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
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

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Collection {
    pub id: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub revision: Option<u64>,
}

/// The client set the launcher expects in the "Aetherial Dawn" profile
/// (docs/vortex-collection-design.md 6.1): the required Nexus files and,
/// once there is one, the Aetherial Dawn collection by slug and revision.
/// Milestone 1 (Timothy's existing profile, PR #7 5876324959) has no
/// collection yet: only the files are checked.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct ClientSet {
    #[serde(default)]
    pub collection: Option<CollectionRef>,
    pub mods: Vec<ModEntry>,
}

/// A collection by slug and revision. How Vortex 2.7.1 records these on an
/// installed collection is unverified until observed on the disposable
/// profile.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct CollectionRef {
    pub slug: String,
    pub revision: u64,
}

/// One step line of the Requirements window (design section 3), for Vortex,
/// the profile and the collection.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// The extension didn't answer: Vortex closed, or not paired.
    VortexNotRunning,
    NoProfile,
    TwoProfiles(usize),
    OtherProfileActive(Option<String>),
    CollectionNotAdded,
    WrongRevision { has: Option<u64>, needs: u64 },
    /// Counts of the required files, from Vortex's own state only (never the
    /// launcher's direct-to-Data ledger): installed (state "installed") and
    /// switched on in the profile, with the ones still to come named, and
    /// any other file of a listed mod still switched on.
    Counts { required: usize, installed: usize, enabled: usize, waiting: Vec<String>, other_versions_on: Vec<String> },
    Ready,
}

impl Step {
    pub fn ok(&self) -> bool {
        *self == Step::Ready
    }

    /// The line the player reads.
    pub fn describe(&self) -> String {
        match self {
            Step::VortexNotRunning => "Vortex: open Vortex (with the Aetherial Dawn extension) so the launcher can check your mods".into(),
            Step::NoProfile => "Vortex: make a profile named \"Aetherial Dawn\" for Skyrim Special Edition".into(),
            Step::TwoProfiles(n) => format!("Vortex: {n} profiles are named \"Aetherial Dawn\"; keep one"),
            Step::OtherProfileActive(name) => format!("Vortex: switch from {} to the \"Aetherial Dawn\" profile", name.as_deref().map(|n| format!("\"{n}\"")).unwrap_or_else(|| "another profile".into())),
            Step::CollectionNotAdded => "Collection: add the Aetherial Dawn collection in Vortex".into(),
            Step::WrongRevision { has, needs } => match has {
                Some(h) => format!("Collection: revision {h} is installed; update to revision {needs}"),
                None => format!("Collection: update to revision {needs}"),
            },
            Step::Counts { required, installed, enabled, waiting, other_versions_on } => format!(
                "Aetherial Dawn profile: {installed} of {required} installed · {enabled} of {required} switched on{}{}",
                if waiting.is_empty() { String::new() } else { format!(" · waiting: {}", waiting.join(", ")) },
                if other_versions_on.is_empty() { String::new() } else { format!(" · another version still on: {}", other_versions_on.join(", ")) }
            ),
            Step::Ready => "Aetherial Dawn profile: every required mod installed and switched on".into(),
        }
    }
}

/// The Vortex step from the extension's status (None: it didn't answer).
/// Pure, so each state has a fixture test. Ready only from Vortex's state.
pub fn step(set: &ClientSet, status: Option<&Status>) -> Step {
    let Some(st) = status else { return Step::VortexNotRunning };
    match st.aetherial_profiles {
        0 => return Step::NoProfile,
        1 => {}
        n => return Step::TwoProfiles(n),
    }
    if !st.profile.as_ref().is_some_and(|p| p.active) {
        return Step::OtherProfileActive(st.active_profile.as_ref().and_then(|a| a.name.clone()));
    }
    if let Some(c) = &set.collection {
        let mine: Vec<&Collection> = st.collections.iter().filter(|x| x.slug.as_deref() == Some(c.slug.as_str())).collect();
        if mine.is_empty() {
            return Step::CollectionNotAdded;
        }
        if !mine.iter().any(|x| x.revision == Some(c.revision)) {
            return Step::WrongRevision { has: mine.iter().filter_map(|x| x.revision).max(), needs: c.revision };
        }
    }
    let installed_state = |m: &VortexMod| m.state.as_deref().is_none_or(|s| s == "installed");
    let mut required = 0;
    let (mut installed, mut enabled) = (0, 0);
    let mut waiting = Vec::new();
    for e in &set.mods {
        let Some(n) = &e.nexus else { continue };
        let Some(file) = n.file else { continue };
        required += 1;
        let found: Vec<&VortexMod> = st.mods.iter().filter(|m| m.nexus_mod_id == Some(n.mod_id) && m.nexus_file_id == Some(file) && installed_state(m)).collect();
        if !found.is_empty() {
            installed += 1;
        }
        if found.iter().any(|m| m.enabled) {
            enabled += 1;
        } else {
            waiting.push(e.name.clone());
        }
    }
    let other_versions_on = membership(&set.mods, st).other_versions_on;
    if required > 0 && installed == required && enabled == required && other_versions_on.is_empty() {
        Step::Ready
    } else {
        Step::Counts { required, installed, enabled, waiting, other_versions_on }
    }
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

/// The extension's folder name inside Vortex's plugins folder.
pub const EXT_DIR: &str = "aetherial-dawn";

/// The extension as this launcher carries it (vortex-extension/), top-level
/// files only. Its version is the one in its info.json.
pub const EXTENSION: &[(&str, &[u8])] = &[
    ("index.js", include_bytes!("../../vortex-extension/index.js")),
    ("jobs.js", include_bytes!("../../vortex-extension/jobs.js")),
    ("info.json", include_bytes!("../../vortex-extension/info.json")),
];

/// Vortex's per-user extensions folder under the roaming app data folder,
/// `%APPDATA%\Vortex\plugins` (verify on Timothy's PC, design 2).
pub fn plugins_dir(roaming: &Path) -> PathBuf {
    roaming.join("Vortex").join("plugins")
}

/// What installing the extension did.
#[derive(Debug, Clone, PartialEq)]
pub enum Installed {
    /// It wasn't there; Vortex loads it on its next start.
    Fresh,
    /// An older or changed copy was replaced; Vortex needs a restart.
    Updated { from: Option<String> },
    /// The same files are already there.
    Current,
    /// A newer launcher already put a later version there; it's kept.
    NewerKept { installed: String },
}

impl Installed {
    /// Vortex has to be (re)started before it runs these files.
    pub fn needs_restart(&self) -> bool {
        matches!(self, Installed::Fresh | Installed::Updated { .. })
    }
}

fn info_version(info: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(info).ok()?.get("version")?.as_str().map(str::to_string)
}

fn version_parts(v: &str) -> Vec<u64> {
    v.split('.').map(|p| p.trim().parse().unwrap_or(0)).collect()
}

/// Puts the extension into `plugins/aetherial-dawn`. Each file is written in
/// full beside Vortex's folder, then renamed over the old one, info.json
/// last, so Vortex never loads a half-written file and a stopped install is
/// finished by the next one. Files there that aren't the extension's are
/// left alone, and a later version put there by a newer launcher is kept.
pub fn install_extension(plugins: &Path, files: &[(&str, &[u8])]) -> Result<Installed> {
    let vortex = plugins.parent().ok_or_else(|| Error::Game("Vortex's folder has no parent".into()))?;
    if !vortex.is_dir() {
        return Err(Error::Game("Vortex isn't set up for this Windows user (no Vortex folder in AppData)".into()));
    }
    let info = files.iter().find(|(n, _)| *n == "info.json").map(|(_, b)| *b).ok_or_else(|| Error::Game("the extension has no info.json".into()))?;
    let bundled = info_version(info).ok_or_else(|| Error::Game("the extension's info.json has no version".into()))?;
    for (name, _) in files {
        if name.is_empty() || name.contains(['/', '\\', ':']) || name.starts_with('.') {
            return Err(Error::Game(format!("the extension file name {name:?} isn't a plain file name")));
        }
    }
    let dir = plugins.join(EXT_DIR);
    let installed = std::fs::read(dir.join("info.json")).ok().and_then(|b| info_version(&b));
    if files.iter().all(|(n, b)| std::fs::read(dir.join(n)).is_ok_and(|have| have == *b)) {
        return Ok(Installed::Current);
    }
    if let Some(v) = installed.as_deref().filter(|v| version_parts(v) > version_parts(&bundled)) {
        return Ok(Installed::NewerKept { installed: v.to_string() });
    }
    let staging = vortex.join("aetherial-dawn-extension.staging");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)?;
    for (name, bytes) in files {
        std::fs::write(staging.join(name), bytes)?;
    }
    std::fs::create_dir_all(&dir)?;
    let fresh = !dir.join("info.json").exists();
    let (last, rest): (Vec<_>, Vec<_>) = files.iter().partition(|(n, _)| *n == "info.json");
    for (name, _) in rest.iter().chain(last.iter()) {
        std::fs::rename(staging.join(name), dir.join(name))?;
    }
    let _ = std::fs::remove_dir_all(&staging);
    Ok(if fresh { Installed::Fresh } else { Installed::Updated { from: installed } })
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

    fn e(id: &str, m: u64, f: u64) -> ModEntry {
        ModEntry { id: id.into(), name: id.to_uppercase(), nexus: Some(NexusRef { mod_id: m, file: Some(f), pick: None }), ..Default::default() }
    }

    fn vm(id: &str, m: u64, f: u64, on: bool) -> VortexMod {
        VortexMod { id: id.into(), state: Some("installed".into()), nexus_mod_id: Some(m), nexus_file_id: Some(f), enabled: on }
    }

    fn active(mods: Vec<VortexMod>) -> Status {
        Status {
            profile: Some(Profile { id: "p1".into(), name: "Aetherial Dawn".into(), active: true }),
            aetherial_profiles: 1,
            active_profile: Some(ActiveProfile { id: "p1".into(), name: Some("Aetherial Dawn".into()) }),
            mods,
            collections: vec![],
        }
    }

    #[test]
    fn each_vortex_state_has_its_own_line() {
        let set = ClientSet { collection: Some(CollectionRef { slug: "adcol".into(), revision: 2 }), mods: vec![e("ussep", 266, 733846), e("skyui", 12604, 35407)] };
        let mut good = active(vec![vm("u", 266, 733846, true), vm("s", 12604, 35407, true)]);
        good.collections = vec![Collection { id: "c".into(), state: Some("installed".into()), enabled: true, slug: Some("adcol".into()), revision: Some(2) }];
        assert_eq!(step(&set, None), Step::VortexNotRunning);
        assert_eq!(step(&set, Some(&good)), Step::Ready);
        assert!(step(&set, Some(&good)).ok());
        let with = |f: &dyn Fn(&mut Status)| {
            let mut s = good.clone();
            f(&mut s);
            step(&set, Some(&s))
        };
        assert_eq!(with(&|s| { s.aetherial_profiles = 0; s.profile = None }), Step::NoProfile);
        assert_eq!(with(&|s| { s.aetherial_profiles = 2; s.profile = None }), Step::TwoProfiles(2));
        let other = with(&|s| { s.profile.as_mut().unwrap().active = false; s.active_profile = Some(ActiveProfile { id: "p2".into(), name: Some("Default".into()) }) });
        assert_eq!(other, Step::OtherProfileActive(Some("Default".into())));
        assert!(other.describe().contains("switch from \"Default\""));
        assert_eq!(with(&|s| s.collections.clear()), Step::CollectionNotAdded);
        assert_eq!(with(&|s| s.collections[0].revision = Some(1)), Step::WrongRevision { has: Some(1), needs: 2 });
        // Downloading: SkyUI not there yet.
        let dl = with(&|s| { s.mods.pop(); });
        assert_eq!(dl, Step::Counts { required: 2, installed: 1, enabled: 1, waiting: vec!["SKYUI".into()], other_versions_on: vec![] });
        assert_eq!(dl.describe(), "Aetherial Dawn profile: 1 of 2 installed · 1 of 2 switched on · waiting: SKYUI");
        // Installed but off.
        assert_eq!(with(&|s| s.mods[1].enabled = false), Step::Counts { required: 2, installed: 2, enabled: 1, waiting: vec!["SKYUI".into()], other_versions_on: vec![] });
        // Mid-install doesn't count as installed.
        assert_eq!(with(&|s| s.mods[1].state = Some("installing".into())), Step::Counts { required: 2, installed: 1, enabled: 1, waiting: vec!["SKYUI".into()], other_versions_on: vec![] });
        // Another file of a listed mod still on (the 4.3.9c case): not ready, and said.
        let two = with(&|s| s.mods.push(vm("u439c", 266, 999999, true)));
        assert!(!two.ok());
        assert!(two.describe().ends_with("another version still on: u439c"), "{}", two.describe());
    }

    /// Milestone 1: Timothy's existing profile, with no collection yet. His
    /// 11 Vortex rows (2026-09-28 screenshot) against a larger client set
    /// read as incomplete, and the launcher's own Data installs never count.
    #[test]
    fn the_existing_11_row_profile_reads_as_incomplete_not_ready() {
        // Stand-in ids: the real file ids come from the Mods chat's
        // read-only inventory of his profile.
        let his: Vec<VortexMod> = (0..10).map(|i| vm(&format!("m{i}"), 1000 + i, 5000 + i, true)).chain([vm("ussep439c", 266, 999999, true)]).collect();
        let mut set = ClientSet { collection: None, mods: (0..10).map(|i| e(&format!("m{i}"), 1000 + i, 5000 + i)).collect() };
        set.mods.push(e("ussep", 266, 733846));
        set.mods.push(e("address-library", 32444, 720756));
        set.mods.push(e("mcm-helper", 53000, 746161));
        let st = active(his);
        let got = step(&set, Some(&st));
        assert_eq!(got, Step::Counts { required: 13, installed: 10, enabled: 10, waiting: vec!["USSEP".into(), "ADDRESS-LIBRARY".into(), "MCM-HELPER".into()], other_versions_on: vec!["ussep439c".into()] });
        assert!(!got.ok());
        // No collection is asked for while none is pinned.
        assert!(!matches!(got, Step::CollectionNotAdded | Step::WrongRevision { .. }));
        // An empty client set is never "ready".
        assert!(!step(&ClientSet::default(), Some(&st)).ok());
    }

    #[test]
    fn reads_the_extensions_status_answer() {
        let json = r#"{"ok":true,"activeProfile":{"id":"p1","name":"Aetherial Dawn"},"aetherialProfiles":1,"profile":{"id":"p1","name":"Aetherial Dawn","active":true},
            "mods":[{"id":"u","state":"installed","nexusModId":266,"nexusFileId":733846,"enabled":true,"installerChoices":null}],
            "collections":[{"id":"c","state":"installed","enabled":true,"slug":"adcol","revision":2}]}"#;
        let s: Status = serde_json::from_str(json).unwrap();
        assert_eq!((s.aetherial_profiles, s.mods[0].nexus_file_id, s.collections[0].revision), (1, Some(733846), Some(2)));
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
        let list = vec![e("ussep", 266, 733846), e("skyui", 12604, 35407), ModEntry { id: "github-one".into(), ..Default::default() }];
        // Timothy's profile today: 4.3.9c on, SkyUI on.
        let mut st = Status { profile: Some(Profile { id: "p1".into(), name: "Aetherial Dawn".into(), active: true }), mods: vec![vm("u439c", 266, 999999, true), vm("skyui", 12604, 35407, true)], ..Default::default() };
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

    fn fake_vortex() -> (tempfile::TempDir, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let plugins = plugins_dir(t.path());
        std::fs::create_dir_all(t.path().join("Vortex")).unwrap();
        (t, plugins)
    }

    #[test]
    fn the_carried_extension_is_the_read_only_one_with_a_version() {
        let info = EXTENSION.iter().find(|(n, _)| *n == "info.json").unwrap().1;
        assert!(info_version(info).is_some());
        let jobs = std::str::from_utf8(EXTENSION.iter().find(|(n, _)| *n == "jobs.js").unwrap().1).unwrap();
        assert!(jobs.contains("VERBS = ['status']") || jobs.contains("VERBS=['status']"), "the extension answers status only");
    }

    #[test]
    fn installs_once_then_reads_as_current() {
        let (_t, plugins) = fake_vortex();
        assert_eq!(install_extension(&plugins, EXTENSION).unwrap(), Installed::Fresh);
        for (n, b) in EXTENSION {
            assert_eq!(std::fs::read(plugins.join(EXT_DIR).join(n)).unwrap(), *b);
        }
        assert_eq!(install_extension(&plugins, EXTENSION).unwrap(), Installed::Current);
        assert!(!plugins.parent().unwrap().join("aetherial-dawn-extension.staging").exists());
    }

    #[test]
    fn an_update_replaces_its_own_files_and_keeps_others() {
        let (_t, plugins) = fake_vortex();
        let dir = plugins.join(EXT_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("info.json"), r#"{"version":"0.1.0"}"#).unwrap();
        std::fs::write(dir.join("index.js"), "old").unwrap();
        std::fs::write(dir.join("notes.txt"), "mine").unwrap();
        std::fs::create_dir_all(plugins.parent().unwrap().join("aetherial-dawn-extension.staging")).unwrap();
        assert_eq!(install_extension(&plugins, EXTENSION).unwrap(), Installed::Updated { from: Some("0.1.0".into()) });
        assert_eq!(std::fs::read_to_string(dir.join("notes.txt")).unwrap(), "mine");
        assert_eq!(install_extension(&plugins, EXTENSION).unwrap(), Installed::Current);
    }

    #[test]
    fn a_later_version_from_a_newer_launcher_is_kept() {
        let (_t, plugins) = fake_vortex();
        let dir = plugins.join(EXT_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("info.json"), r#"{"version":"0.10.0"}"#).unwrap();
        assert_eq!(install_extension(&plugins, EXTENSION).unwrap(), Installed::NewerKept { installed: "0.10.0".into() });
        assert!(!dir.join("index.js").exists());
    }

    #[test]
    fn without_vortex_nothing_is_written() {
        let t = tempfile::tempdir().unwrap();
        let plugins = plugins_dir(t.path());
        assert!(install_extension(&plugins, EXTENSION).is_err());
        assert!(!t.path().join("Vortex").exists());
        let (_t, plugins) = fake_vortex();
        assert!(install_extension(&plugins, &[("../x.js", b"x"), ("info.json", br#"{"version":"1"}"#)]).is_err());
        assert!(!plugins.exists());
    }
}
