//! "Download all mods" (Timothy, 2026-09-26): one click fetches and installs
//! every mod on the server's list that isn't installed yet. Premium Nexus
//! members get every Nexus mod straight from the API. Free members press
//! "Mod manager download" on each mod's page, which the launcher opens in
//! turn; the nxm:// link Nexus answers with comes back here through the
//! single-instance hook. Mods on GitHub download directly either way.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use launcher_core::{auth, fetch, modlist, modlist::ModEntry, nexus, Error};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{log, AppState, CmdResult};

/// How long a free member's page waits for a press before it's opened again
/// as a reminder. The wait itself only ends with a press or Stop: a
/// player who steps away finds the queue where they left it (free-account
/// check, 2026-09-28).
const REMIND_AFTER: std::time::Duration = std::time::Duration::from_secs(10 * 60);
/// How many times a reminder opens the page again.
const REOPEN_MAX: u32 = 3;
/// A free member is asked to press again this many times for a download
/// that keeps breaking off before that mod is shown as not installed.
const FREE_TRIES: u32 = 3;
/// How long a Premium member's own file page gets to hand over its nxm:// link.
const WAIT_FOR_PAGE: std::time::Duration = std::time::Duration::from_secs(90);

/// Catches nxm:// links for one Download all. The launcher only takes over
/// nxm:// when it needs a link (always for free members; for Premium only
/// when the API won't give a file directly) and gives it back at the end.
struct Catcher {
    rx: tokio::sync::mpsc::UnboundedReceiver<nexus::Nxm>,
    claimed: bool,
    /// Mods whose Nexus page was already opened in this run: one browser tab
    /// per mod per Play, retries included (quality check P3).
    paged: std::collections::HashSet<u64>,
    /// A link came back in this run: the browser's "open this app?" question
    /// is only explained until then.
    got_a_link: bool,
}

impl Catcher {
    fn claim(&mut self, app: &AppHandle) {
        if self.claimed {
            return;
        }
        self.claimed = true;
        match std::env::current_exe().map_err(Error::from).and_then(|exe| nexus::claim_nxm_handler(&exe)) {
            Ok(prev) => {
                if let Some(p) = previous_handler_path(app) {
                    let _ = std::fs::create_dir_all(p.parent().unwrap());
                    let _ = std::fs::write(p, prev.unwrap_or_default());
                }
                log::line("mods: the launcher takes Nexus download links until the mods are in");
            }
            Err(e) => log::line(&format!("mods: couldn't take nxm:// links: {e}")),
        }
    }
}

#[derive(Default)]
pub struct ModsState {
    /// The running queue's stop switch.
    cancel: std::sync::Mutex<Option<Arc<AtomicBool>>>,
    /// Where caught nxm:// links go while a free queue waits for them.
    nxm_tx: std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedSender<nexus::Nxm>>>,
    /// Stops a "Sign in with Nexus" that's waiting.
    sso_stop: std::sync::Mutex<Option<Arc<AtomicBool>>>,
    /// The server's mods.json, once fetched.
    server_list: tokio::sync::Mutex<Option<modlist::ModList>>,
    /// Exactly which mods.json that was (sha256 of the bytes, revision).
    receipt: tokio::sync::Mutex<Option<launcher_core::inventory::FeedReceipt>>,
}

fn key_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("nexus.bin"))
}

pub(crate) fn nexus_key(app: &AppHandle) -> Option<String> {
    key_path(app).and_then(|p| auth::load_token(&p))
}

/// Where the nxm:// command that was there before is kept while the launcher
/// holds it, so it can be put back even after a crash.
fn previous_handler_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("nxm-previous.txt"))
}

/// Gives nxm:// back to whoever had it, if a run was cut short.
pub fn restore_left_handler(app: &AppHandle) {
    let Some(p) = previous_handler_path(app) else { return };
    if let Ok(prev) = std::fs::read_to_string(&p) {
        let prev = prev.trim();
        let _ = nexus::restore_nxm_handler(if prev.is_empty() { None } else { Some(prev) });
        let _ = std::fs::remove_file(&p);
        log::line("mods: gave the nxm:// links back to the program that had them");
    }
}

/// Called with the arguments of a second launch (Windows starts the launcher
/// again for each nxm:// link).
pub fn on_second_launch(app: &AppHandle, args: &[String]) {
    for a in args {
        if let Some(n) = nexus::parse_nxm(a) {
            log::line(&format!("mods: caught a Nexus download link for mod {} file {}", n.mod_id, n.file_id));
            let st = app.state::<AppState>();
            let sent = st.mods.nxm_tx.lock().unwrap().as_ref().map(|tx| tx.send(n).is_ok()).unwrap_or(false);
            if !sent {
                log::line("mods: no download was waiting for that link");
            }
        }
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// The server's mods.json as already fetched; never fetches.
pub async fn fetched_server_list(state: &AppState) -> Option<modlist::ModList> {
    state.mods.server_list.lock().await.clone()
}

pub async fn server_list(state: &AppState) -> Option<modlist::ModList> {
    if let Some(l) = state.mods.server_list.lock().await.clone() {
        return Some(l);
    }
    fetch_server_list(state).await
}

async fn fetch_server_list(state: &AppState) -> Option<modlist::ModList> {
    let base = state.config.lock().await.base_url.clone();
    let url = format!("{}/mods.json", base.trim_end_matches('/'));
    let bytes = match state.http.get(&url).timeout(std::time::Duration::from_secs(8)).send().await {
        Ok(r) if r.status().is_success() => r.bytes().await.ok(),
        _ => None,
    };
    let got = bytes.as_ref().and_then(|b| serde_json::from_slice::<modlist::ModList>(b).ok().map(|l| (l, b)));
    let (l, b) = got?;
    let receipt = launcher_core::inventory::FeedReceipt::of(b, &l);
    if state.mods.receipt.lock().await.as_ref() != Some(&receipt) {
        log::line(&format!("mods: fetched {url}: {}", receipt.describe()));
    }
    *state.mods.receipt.lock().await = Some(receipt);
    *state.mods.server_list.lock().await = Some(l.clone());
    Some(l)
}

/// Play fetches mods.json again, so a list changed while the launcher was
/// open (cutover day) is the one used; when the server doesn't answer, the
/// last one stays.
pub async fn refresh_server_list(state: &AppState) {
    let before = state.mods.server_list.lock().await.as_ref().map(|l| l.mods.len());
    let got = fetch_server_list(state).await;
    // Saved now, so Play's tidying (removed mods) goes by this list.
    if let (Some(l), Some(dir)) = (&got, state.config.lock().await.game_dir.clone()) {
        launcher_core::allowlist::save_server_list(&dir, l);
    }
    match got {
        Some(l) if before.is_some_and(|b| b != l.mods.len()) => log::line(&format!("mods: the server's list changed while the launcher was open ({} entries, was {})", l.mods.len(), before.unwrap_or(0))),
        Some(_) => {}
        None => log::line("mods: couldn't fetch the server's list again; using the last one"),
    }
}

/// The served client set (`aetherial-collection.json`, design 6.1), when the
/// server publishes one. Until it does, Play is not gated on Vortex.
pub async fn served_client_set(state: &AppState) -> Option<launcher_core::vortex::ClientSet> {
    let base = state.config.lock().await.base_url.clone();
    let url = format!("{}/aetherial-collection.json", base.trim_end_matches('/'));
    match state.http.get(&url).timeout(std::time::Duration::from_secs(8)).send().await {
        Ok(r) if r.status().is_success() => r.json().await.ok(),
        _ => None,
    }
}

/// The Vortex step line (design section 3) from the Aetherial Dawn
/// extension, read-only. None when the launcher isn't paired with the
/// extension yet: then nothing is shown and nothing is gated.
pub async fn vortex_step(app: &AppHandle, state: &AppState, set: &launcher_core::vortex::ClientSet) -> Option<launcher_core::vortex::Step> {
    use launcher_core::vortex;
    let home = vortex::home(&app.path().app_local_data_dir().ok()?);
    let token = vortex::token(&home).ok()?;
    let status = match vortex::call(&state.http, &home, &token, "status", &serde_json::json!({}), "").await {
        Ok(v) => serde_json::from_value::<vortex::Status>(v).ok(),
        Err(e) => {
            if !matches!(e, vortex::JobError::NotRunning) {
                log::line(&format!("mods: the Vortex extension didn't answer: {e}"));
            }
            None
        }
    };
    Some(vortex::step(set, status.as_ref()))
}

pub async fn full_list(state: &AppState) -> Vec<ModEntry> {
    let version = state.manifest.lock().await.as_ref().and_then(|m| m.game.as_ref()).and_then(|g| g.version.clone());
    let server = server_list(state).await;
    if let (Some(l), Some(dir)) = (&server, state.config.lock().await.game_dir.clone()) {
        launcher_core::allowlist::save_server_list(&dir, l);
    }
    modlist::merged(version.as_deref(), server.as_ref())
}

#[derive(Serialize)]
pub struct Row {
    id: String,
    name: String,
    page: Option<String>,
    hint: Option<String>,
    looks_for: String,
    /// Its files are in the game folder (the game-files inventory).
    installed: bool,
    /// Vortex deployed it (Vortex's deployment record); None without one.
    in_vortex: Option<bool>,
    from: &'static str,
}

pub fn row(m: &ModEntry, game_dir: &Path) -> Row {
    let files = launcher_core::allowlist::vortex_files(game_dir);
    row_with(m, game_dir, launcher_core::inventory::standing(m, game_dir, &files, launcher_core::inventory::has_vortex_record(game_dir)))
}

fn row_with(m: &ModEntry, _game_dir: &Path, st: launcher_core::inventory::Standing) -> Row {
    Row {
        id: m.id.clone(),
        name: m.name.clone(),
        page: m.download_page(),
        hint: m.hint.clone(),
        looks_for: m.check.join(", "),
        installed: st.game_files,
        in_vortex: st.vortex_deployed,
        from: if m.nexus.is_some() { "nexus" } else { "direct" },
    }
}

#[derive(Serialize)]
pub struct ModsView {
    mods: Vec<Row>,
    nexus: Option<nexus::User>,
    vortex: bool,
    running: bool,
    /// "Sign in with Nexus" works (Nexus has registered the launcher).
    sso: bool,
    /// Both inventories, each naming what it measures.
    counts: launcher_core::inventory::Counts,
    /// "Game files: 36 of 37 present · Vortex: 0 of 37 deployed · …"
    counts_text: String,
    /// Which mods.json the counts are for; None when the server's list
    /// couldn't be fetched (the launcher's own list only).
    feed: Option<String>,
    /// The Aetherial Dawn profile line from Vortex's own state, when the
    /// launcher is paired with the extension.
    vortex_line: Option<String>,
    vortex_ready: Option<bool>,
}

#[tauri::command]
pub async fn mods_state(app: AppHandle, state: State<'_, AppState>) -> CmdResult<ModsView> {
    let dir = state.config.lock().await.game_dir.clone().ok_or("Pick your Skyrim folder first.")?;
    let list = full_list(&state).await;
    let user = if nexus_key(&app).is_some() { state.config.lock().await.nexus_user.clone() } else { None };
    let sso = nexus_app(&state).await.is_some();
    let running = state.mods.cancel.lock().unwrap().is_some();
    let (st, counts) = launcher_core::inventory::count(&list, &dir);
    let feed = state.mods.receipt.lock().await.as_ref().map(|r| r.describe());
    // Without a served client set, the server's Nexus-pinned mods are the
    // set shown (information only).
    let set = match served_client_set(&state).await {
        Some(set) => set,
        None => launcher_core::vortex::ClientSet { collection: None, mods: list.clone() },
    };
    let step = vortex_step(&app, &state, &set).await;
    Ok(ModsView {
        mods: list.iter().zip(st).map(|(m, s)| row_with(m, &dir, s)).collect(),
        nexus: user,
        vortex: modlist::vortex_manages(&dir),
        running,
        sso,
        counts_text: counts.describe(),
        counts,
        feed,
        vortex_line: step.as_ref().map(|s| s.describe()),
        vortex_ready: step.as_ref().map(|s| s.ok()),
    })
}

/// The application name Nexus registered for the launcher's SSO: from the
/// server's mods.json, or built in with AD_NEXUS_APP.
async fn nexus_app(state: &AppState) -> Option<String> {
    let from_server = server_list(state).await.and_then(|l| l.nexus_app).filter(|a| !a.trim().is_empty());
    from_server.or_else(|| option_env!("AD_NEXUS_APP").map(str::to_string))
}

/// `stop`: a sign-in the player left while Nexus was checking the key; the
/// key is then not kept.
async fn keep_key(app: &AppHandle, state: &AppState, key: &str, stop: Option<&AtomicBool>) -> CmdResult<nexus::User> {
    let version = app.package_info().version.to_string();
    let user = nexus::Client { http: &state.http, key, app_version: &version }.validate().await.map_err(|e| e.to_string())?;
    if stop.is_some_and(|s| s.load(Ordering::SeqCst)) {
        return Err("sign-in cancelled".to_string());
    }
    let path = key_path(app).ok_or("Couldn't find the launcher's settings folder.")?;
    auth::save_token(&path, key).map_err(|e| e.to_string())?;
    let mut c = state.config.lock().await;
    c.nexus_user = Some(user.clone());
    crate::save_config(app, &c)?;
    log::line(&format!("mods: signed in to Nexus as {} ({})", user.name, if user.is_premium { "Premium" } else { "free" }));
    Ok(user)
}

/// "Sign in with Nexus": opens the Nexus page where the player approves the
/// launcher; Nexus sends their key back and it's kept encrypted here.
#[tauri::command]
pub async fn nexus_sso(app: AppHandle, state: State<'_, AppState>) -> CmdResult<nexus::User> {
    let slug = nexus_app(&state).await.ok_or("NEXUS_SSO_UNAVAILABLE")?;
    let stop = Arc::new(AtomicBool::new(false));
    *state.mods.sso_stop.lock().unwrap() = Some(stop.clone());
    log::line("mods: waiting for the player to approve the launcher on Nexus");
    let app2 = app.clone();
    let r = nexus::sso(&slug, move |url| open_url(&app2, url).map_err(Error::Game), &stop).await;
    *state.mods.sso_stop.lock().unwrap() = None;
    let key = r.map_err(|e| {
        log::line(&format!("mods: Nexus sign-in didn't finish: {e}"));
        e.to_string()
    })?;
    keep_key(&app, &state, &key, None).await
}

/// A Nexus personal API key: one long token of base64-style characters.
fn looks_like_key(s: &str) -> bool {
    let s = s.trim();
    (30..=400).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric() || "+/=_-".contains(c))
}

/// Sign-in until Nexus registers the launcher for SSO: opens the player's
/// own API key page in their browser (where they're already logged in) and
/// waits for them to press Nexus's Copy button. Only clipboard text shaped
/// like a key is tried, it's checked with Nexus before it's kept, never
/// logged, and cleared from the clipboard afterwards.
#[tauri::command]
pub async fn nexus_copy_sign_in(app: AppHandle, state: State<'_, AppState>) -> CmdResult<nexus::User> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    let stop = Arc::new(AtomicBool::new(false));
    *state.mods.sso_stop.lock().unwrap() = Some(stop.clone());
    let result = async {
        let before = app.clipboard().read_text().unwrap_or_default();
        open_url(&app, nexus::API_KEY_PAGE)?;
        log::line("mods: opened the Nexus key page; waiting for the player to copy their key");
        let mut tried: Vec<String> = vec![before];
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3 * 60);
        loop {
            if stop.load(Ordering::SeqCst) {
                return Err("sign-in cancelled".to_string());
            }
            if std::time::Instant::now() > deadline {
                return Err("No key was copied. Click Sign in with Nexus to try again.".to_string());
            }
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            // Left during the wait: the clipboard is not read again.
            if stop.load(Ordering::SeqCst) {
                return Err("sign-in cancelled".to_string());
            }
            let text = app.clipboard().read_text().unwrap_or_default();
            let key = text.trim().to_string();
            if tried.contains(&text) || !looks_like_key(&key) {
                continue;
            }
            tried.push(text);
            match keep_key(&app, &state, &key, Some(&stop)).await {
                Ok(user) => {
                    let _ = app.clipboard().write_text(String::new());
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.set_focus();
                    }
                    return Ok(user);
                }
                Err(e) => log::line(&format!("mods: copied text wasn't a working Nexus key: {e}")),
            }
        }
    }
    .await;
    *state.mods.sso_stop.lock().unwrap() = None;
    result
}

#[tauri::command]
pub fn nexus_sso_cancel(state: State<'_, AppState>) {
    if let Some(s) = state.mods.sso_stop.lock().unwrap().as_ref() {
        s.store(true, Ordering::SeqCst);
    }
}

/// Checks a personal API key with Nexus and keeps it, encrypted, on this PC.
#[tauri::command]
pub async fn nexus_sign_in(app: AppHandle, state: State<'_, AppState>, key: String) -> CmdResult<nexus::User> {
    let key = key.trim().to_string();
    if key.len() < 20 || key.chars().any(char::is_whitespace) {
        return Err("That doesn't look like a Nexus API key. Copy the whole key from the Nexus page.".into());
    }
    keep_key(&app, &state, &key, None).await
}

#[tauri::command]
pub async fn nexus_sign_out(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    if let Some(p) = key_path(&app) {
        auth::forget_token(&p);
    }
    let mut c = state.config.lock().await;
    c.nexus_user = None;
    crate::save_config(&app, &c)?;
    log::line("mods: signed out of Nexus");
    Ok(())
}

fn open_url(app: &AppHandle, url: &str) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_nexus_key_page(app: AppHandle) -> CmdResult<()> {
    open_url(&app, nexus::API_KEY_PAGE)
}

/// Opens a Nexus Mods page for a required mod (only nexusmods.com pages).
#[tauri::command]
pub fn open_mod_page(app: AppHandle, url: String) -> CmdResult<()> {
    if !url.starts_with("https://www.nexusmods.com/") {
        return Err("That isn't a Nexus Mods page.".into());
    }
    open_url(&app, &url)
}

#[tauri::command]
pub fn cancel_mods(state: State<'_, AppState>) {
    if let Some(c) = state.mods.cancel.lock().unwrap().as_ref() {
        c.store(true, Ordering::SeqCst);
    }
}

#[derive(Clone, Serialize)]
struct Progress<'a> {
    id: &'a str,
    name: &'a str,
    /// queued, waiting, download, install, done, failed
    stage: &'a str,
    done: u64,
    total: u64,
    message: String,
}

fn emit(app: &AppHandle, m: &ModEntry, stage: &str, done: u64, total: u64, message: impl Into<String>) {
    let _ = app.emit("mods-progress", Progress { id: &m.id, name: &m.name, stage, done, total, message: message.into() });
}

#[derive(Serialize, Default)]
pub struct RunResult {
    installed: Vec<String>,
    failed: Vec<(String, String)>,
    cancelled: bool,
}

/// Downloads to `path`, reporting progress. Only https addresses. `key`
/// names the exact file, so a download that broke off (a dropped
/// connection, Stop, a closed launcher) carries on from where it stopped.
async fn download(app: &AppHandle, http: &reqwest::Client, m: &ModEntry, url: &str, path: &Path, key: &str, cancel: &AtomicBool) -> Result<(), String> {
    let got = fetch::fetch(http, url, path, key, cancel, |done, total| {
        emit(app, m, "download", done, total, "");
        overall(app, &m.id, done, Some(total), false);
    })
    .await?;
    if got.reused > 0 || got.resumed > 0 {
        log::line(&format!(
            "mods: {} download {}: {} MB already here, {} MB fetched, {} dropped connection(s) picked up",
            m.name,
            if got.fetched == 0 { "was already complete" } else { "carried on" },
            got.reused >> 20,
            got.fetched >> 20,
            got.resumed
        ));
    }
    Ok(())
}

/// The whole run's progress, for "3.2 of 27.4 GB, about 1 h 40 min left".
/// Each mod counts with the list's size until its download says its real one.
struct Overall {
    start: std::time::Instant,
    mods: std::collections::HashMap<String, (u64, u64)>,
    pace: fetch::Pace,
    shown: Option<std::time::Instant>,
    count: usize,
    finished: usize,
}

static OVERALL: std::sync::Mutex<Option<Overall>> = std::sync::Mutex::new(None);

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct OverallView {
    done: u64,
    total: u64,
    /// None until the pace is known (the first 10 s of downloading).
    secs_left: Option<u64>,
    finished: usize,
    count: usize,
}

/// Records `done` bytes of `total` for one mod (`total` None keeps the one
/// it has) and tells the page at most every half second, or at once with `force`.
fn overall(app: &AppHandle, id: &str, done: u64, total: Option<u64>, force: bool) {
    let view = {
        let mut g = OVERALL.lock().unwrap();
        let Some(o) = g.as_mut() else { return };
        let e = o.mods.entry(id.to_string()).or_insert((0, 0));
        if let Some(t) = total.filter(|t| *t > 0) {
            e.0 = t;
        }
        if done != u64::MAX {
            e.1 = done;
        }
        let (sum_total, sum_done) = o.mods.values().fold((0, 0), |a, v| (a.0 + v.0, a.1 + v.1.min(v.0)));
        o.pace.add(o.start.elapsed().as_secs_f64(), sum_done);
        if !force && o.shown.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(500)) {
            return;
        }
        o.shown = Some(std::time::Instant::now());
        OverallView { done: sum_done, total: sum_total, secs_left: o.pace.eta(sum_total.saturating_sub(sum_done)), finished: o.finished, count: o.count }
    };
    let _ = app.emit("mods-overall", view);
}

/// One mod is over: installed (all its bytes count) or not (its bytes leave
/// the total, so the time left isn't waiting for it).
fn overall_finish(app: &AppHandle, id: &str, installed: bool) {
    {
        let mut g = OVERALL.lock().unwrap();
        let Some(o) = g.as_mut() else { return };
        o.finished += 1;
        if let Some(e) = o.mods.get_mut(id) {
            if installed {
                e.1 = e.0;
            } else {
                e.0 = e.1;
            }
        }
    }
    overall(app, id, u64::MAX, None, true);
}

/// Bytes of a mod's download already in the downloads folder (a finished
/// archive or a part), whatever file name Nexus gave it.
fn already_here(game_dir: &Path, m: &ModEntry) -> u64 {
    let dir = game_dir.join(modlist::MODS_DIR).join("downloads");
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.strip_prefix(&format!("{}.", m.id)).is_some_and(|rest| !rest.ends_with(".json"))
        })
        .filter_map(|e| e.metadata().ok().map(|md| md.len()))
        .max()
        .unwrap_or(0)
}

/// Where to say how much space is on the drive, in GB with one decimal.
fn gb(n: u64) -> String {
    format!("{:.1} GB", n as f64 / 1e9)
}

use launcher_core::modlist::Outcome;

/// Unpacks, checks and installs one downloaded archive (modlist::install_archive).
async fn install(m: &ModEntry, archive: &Path, game_dir: &Path, file_id: Option<u64>, version: Option<String>, allow_too_new: bool) -> Result<Outcome, String> {
    let (m, archive, game_dir) = (m.clone(), archive.to_path_buf(), game_dir.to_path_buf());
    tokio::task::spawn_blocking(move || modlist::install_archive(&m, &archive, &game_dir, file_id, version, allow_too_new, plugins_txt().as_deref(), &|l: &str| log::line(l)))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Skyrim's load order, %LOCALAPPDATA%\\Skyrim Special Edition\\plugins.txt.
fn plugins_txt() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Skyrim Special Edition").join("plugins.txt"))
}

fn archive_path(game_dir: &Path, m: &ModEntry, file: &str) -> PathBuf {
    let ext = file.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).filter(|e| e.len() <= 4).unwrap_or_else(|| "bin".into());
    game_dir.join(modlist::MODS_DIR).join("downloads").join(format!("{}.{ext}", m.id))
}

/// Premium: pick the file, fetch it through the API, install it. A file made
/// for a newer Skyrim falls back to the next older one (unless the list pins
/// a file).
async fn premium_one(app: &AppHandle, api: &nexus::Client<'_>, m: &ModEntry, game_dir: &Path, cancel: &AtomicBool, catcher: &mut Catcher) -> Result<(), String> {
    let n = m.nexus.as_ref().unwrap();
    let mut files = api.files(modlist::NEXUS_GAME, n.mod_id).await.map_err(|e| e.to_string())?;
    // Archived files only when no current file matches the pick (one extra
    // call, for the Unofficial Patch 4.3.8a), so broad picks like "AE" never
    // pull in old archived builds.
    // A pinned file the list leaves out (an archived one, like the
    // Unofficial Patch 4.3.8a) is asked for by its id directly: no extra
    // lookups, and it still works if Nexus changes its lists.
    if let Some(id) = n.file.filter(|id| !files.iter().any(|f| f.file_id == *id)) {
        log::line(&format!("mods: {} asking Nexus for pinned file id {id}", m.name));
        files.push(nexus::NexusFile { file_id: id, name: m.name.clone(), version: None, category_name: Some("PINNED".into()), uploaded_timestamp: 0, file_name: String::new(), size_in_bytes: None });
    } else if n.file.is_none() && n.pick.as_deref().is_some_and(|p| !nexus::pick_is_current(&files, p)) {
        let every = match api.game_id(modlist::NEXUS_GAME).await {
            Ok(g) => api.all_files(g, n.mod_id).await,
            Err(e) => Err(e),
        };
        match every {
            Ok(more) => {
                let archived: Vec<String> = more.iter().filter(|f| f.category_name.as_deref() == Some("ARCHIVED")).map(|f| format!("{} (id {})", f.version.as_deref().unwrap_or("?"), f.file_id)).collect();
                log::line(&format!("mods: {} has {} archived file(s) on Nexus: {}", m.name, archived.len(), archived.join(", ")));
                for f in more {
                    if !files.iter().any(|x| x.file_id == f.file_id) {
                        files.push(f);
                    }
                }
            }
            Err(e) => log::line(&format!("mods: couldn't list {}'s archived files on Nexus: {e}", m.name)),
        }
    }
    let mut cands = nexus::candidates(&files, n.file, n.pick.as_deref());
    // After the picked files, the page's other files, in case the picked
    // one is a build for another Skyrim (True Directional Movement, 2026-09-26).
    if n.file.is_none() {
        for f in nexus::candidates(&files, None, None) {
            if !cands.iter().any(|c| c.file_id == f.file_id) {
                cands.push(f);
            }
        }
    }
    if cands.is_empty() {
        return Err("no matching file on Nexus".into());
    }
    let mut too_new = Vec::new();
    let mut wrong = Vec::new();
    let mut unreachable = Vec::new();
    for f in cands.iter().take(8) {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        emit(app, m, "download", 0, f.size_in_bytes.unwrap_or(0), f.name.clone());
        let cat = f.category_name.as_deref().unwrap_or("?");
        let url = match api.download_link(modlist::NEXUS_GAME, n.mod_id, f.file_id, None).await {
            Ok(u) => u,
            Err(e) => {
                // Nexus may not hand out archived files through the API (the
                // Unofficial Patch 4.3.8a is archived): ask the file's own page
                // for it instead, which answers Premium members with an nxm://
                // link at once.
                log::line(&format!("mods: Nexus API gave no link for {} file {} {} ({cat}, id {}): {e}", m.name, f.name, f.version.as_deref().unwrap_or(""), f.file_id));
                // One page per mod per run (the best file), so a refusing API
                // doesn't open a tab per file or per retry.
                if !catcher.paged.insert(n.mod_id) {
                    unreachable.push(f.name.clone());
                    continue;
                }
                match via_page(app, api, m, f.file_id, cancel, catcher).await {
                    Ok(u) => u,
                    Err(e) if e == "cancelled" => return Err(e),
                    Err(e) => {
                        log::line(&format!("mods: {} file {} not reachable through its page either: {e}", m.name, f.name));
                        unreachable.push(f.name.clone());
                        continue;
                    }
                }
            }
        };
        log::line(&format!("mods: {} downloading {} {} ({cat}, id {})", m.name, f.name, f.version.as_deref().unwrap_or(""), f.file_id));
        // A pinned file has no name from the list: the address ends in it.
        let name = if f.file_name.is_empty() { url.split('?').next().unwrap_or("") } else { &f.file_name };
        let path = archive_path(game_dir, m, name);
        download(app, api.http, m, &url, &path, &format!("nexus-{}-{}", n.mod_id, f.file_id), cancel).await?;
        emit(app, m, "install", 0, 0, "");
        let done = install(m, &path, game_dir, Some(f.file_id), f.version.clone(), n.file.is_some()).await;
        // Installed or refused, this archive is finished with; a broken one
        // is fetched again next time rather than tried again as it is.
        fetch::forget(&path);
        match done? {
            Outcome::Installed => return Ok(()),
            Outcome::TooNew(p) => {
                log::line(&format!("mods: {} file {} is for a newer Skyrim ({}); trying an older one", m.name, f.name, p.join(", ")));
                too_new.extend(p);
                let _ = std::fs::remove_file(&path);
            }
            Outcome::WrongBuild(p) => {
                log::line(&format!("mods: {} file {} has a build SKSE won't load on 1.6.1170 ({}); trying another", m.name, f.name, p.join(", ")));
                wrong.extend(p);
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    // The picked file (4.3.8a) was found but Nexus wouldn't hand it over:
    // say that, not that the other files are the wrong build.
    if !unreachable.is_empty() && too_new.is_empty() {
        return Err("Nexus didn't hand over the file just now. The launcher tries again the next time you press Play".into());
    }
    if !wrong.is_empty() && too_new.is_empty() {
        let tried: Vec<String> = cands.iter().take(8).map(|f| format!("{} {}", f.name, f.version.as_deref().unwrap_or(""))).collect();
        log::line(&format!("mods: {} files tried, none works on Skyrim 1.6.1170: {}", m.name, tried.join("; ")));
        return Err(format!("none of the files on its Nexus page that the launcher could get works on Skyrim 1.6.1170 (it needs {})", m.hint.as_deref().unwrap_or("an older version")));
    }
    Err(format!("every recent file is made for a newer Skyrim ({})", too_new.join(", ")))
}

/// Opens one file's Nexus page with the mod-manager download started
/// (`nmm=1`) and catches the nxm:// link it answers with; for a Premium
/// member nothing needs pressing. Returns the download address.
async fn via_page(app: &AppHandle, api: &nexus::Client<'_>, m: &ModEntry, file_id: u64, cancel: &AtomicBool, catcher: &mut Catcher) -> Result<String, String> {
    let n = m.nexus.as_ref().unwrap();
    catcher.claim(app);
    while catcher.rx.try_recv().is_ok() {}
    open_url(app, &format!("https://www.nexusmods.com/{}/mods/{}?tab=files&file_id={file_id}&nmm=1", modlist::NEXUS_GAME, n.mod_id))?;
    emit(app, m, "waiting", 0, 0, "Getting it from Nexus in your browser. If the browser asks to open Aetherial Dawn Launcher, allow it.");
    let deadline = tokio::time::Instant::now() + WAIT_FOR_PAGE;
    let link = loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        match tokio::time::timeout(std::time::Duration::from_millis(500), catcher.rx.recv()).await {
            Ok(Some(l)) => match modlist::link_fits(n, &l.game, l.mod_id, l.file_id) {
                modlist::LinkFits::Take => break l,
                modlist::LinkFits::OtherFile { pinned } => log::line(&format!("mods: refused file {} for {}: the list pins {pinned}", l.file_id, m.name)),
                modlist::LinkFits::OtherMod => log::line(&format!("mods: ignored a link for mod {} while waiting for {}", l.mod_id, m.name)),
            },
            Ok(None) => return Err("stopped waiting".into()),
            Err(_) if tokio::time::Instant::now() > deadline => return Err("the Nexus page sent no download link".into()),
            Err(_) => {}
        }
    };
    if link.file_id != file_id {
        log::line(&format!("mods: the page sent file {} instead of {file_id} for {}", link.file_id, m.name));
    }
    let url = api.download_link(modlist::NEXUS_GAME, n.mod_id, link.file_id, Some(&link)).await.map_err(|e| e.to_string())?;
    log::line(&format!("mods: {} link caught from its Nexus page (file {})", m.name, link.file_id));
    Ok(url)
}

/// Free: open the file's page and wait for the player to press its download
/// button; Nexus then hands the launcher an nxm:// link for that file. The
/// text is the Systems Designer's guided-download text (2026-09-28).
/// - A pinned file is the only one taken: any other is refused before it's
///   downloaded and the right page opens again; a second wrong file skips
///   the mod for this run (it's tried again next time).
/// - The wait never gives up by itself: the page is opened again every
///   `REMIND_AFTER` (a few times), and the player can Stop and carry on.
/// - A download that breaks off, or a busy Nexus, is tried again with the
///   same link before the player is asked to press again.
///
/// `n_of` is "Mod 12 of 50".
async fn free_one(app: &AppHandle, api: &nexus::Client<'_>, m: &ModEntry, game_dir: &Path, cancel: &AtomicBool, catcher: &mut Catcher, n_of: &str) -> Result<(), String> {
    let Catcher { rx, got_a_link, .. } = catcher;
    let n = m.nexus.as_ref().unwrap();
    let page = m.download_page().unwrap();
    let name = &m.name;
    let press = if n.file.is_some() {
        format!("Nexus is open on {name}. Press Slow download there.")
    } else {
        format!("Nexus is open on {name}'s files. Press Mod manager download for {}, then Slow download.", m.hint.as_deref().unwrap_or("the main file"))
    };
    let browser = " Your browser may ask to open Aetherial Dawn Launcher. Tick Always allow and press Open, so it doesn't ask again.";
    let say = |note: &str, seen: bool| emit(app, m, "waiting", 0, 0, format!("{n_of}: {note}{}", if seen { "" } else { browser }));
    while rx.try_recv().is_ok() {}
    open_url(app, &page)?;
    let mut note = press.clone();
    let mut reopened = 0u32;
    let mut wrong = 0u32;
    let mut broke = 0u32;
    loop {
        say(&note, *got_a_link);
        let mut remind_at = tokio::time::Instant::now() + REMIND_AFTER;
        let link = loop {
            if cancel.load(Ordering::SeqCst) {
                return Err("cancelled".into());
            }
            match tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await {
                Ok(Some(l)) => match modlist::link_fits(n, &l.game, l.mod_id, l.file_id) {
                    modlist::LinkFits::Take => break l,
                    modlist::LinkFits::OtherFile { pinned } => {
                        *got_a_link = true;
                        wrong += 1;
                        log::line(&format!("mods: refused file {} for {name}: the list pins {pinned} (wrong file {wrong})", l.file_id));
                        if wrong >= 2 {
                            return Err("skipped for now: Nexus sent a different file twice. The launcher tries it again next time".into());
                        }
                        note = "That was a different file from the one Aetherial Dawn needs, so the launcher didn't use it. It has opened the right page again. Press Slow download there.".into();
                        open_url(app, &page)?;
                        say(&note, true);
                        remind_at = tokio::time::Instant::now() + REMIND_AFTER;
                    }
                    modlist::LinkFits::OtherMod => log::line(&format!("mods: ignored a link for mod {} while waiting for {name}", l.mod_id)),
                },
                Ok(None) => return Err("stopped waiting".into()),
                Err(_) if tokio::time::Instant::now() > remind_at => {
                    remind_at = tokio::time::Instant::now() + REMIND_AFTER;
                    // A few times, so a player who stepped away doesn't come
                    // back to a pile of tabs; the row's Open button is there too.
                    if reopened < REOPEN_MAX {
                        reopened += 1;
                        log::line(&format!("mods: no press for {name} yet; opened its page again"));
                        open_url(app, &page)?;
                        note = format!("Nexus didn't send the download for {name}. The launcher opened the page again. Press Slow download there.");
                    } else {
                        note = format!("Still waiting for {name}. Press Open to show its Nexus page again, or Stop to carry on later.");
                    }
                    say(&note, *got_a_link);
                }
                Err(_) => {}
            }
        };
        *got_a_link = true;
        emit(app, m, "download", 0, 0, format!("Got it. Downloading {name}…"));
        // The same link is tried again for a busy Nexus or a download that
        // broke off, before the player is asked to press again.
        let mut last = String::new();
        let mut path = None;
        for attempt in 0..3u64 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(5 * attempt)).await;
                if cancel.load(Ordering::SeqCst) {
                    return Err("cancelled".into());
                }
            }
            let url = match api.download_link(modlist::NEXUS_GAME, n.mod_id, link.file_id, Some(&link)).await {
                Ok(u) => u,
                Err(e) => {
                    last = e.to_string();
                    log::line(&format!("mods: Nexus gave no link for {name} ({last}), try {}", attempt + 1));
                    emit(app, m, "waiting", 0, 0, format!("{n_of}: Nexus is busy right now. The launcher waits a moment and tries {name} again."));
                    continue;
                }
            };
            let p = archive_path(game_dir, m, url.split('?').next().unwrap_or(""));
            match download(app, api.http, m, &url, &p, &format!("nexus-{}-{}", n.mod_id, link.file_id), cancel).await {
                Ok(()) => {
                    path = Some(p);
                    break;
                }
                Err(e) if e == "cancelled" => return Err(e),
                Err(e) => {
                    last = e;
                    log::line(&format!("mods: {name} download broke off ({last}), try {}", attempt + 1));
                    emit(app, m, "waiting", 0, 0, format!("{n_of}: The download of {name} stopped. The launcher starts it again."));
                }
            }
        }
        let Some(path) = path else {
            broke += 1;
            if broke >= FREE_TRIES {
                return Err(format!("the download from Nexus kept stopping ({last}). The launcher tries it again next time"));
            }
            open_url(app, &page)?;
            note = format!("Nexus didn't send the download for {name}. The launcher opened the page again. Press Slow download there.");
            continue;
        };
        emit(app, m, "install", 0, 0, format!("Installing {name}…"));
        let done = install(m, &path, game_dir, Some(link.file_id), None, n.file.is_some()).await;
        fetch::forget(&path);
        match done? {
            Outcome::Installed => return Ok(()),
            // Only an unpinned mod gets here (a pinned file is the server's
            // own); its page is opened again for another file.
            Outcome::TooNew(p) => {
                let _ = std::fs::remove_file(&path);
                open_url(app, &page)?;
                note = format!("That file is for a newer Skyrim ({}), so the launcher didn't use it. On the page, press Mod manager download on an older file for Skyrim 1.6.1170, then Slow download.", p.join(", "));
            }
            Outcome::WrongBuild(p) => {
                let _ = std::fs::remove_file(&path);
                open_url(app, &page)?;
                note = format!("That file has the old-Skyrim build of {}, so the launcher didn't use it. On the page, press Mod manager download on the Anniversary Edition file (1.6.640 or newer), then Slow download.", p.join(", "));
            }
        }
    }
}

async fn direct_one(app: &AppHandle, http: &reqwest::Client, m: &ModEntry, game_dir: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let url = m.url.as_deref().unwrap();
    let path = archive_path(game_dir, m, url.split('?').next().unwrap_or(""));
    download(app, http, m, url, &path, &format!("url-{}-{}", url.split('?').next().unwrap_or(""), m.sha256.as_deref().unwrap_or("")), cancel).await?;
    emit(app, m, "install", 0, 0, "");
    let done = install(m, &path, game_dir, None, None, true).await;
    fetch::forget(&path);
    match done? {
        Outcome::Installed => Ok(()),
        Outcome::TooNew(_) => unreachable!(),
        Outcome::WrongBuild(p) => Err(format!("the download has a build of {} that SKSE won't load on Skyrim 1.6.1170", p.join(", "))),
    }
}

/// Downloads and installs every missing mod on the list, one after another.
#[tauri::command]
pub async fn download_all_mods(app: AppHandle, state: State<'_, AppState>) -> CmdResult<RunResult> {
    let dir = state.config.lock().await.game_dir.clone().ok_or("Pick your Skyrim folder first.")?;
    let list = full_list(&state).await;
    // Each pinned file once: the built-in Unofficial Patch and the server
    // lane's copy of it are one download.
    let mut todo: Vec<ModEntry> = modlist::to_fetch(&list, &dir).into_iter().cloned().collect();
    // The Black Screen Fix preset that fits the game's resolution.
    let height = app.path().document_dir().ok().and_then(|d| launcher_core::gameini::screen_height(&d));
    for m in todo.iter_mut().filter(|m| m.id == "black-screen-fix") {
        if let Some(n) = m.nexus.as_mut().filter(|n| n.file.is_none()) {
            n.pick = Some(launcher_core::gameini::preset_for(height).into());
            log::line(&format!("mods: Black Screen Fix preset for a screen {} high: {}p", height.map(|h| h.to_string()).unwrap_or_else(|| "of unknown height".into()), n.pick.as_deref().unwrap_or("")));
        }
    }
    if todo.is_empty() {
        return Ok(RunResult::default());
    }
    let key = nexus_key(&app);
    let needs_nexus = todo.iter().any(|m| m.nexus.is_some());
    if needs_nexus && key.is_none() {
        return Err("NEEDS_NEXUS_SIGN_IN".into());
    }
    // Room on the Skyrim drive for everything still to install, plus the
    // biggest mod in flight (files are moved into Data, so its archive).
    let sizes: Vec<(u64, u64, u64)> = todo.iter().map(|m| {
        let (a, u) = m.sizes();
        (a, u, already_here(&dir, m).min(a))
    }).collect();
    let need = fetch::space_needed(&sizes, true);
    let no_sizes = todo.iter().filter(|m| m.archive_bytes.is_none() || m.unpacked_bytes.is_none()).count();
    if no_sizes > 0 {
        log::line(&format!("mods: {no_sizes} of {} mod(s) have no sizes in the list; counted as {} each", todo.len(), gb(modlist::UNKNOWN_ARCHIVE * 4)));
    }
    let download: u64 = sizes.iter().map(|s| s.0.saturating_sub(s.2)).sum();
    match fetch::free_space(&dir) {
        Some(free) if free < need => {
            log::line(&format!("mods: not enough space on the Skyrim drive: {} mod(s) need {} free, {} is free", todo.len(), gb(need), gb(free)));
            return Err(format!("NO_SPACE:{need}:{free}"));
        }
        Some(free) => log::line(&format!("mods: {} to download, {} free space needed at most, {} free", gb(download), gb(need), gb(free))),
        None => log::line(&format!("mods: {} to download, {} free space needed at most (free space unknown)", gb(download), gb(need))),
    }
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut c = state.mods.cancel.lock().unwrap();
        if c.is_some() {
            return Err("The mods are already downloading.".into());
        }
        *c = Some(cancel.clone());
    }
    let version = app.package_info().version.to_string();
    let key = key.unwrap_or_default();
    let api = nexus::Client { http: &state.http, key: &key, app_version: &version };
    let premium = if needs_nexus {
        match api.validate().await {
            Ok(u) => {
                let premium = u.is_premium;
                let mut c = state.config.lock().await;
                c.nexus_user = Some(u);
                let _ = crate::save_config(&app, &c);
                premium
            }
            Err(e) => {
                *state.mods.cancel.lock().unwrap() = None;
                return Err(e.to_string());
            }
        }
    } else {
        false
    };
    log::line(&format!("mods: downloading {} mod(s) ({})", todo.len(), if premium { "Nexus Premium" } else { "Nexus free or direct" }));
    for m in &todo {
        emit(&app, m, "queued", 0, 0, "");
    }

    // nxm:// links: free members always need them; Premium only when the
    // API won't hand a file over. Held while waiting, then given back.
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    *state.mods.nxm_tx.lock().unwrap() = Some(tx);
    let mut catcher = Catcher { rx, claimed: false, paged: Default::default(), got_a_link: false };
    if needs_nexus && !premium {
        catcher.claim(&app);
    }

    let mut result = RunResult::default();
    let count = todo.len();
    *OVERALL.lock().unwrap() = Some(Overall {
        start: std::time::Instant::now(),
        mods: todo.iter().zip(&sizes).map(|(m, s)| (m.id.clone(), (s.0, s.2))).collect(),
        pace: fetch::Pace::default(),
        shown: None,
        count,
        finished: 0,
    });
    overall(&app, "", 0, None, true);
    for (i, m) in todo.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            result.cancelled = true;
            break;
        }
        // Put in by an earlier download of this run (the same files).
        if m.installed(&dir) {
            log::line(&format!("mods: {} is already in; nothing to download", m.name));
            emit(&app, m, "done", 0, 0, "Installed");
            overall_finish(&app, &m.id, true);
            result.installed.push(m.name.clone());
            continue;
        }
        let n_of = format!("Mod {} of {count}", i + 1);
        let mut r = Err(String::new());
        // One quiet retry before a failure is shown (a dropped download, a
        // busy Nexus).
        for attempt in 0..2 {
            r = if m.nexus.is_some() {
                if premium {
                    premium_one(&app, &api, m, &dir, &cancel, &mut catcher).await
                } else {
                    free_one(&app, &api, m, &dir, &cancel, &mut catcher, &n_of).await
                }
            } else {
                direct_one(&app, &state.http, m, &dir, &cancel).await
            };
            match &r {
                // Free members' downloads are asked for again inside free_one.
                Err(e) if attempt == 0 && e != "cancelled" && (m.nexus.is_none() || premium) => {
                    log::line(&format!("mods: {} failed ({e}); trying once more", m.name));
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
                _ => break,
            }
        }
        match r {
            Ok(()) if m.installed(&dir) => {
                emit(&app, m, "done", 0, 0, "Installed");
                overall_finish(&app, &m.id, true);
                result.installed.push(m.name.clone());
            }
            Ok(()) => {
                let msg = format!("installed, but {} still isn't there", m.check.join(", "));
                log::line(&format!("mods: {}: {msg}", m.name));
                emit(&app, m, "failed", 0, 0, msg.clone());
                overall_finish(&app, &m.id, false);
                result.failed.push((m.name.clone(), msg));
            }
            Err(e) if e == "cancelled" => {
                result.cancelled = true;
                emit(&app, m, "failed", 0, 0, "Stopped");
                break;
            }
            Err(e) => {
                log::line(&format!("mods: {} failed: {e}", m.name));
                let e = launcher_core::plain(&e);
                emit(&app, m, "failed", 0, 0, e.clone());
                overall_finish(&app, &m.id, false);
                result.failed.push((m.name.clone(), e));
            }
        }
    }

    *state.mods.nxm_tx.lock().unwrap() = None;
    if catcher.claimed {
        restore_left_handler(&app);
    }
    *state.mods.cancel.lock().unwrap() = None;
    if let Some(o) = OVERALL.lock().unwrap().take() {
        let done: u64 = o.mods.values().map(|v| v.1.min(v.0)).sum();
        log::line(&format!("mods: {} in {} min ({} MB/min over the run)", gb(done), o.start.elapsed().as_secs() / 60, (done >> 20) / (o.start.elapsed().as_secs() / 60).max(1)));
    }
    log::line(&format!("mods: done, {} installed, {} failed{}", result.installed.len(), result.failed.len(), if result.cancelled { ", stopped by the player" } else { "" }));
    Ok(result)
}

#[cfg(test)]
mod tests {
    #[test]
    fn spots_keys() {
        assert!(super::looks_like_key("abcDEF123+/=abcDEF123+/=abcDEF123--xyz--QQ=="));
        assert!(!super::looks_like_key("hello world, this is not a key at all"));
        assert!(!super::looks_like_key("short"));
    }
}
