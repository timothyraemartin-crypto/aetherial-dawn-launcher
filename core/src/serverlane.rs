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

/// Writes `report.json` for this run and returns it.
pub fn report(root: &Path, lane: &ServerLane, hash: &str, plugins: &BTreeMap<String, String>, mods: Vec<ModOutcome>) -> Result<Report> {
    let (missing, extra) = reconcile(lane, plugins);
    let rep = Report {
        list: hash.to_string(),
        declared: lane.mods.iter().map(|m| m.plugins.len()).sum(),
        collected: plugins.len(),
        missing,
        extra,
        first_failure: mods.iter().find_map(|m| m.error.as_ref().map(|e| format!("{}: {e}", m.name))),
        mods,
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
            if !is_plugin(p) || p.contains(['/', '\\']) {
                return Err(Error::Game(format!("server lane: {} names {p:?}, which isn't a plugin file name", e.id)));
            }
            if !names.insert(p.to_ascii_lowercase()) {
                return Err(Error::Game(format!("server lane: {p} is taken from two mods")));
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
            (!name.contains('/') && is_plugin(&name)).then(|| (c.from.clone(), name))
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
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
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
    let mut files = BTreeMap::new();
    for name in plugins.keys() {
        let b = std::fs::read(data.join(name))?;
        files.insert(name.clone(), FileReceipt { sha256: format!("{:x}", Sha256::digest(&b)), bytes: b.len() as u64 });
    }
    let tmp = root.join(format!("{ZIP_NAME}.part"));
    {
        let f = std::fs::File::create(&tmp)?;
        let mut z = zip::ZipWriter::new(std::io::BufWriter::new(f));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).large_file(true);
        for name in plugins.keys() {
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
    let rec = Record { list: hash.to_string(), plugins, zip_sha256: format!("{:x}", h.finalize()), zip_bytes: bytes, files, source };
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

    #[test]
    fn zips_the_plugins_and_remembers_the_list() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join(LANE_DIR);
        let src = t.path().join("a.esp");
        std::fs::write(&src, b"plugin bytes").unwrap();
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
        assert_eq!(rec.files["JKs Skyrim.esp"].bytes, 12);
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
