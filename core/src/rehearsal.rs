//! Cutover day from a player's side (coordinator's ask, 2026-09-28), with
//! stand-in archives only: no Nexus file is in this repository or fetched by
//! it. The archives are made here and served from 127.0.0.1, and go through
//! the launcher's own code: the download (fetch.rs), the installer
//! (modlist::install_archive, what mods.rs runs after a download) and Play's
//! steps in Play's order (the mod list saved, removed mods set aside, the
//! dashed names, the missing-mods check, the server's order, the order
//! check).
//!
//! The lists are shaped like the real ones (world-mods): today's 30 entries
//! (15 with a plugin of their own), the 3.4.6 list that adds 75 (70 server
//! plugins in a frozen order, 47 of them under a dashed name and 5 flagged
//! ESL so they run as "-AD" copies, 2 client-only plugins, 20 entries with
//! no plugin, plugins lifted out of a folder, archives and string files
//! that follow a renamed plugin), then a later append: 2 plugins appended
//! at the end, a server plugin replaced in place by a new pin, a new pin
//! that drops a file, and a removed entry.
//!
//! A returning player on today's list and a new player each go to 3.4.6 and
//! then the append, with dropped connections, a launcher closed during a
//! download and one closed during an install. After every step Play must
//! refuse until everything is in; at the end the game loads exactly the
//! server's plugins in its order (then the list's client-only ones) and
//! nothing of a removed mod or an old pin is left in Data.
//!
//! `cargo test -p launcher-core rehearsal -- --nocapture` prints the numbers.

use std::collections::{BTreeSet, HashMap};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::modlist::{self, ModEntry, ModList, NexusRef, Outcome};
use crate::serverorder::{self, BASE};
use crate::{aliases, allowlist, fetch};

// ---- stand-in files ----

/// A plugin: TES4 header (HEDR 1.71, form version 44, masters), then
/// stand-in record bytes that differ per plugin and per version.
fn plugin(masters: &[String], flags: u32, body: usize, seed: u32) -> Vec<u8> {
    let mut sub = Vec::new();
    sub.extend(b"HEDR");
    sub.extend(12u16.to_le_bytes());
    sub.extend(1.71f32.to_le_bytes());
    sub.extend(0i32.to_le_bytes());
    sub.extend(0x800u32.to_le_bytes());
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
    b.extend((0..body).map(|i| ((i as u32).wrapping_mul(2_654_435_761).wrapping_add(seed.wrapping_mul(40_503)) >> 24) as u8));
    b
}

fn filler(n: usize, seed: u32) -> Vec<u8> {
    (0..n).map(|i| ((i as u32 ^ seed).wrapping_mul(2_246_822_519) >> 24) as u8).collect()
}

fn zip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (n, b) in files {
        w.start_file(n.as_str(), o).unwrap();
        w.write_all(b).unwrap();
    }
    w.finish().unwrap().into_inner()
}

const LIGHT: u32 = 0x200;
const MASTER: u32 = 0x1;

#[derive(Clone)]
struct Plug {
    /// Name in the download.
    name: String,
    masters: Vec<String>,
    flags: u32,
    /// Loaded by the server (else client-only).
    server: bool,
    /// Kept in a folder of choices in the archive and lifted to Data.
    lift: bool,
    /// An archive and a string file that follow the plugin's name.
    companions: bool,
}

#[derive(Clone)]
struct Stand {
    entry: ModEntry,
    plugins: Vec<Plug>,
    /// Other files, archive path -> size.
    loose: Vec<(String, usize)>,
    /// Pin version, so a new pin makes new bytes.
    version: u32,
    /// Bytes of padding in the archive (a big download).
    big: usize,
}

impl Stand {
    fn new(id: &str, n: u64, plugins: Vec<Plug>, loose: Vec<(String, usize)>) -> Stand {
        let mut check: Vec<String> = plugins.iter().map(|p| format!("Data/{}", p.name)).collect();
        if check.is_empty() {
            // Most texture-only entries check nothing and count by the record.
            if n.is_multiple_of(3) {
                check.extend(loose.first().map(|(p, _)| format!("Data/{p}")));
            }
        }
        let lift = plugins.iter().filter(|p| p.lift).map(|p| format!("plugins/esp/{}", p.name)).collect();
        let entry = ModEntry {
            id: id.into(),
            name: format!("Stand-in {id}"),
            nexus: Some(NexusRef { mod_id: 900_000 + n, file: Some(5_000_000 + n * 10), pick: None }),
            check,
            lift,
            ..Default::default()
        };
        Stand { entry, plugins, loose, version: 1, big: 0 }
    }

    fn file_id(&self) -> u64 {
        self.entry.nexus.as_ref().and_then(|n| n.file).unwrap()
    }

    /// The same mod pinned to a newer file.
    fn repin(&self) -> Stand {
        let mut s = self.clone();
        s.version += 1;
        if let Some(n) = s.entry.nexus.as_mut() {
            n.file = n.file.map(|f| f + 1);
        }
        s
    }

    fn seed(&self, name: &str) -> u32 {
        name.bytes().fold(self.version * 7919, |a, b| a.wrapping_mul(31).wrapping_add(b as u32))
    }

    fn plugin_bytes(&self, p: &Plug) -> Vec<u8> {
        plugin(&p.masters, p.flags, 3000 + (self.seed(&p.name) % 5000) as usize, self.seed(&p.name))
    }

    fn archive(&self) -> Vec<u8> {
        let mut files = Vec::new();
        for p in &self.plugins {
            let at = if p.lift { format!("plugins/esp/{}", p.name) } else { p.name.clone() };
            files.push((at, self.plugin_bytes(p)));
            if p.companions {
                let stem = p.name.rsplit_once('.').unwrap().0;
                files.push((format!("{stem}.bsa"), filler(4000, self.seed(stem))));
                files.push((format!("strings/{stem}_english.strings"), filler(300, self.seed(stem) + 1)));
            }
        }
        for (path, size) in &self.loose {
            files.push((path.clone(), filler(*size, self.seed(path))));
        }
        if self.big > 0 {
            files.push((format!("textures/{}/big.dds", self.entry.id), filler(self.big, self.seed("big"))));
        }
        zip(&files)
    }
}

fn plug(name: &str) -> Plug {
    Plug { name: name.into(), masters: Vec::new(), flags: 0, server: true, lift: false, companions: false }
}

/// Names shaped like the real list's: spaces, apostrophes, ampersands and
/// dots that the client can't load, and plain ones.
fn server_name(i: usize) -> String {
    match i % 10 {
        0 => format!("Stand-in City {i:02}.esp"),
        1 => format!("SI_Town{i:02}.esp"),
        2 => format!("Stand-in's Keep {i:02}.esp"),
        3 => format!("Cloaks & Capes {i:02}.esp"),
        4 => format!("Stand-in Hall {i:02} - Patch.esp"),
        5 => format!("SI_Armory{i:02}.esp"),
        6 => format!("Stand-in Road {i:02}. Brighter.esp"),
        7 => format!("Stand-in Farm {i:02}.esp"),
        8 => format!("SI-Outfits-{i:02}.esp"),
        _ => format!("Stand-in Dock {i:02}.esp"),
    }
}

fn textures(id: &str, k: usize) -> Vec<(String, usize)> {
    (0..k).map(|j| (format!("textures/{id}/t{j}.dds"), 2000 + j * 300)).collect()
}

struct Lists {
    today: Vec<Stand>,
    cutover: Vec<Stand>,
    append: Vec<Stand>,
    /// Server plugins in order (names in the download), per list.
    order_cutover: Vec<String>,
    order_append: Vec<String>,
}

fn lists() -> Lists {
    // Today: 30 client-lane entries, 15 with a plugin (a weather and its
    // patch that masters it, like Obsidian Weathers and Obsidian CS).
    let mut today = Vec::new();
    for i in 0..30u64 {
        let id = format!("today-{i:02}");
        let plugins = match i {
            0 => vec![Plug { server: false, ..plug("Stand-in Weathers.esp") }],
            1 => vec![Plug { server: false, masters: vec!["Skyrim.esm".into(), "Stand-in Weathers.esp".into()], ..plug("Stand-in Weathers CS.esp") }],
            2..=14 => vec![Plug { server: false, ..plug(&format!("Stand-in Client {i:02}.esp")) }],
            _ => vec![],
        };
        let mut s = Stand::new(&id, i, plugins, textures(&id, 2 + (i as usize % 3)));
        if i == 3 {
            s.entry.check.push(format!("Data/SKSE/Plugins/StandIn{i}.dll"));
            s.loose.push((format!("SKSE/Plugins/StandIn{i}.dll"), 5000));
        }
        today.push(s);
    }
    // 3.4.6: 75 more entries, 70 server plugins.
    let mut lane: Vec<Stand> = Vec::new();
    let mut order = Vec::new();
    let mut n = 100u64;
    let mut next = |plugins: Vec<Plug>, loose: Vec<(String, usize)>, lane: &mut Vec<Stand>, order: &mut Vec<String>| {
        let id = format!("lane-{:02}", lane.len());
        for p in plugins.iter().filter(|p| p.server) {
            order.push(p.name.clone());
        }
        lane.push(Stand::new(&id, n, plugins, loose));
        n += 1;
    };
    // A master at the top of the order.
    next(vec![Plug { flags: MASTER, ..plug("Stand-in Resources - Cities.esm") }], textures("lane-00", 2), &mut lane, &mut order);
    let mut i = 1;
    while order.len() < 50 {
        let mut p = plug(&server_name(i));
        p.lift = (45..50).contains(&order.len());
        p.companions = i % 7 == 0;
        next(vec![p], textures(&format!("lane-{:02}", lane.len()), 1), &mut lane, &mut order);
        i += 1;
    }
    // Four plugins, one ESL-flagged, one patch mastering a dashed plugin
    // (runs as "-AD"), like the College of Winterhold entry.
    next(
        vec![
            plug("SI Obscure College.esp"),
            Plug { flags: LIGHT, ..plug("SI_CellSettings.esp") },
            Plug { masters: vec!["SI Obscure College.esp".into()], ..plug("SI_College_FEPatch.esp") },
            Plug { masters: vec!["SI Obscure College.esp".into()], ..plug("SI_College_RLSPatch.esp") },
        ],
        vec![],
        &mut lane,
        &mut order,
    );
    // Three, two ESL-flagged, like More Craftable Equipment.
    next(
        vec![
            plug("SI Craftable Equipment.esp"),
            Plug { flags: LIGHT, masters: vec!["SI Craftable Equipment.esp".into()], ..plug("SI_Craftable_Cloaks.esp") },
            Plug { flags: LIGHT, masters: vec!["SI Craftable Equipment.esp".into()], ..plug("SI_Craftable_USSEP.esp") },
        ],
        vec![],
        &mut lane,
        &mut order,
    );
    // Eleven from one download, patches mastering the first, like Sentinel.
    let mut eleven = vec![plug("SI Sentinel.esp")];
    for k in 1..11 {
        eleven.push(Plug { masters: vec!["SI Sentinel.esp".into()], ..plug(&format!("SI Sentinel - Part {k:02}.esp")) });
    }
    next(eleven, textures("lane-sentinel", 2), &mut lane, &mut order);
    next(vec![plug("SI Morthal City.esp"), Plug { masters: vec!["SI Morthal City.esp".into()], ..plug("SI Morthal - JK Patch.esp") }], vec![], &mut lane, &mut order);
    assert_eq!(order.len(), 70);
    // Two client-only plugins (a body patch and a BodySlide file).
    next(vec![Plug { server: false, ..plug("SI Body Patch.esp") }, Plug { server: false, ..plug("SI Sentinel Bodyslide.esp") }], vec![], &mut lane, &mut order);
    // Twenty with no plugin (outfits, textures).
    while lane.len() < 75 {
        let id = format!("lane-{:02}", lane.len());
        next(vec![], textures(&id, 2), &mut lane, &mut order);
    }
    // Three big downloads for the dropped connections and the closed launcher.
    for (k, s) in lane.iter_mut().enumerate().filter(|(k, _)| [3, 17, 60].contains(k)) {
        s.big = if k == 17 { 1_200_000 } else { 600_000 };
    }
    let mut cutover = today.clone();
    cutover.extend(lane);
    // The later append: two plugins appended at the end, a server plugin
    // replaced in place (a new pin, same name and place), a new pin that
    // drops a file, and a removed entry.
    let mut append = cutover.clone();
    let heavy = Stand::new(
        "append-heavy-armory",
        500,
        vec![plug("SI Heavy Armory.esp"), Plug { masters: vec!["SI Heavy Armory.esp".into()], ..plug("SI Heavy Armory - Patch.esp") }],
        textures("append-heavy-armory", 2),
    );
    let mut order_append = order.clone();
    order_append.extend(["SI Heavy Armory.esp".to_string(), "SI Heavy Armory - Patch.esp".to_string()]);
    append.push(heavy);
    // The server plugin at order position 22 (index 27), replaced in place.
    let at = append.iter().position(|s| s.plugins.iter().any(|p| p.name == order[22])).unwrap();
    append[at] = append[at].repin();
    // A client-lane mod whose new file drops one texture and adds another.
    let at = append.iter().position(|s| s.entry.id == "today-20").unwrap();
    let mut r = append[at].repin();
    r.loose.remove(0);
    r.loose.push(("textures/today-20/new.dds".into(), 2500));
    append[at] = r;
    // Removed: a client plugin mod with its textures.
    append.retain(|s| s.entry.id != "today-05");
    Lists { today, cutover, append, order_cutover: order, order_append }
}

// ---- the server ----

/// Serves each stand-in archive at /<id>-<file id>.zip. `drops[path]`
/// connections to a path drop at 60%; `rate[path]` slows one (bytes/s).
struct Server {
    base: String,
    files: Arc<Mutex<HashMap<String, Arc<Vec<u8>>>>>,
    drops: Arc<Mutex<HashMap<String, usize>>>,
    rate: Arc<Mutex<HashMap<String, usize>>>,
    requests: Arc<AtomicUsize>,
    ranged: Arc<AtomicUsize>,
}

impl Server {
    fn start() -> Server {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
        let files: Arc<Mutex<HashMap<String, Arc<Vec<u8>>>>> = Arc::default();
        let drops: Arc<Mutex<HashMap<String, usize>>> = Arc::default();
        let rate: Arc<Mutex<HashMap<String, usize>>> = Arc::default();
        let requests = Arc::new(AtomicUsize::new(0));
        let ranged = Arc::new(AtomicUsize::new(0));
        let (f, d, r, rq, rg) = (files.clone(), drops.clone(), rate.clone(), requests.clone(), ranged.clone());
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(mut s) = s else { continue };
                let (f, d, r, rq, rg) = (f.clone(), d.clone(), r.clone(), rq.clone(), rg.clone());
                std::thread::spawn(move || {
                    rq.fetch_add(1, Ordering::SeqCst);
                    let mut buf = [0u8; 4096];
                    let len = s.read(&mut buf).unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..len]).to_string();
                    let path = head.split_whitespace().nth(1).unwrap_or("/").trim_start_matches('/').to_string();
                    let Some(body) = f.lock().unwrap().get(&path).cloned() else {
                        let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        return;
                    };
                    let start = head.to_ascii_lowercase().lines().find_map(|l| l.strip_prefix("range: bytes=").map(|v| v.trim().trim_end_matches('-').parse::<usize>().unwrap_or(0))).unwrap_or(0);
                    if start > 0 {
                        rg.fetch_add(1, Ordering::SeqCst);
                    }
                    let total = body.len();
                    let hdr = if start > 0 {
                        format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{}/{total}\r\nConnection: close\r\n\r\n", total - start, total - 1)
                    } else {
                        format!("HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n")
                    };
                    let _ = s.write_all(hdr.as_bytes());
                    let drop_here = {
                        let mut d = d.lock().unwrap();
                        match d.get_mut(&path) {
                            Some(n) if *n > 0 => {
                                *n -= 1;
                                true
                            }
                            _ => false,
                        }
                    };
                    let rate = r.lock().unwrap().get(&path).copied().unwrap_or(0);
                    let chunk = if rate > 0 { (rate / 20).max(1) } else { 64 * 1024 };
                    let mut at = start;
                    while at < total {
                        if drop_here && at >= total * 6 / 10 {
                            return;
                        }
                        let end = (at + chunk).min(total);
                        if s.write_all(&body[at..end]).is_err() {
                            return;
                        }
                        at = end;
                        if rate > 0 {
                            std::thread::sleep(Duration::from_millis(50));
                        }
                    }
                });
            }
        });
        Server { base, files, drops, rate, requests, ranged }
    }

    fn path(s: &Stand) -> String {
        format!("{}-{}.zip", s.entry.id, s.file_id())
    }

    fn publish(&self, list: &[Stand]) {
        let mut f = self.files.lock().unwrap();
        for s in list {
            f.insert(Server::path(s), Arc::new(s.archive()));
        }
    }
}

// ---- the server's side of the canonical rename ----

/// masters.json as the server publishes it: its Data (the five masters and
/// every server plugin from the downloads) through canonicalize_dir, in the
/// frozen order, with size and crc32.
fn masters_json(list: &[Stand], order: &[String], base: &Path) -> (serde_json::Value, Vec<String>) {
    let t = tempfile::tempdir().unwrap();
    let data = t.path().join("Data");
    std::fs::create_dir_all(&data).unwrap();
    for b in BASE {
        std::fs::copy(base.join(b), data.join(b)).unwrap();
    }
    for s in list {
        for p in s.plugins.iter().filter(|p| p.server) {
            std::fs::write(data.join(&p.name), s.plugin_bytes(p)).unwrap();
        }
    }
    let out = t.path().join("canonical");
    let canon = aliases::canonicalize_dir(&data, &out).unwrap();
    let mut entries = Vec::new();
    let mut names = Vec::new();
    for b in BASE {
        let p = base.join(b);
        entries.push(serde_json::json!({"name": b, "size": std::fs::metadata(&p).unwrap().len(), "crc32": serverorder::crc32(&p).unwrap()}));
        names.push(b.to_string());
    }
    for o in order {
        let c = canon.iter().find(|c| &c.original == o).unwrap_or_else(|| panic!("{o} not canonicalized"));
        let p = out.join(&c.name);
        entries.push(serde_json::json!({"name": c.name, "size": std::fs::metadata(&p).unwrap().len(), "crc32": serverorder::crc32(&p).unwrap()}));
        names.push(c.name.clone());
    }
    (serde_json::json!({ "masters": entries }), names)
}

// ---- a player's PC ----

struct Pc {
    _t: tempfile::TempDir,
    game: PathBuf,
    txt: PathBuf,
}

impl Pc {
    fn new(base: &Path) -> Pc {
        let t = tempfile::tempdir().unwrap();
        let game = t.path().join("Skyrim Special Edition");
        std::fs::create_dir_all(game.join("Data")).unwrap();
        for b in BASE {
            std::fs::copy(base.join(b), game.join("Data").join(b)).unwrap();
        }
        let txt = t.path().join("plugins.txt");
        std::fs::write(&txt, "# This file is used by Skyrim to keep track of your downloaded content.\r\n").unwrap();
        Pc { _t: t, game, txt }
    }
}

#[derive(Debug, PartialEq)]
enum Gate {
    /// Play stops and offers these downloads (NEEDS_NEXUS_MODS).
    NeedsMods(usize),
    /// Play stops with "Your plugins don't match the server's order".
    Order(usize),
    Plays,
}

fn list_of(s: &[Stand]) -> ModList {
    ModList { mods: s.iter().map(|s| s.entry.clone()).collect(), nexus_app: None }
}

/// Play's steps in 0.1.98's order (main.rs play and tidy_game): the list is
/// saved, removed mods go aside, plugins get their dashed names, missing
/// mods stop Play first, then the server's order is set and checked.
fn play(pc: &Pc, list: &ModList, masters: &serde_json::Value, old_order: bool) -> Gate {
    let order = serverorder::server_order(masters);
    allowlist::save_server_list(&pc.game, list);
    let stamp = format!("{}-removed-mods", STAMP.fetch_add(1, Ordering::SeqCst));
    modlist::retire_unlisted(&pc.game, &allowlist::listed(&pc.game), &stamp).unwrap();
    aliases::ensure(&pc.game, Some(&pc.txt)).unwrap();
    let missing = modlist::missing(&list.mods, &pc.game).len();
    if !old_order && missing > 0 {
        return Gate::NeedsMods(missing);
    }
    if serverorder::beyond_base(&order) {
        serverorder::set_exact(&pc.game, &pc.txt, &order).unwrap();
        let bad = serverorder::mismatches(&pc.game, &pc.txt, &order).len();
        if bad > 0 {
            return Gate::Order(bad);
        }
    }
    if missing > 0 {
        return Gate::NeedsMods(missing);
    }
    Gate::Plays
}

static STAMP: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Default)]
struct Run {
    installed: usize,
    fetched: u64,
    reused: u64,
    resumed: u32,
    failed: usize,
    stopped: bool,
}

/// Download all, as mods.rs runs it for a Premium account: each missing
/// entry (each pinned file once) is fetched to downloads/<id>.zip under the
/// key naming the exact file, installed, and the download forgotten.
/// `close_at` closes the launcher when that entry's download is half done.
fn download_all(pc: &Pc, list: &ModList, srv: &Server, all: &[Stand], close_at: Option<&str>) -> Run {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let http = reqwest::Client::new();
    let mut run = Run::default();
    for m in modlist::to_fetch(&list.mods, &pc.game) {
        let s = all.iter().find(|s| s.entry.id == m.id && s.entry.nexus == m.nexus).unwrap();
        let url = format!("{}/{}", srv.base, Server::path(s));
        let path = pc.game.join(modlist::MODS_DIR).join("downloads").join(format!("{}.zip", m.id));
        let n = m.nexus.as_ref().unwrap();
        let key = format!("nexus-{}-{}", n.mod_id, n.file.unwrap());
        let stop = AtomicBool::new(false);
        let closing = close_at == Some(m.id.as_str());
        let got = rt.block_on(fetch::fetch(&http, &url, &path, &key, &stop, |done, total| {
            if closing && total > 0 && done * 2 >= total {
                stop.store(true, Ordering::SeqCst);
            }
        }));
        match got {
            Ok(f) => {
                run.fetched += f.fetched;
                run.reused += f.reused;
                run.resumed += f.resumed;
            }
            Err(e) if e == "cancelled" => {
                run.stopped = true;
                return run;
            }
            Err(e) => panic!("{}: {e}", m.id),
        }
        let done = modlist::install_archive(m, &path, &pc.game, n.file, None, false, Some(&pc.txt), &|_| {});
        fetch::forget(&path);
        match done {
            Ok(Outcome::Installed) => run.installed += 1,
            Ok(o) => panic!("{}: {o:?}", m.id),
            Err(_) => run.failed += 1,
        }
    }
    run
}

/// What the game will load (serverorder::game_order), and what's in Data
/// that no current record, dashed link or base master accounts for.
fn leftovers(pc: &Pc) -> Vec<String> {
    let rec = modlist::load_installed(&pc.game);
    let mut known: BTreeSet<String> = BASE.iter().map(|b| format!("data/{}", b.to_ascii_lowercase())).collect();
    for r in rec.mods.values() {
        known.extend(r.files.iter().map(|f| f.to_ascii_lowercase()));
    }
    known.extend(aliases::links(&pc.game).into_iter().map(|l| l.to.to_ascii_lowercase()));
    let mut out = Vec::new();
    walk(&pc.game.join("Data"), "Data", &mut out);
    out.retain(|f| !known.contains(&f.to_ascii_lowercase()));
    // The launcher's own folder: nothing half-done may stay there either.
    for d in ["downloads", "unpacked", "installing"] {
        walk(&pc.game.join(modlist::MODS_DIR).join(d), &format!("{}/{d}", modlist::MODS_DIR), &mut out);
    }
    out
}

fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let r = format!("{rel}/{}", e.file_name().to_string_lossy());
        if e.path().is_dir() {
            walk(&e.path(), &r, out);
        } else {
            out.push(r);
        }
    }
}

fn set_aside(pc: &Pc) -> Vec<String> {
    let mut out = Vec::new();
    walk(&pc.game.join(crate::strays::DISABLED_DIR), "", &mut out);
    out
}

/// The game's plugins after Play set the order: exactly the server's, in
/// its order, then the list's client-only plugins (any order), nothing else.
fn check_loads(pc: &Pc, server: &[String], list: &[Stand]) {
    let got = serverorder::game_order(&pc.game, &pc.txt);
    assert_eq!(&got[..server.len().min(got.len())], server, "the server's plugins, in order");
    let client: BTreeSet<String> = list.iter().flat_map(|s| s.plugins.iter().filter(|p| !p.server)).map(|p| aliases::run_as(&pc.game, &p.name).to_ascii_lowercase()).collect();
    let tail: BTreeSet<String> = got[server.len()..].iter().map(|n| n.to_ascii_lowercase()).collect();
    assert_eq!(tail, client, "after the server's: the list's client-only plugins and nothing else");
}

fn say(t: &mut Vec<String>, line: String) {
    println!("{line}");
    t.push(line);
}

#[test]
fn rehearsal_cutover_day_for_a_returning_and_a_new_player() {
    let started = Instant::now();
    let mut lines = Vec::new();
    let base_dir = tempfile::tempdir().unwrap();
    for (k, b) in BASE.iter().enumerate() {
        std::fs::write(base_dir.path().join(b), plugin(&[], MASTER, 20_000 + k * 1000, k as u32)).unwrap();
    }
    let l = lists();
    let srv = Server::start();
    srv.publish(&l.today);
    srv.publish(&l.cutover);
    srv.publish(&l.append);
    let five = serde_json::json!({ "masters": BASE.iter().map(|b| serde_json::json!({"name": b})).collect::<Vec<_>>() });
    let (m346, names346) = masters_json(&l.cutover, &l.order_cutover, base_dir.path());
    let (mapp, names_app) = masters_json(&l.append, &l.order_append, base_dir.path());
    // The in-place replacement keeps its name and place with new bytes.
    let at = BASE.len() + 22;
    assert_eq!(m346["masters"][at]["name"], mapp["masters"][at]["name"]);
    assert_ne!(m346["masters"][at]["crc32"], mapp["masters"][at]["crc32"]);
    let dashed = names346.iter().zip(BASE.iter().map(|b| b.to_string()).chain(l.order_cutover.iter().cloned())).filter(|(c, o)| c != &o).count();
    say(&mut lines, format!("lists: today {} entries; 3.4.6 {} entries, {} server plugins ({} under a canonical name); append {} entries, {} server plugins", l.today.len(), l.cutover.len(), names346.len(), dashed, l.append.len(), names_app.len()));

    // --- The returning player, on today's list. ---
    let back = Pc::new(base_dir.path());
    let today = list_of(&l.today);
    assert_eq!(play(&back, &today, &five, false), Gate::NeedsMods(30));
    let r = download_all(&back, &today, &srv, &l.today, None);
    assert_eq!((r.installed, r.failed), (30, 0));
    assert_eq!(play(&back, &today, &five, false), Gate::Plays);
    let today_plugins = serverorder::game_order(&back.game, &back.txt).len();

    // --- O2: masters.json first. Between the two publishes Play refuses. ---
    let between = play(&back, &today, &m346, false);
    assert!(matches!(between, Gate::Order(_)), "{between:?}");
    say(&mut lines, format!("returning: today's list plays ({today_plugins} plugins); between the masters.json and mods.json publishes Play refuses: {between:?}"));

    // --- Then mods.json. ---
    let cut = list_of(&l.cutover);
    let old = play(&back, &cut, &m346, true);
    let now = play(&back, &cut, &m346, false);
    assert!(matches!(old, Gate::Order(_)), "{old:?}");
    assert_eq!(now, Gate::NeedsMods(75));
    say(&mut lines, format!("returning at cutover: 0.1.97's Play order stops at {old:?} and never offers the downloads; 0.1.98's offers them: {now:?}"));

    // Dropped connections on three downloads, and the launcher closed half
    // way through the biggest.
    for s in l.cutover.iter().filter(|s| s.big > 0) {
        srv.drops.lock().unwrap().insert(Server::path(s), 1);
    }
    let biggest = l.cutover.iter().max_by_key(|s| s.big).unwrap();
    srv.rate.lock().unwrap().insert(Server::path(biggest), 800_000);
    let (req0, rg0) = (srv.requests.load(Ordering::SeqCst), srv.ranged.load(Ordering::SeqCst));
    let r1 = download_all(&back, &cut, &srv, &l.cutover, Some(&biggest.entry.id));
    assert!(r1.stopped);
    assert!(r1.resumed >= 1, "{r1:?}");
    let g1 = play(&back, &cut, &m346, false);
    assert!(matches!(g1, Gate::NeedsMods(n) if n > 0), "{g1:?}");
    // The next start: an install is stopped part way (a file it can't
    // write), after its plugin is already in Data.
    let victim = l.cutover.iter().find(|s| s.entry.id == "lane-30").unwrap();
    let blocker = back.game.join("Data").join("textures/lane-30/t0.dds");
    std::fs::create_dir_all(&blocker).unwrap();
    let r2 = download_all(&back, &cut, &srv, &l.cutover, None);
    assert_eq!(r2.failed, 1, "{r2:?}");
    assert!(r2.reused > 0, "the closed launcher's part is used again: {r2:?}");
    let half = &victim.plugins[0].name;
    assert!(back.game.join("Data").join(half).is_file(), "the half-installed mod's plugin is in Data");
    assert!(back.game.join("Data").join(&victim.entry.check[0][5..]).is_file());
    assert_eq!(modlist::half_installed(&back.game), vec!["lane-30".to_string()]);
    let g2 = play(&back, &cut, &m346, false);
    assert_eq!(g2, Gate::NeedsMods(1), "a half-installed mod never passes, though its plugin is there");
    std::fs::remove_dir_all(&blocker).unwrap();
    let r3 = download_all(&back, &cut, &srv, &l.cutover, None);
    assert_eq!((r3.installed, r3.failed), (1, 0));
    let g3 = play(&back, &cut, &m346, false);
    assert_eq!(g3, Gate::Plays);
    check_loads(&back, &names346, &l.cutover);
    assert_eq!(leftovers(&back), Vec::<String>::new());
    let reqs = srv.requests.load(Ordering::SeqCst) - req0;
    let ranged = srv.ranged.load(Ordering::SeqCst) - rg0;
    let ret_bytes = r1.fetched + r2.fetched + r3.fetched;
    say(
        &mut lines,
        format!(
            "returning: 75 entries in 3 starts ({} + {} + {} installed), {} resumed after a drop, {} bytes of the closed launcher's download used again, 1 install stopped part way: Play refused each time ({g1:?}, {g2:?}), then plays with exactly the {} server plugins in order; {} requests, {} of them ranged, {:.1} MB",
            r1.installed,
            r2.installed,
            r3.installed,
            r1.resumed + r2.resumed + r3.resumed,
            r2.reused,
            names346.len(),
            reqs,
            ranged,
            ret_bytes as f64 / 1e6
        ),
    );

    // --- The new player, straight to 3.4.6. ---
    for s in l.cutover.iter().filter(|s| s.big > 0) {
        srv.drops.lock().unwrap().insert(Server::path(s), 1);
    }
    let new = Pc::new(base_dir.path());
    assert_eq!(play(&new, &cut, &m346, false), Gate::NeedsMods(105));
    let n1 = download_all(&new, &cut, &srv, &l.cutover, Some(&biggest.entry.id));
    assert!(n1.stopped);
    assert!(matches!(play(&new, &cut, &m346, false), Gate::NeedsMods(_)));
    let n2 = download_all(&new, &cut, &srv, &l.cutover, None);
    assert_eq!(n1.installed + n2.installed, 105);
    assert_eq!(play(&new, &cut, &m346, false), Gate::Plays);
    check_loads(&new, &names346, &l.cutover);
    assert_eq!(leftovers(&new), Vec::<String>::new());
    say(&mut lines, format!("new: 105 entries in 2 starts ({} + {}), {} resumed after a drop, {} bytes used again after the close; plays with exactly the {} server plugins in order", n1.installed, n2.installed, n1.resumed + n2.resumed, n2.reused, names346.len()));

    // --- The later append, for both. ---
    let app = list_of(&l.append);
    for (who, pc) in [("returning", &back), ("new", &new)] {
        let aside_before = set_aside(pc).len();
        let g = play(pc, &app, &mapp, false);
        // The appended entry, the in-place pin and the new client pin.
        assert_eq!(g, Gate::NeedsMods(3), "{who}");
        // The removed entry's files went aside at that Play.
        let removed = l.cutover.iter().find(|s| s.entry.id == "today-05").unwrap();
        assert!(!pc.game.join("Data").join(&removed.plugins[0].name).exists(), "{who}: the removed mod's plugin left Data");
        let r = download_all(pc, &app, &srv, &l.append, None);
        assert_eq!((r.installed, r.failed), (3, 0), "{who}");
        assert_eq!(play(pc, &app, &mapp, false), Gate::Plays, "{who}");
        check_loads(pc, &names_app, &l.append);
        assert_eq!(leftovers(pc), Vec::<String>::new(), "{who}");
        // The old pin's dropped texture and the removed mod went aside.
        let aside = set_aside(pc);
        assert!(aside.iter().any(|f| f.ends_with("textures/today-20/t0.dds")), "{who}: {aside:?}");
        assert!(aside.iter().any(|f| f.ends_with(&removed.plugins[0].name)), "{who}: {aside:?}");
        assert!(!pc.game.join("Data/textures/today-20/t0.dds").exists());
        let replaced = &l.order_append[22];
        say(
            &mut lines,
            format!(
                "{who} after the append: Play offered 3 downloads, then plays with exactly the {} server plugins in order ({replaced} replaced in place, 2 appended); {} file(s) moved aside (the removed entry and the old pin's dropped file), nothing else left in Data",
                names_app.len(),
                set_aside(pc).len() - aside_before
            ),
        );
    }
    say(&mut lines, format!("rehearsal took {:.1} s", started.elapsed().as_secs_f64()));
    if let Ok(out) = std::env::var("REHEARSAL_OUT") {
        let _ = std::fs::write(out, lines.join("\n") + "\n");
    }
}
