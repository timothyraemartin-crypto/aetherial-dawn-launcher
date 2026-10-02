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

use crate::allowlist::VortexFile;
use crate::modlist::{safe_rel, ModEntry};
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
    /// The version of the extension Vortex has loaded, from its own
    /// info.json. Missing from extensions before 0.2.1.
    #[serde(default, rename = "extensionVersion")]
    pub extension_version: Option<String>,
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
    /// Vortex's staging folder for this package. Deployment records use
    /// this exact string as their `source`, rather than the Nexus mod id.
    #[serde(default, rename = "installationPath")]
    pub installation_path: Option<String>,
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
    CollectionNotReady,
    WrongRevision { has: Option<u64>, needs: u64 },
    /// Counts of the required packages, from Vortex's own state only (never
    /// the launcher's direct-to-Data ledger): installed (state "installed")
    /// and switched on in the profile, with unresolved mods named and any
    /// ambiguous or wrong version still switched on.
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
            Step::CollectionNotReady => "Collection: install and switch on the Aetherial Dawn collection in Vortex".into(),
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
        if !mine.iter().any(|x| x.revision == Some(c.revision) && x.state.as_deref() == Some("installed") && x.enabled) {
            return Step::CollectionNotReady;
        }
    }
    let installed_state = |m: &VortexMod| m.state.as_deref() == Some("installed");
    let mut required = 0;
    let (mut installed, mut enabled) = (0, 0);
    let mut waiting = Vec::new();
    for e in &set.mods {
        let Some(n) = &e.nexus else { continue };
        required += 1;
        let found: Vec<&VortexMod> = st.mods.iter().filter(|m| m.nexus_mod_id == Some(n.mod_id)
            && n.file.is_none_or(|file| m.nexus_file_id == Some(file)) && installed_state(m)).collect();
        if !found.is_empty() {
            installed += 1;
        }
        let on = st.mods.iter().filter(|m| m.nexus_mod_id == Some(n.mod_id) && m.enabled).collect::<Vec<_>>();
        let enabled_exact = if n.file.is_some() {
            found.iter().any(|m| m.enabled)
        } else {
            on.len() == 1 && installed_state(on[0])
        };
        if enabled_exact {
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

/// Checks the list against Vortex's own state. Pinned entries need the exact
/// Nexus mod and file id; unpinned Nexus entries need exactly one enabled,
/// installed package for that mod id. Non-Nexus entries are the launcher's.
pub fn membership(list: &[ModEntry], status: &Status) -> Membership {
    let profile_active = status.profile.as_ref().is_some_and(|p| p.active);
    let installed = |m: &VortexMod| m.state.as_deref() == Some("installed");
    let mut missing = Vec::new();
    let mut other_versions_on = Vec::new();
    for e in list {
        let Some(n) = &e.nexus else { continue };
        let on: Vec<&VortexMod> = status.mods.iter().filter(|m| m.nexus_mod_id == Some(n.mod_id) && m.enabled).collect();
        let exact = match n.file {
            Some(file) => on.iter().any(|m| m.nexus_file_id == Some(file) && installed(m)),
            None => on.len() == 1 && installed(on[0]),
        };
        if !exact {
            missing.push(e.id.clone());
        }
        match n.file {
            Some(file) => for m in &on {
                if m.nexus_file_id != Some(file)
                    && !(male_face_selected(list) && n.mod_id == 22487 && m.nexus_file_id == Some(104828))
                    && !list.iter().any(|o| o.nexus.as_ref().is_some_and(|on| on.mod_id == n.mod_id && on.file == m.nexus_file_id))
                    && !other_versions_on.contains(&m.id) {
                    other_versions_on.push(m.id.clone());
                }
            },
            None if on.len() > 1 => for m in on {
                if !other_versions_on.contains(&m.id) { other_versions_on.push(m.id.clone()); }
            },
            None => {}
            }
    }
    Membership { profile_active, missing, other_versions_on }
}

fn male_face_selected(list: &[ModEntry]) -> bool {
    list.iter().any(|e| e.id == "community-overlays-1-male-face"
        && e.nexus.as_ref().is_some_and(|n| n.mod_id == 22487 && n.file == Some(104868))
        && e.check == crate::modlist::selected_male_face_checks() && e.owns.is_empty())
        && !list.iter().any(|e| e.id == "community-overlays-1-female-face")
}

/// The selected male face archive is required to deploy. The female archive
/// can remain installed as an optional alternative, never as proof of a
/// deployed file or as a substitute for the selected archive.
pub fn female_face_alternative_installed(list: &[ModEntry], status: &Status) -> bool {
    male_face_selected(list) && status.mods.iter().any(|m| m.nexus_mod_id == Some(22487)
        && m.nexus_file_id == Some(104828) && m.state.as_deref() == Some("installed"))
}

const COMMUNITY_OVERLAYS_BSA: &str = "Data/CommunityOverlays1_0T30.bsa";

fn menu_framework_support_check(entry: &ModEntry, check: &str) -> bool {
    entry.id == "menu-framework"
        && entry.nexus.as_ref().is_some_and(|n| n.mod_id == 120352 && n.file == Some(806684))
        && ["Data/SKSE/Plugins/SKSEMenuFrameworkStrings*.json",
            "Data/SKSE/Plugins/fonts/*.ttf",
            "Data/SKSE/Plugins/SKSEMenuFrameworkThemes/*.json"]
            .iter().any(|p| check.eq_ignore_ascii_case(p))
}

fn bugfix_replaces_main_archive(list: &[ModEntry], entry: &ModEntry, check: &str,
    status: &Status, files: &[VortexFile], game_dir: &Path) -> bool {
    if entry.id != "community-overlays-1"
        || !entry.nexus.as_ref().is_some_and(|n| n.mod_id == 22487 && n.file == Some(77988))
        || !check.eq_ignore_ascii_case(COMMUNITY_OVERLAYS_BSA)
        || !list.iter().any(|m| m.id == "community-overlays-1-fix"
            && m.nexus.as_ref().is_some_and(|n| n.mod_id == 22487 && n.file == Some(79615))
            && m.check.iter().any(|c| c.eq_ignore_ascii_case(COMMUNITY_OVERLAYS_BSA))
            && m.owns.iter().any(|c| c.eq_ignore_ascii_case(COMMUNITY_OVERLAYS_BSA)))
        || !game_dir.join(COMMUNITY_OVERLAYS_BSA).is_file()
    {
        return false;
    }
    status.mods.iter().filter(|m| m.nexus_mod_id == Some(22487)
        && m.nexus_file_id == Some(79615) && m.enabled
        && m.state.as_deref() == Some("installed"))
        .filter_map(|m| m.installation_path.as_deref().filter(|s| !s.is_empty()))
        .any(|source| files.iter().any(|f| f.source == source && f.rel.eq_ignore_ascii_case(COMMUNITY_OVERLAYS_BSA)))
}

/// Names of Nexus mods whose enabled Vortex package is not proven to supply
/// its required game files. A pinned mod needs its exact Nexus file id; an
/// unpinned mod needs exactly one enabled installed package for its mod id.
/// The deployment record's `source` must be that package's exact Vortex
/// `installationPath`. An absent path, record, or check file fails closed.
/// A package with no Data checks still needs one current file from its source.
pub fn missing_deployment(list: &[ModEntry], status: &Status, files: &[VortexFile], game_dir: &Path) -> Vec<String> {
    fn matches_check(check: &str, rel: &str, directory: bool) -> bool {
        let (c, r) = (check.replace('\\', "/").to_ascii_lowercase(), rel.replace('\\', "/").to_ascii_lowercase());
        if let Some((head, tail)) = c.split_once('*') {
            return !tail.contains('*') && !tail.contains('/') && r.len() >= head.len() + tail.len()
                && r.starts_with(head) && r.ends_with(tail) && !r[head.len()..r.len() - tail.len()].contains('/');
        }
        c == r || (directory && r.starts_with(&(c + "/")))
    }

    list.iter().filter_map(|e| {
        let n = e.nexus.as_ref()?;
        let checks: Vec<&String> = e.check.iter().filter(|c| c.replace('\\', "/").to_ascii_lowercase().starts_with("data/")).collect();
        let enabled: Vec<&VortexMod> = status.mods.iter().filter(|m| m.nexus_mod_id == Some(n.mod_id) && m.enabled).collect();
        let valid = status.profile.as_ref().is_some_and(|p| p.active)
            && checks.iter().all(|c| safe_rel(c).is_some())
            && (n.file.is_some() || enabled.len() == 1)
            && enabled.iter().any(|m| {
                let Some(source) = m.installation_path.as_deref().filter(|s| !s.is_empty()) else { return false };
                n.file.is_none_or(|file| m.nexus_file_id == Some(file))
                    && m.state.as_deref() == Some("installed")
                    && files.iter().any(|f| f.source == source && safe_rel(&f.rel).is_some_and(|rel| game_dir.join(rel).is_file()))
                    && checks.iter().all(|check| {
                        // This exact Vortex archive supplies only its DLL.
                        // Existing support files still have to be physically
                        // present, but cannot be attributed to this source.
                        if menu_framework_support_check(e, check) {
                            return e.clone_with_check(check).game_files_present(game_dir);
                        }
                        let check_path = game_dir.join(safe_rel(check).expect("checked above"));
                        files.iter().filter(|f| f.source == source).any(|f| {
                            safe_rel(&f.rel).is_some_and(|rel| {
                                let path = game_dir.join(rel);
                                path.is_file() && matches_check(check, &f.rel, check_path.is_dir())
                            })
                        }) || bugfix_replaces_main_archive(list, e, check, status, files, game_dir)
                    })
            });
        (!valid).then(|| e.name.clone())
    }).collect()
}

/// The one entry's deployment answer with its narrowly defined replacement
/// context. Requirements and Play must use the same source check.
pub fn deployment_ready_for(list: &[ModEntry], entry: &ModEntry, status: &Status,
    files: &[VortexFile], game_dir: &Path) -> bool {
    if entry.nexus.is_none() { return true; }
    let mut context = vec![entry.clone()];
    if entry.id == "community-overlays-1" {
        if let Some(fix) = list.iter().find(|e| e.id == "community-overlays-1-fix") {
            context.push(fix.clone());
        }
    }
    missing_deployment(&context, status, files, game_dir).is_empty()
}

/// A listed mod's three Vortex answers, each on its own (Codex 5910357069,
/// 5917004813): its package is installed in Vortex, switched on in the
/// Aetherial Dawn profile, and its files are deployed into the game from
/// that package. None means unknown: the extension hasn't answered, or the
/// mod isn't a Nexus mod Vortex would hold. Play still goes by
/// `missing_deployment`; this only says which step is missing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct PackageStates {
    pub installed: Option<bool>,
    pub enabled: Option<bool>,
    pub deployed: Option<bool>,
}

pub fn package_states(list: &[ModEntry], entry: &ModEntry, status: Option<&Status>, files: &[VortexFile], game_dir: &Path) -> PackageStates {
    let (Some(n), Some(status)) = (entry.nexus.as_ref(), status) else { return PackageStates::default() };
    // Without exactly one Aetherial Dawn profile, active, Vortex's answers
    // aren't about the server's mods: every step is unknown.
    if status.aetherial_profiles != 1 || !status.profile.as_ref().is_some_and(|p| p.active) {
        return PackageStates::default();
    }
    let installed = |m: &&VortexMod| m.nexus_mod_id == Some(n.mod_id) && m.state.as_deref() == Some("installed")
        && n.file.is_none_or(|f| m.nexus_file_id == Some(f));
    let packages: Vec<&VortexMod> = status.mods.iter().filter(installed).collect();
    let on = packages.iter().filter(|m| m.enabled).count();
    // A pinned file is only "switched on" alone: another file of the same
    // mod on beside it (one the list doesn't pin itself) blocks Play too.
    let other_on = n.file.is_some_and(|file| status.mods.iter().any(|m| m.nexus_mod_id == Some(n.mod_id) && m.enabled
        && m.nexus_file_id != Some(file)
        && !(male_face_selected(list) && n.mod_id == 22487 && m.nexus_file_id == Some(104828))
        && !list.iter().any(|o| o.nexus.as_ref().is_some_and(|x| x.mod_id == n.mod_id && x.file == m.nexus_file_id))));
    PackageStates {
        installed: Some(!packages.is_empty()),
        enabled: Some(if n.file.is_some() { on > 0 && !other_on } else { on == 1 }),
        deployed: Some(deployment_ready_for(list, entry, status, files, game_dir)),
    }
}

/// Exact deployment sources of the required, currently enabled packages.
/// The caller saves this only after the whole profile and deployment gate
/// passes, so tidying can keep checkless packages with opaque folder names.
pub fn approved(list: &[ModEntry], status: &Status, files: &[VortexFile], game_dir: &Path) -> Vec<crate::allowlist::Approved> {
    if status.aetherial_profiles != 1 || !status.profile.as_ref().is_some_and(|p| p.active) {
        return Vec::new();
    }
    list.iter().filter_map(|entry| {
        let nexus = entry.nexus.as_ref()?;
        let enabled: Vec<&VortexMod> = status.mods.iter().filter(|m| m.nexus_mod_id == Some(nexus.mod_id)
            && m.enabled && m.state.as_deref() == Some("installed")).collect();
        let deploys = |m: &VortexMod| {
            let mut one = status.clone();
            one.mods = vec![m.clone()];
            if entry.id == "community-overlays-1" {
                one.mods.extend(status.mods.iter().filter(|v| v.nexus_mod_id == Some(22487)
                    && v.nexus_file_id == Some(79615)).cloned());
            }
            deployment_ready_for(list, entry, &one, files, game_dir)
        };
        let package = match nexus.file {
            Some(file) => enabled.into_iter().find(|m| m.nexus_file_id == Some(file) && deploys(m))?,
            None if enabled.len() == 1 && deploys(enabled[0]) => enabled[0],
            None => return None,
        };
        Some(crate::allowlist::Approved {
            vortex_id: package.installation_path.clone()?,
            nexus_mod_id: nexus.mod_id,
            nexus_file_id: package.nexus_file_id,
        })
    }).collect()
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

/// The marker written last into a complete staged version, and kept in the
/// live folder: the version and each of the extension's files with its
/// sha256. A folder whose files don't match its marker is not a whole version.
pub const EXT_MARKER: &str = ".aetherial-dawn-complete";

fn staging_dir(vortex: &Path) -> PathBuf {
    vortex.join("aetherial-dawn-extension.staging")
}

fn previous_dir(vortex: &Path) -> PathBuf {
    vortex.join("aetherial-dawn-extension.previous")
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

#[derive(Debug, Serialize, Deserialize)]
struct Marker {
    version: String,
    files: std::collections::BTreeMap<String, String>,
}

/// The version a folder holds when every file its marker names is there
/// with the marker's sha256; `None` for a missing, partial or mixed folder.
fn whole_version(dir: &Path) -> Option<String> {
    let marker: Marker = serde_json::from_slice(&std::fs::read(dir.join(EXT_MARKER)).ok()?).ok()?;
    if marker.files.is_empty() || !marker.files.contains_key("info.json") {
        return None;
    }
    let whole = marker.files.iter().all(|(name, sum)| std::fs::read(dir.join(name)).is_ok_and(|b| sha256_hex(&b) == *sum));
    whole.then_some(marker.version)
}

/// What Vortex would load from its plugins folder right now.
#[derive(Debug, Clone, PartialEq)]
pub enum ExtensionState {
    /// No extension folder.
    Absent,
    /// Exactly the files this launcher carries.
    Current,
    /// A whole version, but not this launcher's (older or newer).
    Other { version: String },
    /// Files that don't make one whole version: never pair with it.
    Mixed,
}

/// Reads, without changing anything, what is in `plugins/aetherial-dawn`.
pub fn extension_state(plugins: &Path, files: &[(&str, &[u8])]) -> ExtensionState {
    let dir = plugins.join(EXT_DIR);
    if !dir.exists() {
        return ExtensionState::Absent;
    }
    match whole_version(&dir) {
        Some(_) if files.iter().all(|(n, b)| std::fs::read(dir.join(n)).is_ok_and(|have| have == *b)) => ExtensionState::Current,
        Some(version) => ExtensionState::Other { version },
        None => ExtensionState::Mixed,
    }
}

/// The version this launcher carries.
pub fn bundled_version(files: &[(&str, &[u8])]) -> Option<String> {
    files.iter().find(|(n, _)| *n == "info.json").and_then(|(_, b)| info_version(b))
}

/// Whether the extension answering is the one this launcher put in Vortex.
/// Vortex keeps running the version it loaded at start, so after an update
/// its answers aren't trusted until Vortex is restarted.
pub fn loaded_is_current(status: &Status, files: &[(&str, &[u8])]) -> bool {
    status.extension_version.is_some() && status.extension_version == bundled_version(files)
}

/// Whether installing would write anything: the folder is missing, mixed,
/// or holds an older or different whole version. A later whole version is
/// kept and needs no write.
pub fn needs_write(plugins: &Path, files: &[(&str, &[u8])]) -> bool {
    match extension_state(plugins, files) {
        ExtensionState::Current => false,
        ExtensionState::Other { version } => {
            bundled_version(files).is_none_or(|b| version_parts(&version) <= version_parts(&b))
        }
        ExtensionState::Absent | ExtensionState::Mixed => true,
    }
}

/// The steps of an update, so a test can stop it after any one of them.
#[derive(Debug, Clone, Copy, PartialEq)]
enum UpdateStep {
    Staged,
    Marked,
    LiveMovedAside,
    Activated,
}

/// Finishes or undoes an update a crash or power cut interrupted. Runs
/// before every install. A complete staged version with no live folder is
/// activated; otherwise the previous live folder is put back; leftovers go.
fn recover(vortex: &Path, dir: &Path) -> Result<()> {
    let staging = staging_dir(vortex);
    let previous = previous_dir(vortex);
    if !dir.exists() && (staging.exists() || previous.exists()) {
        if let Some(plugins) = dir.parent() {
            std::fs::create_dir_all(plugins)?;
        }
        if staging.exists() && whole_version(&staging).is_some() {
            std::fs::rename(&staging, dir)?;
        } else if previous.exists() {
            std::fs::rename(&previous, dir)?;
        }
    }
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    if previous.exists() && dir.exists() {
        std::fs::remove_dir_all(&previous)?;
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Puts the extension into `plugins/aetherial-dawn` as one whole version.
///
/// The new version is built in full outside Vortex's plugins folder, with
/// any files in the live folder that aren't the extension's, and a marker
/// holding every file's sha256 is written last. It then goes live by two
/// folder renames: the old folder is moved aside, the new one moved in.
/// Between the two Vortex finds no extension, never a mix of two versions,
/// and the next run finishes or undoes an interrupted update (`recover`).
/// A whole later version put there by a newer launcher is kept; a folder
/// that isn't one whole version is replaced.
pub fn install_extension(plugins: &Path, files: &[(&str, &[u8])]) -> Result<Installed> {
    install_extension_stopping(plugins, files, &mut |_| Ok(()))
}

fn install_extension_stopping(plugins: &Path, files: &[(&str, &[u8])], after: &mut dyn FnMut(UpdateStep) -> Result<()>) -> Result<Installed> {
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
    recover(vortex, &dir)?;
    let installed = std::fs::read(dir.join("info.json")).ok().and_then(|b| info_version(&b));
    match extension_state(plugins, files) {
        ExtensionState::Current => return Ok(Installed::Current),
        ExtensionState::Other { version } if version_parts(&version) > version_parts(&bundled) => {
            return Ok(Installed::NewerKept { installed: version });
        }
        _ => {}
    }

    let staging = staging_dir(vortex);
    let fresh = !dir.exists();
    let ours = |n: &str| n == EXT_MARKER || files.iter().any(|(f, _)| *f == n);
    std::fs::create_dir_all(&staging)?;
    if !fresh {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            if ours(&entry.file_name().to_string_lossy()) {
                continue;
            }
            let target = staging.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                copy_tree(&entry.path(), &target)?;
            } else {
                std::fs::copy(entry.path(), &target)?;
            }
        }
    }
    for (name, bytes) in files {
        std::fs::write(staging.join(name), bytes)?;
    }
    after(UpdateStep::Staged)?;
    let marker = Marker { version: bundled, files: files.iter().map(|(n, b)| (n.to_string(), sha256_hex(b))).collect() };
    let body = serde_json::to_vec_pretty(&marker).map_err(|e| Error::Game(e.to_string()))?;
    {
        use std::io::Write;
        let mut f = std::fs::File::create(staging.join(EXT_MARKER))?;
        f.write_all(&body)?;
        f.sync_all()?;
    }
    after(UpdateStep::Marked)?;

    std::fs::create_dir_all(plugins)?;
    let previous = previous_dir(vortex);
    if !fresh {
        std::fs::rename(&dir, &previous)?;
    }
    after(UpdateStep::LiveMovedAside)?;
    if let Err(e) = std::fs::rename(&staging, &dir) {
        if !fresh {
            let _ = std::fs::rename(&previous, &dir);
        }
        return Err(e.into());
    }
    after(UpdateStep::Activated)?;
    if previous.exists() {
        std::fs::remove_dir_all(&previous)?;
    }
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

    fn unpinned(id: &str, mod_id: u64) -> ModEntry {
        let mut entry = e(id, mod_id, 0);
        entry.nexus.as_mut().unwrap().file = None;
        entry
    }

    fn vm(id: &str, m: u64, f: u64, on: bool) -> VortexMod {
        VortexMod { id: id.into(), installation_path: Some(format!("{id}-folder")), state: Some("installed".into()), nexus_mod_id: Some(m), nexus_file_id: Some(f), enabled: on }
    }

    fn active(mods: Vec<VortexMod>) -> Status {
        Status {
            profile: Some(Profile { id: "p1".into(), name: "Aetherial Dawn".into(), active: true }),
            aetherial_profiles: 1,
            active_profile: Some(ActiveProfile { id: "p1".into(), name: Some("Aetherial Dawn".into()) }),
            mods,
            collections: vec![],
            extension_version: None,
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
        assert_eq!(with(&|s| s.collections[0].enabled = false), Step::CollectionNotReady);
        assert_eq!(with(&|s| s.collections[0].state = Some("installing".into())), Step::CollectionNotReady);
        // Downloading: SkyUI not there yet.
        let dl = with(&|s| { s.mods.pop(); });
        assert_eq!(dl, Step::Counts { required: 2, installed: 1, enabled: 1, waiting: vec!["SKYUI".into()], other_versions_on: vec![] });
        assert_eq!(dl.describe(), "Aetherial Dawn profile: 1 of 2 installed · 1 of 2 switched on · waiting: SKYUI");
        // Installed but off.
        assert_eq!(with(&|s| s.mods[1].enabled = false), Step::Counts { required: 2, installed: 2, enabled: 1, waiting: vec!["SKYUI".into()], other_versions_on: vec![] });
        // Mid-install doesn't count as installed.
        assert_eq!(with(&|s| s.mods[1].state = Some("installing".into())), Step::Counts { required: 2, installed: 1, enabled: 1, waiting: vec!["SKYUI".into()], other_versions_on: vec![] });
        // An absent state is unknown, not proof of installation.
        assert_eq!(with(&|s| s.mods[1].state = None), Step::Counts { required: 2, installed: 1, enabled: 1, waiting: vec!["SKYUI".into()], other_versions_on: vec![] });
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
            "mods":[{"id":"u","installationPath":"USSEP 4.3.8a folder","state":"installed","nexusModId":266,"nexusFileId":733846,"enabled":true,"installerChoices":null}],
            "collections":[{"id":"c","state":"installed","enabled":true,"slug":"adcol","revision":2}]}"#;
        let s: Status = serde_json::from_str(json).unwrap();
        assert_eq!((s.aetherial_profiles, s.mods[0].nexus_file_id, s.collections[0].revision), (1, Some(733846), Some(2)));
        assert_eq!(s.mods[0].installation_path.as_deref(), Some("USSEP 4.3.8a folder"));
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
        st.mods[2].state = None;
        assert_eq!(membership(&list, &st).missing, ["ussep"], "unknown install state is not installed");
    }

    #[test]
    fn a_different_file_of_a_built_in_cannot_pass_play_even_when_deployed() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("Data/SKSE/Plugins/MCMHelper.dll");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"present").unwrap();
        let entry = crate::modlist::builtin(Some("1.6.1170.0"))
            .into_iter().find(|m| m.id == "mcm-helper").unwrap();
        let set = ClientSet { collection: None, mods: vec![entry] };
        let files = [VortexFile { rel: "Data/SKSE/Plugins/MCMHelper.dll".into(), source: "mcm-folder".into() }];
        let mut status = active(vec![vm("mcm", 53000, 795510, true)]);
        assert!(step(&set, Some(&status)).ok());
        assert!(missing_deployment(&set.mods, &status, &files, t.path()).is_empty());

        status.mods[0].nexus_file_id = Some(795511);
        assert!(!step(&set, Some(&status)).ok());
        assert_eq!(missing_deployment(&set.mods, &status, &files, t.path()), ["MCM Helper"]);
    }

    #[test]
    fn approved_sources_bind_an_unpinned_checkless_package_to_current_deployment() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("Data/textures/required.dds");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"texture").unwrap();
        let list = vec![unpinned("texture", 42)];
        let mut status = active(vec![vm("internal-id", 42, 73, true)]);
        status.mods[0].installation_path = Some("opaque-staging-folder".into());
        let files = [VortexFile { rel: "Data/textures/required.dds".into(), source: "opaque-staging-folder".into() }];
        let sources = approved(&list, &status, &files, t.path());
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].vortex_id, "opaque-staging-folder");
        assert_eq!(sources[0].nexus_file_id, Some(73));

        status.mods[0].enabled = false;
        assert!(approved(&list, &status, &files, t.path()).is_empty());
        status.mods[0].enabled = true;
        let stale = [VortexFile { rel: "Data/textures/required.dds".into(), source: "old-folder".into() }];
        assert!(approved(&list, &status, &stale, t.path()).is_empty());
        status.mods.push(vm("other-file", 42, 74, true));
        assert!(approved(&list, &status, &files, t.path()).is_empty(), "an unpinned mod needs one enabled package");

        // A pinned file can appear twice under different staging folders.
        // The approval must name the one actually deployed, not the first
        // enabled match in Vortex's state array.
        let pinned = vec![e("texture", 42, 73)];
        status.mods[1].nexus_file_id = Some(73);
        status.mods[1].installation_path = Some("old-undeloyed-folder".into());
        status.mods.swap(0, 1);
        let exact = approved(&pinned, &status, &files, t.path());
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].vortex_id, "opaque-staging-folder");
    }

    #[test]
    fn deployment_binds_the_enabled_pinned_file_to_its_exact_vortex_source() {
        let t = tempfile::tempdir().unwrap();
        let rel = "Data/Unofficial Skyrim Special Edition Patch.esp";
        let target = t.path().join(rel);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"plugin").unwrap();
        let mut pinned = e("ussep", 266, 733846);
        pinned.check = vec![rel.into()];
        let list = vec![pinned];
        let mut status = active(vec![vm("u438a", 266, 733846, true), vm("u439c", 266, 999999, false)]);
        status.mods[0].installation_path = Some("USSEP 4.3.8a folder".into());
        status.mods[1].installation_path = Some("USSEP 4.3.9c folder".into());
        let mut deployed = vec![VortexFile { rel: rel.into(), source: "USSEP 4.3.9c folder".into() }];
        let missing = |status: &Status, files: &[VortexFile]| missing_deployment(&list, status, files, t.path());

        assert_eq!(missing(&status, &deployed), ["USSEP"], "an older file of the same Nexus mod is not the pinned file");
        deployed[0].source = "USSEP 4.3.8a folder".into();
        assert!(missing(&status, &deployed).is_empty(), "the exact enabled file, source and present check path agree");
        status.mods[0].enabled = false;
        assert_eq!(missing(&status, &deployed), ["USSEP"]);
        status.mods[0].enabled = true;
        status.mods[0].nexus_file_id = Some(999999);
        assert_eq!(missing(&status, &deployed), ["USSEP"]);
        status.mods[0].nexus_file_id = Some(733846);
        status.mods[0].installation_path = None;
        assert_eq!(missing(&status, &deployed), ["USSEP"], "an old extension without installationPath fails closed");
        status.mods[0].installation_path = Some("USSEP 4.3.8a folder".into());
        std::fs::remove_file(target).unwrap();
        assert_eq!(missing(&status, &deployed), ["USSEP"], "a stale deployment record is not enough without the actual file");
    }

    #[test]
    fn community_overlays_bugfix_may_replace_only_the_main_bsa_and_male_face_is_selected() {
        let t = tempfile::tempdir().unwrap();
        let put = |rel: &str| {
            let path = t.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"present").unwrap();
        };
        let esp = "Data/CommunityOverlays1_0T30.esp";
        let bsa = "Data/CommunityOverlays1_0T30.bsa";
        for rel in [esp, bsa] { put(rel); }
        let mut main = e("community-overlays-1", 22487, 77988);
        main.check = vec![esp.into(), bsa.into()];
        let mut fix = e("community-overlays-1-fix", 22487, 79615);
        fix.check = vec![bsa.into()];
        fix.owns = vec![bsa.into()];
        let female = e("community-overlays-1-female-face", 22487, 104828);
        let male = e("community-overlays-1-male-face", 22487, 104868);
        let list = crate::modlist::play_required(vec![main, fix, female, male]);
        assert_eq!(list.len(), 3);
        let face_checks = list.iter().find(|e| e.id == "community-overlays-1-male-face").unwrap().check.clone();
        assert_eq!(face_checks.len(), 25);
        for rel in &face_checks { put(rel); }
        let set = ClientSet { collection: None, mods: list.clone() };
        let mut status = active(vec![vm("main", 22487, 77988, true), vm("fix", 22487, 79615, true),
            vm("female", 22487, 104828, true), vm("male", 22487, 104868, true)]);
        let mut files = vec![VortexFile { rel: esp.into(), source: "main-folder".into() },
            VortexFile { rel: bsa.into(), source: "fix-folder".into() }];
        files.extend(face_checks.iter().map(|rel| VortexFile { rel: rel.clone(), source: "male-folder".into() }));
        assert!(step(&set, Some(&status)).ok(), "the enabled female archive is a reported alternative");
        assert!(female_face_alternative_installed(&list, &status));
        assert!(missing_deployment(&list, &status, &files, t.path()).is_empty());
        assert_eq!(approved(&list, &status, &files, t.path()).len(), 3);

        files[1].source = "unrelated-folder".into();
        assert_eq!(missing_deployment(&list, &status, &files, t.path()), ["COMMUNITY-OVERLAYS-1", "COMMUNITY-OVERLAYS-1-FIX"]);
        files[1].source = "fix-folder".into();
        status.mods[1].enabled = false;
        assert!(!missing_deployment(&list, &status, &files, t.path()).is_empty(), "the exact bugfix must be active");
        status.mods[1].enabled = true;
        status.mods[1].nexus_file_id = Some(79616);
        assert!(!missing_deployment(&list, &status, &files, t.path()).is_empty(), "another bugfix file is not approved");
        status.mods[1].nexus_file_id = Some(79615);
        std::fs::remove_file(t.path().join(esp)).unwrap();
        assert!(!missing_deployment(&list, &status, &files, t.path()).is_empty(), "the main ESP still has to deploy");
        put(esp);
        files[2].source = "female-folder".into();
        assert_eq!(missing_deployment(&list, &status, &files, t.path()), ["COMMUNITY-OVERLAYS-1-MALE-FACE"]);
        files[2].source = "male-folder".into();
        std::fs::remove_file(t.path().join(&face_checks[0])).unwrap();
        assert_eq!(missing_deployment(&list, &status, &files, t.path()), ["COMMUNITY-OVERLAYS-1-MALE-FACE"]);
        put(&face_checks[0]);
        status.mods[3].nexus_file_id = Some(104869);
        assert!(!step(&set, Some(&status)).ok(), "the selected male file ID must remain exact");
    }

    #[test]
    fn menu_framework_support_files_are_physical_checks_not_attributed_to_its_dll_package() {
        let t = tempfile::tempdir().unwrap();
        let entry = crate::modlist::builtin(Some("1.6.1170.0"))
            .into_iter().find(|e| e.id == "menu-framework").unwrap();
        let dll = "Data/SKSE/Plugins/SKSEMenuFramework.dll";
        for rel in [dll, "Data/SKSE/Plugins/SKSEMenuFrameworkStrings_EN.json",
            "Data/SKSE/Plugins/fonts/face.ttf", "Data/SKSE/Plugins/SKSEMenuFrameworkThemes/dark.json"] {
            let path = t.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"present").unwrap();
        }
        let list = vec![entry.clone()];
        let mut status = active(vec![vm("menu", 120352, 806684, true)]);
        let files = [VortexFile { rel: dll.into(), source: "menu-folder".into() }];
        assert!(entry.game_files_present(t.path()));
        assert!(missing_deployment(&list, &status, &files, t.path()).is_empty());
        std::fs::remove_file(t.path().join("Data/SKSE/Plugins/fonts/face.ttf")).unwrap();
        assert_eq!(missing_deployment(&list, &status, &files, t.path()), ["SKSE Menu Framework"]);
        status.mods[0].nexus_file_id = Some(806685);
        assert!(!missing_deployment(&list, &status, &files, t.path()).is_empty());
    }

    #[test]
    fn pinned_mod_without_checks_needs_a_current_file_from_its_exact_source() {
        let t = tempfile::tempdir().unwrap();
        let rel = "Data/Textures/Example.dds";
        let target = t.path().join(rel);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"texture").unwrap();
        let list = vec![e("textures", 1234, 5678)];
        let status = active(vec![vm("textures", 1234, 5678, true)]);
        let from = |source: &str| vec![VortexFile { rel: rel.into(), source: source.into() }];

        assert_eq!(missing_deployment(&list, &status, &[], t.path()), ["TEXTURES"]);
        assert_eq!(missing_deployment(&list, &status, &from("other-file-of-mod-1234"), t.path()), ["TEXTURES"]);
        assert!(missing_deployment(&list, &status, &from("textures-folder"), t.path()).is_empty());
        std::fs::remove_file(target).unwrap();
        assert_eq!(missing_deployment(&list, &status, &from("textures-folder"), t.path()), ["TEXTURES"], "the deployment record cannot stand in for a missing file");
    }

    #[test]
    fn unpinned_nexus_mod_needs_one_installed_package_switched_on() {
        let list = vec![unpinned("skyui", 12604)];
        let set = ClientSet { collection: None, mods: list.clone() };
        let mut status = active(vec![vm("skyui-a", 12604, 101, true)]);
        assert_eq!(step(&set, Some(&status)), Step::Ready);
        assert!(membership(&list, &status).ready());

        status.mods.push(vm("skyui-b", 12604, 102, true));
        let ambiguous = step(&set, Some(&status));
        assert!(!ambiguous.ok());
        assert!(ambiguous.describe().contains("another version still on: skyui-a, skyui-b"), "{}", ambiguous.describe());
        assert_eq!(membership(&list, &status).missing, ["skyui"]);

        status.mods[1].enabled = false;
        assert_eq!(step(&set, Some(&status)), Step::Ready);
        status.mods[0].state = None;
        assert_eq!(step(&set, Some(&status)), Step::Counts { required: 1, installed: 1, enabled: 0, waiting: vec!["SKYUI".into()], other_versions_on: vec![] });
        assert_eq!(membership(&list, &status).missing, ["skyui"]);
    }

    #[test]
    fn unpinned_deployment_needs_the_unique_enabled_package_source_and_live_file() {
        let t = tempfile::tempdir().unwrap();
        let rel = "Data/Interface/SkyUI.esp";
        let target = t.path().join(rel);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, b"plugin").unwrap();
        let mut entry = unpinned("skyui", 12604);
        entry.check = vec![rel.into()];
        let mut list = vec![entry];
        let mut status = active(vec![vm("skyui-a", 12604, 101, true)]);
        let from = |source: &str| vec![VortexFile { rel: rel.into(), source: source.into() }];

        assert_eq!(missing_deployment(&list, &status, &from("skyui-b-folder"), t.path()), ["SKYUI"]);
        assert!(missing_deployment(&list, &status, &from("skyui-a-folder"), t.path()).is_empty());
        status.mods.push(vm("skyui-b", 12604, 102, true));
        assert_eq!(missing_deployment(&list, &status, &from("skyui-a-folder"), t.path()), ["SKYUI"], "two enabled files are ambiguous");
        status.mods[1].enabled = false;
        list[0].check.clear();
        assert!(missing_deployment(&list, &status, &from("skyui-a-folder"), t.path()).is_empty(), "an unpinned package without checks still needs a live source file");
        std::fs::remove_file(target).unwrap();
        assert_eq!(missing_deployment(&list, &status, &from("skyui-a-folder"), t.path()), ["SKYUI"]);
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

    /// Writes a whole extension version into `dir`, marker included, the way
    /// a launcher carrying `files` would have left it.
    fn put_whole(dir: &Path, files: &[(&str, &[u8])]) {
        std::fs::create_dir_all(dir).unwrap();
        let mut sums = std::collections::BTreeMap::new();
        for (n, b) in files {
            std::fs::write(dir.join(n), b).unwrap();
            sums.insert(n.to_string(), sha256_hex(b));
        }
        let info = files.iter().find(|(n, _)| *n == "info.json").unwrap().1;
        let marker = Marker { version: info_version(info).unwrap(), files: sums };
        std::fs::write(dir.join(EXT_MARKER), serde_json::to_vec(&marker).unwrap()).unwrap();
    }

    const OLD: &[(&str, &[u8])] = &[
        ("index.js", b"old index"),
        ("jobs.js", b"old jobs"),
        ("info.json", br#"{"version":"0.1.0"}"#),
    ];

    #[test]
    fn a_later_version_from_a_newer_launcher_is_kept() {
        let (_t, plugins) = fake_vortex();
        let dir = plugins.join(EXT_DIR);
        put_whole(&dir, &[("index.js", b"new"), ("jobs.js", b"new"), ("info.json", br#"{"version":"0.10.0"}"#)]);
        assert_eq!(install_extension(&plugins, EXTENSION).unwrap(), Installed::NewerKept { installed: "0.10.0".into() });
        assert_eq!(std::fs::read(dir.join("index.js")).unwrap(), b"new");
    }

    #[test]
    fn a_later_version_number_on_a_mixed_folder_is_not_kept() {
        let (_t, plugins) = fake_vortex();
        let dir = plugins.join(EXT_DIR);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("info.json"), r#"{"version":"0.10.0"}"#).unwrap();
        assert_eq!(extension_state(&plugins, EXTENSION), ExtensionState::Mixed);
        assert!(matches!(install_extension(&plugins, EXTENSION).unwrap(), Installed::Updated { .. }));
        assert_eq!(extension_state(&plugins, EXTENSION), ExtensionState::Current);
    }

    #[test]
    fn a_mixed_old_and_new_set_reads_as_mixed_never_current() {
        let (_t, plugins) = fake_vortex();
        let dir = plugins.join(EXT_DIR);
        assert_eq!(extension_state(&plugins, EXTENSION), ExtensionState::Absent);
        put_whole(&dir, OLD);
        assert_eq!(extension_state(&plugins, EXTENSION), ExtensionState::Other { version: "0.1.0".into() });
        // What the old per-file renames could leave: the new index.js beside
        // the old jobs.js and info.json.
        let new_index = EXTENSION.iter().find(|(n, _)| *n == "index.js").unwrap().1;
        std::fs::write(dir.join("index.js"), new_index).unwrap();
        assert_eq!(extension_state(&plugins, EXTENSION), ExtensionState::Mixed);
    }

    /// An update stopped after each of its steps (a crash or power cut),
    /// with Vortex started before the next run: Vortex finds the old whole
    /// version or nothing, never a mix, and the next run finishes the job
    /// and keeps the player's own file.
    #[test]
    fn an_interrupted_update_never_leaves_a_mixed_extension() {
        for stop in [UpdateStep::Staged, UpdateStep::Marked, UpdateStep::LiveMovedAside, UpdateStep::Activated] {
            let (_t, plugins) = fake_vortex();
            let dir = plugins.join(EXT_DIR);
            put_whole(&dir, OLD);
            std::fs::write(dir.join("notes.txt"), "mine").unwrap();
            let crashed = install_extension_stopping(&plugins, EXTENSION, &mut |s| {
                if s == stop { Err(Error::Game("power cut".into())) } else { Ok(()) }
            });
            assert!(crashed.is_err(), "{stop:?}");
            let seen = extension_state(&plugins, EXTENSION);
            assert!(
                matches!(seen, ExtensionState::Absent | ExtensionState::Current | ExtensionState::Other { .. }),
                "after {stop:?} Vortex would load {seen:?}"
            );
            if let ExtensionState::Other { version } = &seen {
                assert_eq!(version, "0.1.0", "{stop:?}");
            }
            let again = install_extension(&plugins, EXTENSION).unwrap();
            assert!(matches!(again, Installed::Updated { .. } | Installed::Current), "{stop:?}: {again:?}");
            assert_eq!(extension_state(&plugins, EXTENSION), ExtensionState::Current, "{stop:?}");
            assert_eq!(std::fs::read_to_string(dir.join("notes.txt")).unwrap(), "mine", "{stop:?}");
            let vortex = plugins.parent().unwrap();
            assert!(!staging_dir(vortex).exists() && !previous_dir(vortex).exists(), "{stop:?}");
        }
    }

    #[test]
    fn a_complete_staged_version_with_no_live_folder_is_activated() {
        let (_t, plugins) = fake_vortex();
        let vortex = plugins.parent().unwrap();
        put_whole(&staging_dir(vortex), EXTENSION);
        put_whole(&previous_dir(vortex), OLD);
        assert_eq!(install_extension(&plugins, EXTENSION).unwrap(), Installed::Current);
        assert!(!previous_dir(vortex).exists());
    }

    #[test]
    fn a_partial_staged_version_is_dropped_and_the_old_one_put_back() {
        let (_t, plugins) = fake_vortex();
        let vortex = plugins.parent().unwrap();
        std::fs::create_dir_all(staging_dir(vortex)).unwrap();
        std::fs::write(staging_dir(vortex).join("index.js"), "half").unwrap();
        put_whole(&previous_dir(vortex), OLD);
        recover(vortex, &plugins.join(EXT_DIR)).unwrap();
        assert_eq!(extension_state(&plugins, EXTENSION), ExtensionState::Other { version: "0.1.0".into() });
        assert!(!staging_dir(vortex).exists());
    }

    #[test]
    fn only_the_carried_version_answering_is_trusted() {
        let mut s: Status = serde_json::from_str(r#"{"mods":[]}"#).unwrap();
        assert!(!loaded_is_current(&s, EXTENSION), "an extension before 0.2.1 doesn't say its version");
        s.extension_version = Some("0.2.0".into());
        assert!(!loaded_is_current(&s, EXTENSION));
        s.extension_version = bundled_version(EXTENSION);
        assert!(loaded_is_current(&s, EXTENSION));
    }

    #[test]
    fn a_write_is_needed_only_when_the_folder_isnt_this_or_a_later_whole_version() {
        let (_t, plugins) = fake_vortex();
        let dir = plugins.join(EXT_DIR);
        assert!(needs_write(&plugins, EXTENSION));
        put_whole(&dir, OLD);
        assert!(needs_write(&plugins, EXTENSION));
        install_extension(&plugins, EXTENSION).unwrap();
        assert!(!needs_write(&plugins, EXTENSION));
        std::fs::remove_dir_all(&dir).unwrap();
        put_whole(&dir, &[("index.js", b"n"), ("jobs.js", b"n"), ("info.json", br#"{"version":"9.0.0"}"#)]);
        assert!(!needs_write(&plugins, EXTENSION));
        std::fs::write(dir.join("jobs.js"), "changed").unwrap();
        assert!(needs_write(&plugins, EXTENSION));
    }

    #[test]
    fn a_players_folders_inside_the_extension_are_kept_on_update() {
        let (_t, plugins) = fake_vortex();
        let dir = plugins.join(EXT_DIR);
        put_whole(&dir, OLD);
        std::fs::create_dir_all(dir.join("logs")).unwrap();
        std::fs::write(dir.join("logs").join("a.txt"), "kept").unwrap();
        install_extension(&plugins, EXTENSION).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("logs").join("a.txt")).unwrap(), "kept");
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

    #[test]
    fn installed_switched_on_and_deployed_are_three_answers() {
        let t = tempfile::tempdir().unwrap();
        let entry = ModEntry { id: "u".into(), name: "USSEP".into(), nexus: Some(NexusRef { mod_id: 266, file: Some(733846), ..Default::default() }), check: vec!["Data/U.esp".into()], ..Default::default() };
        let list = vec![entry.clone()];
        let pkg = |file: u64, on: bool| VortexMod { id: format!("v{file}"), installation_path: Some(format!("USSEP-{file}")), state: Some("installed".into()), nexus_mod_id: Some(266), nexus_file_id: Some(file), enabled: on };
        let mut status = Status { profile: Some(Profile { id: "p".into(), name: "Aetherial Dawn".into(), active: true }), aetherial_profiles: 1, ..Default::default() };
        let states = |status: &Status, files: &[VortexFile]| package_states(&list, &entry, Some(status), files, t.path());
        // Without an answer from Vortex every step is unknown, never "no".
        assert_eq!(package_states(&list, &entry, None, &[], t.path()), PackageStates::default());
        // Another file of the same mod doesn't count as the pinned one.
        status.mods = vec![pkg(1, true)];
        assert_eq!(states(&status, &[]), PackageStates { installed: Some(false), enabled: Some(false), deployed: Some(false) });
        status.mods.push(pkg(733846, false));
        assert_eq!(states(&status, &[]), PackageStates { installed: Some(true), enabled: Some(false), deployed: Some(false) });
        status.mods[1].enabled = true;
        // The pinned file on beside another file of the mod isn't "switched
        // on": Play is blocked until the other one is off.
        assert_eq!(states(&status, &[]), PackageStates { installed: Some(true), enabled: Some(false), deployed: Some(false) });
        status.mods[0].enabled = false;
        assert_eq!(states(&status, &[]), PackageStates { installed: Some(true), enabled: Some(true), deployed: Some(false) });
        std::fs::create_dir_all(t.path().join("Data")).unwrap();
        std::fs::write(t.path().join("Data/U.esp"), b"x").unwrap();
        let files = [VortexFile { rel: "Data/U.esp".into(), source: "USSEP-733846".into() }];
        assert_eq!(states(&status, &files), PackageStates { installed: Some(true), enabled: Some(true), deployed: Some(true) });
        // No single active Aetherial Dawn profile: unknown, never yes or no.
        for (count, active) in [(0, true), (2, true), (1, false)] {
            let mut odd = status.clone();
            odd.aetherial_profiles = count;
            odd.profile.as_mut().unwrap().active = active;
            assert_eq!(states(&odd, &files), PackageStates::default(), "{count} profiles, active {active}");
        }
        // A launcher-only mod has no Vortex answers.
        let direct = ModEntry { id: "d".into(), name: "Direct".into(), ..Default::default() };
        assert_eq!(package_states(&list, &direct, Some(&status), &files, t.path()), PackageStates::default());
    }
}
