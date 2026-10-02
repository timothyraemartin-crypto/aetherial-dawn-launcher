//! The server-mods export (staging runbook A1, 2026-09-27): the plugins the
//! test server loads come from Nexus through the owner's own Premium, the
//! same files every player's launcher installs. The launcher downloads the
//! server's list into its own folder (never the game folder), keeps the
//! plugins and zips them to server-lane.zip. Nothing here uploads anything:
//! the zip stays on the PC until someone carries it over.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::modlist::{self, ModEntry};
use crate::{Error, Result};

/// The served list, `<base_url>/server-lane.json`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ServerLane {
    /// Only the launcher signed in with this Discord account exports.
    pub for_discord_id: String,
    pub mods: Vec<LaneMod>,
    /// The server's masters before the lane's plugins, when the list widens
    /// them past the five base masters (world-1.5 adds five Creation Club
    /// masters). It must start with the five base masters in order and names
    /// only full plugins (run names). Left out, it is the five base masters,
    /// and the list hash is unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub masters: Option<Vec<String>>,
    /// Masters the game has only as light plugins, run as "<stem>.esm"
    /// copies (desync/esl-on-server.md): `from` "_ResourcePack.esl", `run`
    /// "_ResourcePack.esm". The same field as server-plugins-order.
    #[serde(default, rename = "masterSources", skip_serializing_if = "Option::is_none")]
    pub master_sources: Option<Vec<MasterSource>>,
    /// ESL-flagged .esp/.esm lane plugins the list runs as flag-cleared
    /// copies. Any other light plugin stops the export, as check_masters.py.
    #[serde(default, rename = "lightCleared", skip_serializing_if = "Option::is_none")]
    pub light_cleared: Option<Vec<String>>,
    /// A list-wide light rule. "convert-all" (the order file may add a
    /// description after it) runs every ESL-flagged .esp/.esm lane plugin as
    /// its flag-cleared copy, as if each were in lightCleared; an .esl lane
    /// plugin is still refused. Same test as check_masters.py.
    #[serde(default, rename = "lightPolicy", skip_serializing_if = "Option::is_none")]
    pub light_policy: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct MasterSource {
    /// The master's name in the server's masters, "_ResourcePack.esm".
    pub run: String,
    /// The game's file, "_ResourcePack.esl".
    pub from: String,
}

/// The lightPolicy that runs every ESL-flagged lane plugin as a full one.
pub const CONVERT_ALL: &str = "convert-all";

/// The most full plugins a PC can load (indexes 00-FD).
pub const MAX_FULL: usize = 254;

impl ServerLane {
    /// The master contract: the list's own, or the five base masters.
    pub fn masters(&self) -> Vec<String> {
        self.masters.clone().unwrap_or_else(|| crate::health::MASTERS.iter().map(|m| m.to_string()).collect())
    }

    /// The light plugins this list runs as full plugins: its masterSources'
    /// `from` files.
    pub fn light_as_full(&self) -> Vec<String> {
        self.master_sources.iter().flatten().map(|s| s.from.clone()).collect()
    }

    /// The game file each master past the base five comes from: its
    /// masterSources `from`, or the master itself. (file, master name)
    pub fn master_files(&self) -> Vec<(String, String)> {
        self.masters()
            .into_iter()
            .skip(crate::health::MASTERS.len())
            .map(|m| (self.master_sources.iter().flatten().find(|s| s.run.eq_ignore_ascii_case(&m)).map(|s| s.from.clone()).unwrap_or_else(|| m.clone()), m))
            .collect()
    }

    /// True when the list's lightPolicy is convert-all.
    pub fn convert_all(&self) -> bool {
        self.light_policy.as_deref().is_some_and(|p| p.starts_with(CONVERT_ALL))
    }

    fn cleared(&self, plugin: &str) -> bool {
        self.convert_all() || self.light_cleared.iter().flatten().any(|p| p.eq_ignore_ascii_case(plugin))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct LaneMod {
    #[serde(flatten)]
    pub entry: ModEntry,
    /// The plugins to take from this download, by file name. Empty takes
    /// every plugin the install would put at the top of Data. Named ones
    /// are found anywhere in the archive (a patch collection's options),
    /// the installer's own pick first.
    #[serde(default)]
    pub plugins: Vec<String>,
    /// The download's size in bytes, when the list pins it (with the
    /// entry's `sha256`).
    #[serde(default)]
    pub size: Option<u64>,
    /// Plugin name -> its exact path in the archive, for downloads whose
    /// installer options hold several files with that name. A listed path
    /// is taken exactly or the mod is refused; it never falls back.
    #[serde(default)]
    pub paths: BTreeMap<String, String>,
    /// What the list expects the download to be (name, version, size as
    /// Nexus lists it). Logged beside what came, not enforced: `size` and
    /// `sha256` are the pins.
    #[serde(default)]
    pub archive: Option<ArchiveInfo>,
    /// Plugins this download's install gives every PC that load after the
    /// server's order and never on the server (OCW_AO_FEPatch.esp masters
    /// a client-only mod). The export never collects them. Left out, the
    /// list hash is unchanged.
    #[serde(default, rename = "clientOnly", skip_serializing_if = "Vec::is_empty")]
    pub client_only: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct ArchiveInfo {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default, rename = "sizeBytes")]
    pub size_bytes: Option<u64>,
}

/// A download slower than this on average (after its first two minutes)
/// is stopped, so a trickle that beats the stall timeout can't hold the
/// export forever.
pub const MIN_RATE: u64 = 16 * 1024;
const RATE_GRACE: std::time::Duration = std::time::Duration::from_secs(120);

/// True when `got` bytes in `took` is below MIN_RATE, after the grace time.
pub fn too_slow(got: u64, took: std::time::Duration) -> bool {
    took > RATE_GRACE && (got as u128) < MIN_RATE as u128 * took.as_millis() / 1000
}

/// Checks a downloaded archive against the list's size and sha256.
pub fn verify(m: &LaneMod, archive: &Path) -> Result<()> {
    if let Some(want) = m.size {
        let got = std::fs::metadata(archive)?.len();
        if got != want {
            return Err(Error::Game(format!("{} downloaded {got} bytes, the list says {want}", m.entry.name)));
        }
    }
    modlist::verify(&m.entry, archive)
}

/// What an export left, `server-lane/export.json`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Record {
    /// `list_hash` of the list it was made from.
    pub list: String,
    /// Plugin name -> the mod it came from.
    pub plugins: BTreeMap<String, String>,
    /// SHA-256 of server-lane.zip.
    pub zip_sha256: String,
    pub zip_bytes: u64,
    /// Per-file receipt: plugin name -> its sha256 and size in the zip.
    #[serde(default)]
    pub files: BTreeMap<String, FileReceipt>,
    /// Where the list came from: the served server-lane.json, or the local
    /// override (then with the sha256 of override.json's exact bytes).
    #[serde(default)]
    pub source: Source,
    /// The light plugins this list runs as full "<stem>.esm" plugins, for
    /// `canonical-plugins --esl-as-esm` on the server.
    #[serde(default)]
    pub light_as_full: Vec<String>,
    /// The masters past the base five: names and the hashes each PC's
    /// converted copy must match. Their bytes are never in the zip.
    #[serde(default)]
    pub masters: Vec<MasterReceipt>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Source {
    /// "served" or "local-override".
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub override_sha256: Option<String>,
}

impl Source {
    pub fn served() -> Source {
        Source { kind: "served".into(), override_sha256: None }
    }
    pub fn local(sha256: &str) -> Source {
        Source { kind: "local-override".into(), override_sha256: Some(sha256.to_string()) }
    }
}

/// The test-only local list (PR #7, Codex 5875976813): `override.json` in
/// the export folder with `override.sha256` beside it holding the sha256 of
/// its exact bytes. Both files present turns it on; it is never fetched and
/// never served. Its runs keep their own state in `OVERRIDE_RUN`, apart from
/// the served list's.
pub const OVERRIDE_LIST: &str = "override.json";
pub const OVERRIDE_SHA: &str = "override.sha256";
pub const OVERRIDE_RUN: &str = "override-run";

/// The local override list and the sha256 of its bytes, or None when there
/// is none. Fails closed: one file without the other, a sha256 line that
/// doesn't match the bytes, or a list that doesn't read is an error, never a
/// fall back to the served list. The bytes are read once, hashed and parsed
/// from that same read.
pub fn local_override(root: &Path) -> Result<Option<(ServerLane, String)>> {
    use sha2::{Digest, Sha256};
    let (list, sha) = (root.join(OVERRIDE_LIST), root.join(OVERRIDE_SHA));
    match (list.exists(), sha.exists()) {
        (false, false) => return Ok(None),
        (true, false) => return Err(Error::Game(format!("{OVERRIDE_LIST} is there without {OVERRIDE_SHA}; the export won't run until both are there or both are gone"))),
        (false, true) => return Err(Error::Game(format!("{OVERRIDE_SHA} is there without {OVERRIDE_LIST}; the export won't run until both are there or both are gone"))),
        (true, true) => {}
    }
    let bytes = std::fs::read(&list)?;
    let got = format!("{:x}", Sha256::digest(&bytes));
    let want = std::fs::read_to_string(&sha)?.split_whitespace().next().unwrap_or_default().to_ascii_lowercase();
    if want != got {
        return Err(Error::Game(format!("{OVERRIDE_LIST} has sha256 {got}, not the {} in {OVERRIDE_SHA}; the export won't run", if want.is_empty() { "(empty)" } else { &want })));
    }
    let lane: ServerLane = serde_json::from_slice(&bytes).map_err(|e| Error::Game(format!("{OVERRIDE_LIST} doesn't read: {e}")))?;
    Ok(Some((lane, got)))
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct FileReceipt {
    pub sha256: String,
    pub bytes: u64,
    /// Its index in the server's load order: the list's masters first (the
    /// five base masters are 00-04), then the lane's plugins in list order.
    #[serde(default)]
    pub index: usize,
    /// The masters its TES4 header names, in header order.
    #[serde(default)]
    pub masters: Vec<String>,
    /// Whether its header carries the light (ESL) flag.
    #[serde(default)]
    pub light: bool,
    /// The name the server and every PC load it as (aliases.rs): a dashed
    /// alias, "<name>-AD.esp", "<stem>.esm" for a listed .esl, or its own.
    #[serde(default)]
    pub run_name: String,
    /// sha256 of its canonical bytes (masters renamed, light flag cleared),
    /// what the server's copy and every PC's copy hold.
    #[serde(default)]
    pub canonical_sha256: String,
    /// Size and CRC-32 of the canonical bytes, what SkyMP's load-order
    /// check compares (name, size, crc32) on every PC.
    #[serde(default)]
    pub canonical_bytes: u64,
    #[serde(default)]
    pub canonical_crc32: u32,
}

/// One mod's outcome in an export run, kept in `report.json` whether the
/// run finished or not, so a failed export says which mod failed first and
/// why without the launcher log (PR #7: 53 of 70 plugins, no zip).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct ModOutcome {
    pub id: String,
    pub name: String,
    /// The plugins the list declares for it.
    pub declared: Vec<String>,
    /// The plugins it gave.
    pub gave: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Report {
    pub list: String,
    pub declared: usize,
    pub collected: usize,
    /// (mod id, plugin) declared but not collected, in list order.
    pub missing: Vec<(String, String)>,
    /// Plugins collected that the list doesn't declare.
    pub extra: Vec<String>,
    /// The first mod that failed, with its reason.
    pub first_failure: Option<String>,
    pub mods: Vec<ModOutcome>,
    /// Collected plugins whose header masters the server couldn't load
    /// (see `master_problems`).
    #[serde(default)]
    pub master_problems: Vec<String>,
}

const REPORT: &str = "report.json";

/// Declared plugins against what `<root>/Data` holds and what was recorded.
pub fn reconcile(lane: &ServerLane, plugins: &BTreeMap<String, String>) -> (Vec<(String, String)>, Vec<String>) {
    let mut missing = Vec::new();
    for m in &lane.mods {
        for p in &m.plugins {
            if !plugins.keys().any(|k| k.eq_ignore_ascii_case(p)) {
                missing.push((m.entry.id.clone(), p.clone()));
            }
        }
    }
    let extra = plugins.keys().filter(|k| !lane.mods.iter().any(|m| m.plugins.iter().any(|p| p.eq_ignore_ascii_case(k)))).cloned().collect();
    (missing, extra)
}

/// The lane's plugins in load order (list order), each with its mod id.
fn order(lane: &ServerLane) -> Vec<(&str, &str)> {
    lane.mods.iter().flat_map(|m| m.plugins.iter().map(move |p| (m.entry.id.as_str(), p.as_str()))).collect()
}

/// Every collected plugin in `data`, and each master past the base five,
/// checked against what the server can load, the same rules as the Mods
/// chat's check_masters.py (desync/esl-on-server.md, 2026-09-30):
/// - a light plugin runs as a full one only where the list says so: a
///   master's .esl through masterSources ("<stem>.esm"), an ESL-flagged
///   .esp/.esm through lightCleared (its flag-cleared copy). Any other light
///   plugin stops the export, as does lightCleared on a Creation Club name,
///   which the "-AD" rule leaves light;
/// - each master a header names must be one of the server's masters (a
///   masterSources `from` counts as its `run`) or a lane plugin loaded
///   earlier; a Creation Club master nobody lists, a master listed later,
///   or a file with no readable header is named (PR #7, Codex 5916681442);
/// - at most MAX_FULL full plugins.
///
/// Files not collected are skipped: `reconcile` and `master_receipts` name
/// them.
pub fn master_problems(lane: &ServerLane, data: &Path) -> Vec<String> {
    master_problems_with(lane, data, None)
}

/// `master_problems`, with the masters past the base five read from
/// `game_data` (the exporting PC's Data) when given.
pub fn master_problems_with(lane: &ServerLane, data: &Path, game_data: Option<&Path>) -> Vec<String> {
    let all = order(lane);
    let contract = lane.masters();
    let mut out = Vec::new();
    if contract.len() + all.len() > MAX_FULL {
        out.push(format!("the list loads {} full plugins ({} masters and {} plugins), more than the {MAX_FULL} a PC can", contract.len() + all.len(), contract.len(), all.len()));
    }
    let sources = lane.master_sources.clone().unwrap_or_default();
    let base = crate::health::MASTERS.len();
    let in_contract = |m: &str, upto: usize| {
        contract[..upto].iter().any(|b| b.eq_ignore_ascii_case(m)) || sources.iter().any(|s| s.from.eq_ignore_ascii_case(m) && contract[..upto].iter().any(|b| b.eq_ignore_ascii_case(&s.run)))
    };
    let headers = |name: &str, path: &Path, ok: &dyn Fn(&str) -> bool, later: &dyn Fn(&str) -> bool, out: &mut Vec<String>| match crate::loadorder::masters(path) {
        None => out.push(format!("{name} has no readable plugin header")),
        Some(ms) => {
            for m in ms {
                if ok(&m) {
                    continue;
                }
                if later(&m) {
                    out.push(format!("{name} needs {m}, which the list loads after it"));
                } else {
                    out.push(format!("{name} needs {m}, which isn't one of the server's masters or in the list"));
                }
            }
        }
    };
    // The masters past the base five, from the game's files.
    for (k, (file, master)) in lane.master_files().iter().enumerate() {
        let Some(path) = game_data.and_then(|g| find_file(g, file)) else { continue };
        let esl = file.to_ascii_lowercase().ends_with(".esl");
        if esl != sources.iter().any(|s| s.from.eq_ignore_ascii_case(file)) || (!esl && light_flag(&path)) {
            out.push(format!("{file} (the server's master {master}) is a light plugin no masterSources entry runs as a full one"));
        }
        let at = base + k;
        headers(file, &path, &|m| in_contract(m, at), &|m| in_contract(m, contract.len()), &mut out);
    }
    let mut before: Vec<&str> = Vec::new();
    for (i, (_, name)) in all.iter().enumerate() {
        let path = data.join(name);
        if path.is_file() {
            let esl = name.to_ascii_lowercase().ends_with(".esl");
            if esl {
                out.push(format!("{name} is a light plugin (.esl); a lane plugin can't be one"));
            } else if light_flag(&path) {
                if !lane.cleared(name) {
                    out.push(format!("{name} is ESL-flagged, and the list doesn't declare it lightCleared"));
                } else if crate::aliases::shipped_with_game(name) {
                    out.push(format!("{name} is a light (ESL) plugin with a Creation Club name, which no rule runs as a full plugin"));
                }
            }
            headers(name, &path, &|m| in_contract(m, contract.len()) || before.iter().any(|b| b.eq_ignore_ascii_case(m)), &|m| all[i + 1..].iter().any(|(_, l)| l.eq_ignore_ascii_case(m)), &mut out);
        }
        before.push(name);
    }
    out
}

/// A file at the top of `dir`, matched without regard to case, under its
/// name as it's spelled on disk (Windows would also open the list's
/// spelling, and the receipt should name the real file).
fn find_file(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().filter(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(name)).map(|e| e.path()).find(|p| p.is_file())
}

/// What the export records for a master past the base five. The export
/// never carries its bytes (Creation Club content is Bethesda's): each PC
/// converts its own copy by the rule, and these are the names and hashes it
/// must arrive at (Quality checks, 2026-09-30).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct MasterReceipt {
    /// The game's file, "_ResourcePack.esl".
    pub file: String,
    /// The name the server and every PC load it as, "_ResourcePack.esm".
    pub run_name: String,
    /// Its index in the server's load order.
    pub index: usize,
    /// sha256 of the game's file as read on the exporting PC.
    pub sha256: String,
    /// sha256 of the converted bytes every PC's copy must match.
    pub canonical_sha256: String,
    /// Size and CRC-32 of the canonical bytes, what SkyMP's load-order
    /// check compares (name, size, crc32) on every PC.
    #[serde(default)]
    pub canonical_bytes: u64,
    #[serde(default)]
    pub canonical_crc32: u32,
}

/// The receipts for the masters past the base five, read from `game_data`
/// (the exporting PC's own Data). A missing file stops the export.
pub fn master_receipts(lane: &ServerLane, game_data: &Path) -> Result<Vec<MasterReceipt>> {
    use sha2::{Digest, Sha256};
    let full = lane.light_as_full();
    let mut out = Vec::new();
    for (k, (file, master)) in lane.master_files().into_iter().enumerate() {
        let path = find_file(game_data, &file).ok_or_else(|| Error::Game(format!("the game folder has no {file}, which the server's master {master} comes from")))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(file);
        let b = std::fs::read(&path)?;
        let (run_name, canon) = crate::aliases::canonical_full(game_data, &name, &b, &full);
        if !run_name.eq_ignore_ascii_case(&master) {
            return Err(Error::Game(format!("{name} would load as {run_name}, not as the server's master {master}")));
        }
        out.push(MasterReceipt {
            file: name,
            run_name,
            index: crate::health::MASTERS.len() + k,
            sha256: format!("{:x}", Sha256::digest(&b)),
            canonical_sha256: format!("{:x}", Sha256::digest(canon.as_deref().unwrap_or(&b))),
            canonical_bytes: canon.as_deref().unwrap_or(&b).len() as u64,
            canonical_crc32: crate::serverorder::crc32_bytes(canon.as_deref().unwrap_or(&b)),
        });
    }
    Ok(out)
}

/// Whether a plugin's TES4 header carries the light (ESL) flag.
fn light_flag(path: &Path) -> bool {
    use std::io::Read;
    let mut h = [0u8; 12];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut h)).is_ok() && &h[..4] == b"TES4" && u32::from_le_bytes([h[8], h[9], h[10], h[11]]) & 0x200 != 0
}

/// Writes `report.json` for this run and returns it.
pub fn report(root: &Path, lane: &ServerLane, hash: &str, plugins: &BTreeMap<String, String>, mods: Vec<ModOutcome>) -> Result<Report> {
    report_with(root, lane, hash, plugins, mods, None)
}

/// `report`, with the masters past the base five read from `game_data`.
pub fn report_with(root: &Path, lane: &ServerLane, hash: &str, plugins: &BTreeMap<String, String>, mods: Vec<ModOutcome>, game_data: Option<&Path>) -> Result<Report> {
    let (missing, extra) = reconcile(lane, plugins);
    let rep = Report {
        list: hash.to_string(),
        declared: lane.mods.iter().map(|m| m.plugins.len()).sum(),
        collected: plugins.len(),
        missing,
        extra,
        first_failure: mods.iter().find_map(|m| m.error.as_ref().map(|e| format!("{}: {e}", m.name))),
        mods,
        master_problems: master_problems_with(lane, &root.join("Data"), game_data),
    };
    std::fs::create_dir_all(root)?;
    std::fs::write(root.join(REPORT), serde_json::to_vec_pretty(&rep)?)?;
    Ok(rep)
}

pub const LANE_DIR: &str = "server-lane";
pub const ZIP_NAME: &str = "server-lane.zip";
const RECORD: &str = "export.json";
const FAILURES: &str = "failures.json";
/// Failed exports of one list before the launcher stops trying until the
/// list changes.
pub const MAX_FAILURES: u32 = 2;

#[derive(Debug, Default, Deserialize, Serialize)]
struct Failures {
    list: String,
    count: u32,
}

fn failures(root: &Path) -> Failures {
    std::fs::read(root.join(FAILURES)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// True when this list already failed MAX_FAILURES times.
pub fn gave_up(root: &Path, hash: &str) -> bool {
    let f = failures(root);
    f.list == hash && f.count >= MAX_FAILURES
}

/// Records one failed export of this list; returns how many so far.
pub fn failed(root: &Path, hash: &str) -> Result<u32> {
    let mut f = failures(root);
    if f.list != hash {
        f = Failures { list: hash.to_string(), count: 0 };
    }
    f.count += 1;
    std::fs::create_dir_all(root)?;
    std::fs::write(root.join(FAILURES), serde_json::to_vec(&f)?)?;
    Ok(f.count)
}

/// A stable fingerprint of the list: a changed list exports again.
pub fn list_hash(lane: &ServerLane) -> String {
    use sha2::{Digest, Sha256};
    let text = serde_json::to_vec(lane).unwrap_or_default();
    format!("{:x}", Sha256::digest(&text))
}

/// Checks the list before anything downloads: every mod pinned to one Nexus
/// file (the server needs exactly the bytes players get), ids usable as
/// file names, no plugin taken twice.
pub fn check(lane: &ServerLane) -> Result<()> {
    if let Some(ms) = &lane.masters {
        let base = crate::health::MASTERS;
        if ms.len() < base.len() || !ms.iter().zip(base.iter()).all(|(a, b)| a.eq_ignore_ascii_case(b)) {
            return Err(Error::Game(format!("server lane: masters must start with {}", base.join(", "))));
        }
        for m in ms {
            if !crate::serverorder::plain_plugin_name(m) {
                return Err(Error::Game(format!("server lane: master {m:?} isn't a plugin file name")));
            }
            if m.to_ascii_lowercase().ends_with(".esl") {
                return Err(Error::Game(format!("server lane: master {m} is a light plugin (.esl); name its .esm run name and add it to masterSources")));
            }
        }
    }
    for src in lane.master_sources.iter().flatten() {
        let from = src.from.to_ascii_lowercase();
        let want = from.strip_suffix(".esl").map(|s| format!("{s}.esm"));
        if src.from.contains(['/', '\\']) || want.as_deref() != Some(src.run.to_ascii_lowercase().as_str()) {
            return Err(Error::Game(format!("server lane: masterSources {} -> {} isn't an .esl run as the .esm of the same stem", src.from, src.run)));
        }
        let ms = lane.masters();
        if !ms.iter().skip(crate::health::MASTERS.len()).any(|m| m.eq_ignore_ascii_case(&src.run)) {
            return Err(Error::Game(format!("server lane: masterSources names {}, which isn't one of its masters", src.run)));
        }
    }
    if let Some(p) = &lane.light_policy {
        if !lane.convert_all() {
            return Err(Error::Game(format!("server lane: lightPolicy {p:?} isn't one the launcher knows (\"{CONVERT_ALL}\")")));
        }
    }
    for p in lane.light_cleared.iter().flatten() {
        if p.to_ascii_lowercase().ends_with(".esl") || !lane.mods.iter().any(|m| m.plugins.iter().any(|x| x.eq_ignore_ascii_case(p))) {
            return Err(Error::Game(format!("server lane: lightCleared names {p}, which isn't an .esp or .esm the list takes")));
        }
    }
    let mut ids = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for m in &lane.mods {
        let e = &m.entry;
        if e.id.is_empty() || !e.id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return Err(Error::Game(format!("server lane: mod id {:?} isn't a plain name", e.id)));
        }
        if !ids.insert(e.id.clone()) {
            return Err(Error::Game(format!("server lane: {} is listed twice", e.id)));
        }
        if e.nexus.as_ref().and_then(|n| n.file).is_none() {
            return Err(Error::Game(format!("server lane: {} isn't pinned to one Nexus file", e.id)));
        }
        for p in &m.plugins {
            if !crate::serverorder::plain_plugin_name(p) {
                return Err(Error::Game(format!("server lane: {} names {p:?}, which isn't a plugin file name", e.id)));
            }
            if !names.insert(p.to_ascii_lowercase()) {
                return Err(Error::Game(format!("server lane: {p} is taken from two mods")));
            }
        }
        for c in &m.client_only {
            if !crate::serverorder::plain_plugin_name(c) {
                return Err(Error::Game(format!("server lane: {} names client-only {c:?}, which isn't a plugin file name", e.id)));
            }
            if lane.mods.iter().any(|o| o.plugins.iter().any(|p| p.eq_ignore_ascii_case(c))) {
                return Err(Error::Game(format!("server lane: {c} is both a server plugin and client-only")));
            }
        }
        for (name, path) in &m.paths {
            if !m.plugins.iter().any(|p| p == name) {
                return Err(Error::Game(format!("server lane: {} gives a path for {name}, which it doesn't list", e.id)));
            }
            let file = path.replace('\\', "/");
            let last = file.rsplit('/').next().unwrap_or("");
            if modlist::safe_rel(&file).is_none() || !last.eq_ignore_ascii_case(name) {
                return Err(Error::Game(format!("server lane: {}'s path {path:?} isn't a safe path ending in {name}", e.id)));
            }
        }
    }
    Ok(())
}

fn is_plugin(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    [".esp", ".esm", ".esl"].iter().any(|x| l.ends_with(x))
}

/// The export's folders under the launcher's own data folder.
pub fn lane_dir(app_data: &Path) -> PathBuf {
    app_data.join(LANE_DIR)
}

/// True when this list was already exported and its zip is still there.
pub fn done(root: &Path, hash: &str) -> bool {
    let Ok(text) = std::fs::read(root.join(RECORD)) else { return false };
    let Ok(rec) = serde_json::from_slice::<Record>(&text) else { return false };
    rec.list == hash && std::fs::metadata(root.join(ZIP_NAME)).map(|m| m.len() == rec.zip_bytes).unwrap_or(false)
}

/// Picks the plugins to keep from one unpacked download, as (source, plugin
/// name). `planned` is what the normal install would copy.
pub fn pick(m: &LaneMod, unpacked: &Path) -> Result<Vec<(PathBuf, String)>> {
    // Named plugins come only from what the installer's plan (with the
    // entry's fomod picks) puts in Data, as a player's launcher installs it,
    // or from a pinned path. The archive is never searched past the plan,
    // so a patch the players' options leave out never reaches the server
    // (PR #7, Codex 5875889405). A plan that fails leaves only pinned paths.
    let (planned, plan_err) = match modlist::plan(&m.entry, unpacked) {
        Ok(p) => (p, None),
        Err(e) if !m.plugins.is_empty() => (Vec::new(), Some(e.to_string())),
        Err(e) => return Err(e),
    };
    let top: Vec<(PathBuf, String)> = planned
        .iter()
        .filter_map(|c| {
            let rel = c.to.to_string_lossy().replace('\\', "/");
            let name = rel.strip_prefix("Data/")?.to_string();
            (!name.contains('/') && is_plugin(&name) && !m.client_only.iter().any(|x| x.eq_ignore_ascii_case(&name))).then(|| (c.from.clone(), name))
        })
        .collect();
    if m.plugins.is_empty() {
        if top.is_empty() {
            return Err(Error::Game(format!("{} has no plugin to take", m.entry.name)));
        }
        return Ok(top);
    }
    let mut out = Vec::new();
    // Every problem in the mod is named, not just the first.
    let mut problems: Vec<String> = Vec::new();
    for want in &m.plugins {
        // A listed path: exactly that file, or the mod is refused.
        if let Some(path) = m.paths.get(want) {
            match at_path(unpacked, path) {
                Some(f) => out.push((f, want.clone())),
                None => problems.push(format!("no {path} in its download")),
            }
            continue;
        }
        if let Some(t) = top.iter().find(|(_, n)| n.eq_ignore_ascii_case(want)) {
            out.push((t.0.clone(), want.clone()));
            continue;
        }
        match &plan_err {
            Some(e) => problems.push(format!("its installer couldn't be followed ({e}), so {want} needs a pinned path")),
            None if !files_named(unpacked, want).is_empty() => problems.push(format!("{want} is in its download but the installer's options don't pick it")),
            None => problems.push(format!("no {want} in its download")),
        }
    }
    if !problems.is_empty() {
        return Err(Error::Game(format!("{}: {}", m.entry.name, problems.join("; "))));
    }
    Ok(out)
}

/// The file at `rel` under `dir`, matching each part's case loosely (the
/// archive's spelling may differ from the list's).
fn at_path(dir: &Path, rel: &str) -> Option<PathBuf> {
    let rel = rel.replace('\\', "/");
    modlist::safe_rel(&rel)?;
    let mut at = dir.to_path_buf();
    for part in rel.split('/').filter(|p| !p.is_empty()) {
        let exact = at.join(part);
        at = if exact.exists() {
            exact
        } else {
            std::fs::read_dir(&at).ok()?.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))?.path()
        };
    }
    std::fs::symlink_metadata(&at).ok().filter(|m| m.is_file()).map(|_| at)
}

/// Every plugin file in an unpacked download, as its path inside the
/// archive ("/" between folders), sorted; at most 200. Logged for a mod
/// listed with no plugins, so its plugin names can be learned from a run.
pub fn plugins_in(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(e.path());
            } else if ft.is_file() && is_plugin(&e.file_name().to_string_lossy()) {
                if let Ok(rel) = e.path().strip_prefix(dir) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    out.sort();
    out.truncate(200);
    out
}

fn files_named(dir: &Path, name: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(e.path());
            } else if ft.is_file() && e.file_name().to_string_lossy().eq_ignore_ascii_case(name) {
                out.push(e.path());
            }
        }
    }
    out.sort();
    out
}

/// Copies the picked plugins into `<root>/Data`, refusing two different
/// files under one name.
pub fn collect(root: &Path, mod_id: &str, picked: &[(PathBuf, String)], seen: &mut BTreeMap<String, String>) -> Result<()> {
    let data = root.join("Data");
    std::fs::create_dir_all(&data)?;
    for (from, name) in picked {
        if let Some((prev_name, prev)) = seen.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            if prev != mod_id {
                return Err(Error::Game(format!("{name} comes from both {prev} and {mod_id} (as {prev_name})")));
            }
        }
        let tmp = data.join(format!("{name}.part"));
        std::fs::copy(from, &tmp)?;
        std::fs::rename(&tmp, data.join(name))?;
        seen.insert(name.clone(), mod_id.to_string());
    }
    Ok(())
}

/// Zips `<root>/Data` to `<root>/server-lane.zip` (entries "Data/<name>",
/// stored: plugins barely compress and the VPS unzips quicker) and writes the
/// record. Returns it.
pub fn finish(root: &Path, lane: &ServerLane, hash: &str, plugins: BTreeMap<String, String>, source: Source) -> Result<Record> {
    finish_with(root, lane, hash, plugins, source, None)
}

/// `finish`, reading the masters past the base five from `game_data` for
/// their receipts. A list with such masters needs it; their bytes never go
/// in the zip.
pub fn finish_with(root: &Path, lane: &ServerLane, hash: &str, plugins: BTreeMap<String, String>, source: Source, game_data: Option<&Path>) -> Result<Record> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    // Never the game's own plugins (base masters, Creation Club content,
    // _ResourcePack): they're Bethesda's, and the zip must never carry them,
    // even by a mistake in the list.
    let licensed: Vec<&str> = plugins.keys().filter(|n| crate::aliases::shipped_with_game(n)).map(String::as_str).collect();
    if !licensed.is_empty() {
        return Err(Error::Game(format!("the export would carry the game's own plugins ({}), which never go in the zip; no zip made", licensed.join(", "))));
    }
    // Never a zip short of what the list declares, or with more.
    let (missing, extra) = reconcile(lane, &plugins);
    if !missing.is_empty() || !extra.is_empty() {
        return Err(Error::Game(format!(
            "the export has {} of {} declared plugins (missing: {}; not declared: {}); no zip made",
            plugins.len() - extra.len(),
            lane.mods.iter().map(|m| m.plugins.len()).sum::<usize>(),
            missing.iter().map(|(m, p)| format!("{p} ({m})")).collect::<Vec<_>>().join(", "),
            extra.join(", ")
        )));
    }
    let data = root.join("Data");
    // Never a zip the server couldn't load: every header master must be a
    // base master or come earlier in the list.
    let bad = master_problems_with(lane, &data, game_data);
    if !bad.is_empty() {
        return Err(Error::Game(format!("the export's plugins name masters the server can't load ({}); no zip made", bad.join("; "))));
    }
    let at: Vec<(&str, &str)> = order(lane);
    let full = lane.light_as_full();
    // The masters past the base five: names and hashes only, never bytes.
    let masters = match (lane.master_files().is_empty(), game_data) {
        (true, _) => Vec::new(),
        (false, Some(g)) => master_receipts(lane, g)?,
        (false, None) => return Err(Error::Game("the server's list has masters past the base five, and the export wasn't given the game folder to check them; no zip made".into())),
    };
    let mut files = BTreeMap::new();
    let names: Vec<(String, usize)> = plugins.keys().map(|n| (n.clone(), lane.masters().len() + at.iter().position(|(_, p)| p.eq_ignore_ascii_case(n)).unwrap_or_default())).collect();
    for (name, index) in &names {
        let (name, index) = (name, *index);
        let path = data.join(name);
        let b = std::fs::read(&path)?;
        let light = b.len() >= 12 && u32::from_le_bytes([b[8], b[9], b[10], b[11]]) & 0x200 != 0;
        let (run_name, canon) = crate::aliases::canonical_full(&data, name, &b, &full);
        let cb = canon.as_deref().unwrap_or(&b);
        let (canonical_sha256, canonical_bytes, canonical_crc32) = (format!("{:x}", Sha256::digest(cb)), cb.len() as u64, crate::serverorder::crc32_bytes(cb));
        files.insert(
            name.clone(),
            FileReceipt { sha256: format!("{:x}", Sha256::digest(&b)), bytes: b.len() as u64, index, masters: crate::loadorder::masters(&path).unwrap_or_default(), light, run_name, canonical_sha256, canonical_bytes, canonical_crc32 },
        );
    }
    let tmp = root.join(format!("{ZIP_NAME}.part"));
    {
        let f = std::fs::File::create(&tmp)?;
        let mut z = zip::ZipWriter::new(std::io::BufWriter::new(f));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).large_file(true);
        for (name, _) in &names {
            z.start_file(format!("Data/{name}"), opts).map_err(|e| Error::Game(format!("zip: {e}")))?;
            let mut src = std::fs::File::open(data.join(name))?;
            std::io::copy(&mut src, &mut z)?;
        }
        z.finish().map_err(|e| Error::Game(format!("zip: {e}")))?.flush()?;
    }
    let zip = root.join(ZIP_NAME);
    std::fs::rename(&tmp, &zip)?;
    let mut h = Sha256::new();
    let mut f = std::fs::File::open(&zip)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut bytes = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        h.update(&buf[..n]);
    }
    let rec = Record { list: hash.to_string(), plugins, zip_sha256: format!("{:x}", h.finalize()), zip_bytes: bytes, files, source, light_as_full: full, masters };
    std::fs::write(root.join(RECORD), serde_json::to_vec_pretty(&rec)?)?;
    let _ = std::fs::remove_file(root.join(FAILURES));
    // The sha256sum line the VPS checks the carried zip against.
    std::fs::write(root.join(format!("{ZIP_NAME}.sha256")), format!("{}  {ZIP_NAME}\n", rec.zip_sha256))?;
    Ok(rec)
}

/// Clears what a previous export left (a changed list starts over), keeping
/// the downloads folder so finished downloads aren't fetched again.
pub fn start_over(root: &Path) -> Result<()> {
    for p in ["Data", "unpacked"] {
        let d = root.join(p);
        if d.exists() {
            std::fs::remove_dir_all(&d)?;
        }
    }
    for f in [RECORD, ZIP_NAME, REPORT] {
        let _ = std::fs::remove_file(root.join(f));
    }
    let _ = std::fs::remove_file(root.join(format!("{ZIP_NAME}.sha256")));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(json: &str) -> ServerLane {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn reads_the_served_list_and_checks_pins() {
        let l = lane(r#"{"_note":"draft","for_discord_id":"1","mods":[{"id":"jks","name":"JK's Skyrim","nexus":{"mod":6289,"file":1},"plugins":["JKs Skyrim.esp"]}]}"#);
        assert_eq!(l.mods[0].plugins, vec!["JKs Skyrim.esp"]);
        assert_eq!(l.mods[0].entry.nexus.as_ref().unwrap().file, Some(1));
        check(&l).unwrap();
        let unpinned = lane(r#"{"for_discord_id":"1","mods":[{"id":"x","name":"X","nexus":{"mod":2}}]}"#);
        assert!(check(&unpinned).unwrap_err().to_string().contains("pinned"));
        let twice = lane(r#"{"for_discord_id":"1","mods":[{"id":"a","name":"A","nexus":{"mod":2,"file":3},"plugins":["P.esp"]},{"id":"b","name":"B","nexus":{"mod":4,"file":5},"plugins":["p.ESP"]}]}"#);
        assert!(check(&twice).is_err());
        let path = lane(r#"{"for_discord_id":"1","mods":[{"id":"a","name":"A","nexus":{"mod":2,"file":3},"plugins":["../x.esp"]}]}"#);
        assert!(check(&path).is_err());
        let bad_id = lane(r#"{"for_discord_id":"1","mods":[{"id":"../a","name":"A","nexus":{"mod":2,"file":3}}]}"#);
        assert!(check(&bad_id).is_err());
    }

    #[test]
    fn the_local_override_needs_both_files_and_matching_bytes() {
        use sha2::{Digest, Sha256};
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        assert!(local_override(root).unwrap().is_none(), "none: the served list is used");
        let body = br#"{"for_discord_id":"1","mods":[{"id":"a","name":"A","nexus":{"mod":2,"file":3},"plugins":["A.esp"]}]}"#;
        let sha = format!("{:x}", Sha256::digest(body));
        std::fs::write(root.join(OVERRIDE_LIST), body).unwrap();
        assert!(local_override(root).unwrap_err().to_string().contains("without override.sha256"));
        // A sha256sum line, any case, is accepted.
        std::fs::write(root.join(OVERRIDE_SHA), format!("{}  override.json\n", sha.to_uppercase())).unwrap();
        let (l, got) = local_override(root).unwrap().unwrap();
        assert_eq!((l.mods.len(), got.as_str()), (1, sha.as_str()));
        // One byte changed after the sha256 was written: refused, no fall back.
        let mut other = body.to_vec();
        *other.last_mut().unwrap() = b' ';
        other.push(b'}');
        std::fs::write(root.join(OVERRIDE_LIST), &other).unwrap();
        assert!(local_override(root).unwrap_err().to_string().contains("won't run"));
        std::fs::write(root.join(OVERRIDE_SHA), "").unwrap();
        assert!(local_override(root).is_err());
        std::fs::remove_file(root.join(OVERRIDE_LIST)).unwrap();
        assert!(local_override(root).unwrap_err().to_string().contains("without override.json"));
        // Bytes that match but don't read as a list.
        std::fs::write(root.join(OVERRIDE_LIST), b"not json").unwrap();
        std::fs::write(root.join(OVERRIDE_SHA), format!("{:x}", Sha256::digest(b"not json"))).unwrap();
        assert!(local_override(root).unwrap_err().to_string().contains("doesn't read"));
    }

    #[test]
    fn named_plugins_come_only_from_what_the_installer_puts_in_data() {
        let t = tempfile::tempdir().unwrap();
        let u = t.path().join("u");
        std::fs::create_dir_all(u.join("Patches/COTN")).unwrap();
        std::fs::write(u.join("Main.esp"), b"main").unwrap();
        std::fs::write(u.join("Patches/COTN/Patch A.esp"), b"a").unwrap();
        let mut m = lane(r#"{"for_discord_id":"1","mods":[{"id":"ocw","name":"OCW","nexus":{"mod":1,"file":2}}]}"#).mods.remove(0);
        let all = pick(&m, &u).unwrap();
        assert_eq!(all.iter().map(|p| p.1.as_str()).collect::<Vec<_>>(), vec!["Main.esp"]);
        m.plugins = vec!["main.ESP".into()];
        let a = pick(&m, &u).unwrap();
        assert!(a[0].0.ends_with("Main.esp"));
        // The list's spelling names the file on the server.
        assert_eq!(a[0].1, "main.ESP");
        // In the archive but not installed: refused, never searched for.
        m.plugins = vec!["Patch A.esp".into()];
        let e = pick(&m, &u).unwrap_err().to_string();
        assert!(e.contains("don't pick it"), "{e}");
        m.plugins = vec!["Missing.esp".into()];
        assert!(pick(&m, &u).unwrap_err().to_string().contains("no Missing.esp"));
        // A pinned path still takes exactly that file.
        m.paths.insert("Patch A.esp".into(), "Patches/COTN/Patch A.esp".into());
        m.plugins = vec!["Patch A.esp".into()];
        assert_eq!(std::fs::read(&pick(&m, &u).unwrap()[0].0).unwrap(), b"a");
    }

    #[test]
    fn client_only_plugins_never_reach_the_server_zip_and_change_the_list_hash() {
        // OCW_AO_FEPatch.esp masters Audio Overhaul, a client-only mod: every
        // PC installs it, the server never loads it (world-1.6).
        let t = tempfile::tempdir().unwrap();
        let u = t.path().join("u");
        for f in ["OCW.esp", "OCW_AO_FEPatch.esp"] {
            std::fs::create_dir_all(&u).unwrap();
            std::fs::write(u.join(f), f).unwrap();
        }
        let json = |extra: &str| format!(r#"{{"for_discord_id":"1","mods":[{{"id":"ocw","name":"OCW","nexus":{{"mod":1,"file":2}},"plugins":[]{extra}}}]}}"#);
        let plain = lane(&json(""));
        let with = lane(&json(r#","clientOnly":["OCW_AO_FEPatch.esp"]"#));
        check(&with).unwrap();
        let names = |l: &ServerLane| pick(&l.mods[0], &u).unwrap().into_iter().map(|p| p.1).collect::<Vec<_>>();
        assert_eq!(names(&plain), ["OCW.esp", "OCW_AO_FEPatch.esp"]);
        assert_eq!(names(&with), ["OCW.esp"]);
        assert_ne!(list_hash(&plain), list_hash(&with));
        // An old list without the field hashes as before.
        assert!(!serde_json::to_string(&plain).unwrap().contains("clientOnly"));
        let both = lane(&json(r#","clientOnly":["OCW.esp"]"#).replace(r#""plugins":[]"#, r#""plugins":["OCW.esp"]"#));
        assert!(check(&both).unwrap_err().to_string().contains("both a server plugin and client-only"));
    }

    #[test]
    fn convert_all_runs_every_esl_flagged_lane_plugin_as_full_but_not_an_esl() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("Data");
        std::fs::create_dir_all(&data).unwrap();
        let json = |policy: &str| {
            format!(
                r#"{{"for_discord_id":"1"{policy},"mods":[
                {{"id":"k","name":"K","nexus":{{"mod":1,"file":1}},"plugins":["Kad_MoonMonkRobes.esp"]}},
                {{"id":"s","name":"S","nexus":{{"mod":2,"file":2}},"plugins":["Sentinel.esp"]}}]}}"#
            )
        };
        std::fs::write(data.join("Kad_MoonMonkRobes.esp"), esp(&["Skyrim.esm"], 0x200)).unwrap();
        std::fs::write(data.join("Sentinel.esp"), esp(&["Skyrim.esm", "Kad_MoonMonkRobes.esp"], 0)).unwrap();
        assert_eq!(master_problems(&lane(&json("")), &data), ["Kad_MoonMonkRobes.esp is ESL-flagged, and the list doesn't declare it lightCleared"]);
        // The order file's description after "convert-all" is accepted, as check_masters.py does.
        let l = lane(&json(r#","lightPolicy":"convert-all: every ESL-flagged .esp/.esm runs as a flag-cleared canonical copy""#));
        check(&l).unwrap();
        assert!(master_problems(&l, &data).is_empty(), "{:?}", master_problems(&l, &data));
        let rec = finish(t.path(), &l, "h", [("Kad_MoonMonkRobes.esp", "k"), ("Sentinel.esp", "s")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(), Source::served()).unwrap();
        assert_eq!(rec.files["Kad_MoonMonkRobes.esp"].run_name, "Kad_MoonMonkRobes-AD.esp");
        // A dependent runs with its MAST pointing at the run name.
        let dep = &rec.files["Sentinel.esp"];
        assert_ne!(dep.canonical_sha256, dep.sha256);
        // An .esl lane plugin is still refused, and an unknown policy stops the list.
        std::fs::write(data.join("Lite.esl"), esp(&["Skyrim.esm"], 0)).unwrap();
        let l = lane(&json(r#","lightPolicy":"convert-all""#).replace(r#""Sentinel.esp"]"#, r#""Sentinel.esp","Lite.esl"]"#));
        assert!(master_problems(&l, &data).iter().any(|p| p.contains("Lite.esl is a light plugin")));
        assert!(check(&lane(&json(r#","lightPolicy":"keep""#))).unwrap_err().to_string().contains("lightPolicy"));
    }

    #[test]
    fn a_plugin_the_fomod_picks_leave_out_is_refused_though_the_archive_has_it() {
        // MoreCraftableEquipment_USSEP.esp was collected while the log showed
        // "[x] None": the players' picks leave the patch out, so the server
        // must too (PR #7, Codex 5875889405).
        let t = tempfile::tempdir().unwrap();
        let u = t.path().join("u");
        let xml = r#"<config><requiredInstallFiles><file source="core/MoreCraftableEquipment.esp" destination="MoreCraftableEquipment.esp"/></requiredInstallFiles><installSteps><installStep name="s"><optionalFileGroups>
<group name="Patches" type="SelectExactlyOne"><plugins>
<plugin name="None"><files></files><typeDescriptor><type name="Optional"/></typeDescriptor></plugin>
<plugin name="USSEP"><files><file source="patches/MoreCraftableEquipment_USSEP.esp" destination="MoreCraftableEquipment_USSEP.esp"/></files><typeDescriptor><type name="Optional"/></typeDescriptor></plugin>
</plugins></group></optionalFileGroups></installStep></installSteps></config>"#;
        for (f, b) in [("fomod/ModuleConfig.xml", xml.as_bytes()), ("core/MoreCraftableEquipment.esp", b"main"), ("patches/MoreCraftableEquipment_USSEP.esp", b"patch")] {
            std::fs::create_dir_all(u.join(f).parent().unwrap()).unwrap();
            std::fs::write(u.join(f), b).unwrap();
        }
        let json = |fomod: &str| format!(r#"{{"for_discord_id":"1","mods":[{{"id":"mce","name":"MCE","nexus":{{"mod":1,"file":2}},"fomod":[{fomod}],"plugins":["MoreCraftableEquipment.esp","MoreCraftableEquipment_USSEP.esp"]}}]}}"#);
        let none = lane(&json(r#""None""#)).mods.remove(0);
        let e = pick(&none, &u).unwrap_err().to_string();
        assert!(e.contains("MoreCraftableEquipment_USSEP.esp is in its download but the installer's options don't pick it"), "{e}");
        assert!(!e.contains("MoreCraftableEquipment.esp is"), "the main plugin is fine: {e}");
        // With the patch picked, both come from the plan.
        let both = pick(&lane(&json(r#""USSEP""#)).mods[0], &u).unwrap();
        assert_eq!(both.iter().map(|p| std::fs::read(&p.0).unwrap()).collect::<Vec<_>>(), vec![b"main".to_vec(), b"patch".to_vec()]);
    }

    #[test]
    fn a_listed_path_is_taken_exactly_or_refused() {
        let t = tempfile::tempdir().unwrap();
        let u = t.path().join("u");
        for (dir, body) in [("000 Standard", "npc"), ("001 Crafted Only", "crafted")] {
            std::fs::create_dir_all(u.join(dir)).unwrap();
            std::fs::write(u.join(dir).join("Armors of the Velothi.esp"), body).unwrap();
        }
        let json = |path: &str| format!(r#"{{"for_discord_id":"1","mods":[{{"id":"velothi","name":"Velothi","nexus":{{"mod":62752,"file":624586}},"plugins":["Armors of the Velothi.esp"],"archive":{{"name":"Pt. I","version":"1.3.1","sizeBytes":133472002}},"paths":{{"Armors of the Velothi.esp":"{path}"}}}}]}}"#);
        let l = lane(&json("001 Crafted Only/Armors of the Velothi.esp"));
        check(&l).unwrap();
        assert_eq!(l.mods[0].archive.as_ref().unwrap().size_bytes, Some(133472002));
        let got = pick(&l.mods[0], &u).unwrap();
        assert_eq!(std::fs::read_to_string(&got[0].0).unwrap(), "crafted");
        // The case of the folder may differ.
        let got = pick(&lane(&json("001 crafted only/armors of the velothi.esp")).mods[0], &u).unwrap();
        assert_eq!(std::fs::read_to_string(&got[0].0).unwrap(), "crafted");
        // A missing path refuses the mod; the other copy is never taken.
        let e = pick(&lane(&json("002 Gone/Armors of the Velothi.esp")).mods[0], &u).unwrap_err().to_string();
        assert!(e.contains("no 002 Gone"), "{e}");
        // Even when only one other copy exists.
        std::fs::remove_dir_all(u.join("000 Standard")).unwrap();
        assert!(pick(&lane(&json("000 Standard/Armors of the Velothi.esp")).mods[0], &u).is_err());
        // Unsafe or mismatched paths are refused before any download.
        assert!(check(&lane(&json("../Armors of the Velothi.esp"))).is_err());
        assert!(check(&lane(&json("001 Crafted Only/Other.esp"))).is_err());
    }

    #[test]
    fn lists_every_plugin_in_a_download() {
        let t = tempfile::tempdir().unwrap();
        for f in ["Heavy Armory.esp", "Options/Patch A.esl", "Options/B/Patch.ESM", "readme.txt", "Meshes/x.nif"] {
            let p = t.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        }
        assert_eq!(plugins_in(t.path()), vec!["Heavy Armory.esp", "Options/B/Patch.ESM", "Options/Patch A.esl"]);
    }

    /// A plugin: a TES4 header naming `masters`, with `flags`.
    fn esp(masters: &[&str], flags: u32) -> Vec<u8> {
        let mut sub = Vec::new();
        sub.extend(b"HEDR");
        sub.extend(12u16.to_le_bytes());
        sub.extend(1.71f32.to_le_bytes());
        sub.extend([0u8; 8]);
        for m in masters {
            let mut z = m.as_bytes().to_vec();
            z.push(0);
            sub.extend(b"MAST");
            sub.extend((z.len() as u16).to_le_bytes());
            sub.extend(z);
            sub.extend(b"DATA");
            sub.extend(8u16.to_le_bytes());
            sub.extend([0u8; 8]);
        }
        let mut b = b"TES4".to_vec();
        b.extend((sub.len() as u32).to_le_bytes());
        b.extend(flags.to_le_bytes());
        b.extend([0u8; 8]);
        b.extend(44u16.to_le_bytes());
        b.extend([0u8; 2]);
        b.extend(sub);
        b
    }

    #[test]
    fn a_master_the_server_cant_load_stops_the_export_and_is_named() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join(LANE_DIR);
        let data = root.join("Data");
        std::fs::create_dir_all(&data).unwrap();
        // The shape of PR #7 5916681442: COTN Dawnstar names Creation Club
        // masters outside the five-master contract.
        let l = lane(
            r#"{"for_discord_id":"1","mods":[
            {"id":"tgc","name":"TGC","nexus":{"mod":1,"file":1},"plugins":["TGC.esm"]},
            {"id":"cotn","name":"COTN","nexus":{"mod":2,"file":2},"plugins":["COTN Dawnstar.esp"]},
            {"id":"patch","name":"Patch","nexus":{"mod":3,"file":3},"plugins":["Patch.esp"]},
            {"id":"late","name":"Late","nexus":{"mod":4,"file":4},"plugins":["Late.esm"]},
            {"id":"junk","name":"Junk","nexus":{"mod":5,"file":5},"plugins":["Junk.esp"]}]}"#,
        );
        std::fs::write(data.join("TGC.esm"), esp(&["Skyrim.esm"], 1)).unwrap();
        std::fs::write(data.join("COTN Dawnstar.esp"), esp(&["Skyrim.esm", "ccBGSSSE001-Fish.esm", "TGC.esm"], 0x200)).unwrap();
        std::fs::write(data.join("Patch.esp"), esp(&["tgc.esm", "Late.esm"], 0)).unwrap();
        std::fs::write(data.join("Late.esm"), esp(&["Dragonborn.esm"], 1)).unwrap();
        std::fs::write(data.join("Junk.esp"), b"not a plugin").unwrap();
        let want = vec![
            "COTN Dawnstar.esp is ESL-flagged, and the list doesn't declare it lightCleared".to_string(),
            "COTN Dawnstar.esp needs ccBGSSSE001-Fish.esm, which isn't one of the server's masters or in the list".to_string(),
            "Patch.esp needs Late.esm, which the list loads after it".to_string(),
            "Junk.esp has no readable plugin header".to_string(),
        ];
        assert_eq!(master_problems(&l, &data), want);
        let plugins: BTreeMap<String, String> = [("TGC.esm", "tgc"), ("COTN Dawnstar.esp", "cotn"), ("Patch.esp", "patch"), ("Late.esm", "late"), ("Junk.esp", "junk")]
            .into_iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let r = report(&root, &l, "h", &plugins, vec![]).unwrap();
        assert_eq!(r.master_problems, want);
        let e = finish(&root, &l, "h", plugins.clone(), Source::served()).unwrap_err().to_string();
        assert!(e.contains("ccBGSSSE001-Fish.esm") && e.contains("no zip made"), "{e}");
        assert!(!root.join(ZIP_NAME).exists() && !root.join("export.json").exists());
        // Fixed: the CC master and light flag gone, the order corrected,
        // the junk replaced.
        let l = lane(
            r#"{"for_discord_id":"1","mods":[
            {"id":"tgc","name":"TGC","nexus":{"mod":1,"file":1},"plugins":["TGC.esm"]},
            {"id":"cotn","name":"COTN","nexus":{"mod":2,"file":2},"plugins":["COTN Dawnstar.esp"]},
            {"id":"late","name":"Late","nexus":{"mod":4,"file":4},"plugins":["Late.esm"]},
            {"id":"patch","name":"Patch","nexus":{"mod":3,"file":3},"plugins":["Patch.esp"]},
            {"id":"junk","name":"Junk","nexus":{"mod":5,"file":5},"plugins":["Junk.esp"]}]}"#,
        );
        std::fs::write(data.join("COTN Dawnstar.esp"), esp(&["Skyrim.esm", "TGC.esm"], 0)).unwrap();
        std::fs::write(data.join("Junk.esp"), esp(&[], 0)).unwrap();
        assert!(master_problems(&l, &data).is_empty());
        let rec = finish(&root, &l, "h2", plugins, Source::served()).unwrap();
        let idx: Vec<(&str, usize)> = rec.files.iter().map(|(n, f)| (n.as_str(), f.index)).collect();
        assert_eq!(idx, vec![("COTN Dawnstar.esp", 6), ("Junk.esp", 9), ("Late.esm", 7), ("Patch.esp", 8), ("TGC.esm", 5)]);
        assert!(!rec.files["COTN Dawnstar.esp"].light && !rec.files["TGC.esm"].light);
        assert_eq!(rec.files["Patch.esp"].masters, vec!["tgc.esm", "Late.esm"]);
    }

    #[test]
    fn light_plugins_the_list_declares_export_and_their_masters_ship_as_hashes_only() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("Data");
        let game = t.path().join("game").join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&game).unwrap();
        let base = r#""Skyrim.esm","Update.esm","Dawnguard.esm","HearthFires.esm","Dragonborn.esm""#;
        // WORLD-1.5: USSEP masters _ResourcePack.esl, which the list runs as
        // _ResourcePack.esm through masterSources; COTN is ESL-flagged and
        // declared lightCleared, so it runs as its flag-cleared copy.
        let l = lane(&format!(
            r#"{{"for_discord_id":"1","masters":[{base},"_ResourcePack.esm","ccBGSSSE001-Fish.esm"],
            "masterSources":[{{"run":"_ResourcePack.esm","from":"_ResourcePack.esl"}}],
            "lightCleared":["COTN Dawnstar.esp"],"mods":[
            {{"id":"ussep","name":"USSEP","nexus":{{"mod":1,"file":1}},"plugins":["Unofficial Skyrim Special Edition Patch.esp"]}},
            {{"id":"cotn","name":"COTN","nexus":{{"mod":2,"file":2}},"plugins":["COTN Dawnstar.esp"]}}]}}"#
        ));
        check(&l).unwrap();
        assert_eq!(l.light_as_full(), ["_ResourcePack.esl"]);
        assert_eq!(l.master_files(), [("_ResourcePack.esl".to_string(), "_ResourcePack.esm".to_string()), ("ccBGSSSE001-Fish.esm".to_string(), "ccBGSSSE001-Fish.esm".to_string())]);
        let rp = esp(&["Skyrim.esm"], 0x201);
        std::fs::write(game.join("_ResourcePack.esl"), &rp).unwrap();
        std::fs::write(game.join("ccBGSSSE001-Fish.esm"), esp(&["Skyrim.esm", "Update.esm"], 1)).unwrap();
        std::fs::write(data.join("Unofficial Skyrim Special Edition Patch.esp"), esp(&["Skyrim.esm", "_ResourcePack.esl"], 1)).unwrap();
        std::fs::write(data.join("COTN Dawnstar.esp"), esp(&["Skyrim.esm", "ccBGSSSE001-Fish.esm"], 0x200)).unwrap();
        assert!(master_problems_with(&l, &data, Some(&game)).is_empty(), "{:?}", master_problems_with(&l, &data, Some(&game)));
        let plugins: BTreeMap<String, String> = [("Unofficial Skyrim Special Edition Patch.esp", "ussep"), ("COTN Dawnstar.esp", "cotn")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        // Without the game folder there is nothing to check the masters by.
        assert!(finish(t.path(), &l, "h", plugins.clone(), Source::served()).unwrap_err().to_string().contains("game folder"));
        let rec = finish_with(t.path(), &l, "h", plugins, Source::served(), Some(&game)).unwrap();
        assert_eq!(rec.light_as_full, ["_ResourcePack.esl"]);
        let run: Vec<(&str, &str, usize)> = rec.files.iter().map(|(n, f)| (n.as_str(), f.run_name.as_str(), f.index)).collect();
        assert_eq!(run, [("COTN Dawnstar.esp", "COTN-Dawnstar.esp", 8), ("Unofficial Skyrim Special Edition Patch.esp", "Unofficial-Skyrim-Special-Edition-Patch.esp", 7)]);
        // The masters: names and hashes only; the converted copy is what a
        // PC's launcher makes from its own file.
        let m: Vec<(&str, &str, usize)> = rec.masters.iter().map(|m| (m.file.as_str(), m.run_name.as_str(), m.index)).collect();
        assert_eq!(m, [("_ResourcePack.esl", "_ResourcePack.esm", 5), ("ccBGSSSE001-Fish.esm", "ccBGSSSE001-Fish.esm", 6)]);
        let pc = t.path().join("pc");
        std::fs::create_dir_all(pc.join("Data")).unwrap();
        std::fs::write(pc.join("Data").join("_ResourcePack.esl"), &rp).unwrap();
        crate::aliases::ensure_full(&pc, None, &rec.light_as_full).unwrap();
        use sha2::{Digest, Sha256};
        assert_eq!(format!("{:x}", Sha256::digest(std::fs::read(pc.join("Data").join("_ResourcePack.esm")).unwrap())), rec.masters[0].canonical_sha256);
        assert_ne!(rec.masters[0].canonical_sha256, rec.masters[0].sha256);
        // Size and crc32 as SkyMP's load-order check reads the PC's copy.
        let on_pc = pc.join("Data").join("_ResourcePack.esm");
        assert_eq!((rec.masters[0].canonical_bytes, Some(rec.masters[0].canonical_crc32)), (std::fs::metadata(&on_pc).unwrap().len(), crate::serverorder::crc32(&on_pc)));
        let z = zip::ZipArchive::new(std::fs::File::open(t.path().join(ZIP_NAME)).unwrap()).unwrap();
        let mut in_zip: Vec<&str> = z.file_names().collect();
        in_zip.sort();
        assert_eq!(in_zip, ["Data/COTN Dawnstar.esp", "Data/Unofficial Skyrim Special Edition Patch.esp"], "no Creation Club bytes in the zip");
        // The canonical bytes are what the server tool writes.
        let out = t.path().join("server");
        for c in crate::aliases::canonicalize_dir_full(&data, &out, &rec.light_as_full).unwrap() {
            let f = &rec.files[&c.original];
            assert_eq!((f.run_name.as_str(), f.canonical_sha256.as_str()), (c.name.as_str(), c.sha256.as_str()), "{}", c.original);
        }
        // Without masterSources, USSEP's master isn't met.
        let bare = lane(&format!(r#"{{"for_discord_id":"1","masters":[{base},"_ResourcePack.esm"],"mods":[{{"id":"ussep","name":"USSEP","nexus":{{"mod":1,"file":1}},"plugins":["Unofficial Skyrim Special Edition Patch.esp"]}}]}}"#));
        assert_eq!(master_problems(&bare, &data), ["Unofficial Skyrim Special Edition Patch.esp needs _ResourcePack.esl, which isn't one of the server's masters or in the list"]);
    }

    #[test]
    fn a_light_plugin_the_list_doesnt_declare_stops_the_export() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("Data");
        let game = t.path().join("game");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&game).unwrap();
        let base = r#""Skyrim.esm","Update.esm","Dawnguard.esm","HearthFires.esm","Dragonborn.esm""#;
        let l = lane(&format!(
            r#"{{"for_discord_id":"1","masters":[{base},"ccBGSSSE037-Curios.esm"],"lightCleared":["ccBGSSSE099-Thing.esp"],"mods":[
            {{"id":"cotn","name":"COTN","nexus":{{"mod":1,"file":1}},"plugins":["COTN Morthal.esp"]}},
            {{"id":"cc","name":"CC","nexus":{{"mod":2,"file":2}},"plugins":["ccBGSSSE099-Thing.esp"]}},
            {{"id":"lite","name":"Lite","nexus":{{"mod":3,"file":3}},"plugins":["Lite.esl"]}}]}}"#
        ));
        // Undeclared ESL flag; a Creation Club name the "-AD" rule leaves
        // light even when declared; a lane .esl; a master the game has only
        // as .esl with no masterSources entry.
        std::fs::write(data.join("COTN Morthal.esp"), esp(&["Skyrim.esm"], 0x200)).unwrap();
        std::fs::write(data.join("ccBGSSSE099-Thing.esp"), esp(&["Skyrim.esm"], 0x200)).unwrap();
        std::fs::write(data.join("Lite.esl"), esp(&["Skyrim.esm"], 0)).unwrap();
        std::fs::write(game.join("ccBGSSSE037-Curios.esm"), esp(&["Skyrim.esm"], 0x201)).unwrap();
        assert_eq!(
            master_problems_with(&l, &data, Some(&game)),
            [
                "ccBGSSSE037-Curios.esm (the server's master ccBGSSSE037-Curios.esm) is a light plugin no masterSources entry runs as a full one",
                "COTN Morthal.esp is ESL-flagged, and the list doesn't declare it lightCleared",
                "ccBGSSSE099-Thing.esp is a light (ESL) plugin with a Creation Club name, which no rule runs as a full plugin",
                "Lite.esl is a light plugin (.esl); a lane plugin can't be one",
            ]
        );
        let plugins: BTreeMap<String, String> = [("COTN Morthal.esp", "cotn"), ("ccBGSSSE099-Thing.esp", "cc"), ("Lite.esl", "lite")].into_iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let e = finish_with(t.path(), &l, "h", plugins.clone(), Source::served(), Some(&game)).unwrap_err().to_string();
        assert!(e.contains("the game's own plugins (ccBGSSSE099-Thing.esp)") && e.contains("no zip made"), "{e}");
        let mut rest = plugins;
        rest.remove("ccBGSSSE099-Thing.esp");
        let l = lane(&format!(
            r#"{{"for_discord_id":"1","masters":[{base},"ccBGSSSE037-Curios.esm"],"mods":[
            {{"id":"cotn","name":"COTN","nexus":{{"mod":1,"file":1}},"plugins":["COTN Morthal.esp"]}},
            {{"id":"lite","name":"Lite","nexus":{{"mod":3,"file":3}},"plugins":["Lite.esl"]}}]}}"#
        ));
        let e = finish_with(t.path(), &l, "h", rest, Source::served(), Some(&game)).unwrap_err().to_string();
        assert!(e.contains("lightCleared") && e.contains("no zip made"), "{e}");
        assert!(!t.path().join(ZIP_NAME).exists());
        // A master missing from the game folder stops it too.
        assert!(master_receipts(&l, t.path()).unwrap_err().to_string().contains("the game folder has no ccBGSSSE037-Curios.esm"));
        // A malformed masterSources or lightCleared fails the list check.
        for bad in [
            format!(r#""masters":[{base},"A.esm"],"masterSources":[{{"run":"A.esm","from":"B.esl"}}]"#),
            format!(r#""masters":[{base},"A.esm"],"masterSources":[{{"run":"C.esm","from":"C.esl"}}]"#),
            r#""lightCleared":["Nope.esp"]"#.to_string(),
        ] {
            let l = lane(&format!(r#"{{"for_discord_id":"1",{bad},"mods":[{{"id":"a","name":"A","nexus":{{"mod":1,"file":1}},"plugins":["A.esp"]}}]}}"#));
            assert!(check(&l).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_games_own_plugins_never_go_in_the_zip() {
        // Base masters, Creation Club content and _ResourcePack are
        // Bethesda's: finish refuses them whatever the list says.
        for name in ["Skyrim.esm", "dawnguard.esm", "_ResourcePack.esl", "ccBGSSSE001-Fish.esm", "ccQDRSSE001-SurvivalMode.esl", "ccBGSSSE025-AdvDSGS.esm"] {
            let t = tempfile::tempdir().unwrap();
            let data = t.path().join("Data");
            std::fs::create_dir_all(&data).unwrap();
            std::fs::write(data.join(name), esp(&[], 1)).unwrap();
            let l = lane(&format!(r#"{{"for_discord_id":"1","mods":[{{"id":"x","name":"X","nexus":{{"mod":1,"file":1}},"plugins":["{name}"]}}]}}"#));
            let e = finish(t.path(), &l, "h", [(name.to_string(), "x".to_string())].into(), Source::served()).unwrap_err().to_string();
            assert!(e.contains(&format!("the game's own plugins ({name})")), "{name}: {e}");
            assert!(!t.path().join(ZIP_NAME).exists() && !t.path().join("export.json").exists(), "{name}");
        }
        // A mod's own file with a Creation Club-like name is still a mod.
        assert!(!crate::aliases::shipped_with_game("ccBGSSSE001-Fish - Patch.esp"));
    }

    #[test]
    fn more_than_254_full_plugins_stop_the_export() {
        let mods: Vec<String> = (0..250).map(|i| format!(r#"{{"id":"m{i}","name":"M{i}","nexus":{{"mod":{i},"file":{i}}},"plugins":["P{i}.esp"]}}"#)).collect();
        let l = lane(&format!(r#"{{"for_discord_id":"1","mods":[{}]}}"#, mods.join(",")));
        let t = tempfile::tempdir().unwrap();
        assert_eq!(master_problems(&l, t.path()), ["the list loads 255 full plugins (5 masters and 250 plugins), more than the 254 a PC can"]);
    }

    #[test]
    fn a_list_can_widen_the_masters_but_not_to_light_ones() {
        let base = r#""Skyrim.esm","Update.esm","Dawnguard.esm","HearthFires.esm","Dragonborn.esm""#;
        let mods = r#"[{"id":"cotn","name":"COTN","nexus":{"mod":2,"file":2},"plugins":["COTN Dawnstar.esp"]}]"#;
        // No field: the five base masters, and the hash a list had before.
        let plain = lane(&format!(r#"{{"for_discord_id":"1","mods":{mods}}}"#));
        assert_eq!(plain.masters().len(), 5);
        assert!(!serde_json::to_string(&plain).unwrap().contains("masters\":["));
        let wide = lane(&format!(r#"{{"for_discord_id":"1","mods":{mods},"masters":[{base},"ccBGSSSE001-Fish.esm"]}}"#));
        check(&wide).unwrap();
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("COTN Dawnstar.esp"), esp(&["Skyrim.esm", "ccBGSSSE001-Fish.esm"], 0)).unwrap();
        assert_eq!(master_problems(&plain, &data).len(), 1);
        assert!(master_problems(&wide, &data).is_empty());
        let game = t.path().join("game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("ccbgssse001-fish.esm"), esp(&["Skyrim.esm"], 1)).unwrap();
        let rec = finish_with(t.path(), &wide, "h", [("COTN Dawnstar.esp".to_string(), "cotn".to_string())].into(), Source::served(), Some(&game)).unwrap();
        assert_eq!(rec.files["COTN Dawnstar.esp"].index, 6);
        assert_eq!((rec.masters[0].file.as_str(), rec.masters[0].run_name.as_str()), ("ccbgssse001-fish.esm", "ccbgssse001-fish.esm"));
        // A light master, a list not starting with the base five, a path.
        for bad in [format!(r#"[{base},"ccQDRSSE001-SurvivalMode.esl"]"#), r#"["Skyrim.esm","ccBGSSSE001-Fish.esm"]"#.to_string(), format!(r#"[{base},"x/Fish.esm"]"#)] {
            let l = lane(&format!(r#"{{"for_discord_id":"1","mods":{mods},"masters":{bad}}}"#));
            assert!(check(&l).is_err(), "{bad}");
        }
    }

    #[test]
    fn zips_the_plugins_and_remembers_the_list() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join(LANE_DIR);
        let src = t.path().join("a.esp");
        let bytes = esp(&["Skyrim.esm", "Update.esm"], 0);
        std::fs::write(&src, &bytes).unwrap();
        let mut seen = BTreeMap::new();
        collect(&root, "jks", &[(src.clone(), "JKs Skyrim.esp".into())], &mut seen).unwrap();
        assert!(collect(&root, "other", &[(src, "jks skyrim.esp".into())], &mut seen).is_err());
        let l = lane(r#"{"for_discord_id":"1","mods":[]}"#);
        let h = list_hash(&l);
        assert!(!done(&root, &h));
        // A zip short of the list is refused, and says what's missing.
        let l2 = lane(r#"{"for_discord_id":"1","mods":[{"id":"jks","name":"JK","nexus":{"mod":1,"file":2},"plugins":["JKs Skyrim.esp","JK Patch.esp"]}]}"#);
        let e = finish(&root, &l2, &h, seen.clone(), Source::served()).unwrap_err().to_string();
        assert!(e.contains("1 of 2 declared plugins (missing: JK Patch.esp (jks)"), "{e}");
        assert!(!root.join(ZIP_NAME).exists());
        let l = lane(r#"{"for_discord_id":"1","mods":[{"id":"jks","name":"JK","nexus":{"mod":1,"file":2},"plugins":["JKs Skyrim.esp"]}]}"#);
        let h = list_hash(&l);
        let rec = finish(&root, &l, &h, seen, Source::local("ab")).unwrap();
        assert_eq!(rec.source, Source::local("ab"));
        assert_eq!(rec.files["JKs Skyrim.esp"].bytes, bytes.len() as u64);
        assert_eq!(rec.files["JKs Skyrim.esp"].index, 5);
        assert_eq!(rec.files["JKs Skyrim.esp"].masters, vec!["Skyrim.esm", "Update.esm"]);
        assert!(!rec.files["JKs Skyrim.esp"].light);
        assert_eq!(rec.files["JKs Skyrim.esp"].sha256.len(), 64);
        assert!(done(&root, &h));
        assert!(!done(&root, "other list"));
        let z = zip::ZipArchive::new(std::fs::File::open(root.join(ZIP_NAME)).unwrap()).unwrap();
        assert_eq!(z.file_names().collect::<Vec<_>>(), vec!["Data/JKs Skyrim.esp"]);
        assert_eq!(std::fs::read_to_string(root.join("server-lane.zip.sha256")).unwrap(), format!("{}  server-lane.zip\n", rec.zip_sha256));
        start_over(&root).unwrap();
        assert!(!done(&root, &h) && !root.join("Data").exists());
    }

    /// PR #7: the export folder held 53 plugins for a list declaring 70,
    /// with no zip and no record of why. The report now names every mod's
    /// outcome, the first failure, and each declared plugin not collected.
    #[test]
    fn a_failed_run_reports_the_first_failure_and_every_missing_plugin() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join(LANE_DIR);
        let l = lane(
            r#"{"for_discord_id":"1","mods":[
            {"id":"a","name":"A","nexus":{"mod":1,"file":1},"plugins":["A.esp"]},
            {"id":"sentinel","name":"Sentinel","nexus":{"mod":2,"file":2},"plugins":["Sentinel.esp","Sentinel - City Guards.esp"]},
            {"id":"c","name":"C","nexus":{"mod":3,"file":3},"plugins":["C.esp"]}]}"#,
        );
        let mut plugins = BTreeMap::new();
        plugins.insert("A.esp".to_string(), "a".to_string());
        plugins.insert("C.esp".to_string(), "c".to_string());
        plugins.insert("Stray.esp".to_string(), "c".to_string());
        let outcomes = vec![
            ModOutcome { id: "a".into(), name: "A".into(), declared: vec!["A.esp".into()], gave: vec!["A.esp".into()], error: None },
            ModOutcome { id: "sentinel".into(), name: "Sentinel".into(), declared: vec!["Sentinel.esp".into(), "Sentinel - City Guards.esp".into()], gave: vec![], error: Some("no Sentinel - City Guards.esp in its download".into()) },
            ModOutcome { id: "c".into(), name: "C".into(), declared: vec!["C.esp".into()], gave: vec!["C.esp".into()], error: None },
        ];
        let r = report(&root, &l, "h", &plugins, outcomes).unwrap();
        assert_eq!((r.declared, r.collected), (4, 3));
        assert_eq!(r.missing, vec![("sentinel".to_string(), "Sentinel.esp".to_string()), ("sentinel".to_string(), "Sentinel - City Guards.esp".to_string())]);
        assert_eq!(r.extra, vec!["Stray.esp".to_string()]);
        assert_eq!(r.first_failure.as_deref(), Some("Sentinel: no Sentinel - City Guards.esp in its download"));
        let saved: Report = serde_json::from_slice(&std::fs::read(root.join("report.json")).unwrap()).unwrap();
        assert_eq!(saved, r);
    }

    #[test]
    fn a_trickle_is_too_slow() {
        use std::time::Duration;
        assert!(!too_slow(1, Duration::from_secs(119)), "grace time first");
        // One byte every 59 seconds, 3 minutes in.
        assert!(too_slow(3, Duration::from_secs(180)));
        assert!(!too_slow(180 * 1024 * 1024, Duration::from_secs(180)));
        assert!(too_slow(MIN_RATE * 180 - 1, Duration::from_secs(180)));
    }

    #[test]
    fn checks_the_listed_size_and_sha256() {
        let t = tempfile::tempdir().unwrap();
        let a = t.path().join("a.7z");
        std::fs::write(&a, b"abc").unwrap();
        let sha = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        let m = |extra: &str| lane(&format!(r#"{{"for_discord_id":"1","mods":[{{"id":"a","name":"A","nexus":{{"mod":1,"file":2}}{extra}}}]}}"#)).mods.remove(0);
        verify(&m(""), &a).unwrap();
        verify(&m(&format!(r#","size":3,"sha256":"{sha}""#)), &a).unwrap();
        assert!(verify(&m(r#","size":4"#), &a).is_err());
        assert!(verify(&m(&format!(r#","sha256":"{}""#, "0".repeat(64))), &a).is_err());
    }

    #[test]
    fn stops_after_two_failures_until_the_list_changes() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join(LANE_DIR);
        assert!(!gave_up(&root, "a"));
        assert_eq!(failed(&root, "a").unwrap(), 1);
        assert!(!gave_up(&root, "a"));
        assert_eq!(failed(&root, "a").unwrap(), 2);
        assert!(gave_up(&root, "a"));
        assert!(!gave_up(&root, "b"), "a changed list gets tries again");
        assert_eq!(failed(&root, "b").unwrap(), 1);
        // A good export clears the count.
        finish(&root, &lane(r#"{"for_discord_id":"1","mods":[]}"#), "b", BTreeMap::new(), Source::served()).unwrap();
        assert!(!gave_up(&root, "b") && failed(&root, "b").unwrap() == 1);
    }
}
