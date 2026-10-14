//! Checks a mod pack (a folder with the pack's files laid out like the game's Data folder, or an
//! extracted Nexus collection) against Skyrim 1.6.1170 + SKSE 2.2.6 and the server's plugin order,
//! before anyone presses Play (Timothy, 2026-10-14: "I really need a way to play test the mod packs
//! easily and fast"). Read-only; it never changes the folder and never stops Play by itself.
//!
//! Rev 7 failed three ways that none of the launcher's checks looked at: a Vortex re-deploy, an SKSE
//! DLL that asks the Address Library for an ID the 1.6.1170 file lacks, and a DLL that is not a
//! 1.6.1170 build. `skse::build_of` already knew the second kind of DLL but nothing called it.
//!
//! What it cannot know offline: whether a DLL that SKSE accepts really runs. That is the ledger
//! (`data/dll-ledger.json`): what each DLL did in a real game. A DLL not in it is UNVERIFIED and goes
//! on the play-test list instead of being called fine.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{aliases, loadorder, serverorder, skse};

const BUILT_IN_LEDGER: &str = include_str!("../data/dll-ledger.json");
pub const ADDRESS_LIBRARY_FILE: &str = "versionlib-1-6-1170-0.bin";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Sev {
    Ok,
    Info,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub sev: Sev,
    /// Short check name: skse-build, ledger, address-library, master, order, plugin, light-limit.
    pub check: &'static str,
    pub subject: String,
    pub detail: String,
}

fn f(sev: Sev, check: &'static str, subject: impl Into<String>, detail: impl Into<String>) -> Finding {
    Finding { sev, check, subject: subject.into(), detail: detail.into() }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LedgerEntry {
    pub file: String,
    /// "crashes", "refused" or "loads".
    pub status: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub evidence: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Ledger {
    #[serde(default)]
    pub entries: Vec<LedgerEntry>,
}

impl Ledger {
    pub fn built_in() -> Ledger {
        Ledger::parse(BUILT_IN_LEDGER.as_bytes()).expect("built-in ledger is valid")
    }
    pub fn parse(b: &[u8]) -> Result<Ledger, String> {
        let l: Ledger = serde_json::from_slice(b).map_err(|e| e.to_string())?;
        if let Some(e) = l.entries.iter().find(|e| !matches!(e.status.as_str(), "crashes" | "refused" | "loads")) {
            return Err(format!("ledger entry {}: status must be crashes, refused or loads", e.file));
        }
        Ok(l)
    }
    fn find(&self, file: &str, sha: &str) -> Option<&LedgerEntry> {
        self.entries.iter().find(|e| e.file.eq_ignore_ascii_case(file) && e.sha256.as_deref().is_none_or(|s| s.eq_ignore_ascii_case(sha)))
    }
}

pub struct Input<'a> {
    pub root: &'a Path,
    /// The server's plugins after the five base masters, in order (masters.json).
    pub server: &'a [String],
    /// The pack's plugins in load order (collection pluginOrder); empty = order not checked.
    pub plugin_order: &'a [String],
    pub ledger: &'a Ledger,
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if depth < 8 {
                walk(&p, depth + 1, out);
            }
        } else {
            out.push(p);
        }
    }
}

fn lower_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

fn is_plugin_file(n: &str) -> bool {
    n.ends_with(".esp") || n.ends_with(".esm") || n.ends_with(".esl")
}

fn light(p: &Path) -> bool {
    use std::io::Read;
    let mut h = [0u8; 12];
    std::fs::File::open(p).and_then(|mut f| f.read_exact(&mut h)).is_ok_and(|_| &h[..4] == b"TES4" && h[8] & 0x02 != 0 && h[9] & 0x02 == 0 || lower_name(p).ends_with(".esl"))
}

/// Runs every check. Order of findings: DLLs, Address Library, plugins.
pub fn scan(i: &Input) -> Vec<Finding> {
    let mut files = Vec::new();
    walk(i.root, 0, &mut files);
    files.sort();
    let mut out = Vec::new();

    // ---- SKSE plugin DLLs (a DLL directly inside a ...SKSE/Plugins folder)
    let mut dlls: BTreeMap<String, Vec<(PathBuf, String)>> = BTreeMap::new();
    for p in &files {
        let rel = p.strip_prefix(i.root).unwrap_or(p).to_string_lossy().replace('\\', "/").to_ascii_lowercase();
        if rel.ends_with(".dll") && rel.rsplit_once('/').is_some_and(|(d, _)| d.ends_with("skse/plugins")) {
            use sha2::Digest;
            let Ok(bytes) = std::fs::read(p) else { continue };
            dlls.entry(lower_name(p)).or_default().push((p.clone(), hex::encode(sha2::Sha256::digest(&bytes))));
        }
    }
    let mut needs_al = Vec::new();
    for (name, copies) in &dlls {
        let distinct: BTreeSet<&String> = copies.iter().map(|c| &c.1).collect();
        // Several different builds under one name (FOMOD variants in an unextracted archive): which one
        // gets installed is the installer's choice, so a bad one is a warning, not a failure.
        let variants = distinct.len() > 1;
        for (p, sha) in copies {
            let shown = p.strip_prefix(i.root).unwrap_or(p).display().to_string();
            let cap = |s: Sev| if variants && s == Sev::Fail { Sev::Warn } else { s };
            let tag = if variants { " (one of several builds under this name; the installer picks)" } else { "" };
            let bytes = std::fs::read(p).unwrap_or_default();
            let mut bad = false;
            match skse::build_of(p) {
                skse::Build::Wrong(w) => {
                    bad = true;
                    out.push(f(cap(Sev::Fail), "skse-build", &shown, format!("SKSE 2.2.6 refuses it on 1.6.1170: {w}{tag}")));
                }
                skse::Build::Unknown => out.push(f(Sev::Info, "skse-build", &shown, "not a readable SKSE plugin (a helper DLL?); not judged")),
                skse::Build::Fits => {}
            }
            if skse::uses_address_library(&bytes) == Some(true) {
                needs_al.push(shown.clone());
            }
            match i.ledger.find(name, sha) {
                Some(e) if e.status != "loads" => {
                    out.push(f(cap(Sev::Fail), "ledger", &shown, format!("known {} on 1.6.1170: {} [{}]{tag}", e.status, e.reason, e.evidence)));
                }
                Some(e) => out.push(f(Sev::Ok, "ledger", &shown, format!("seen loading on 1.6.1170 [{}]", e.evidence))),
                None if !bad => out.push(f(Sev::Warn, "ledger", &shown, "UNVERIFIED: never seen loading on 1.6.1170 in a real game; put it on the play-test list")),
                None => {}
            }
        }
    }

    // ---- Address Library
    let al: Vec<&PathBuf> = files.iter().filter(|p| lower_name(p) == ADDRESS_LIBRARY_FILE).collect();
    if !needs_al.is_empty() {
        if al.is_empty() {
            out.push(f(Sev::Fail, "address-library", ADDRESS_LIBRARY_FILE, format!("{} plugin(s) look code up by ID but the pack has no {ADDRESS_LIBRARY_FILE}: {}", needs_al.len(), needs_al.join(", "))));
        } else {
            let detail = match std::fs::read(al[0]).map_err(|e| e.to_string()).and_then(|b| crate::addrlib::parse(&b)) {
                Ok(db) => format!("{} IDs, version {:?}; plugins that use it: {}. Whether each plugin's IDs are in it is only known from a game run (ledger)", db.ids.len(), db.version, needs_al.join(", ")),
                Err(e) => format!("present but not readable here ({e}); plugins that use it: {}", needs_al.join(", ")),
            };
            out.push(f(Sev::Info, "address-library", ADDRESS_LIBRARY_FILE, detail));
        }
    }

    // ---- Plugins
    let mut plugins: BTreeMap<String, PathBuf> = BTreeMap::new();
    for p in &files {
        let n = lower_name(p);
        if is_plugin_file(&n) {
            plugins.entry(n).or_insert_with(|| p.clone());
        }
    }
    let base: Vec<String> = serverorder::BASE.iter().map(|s| s.to_ascii_lowercase()).collect();
    let server: Vec<String> = i.server.iter().map(|s| s.to_ascii_lowercase()).collect();
    let pack_names: BTreeSet<String> = plugins.keys().flat_map(|n| [n.clone(), aliases::alias_name(n).to_ascii_lowercase()]).collect();
    // Position of each plugin: base, then the server's, then the pack's own order.
    let mut pos: BTreeMap<String, usize> = BTreeMap::new();
    for n in base.iter().chain(server.iter()).cloned().chain(i.plugin_order.iter().flat_map(|n| [n.to_ascii_lowercase(), aliases::alias_name(n).to_ascii_lowercase()])) {
        let next = pos.len();
        pos.entry(n).or_insert(next);
    }
    let mut full = base.len() + server.len();
    for (name, p) in &plugins {
        let shown = p.strip_prefix(i.root).unwrap_or(p).display().to_string();
        if let Some(why) = loadorder::broken(p) {
            out.push(f(Sev::Fail, "plugin", &shown, format!("unusable plugin: {why}")));
            continue;
        }
        if !light(p) && !server.contains(name) && !server.contains(&aliases::alias_name(name).to_ascii_lowercase()) {
            full += 1;
        }
        if name.contains(' ') {
            out.push(f(Sev::Info, "plugin", &shown, format!("name has spaces; the launcher runs it as {} (the SkyMP client rejects spaces)", aliases::alias_name(name))));
        }
        if !i.plugin_order.is_empty() && !pos.contains_key(name) {
            out.push(f(Sev::Info, "order", &shown, "not in the collection's plugin order list"));
        }
        for m in loadorder::masters(p).unwrap_or_default() {
            let ml = m.to_ascii_lowercase();
            let known = base.contains(&ml) || server.contains(&ml) || pack_names.contains(&ml) || pack_names.contains(&aliases::alias_name(&m).to_ascii_lowercase()) || crate::modguard::is_cc(&m);
            if !known {
                out.push(f(Sev::Fail, "master", &shown, format!("needs {m}, which is not in the pack, the server's plugins or the base game; the plugin must stay off")));
            } else if let (Some(a), Some(b)) = (pos.get(&ml).or_else(|| pos.get(&aliases::alias_name(&m).to_ascii_lowercase())), pos.get(name).or_else(|| pos.get(&aliases::alias_name(name).to_ascii_lowercase()))) {
                if a > b {
                    out.push(f(Sev::Warn, "order", &shown, format!("loads before its master {m} in the planned order")));
                }
            }
        }
    }
    if full > 254 {
        out.push(f(Sev::Fail, "light-limit", "plugins", format!("{full} full (non-light) plugins with the base and the server's; the limit is 254")));
    }
    out
}

pub fn worst(fs: &[Finding]) -> Sev {
    fs.iter().map(|x| x.sev).max().unwrap_or(Sev::Ok)
}

/// Plain text for a terminal or a CI log, worst first, plus the play-test list.
pub fn render(fs: &[Finding]) -> String {
    let mut sorted: Vec<&Finding> = fs.iter().collect();
    sorted.sort_by_key(|x| std::cmp::Reverse(x.sev));
    let mut s = String::new();
    for x in &sorted {
        s.push_str(&format!("{:<5} {:<15} {} : {}\n", format!("{:?}", x.sev).to_uppercase(), x.check, x.subject, x.detail));
    }
    let test: Vec<&str> = fs.iter().filter(|x| x.check == "ledger" && x.sev == Sev::Warn).map(|x| x.subject.as_str()).collect();
    if !test.is_empty() {
        s.push_str(&format!("\nPlay-test first (UNVERIFIED DLLs): {}\n", test.join(", ")));
    }
    s.push_str(&format!("\nResult: {:?} ({} findings)\n", worst(fs), fs.len()));
    s
}

/// A ledger line proposed from an skse64.log, to paste into data/dll-ledger.json after a play-test.
#[derive(Debug, Serialize, PartialEq)]
pub struct Suggestion {
    pub file: String,
    pub status: &'static str,
    pub reason: String,
}

/// Reads an skse64.log (or the text of the SKSE / Address Library error boxes).
pub fn triage(log: &str) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> = Vec::new();
    let dll_in = |line: &str| -> Option<String> {
        let l = line.to_ascii_lowercase();
        let end = l.find(".dll")? + 4;
        let start = l[..end].rfind(['\\', '/', ' ', '"', '\''])? + 1;
        Some(line[start..end].to_string())
    };
    for line in log.lines() {
        let l = line.to_ascii_lowercase();
        let Some(file) = dll_in(line) else { continue };
        let s = if let Some(at) = l.find("identifier not found") {
            let id: String = line[at..].chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit()).collect();
            Suggestion { file, status: "crashes", reason: format!("Address Library \"Identifier not found, {id}\"") }
        } else if l.contains("disabled, incompatible") || l.contains("disabled, only compatible") || l.contains("disabled, no version data") || l.contains("disabled, bad version") {
            Suggestion { file, status: "refused", reason: line.trim().to_string() }
        } else if l.contains("loaded correctly") {
            Suggestion { file, status: "loads", reason: "SKSE: loaded correctly".into() }
        } else if l.contains("couldn't load") || l.contains("could not load") {
            Suggestion { file, status: "refused", reason: line.trim().to_string() }
        } else {
            continue;
        };
        // A later bad line beats an earlier "loaded correctly" (ActorLimitFix loads, then asks for the missing ID).
        match out.iter_mut().find(|x| x.file.eq_ignore_ascii_case(&s.file)) {
            Some(x) if x.status == "loads" => *x = s,
            Some(_) => {}
            None => out.push(s),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skse::tests as dllgen;

    fn pack() -> tempfile::TempDir {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join("Data/SKSE/Plugins")).unwrap();
        t
    }
    fn input<'a>(t: &'a Path, server: &'a [String], order: &'a [String], l: &'a Ledger) -> Input<'a> {
        Input { root: t, server, plugin_order: order, ledger: l }
    }

    #[test]
    fn built_in_ledger_parses_and_knows_the_two_rev7_dlls() {
        let l = Ledger::built_in();
        assert_eq!(l.find("actorlimitfix.dll", "x").unwrap().status, "crashes");
        assert_eq!(l.find("JContainers64.dll", "x").unwrap().status, "refused");
        assert!(Ledger::parse(br#"{"entries":[{"file":"a.dll","status":"fine"}]}"#).is_err());
    }

    #[test]
    fn an_old_build_and_a_ledger_crash_fail_and_an_unknown_good_dll_is_unverified() {
        let t = pack();
        let d = t.path().join("Data/SKSE/Plugins");
        std::fs::write(d.join("Old.dll"), dllgen::old_dll()).unwrap();
        std::fs::write(d.join("ActorLimitFix.dll"), dllgen::good_dll()).unwrap();
        std::fs::write(d.join("Fresh.dll"), dllgen::good_dll()).unwrap();
        let l = Ledger::built_in();
        let r = scan(&input(t.path(), &[], &[], &l));
        let get = |c: &str, s: &str| r.iter().find(|x| x.check == c && x.subject.contains(s)).map(|x| x.sev);
        assert_eq!(get("skse-build", "Old.dll"), Some(Sev::Fail));
        assert_eq!(get("ledger", "ActorLimitFix.dll"), Some(Sev::Fail));
        assert_eq!(get("ledger", "Fresh.dll"), Some(Sev::Warn));
        assert_eq!(worst(&r), Sev::Fail);
        assert!(render(&r).contains("Play-test first (UNVERIFIED DLLs): Data/SKSE/Plugins/Fresh.dll"));
    }

    #[test]
    fn variants_of_one_dll_name_only_warn() {
        let t = tempfile::tempdir().unwrap();
        for v in ["a", "b"] {
            std::fs::create_dir_all(t.path().join(v).join("SKSE/Plugins")).unwrap();
        }
        std::fs::write(t.path().join("a/SKSE/Plugins/X.dll"), dllgen::old_dll()).unwrap();
        std::fs::write(t.path().join("b/SKSE/Plugins/X.dll"), dllgen::good_dll()).unwrap();
        std::fs::write(t.path().join("b/SKSE/Plugins").join(ADDRESS_LIBRARY_FILE), crate::addrlib::tests::sample()).unwrap();
        let l = Ledger::default();
        let r = scan(&input(t.path(), &[], &[], &l));
        assert_eq!(worst(&r), Sev::Warn, "{}", render(&r));
    }

    #[test]
    fn plugin_with_a_missing_master_fails_and_a_late_master_warns() {
        let t = pack();
        let d = t.path().join("Data");
        std::fs::write(d.join("Needs.esp"), loadorder::tests::plugin_with_masters(&["Gone.esp"])).unwrap();
        std::fs::write(d.join("Early.esp"), loadorder::tests::plugin_with_masters(&["Late.esp"])).unwrap();
        std::fs::write(d.join("Late.esp"), loadorder::tests::plugin_with_masters(&[])).unwrap();
        let order: Vec<String> = ["Early.esp", "Late.esp"].map(String::from).to_vec();
        let l = Ledger::default();
        let r = scan(&input(t.path(), &[], &order, &l));
        assert!(r.iter().any(|x| x.check == "master" && x.sev == Sev::Fail && x.subject.contains("Needs.esp")));
        assert!(r.iter().any(|x| x.check == "order" && x.sev == Sev::Warn && x.subject.contains("Early.esp")));
        assert!(!r.iter().any(|x| x.subject.contains("Late.esp") && x.sev >= Sev::Warn));
    }

    #[test]
    fn address_library_dll_without_the_bin_fails_and_with_it_reports() {
        let t = pack();
        let d = t.path().join("Data/SKSE/Plugins");
        std::fs::write(d.join("Al.dll"), dllgen::good_dll()).unwrap();
        let l = Ledger::default();
        let needs = skse::uses_address_library(&dllgen::good_dll());
        let r = scan(&input(t.path(), &[], &[], &l));
        if needs == Some(true) {
            assert!(r.iter().any(|x| x.check == "address-library" && x.sev == Sev::Fail));
            std::fs::write(d.join(ADDRESS_LIBRARY_FILE), crate::addrlib::tests::sample()).unwrap();
            let r = scan(&input(t.path(), &[], &[], &l));
            assert!(r.iter().any(|x| x.check == "address-library" && x.sev == Sev::Info));
        }
    }

    #[test]
    fn triage_turns_the_rev7_boxes_into_ledger_lines() {
        let log = "plugin D:\\Skyrim\\Data\\SKSE\\Plugins\\ActorLimitFix.dll (00000001 Actor Limit Fix 00000001) loaded correctly\n\
                   AddressLibrary.cpp(102,34): Identifier not found, 523948 ActorLimitFix.dll\n\
                   plugin D:\\x\\JContainers64.dll (00000000 JContainers64 00000000) disabled, incompatible with current version of the game\n\
                   plugin D:\\x\\CustomSkills.dll (00000001 Custom Skills 00000001) loaded correctly\n";
        let s = triage(log);
        assert_eq!(s.len(), 3);
        assert_eq!((s[0].file.as_str(), s[0].status), ("ActorLimitFix.dll", "crashes"));
        assert!(s[0].reason.contains("523948"));
        assert_eq!(s[1].status, "refused");
        assert_eq!(s[2].status, "loads");
    }
}
