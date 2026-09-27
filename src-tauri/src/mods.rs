//! "Download all mods" (Timothy, 2026-09-26): one click fetches and installs
//! every mod on the server's list that isn't installed yet. Premium Nexus
//! members get every Nexus mod straight from the API. Free members press
//! "Mod manager download" on each mod's page, which the launcher opens in
//! turn; the nxm:// link Nexus answers with comes back here through the
//! single-instance hook. Mods on GitHub download directly either way.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use launcher_core::{auth, modlist, modlist::ModEntry, nexus, Error};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{log, AppState, CmdResult};

/// How long to wait for a free member to press a mod's download button.
const WAIT_FOR_CLICK: std::time::Duration = std::time::Duration::from_secs(15 * 60);

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
}

fn key_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("nexus.bin"))
}

fn nexus_key(app: &AppHandle) -> Option<String> {
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

pub async fn server_list(state: &AppState) -> Option<modlist::ModList> {
    if let Some(l) = state.mods.server_list.lock().await.clone() {
        return Some(l);
    }
    let base = state.config.lock().await.base_url.clone();
    let url = format!("{}/mods.json", base.trim_end_matches('/'));
    let got = match state.http.get(&url).send().await {
        Ok(r) if r.status().is_success() => r.json::<modlist::ModList>().await.ok(),
        _ => None,
    };
    if let Some(l) = &got {
        *state.mods.server_list.lock().await = Some(l.clone());
    }
    got
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
    installed: bool,
    from: &'static str,
}

pub fn row(m: &ModEntry, game_dir: &Path) -> Row {
    Row {
        id: m.id.clone(),
        name: m.name.clone(),
        page: m.page(),
        hint: m.hint.clone(),
        looks_for: m.check.join(", "),
        installed: m.installed(game_dir),
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
}

#[tauri::command]
pub async fn mods_state(app: AppHandle, state: State<'_, AppState>) -> CmdResult<ModsView> {
    let dir = state.config.lock().await.game_dir.clone().ok_or("Pick your Skyrim folder first.")?;
    let list = full_list(&state).await;
    let user = if nexus_key(&app).is_some() { state.config.lock().await.nexus_user.clone() } else { None };
    let sso = nexus_app(&state).await.is_some();
    let running = state.mods.cancel.lock().unwrap().is_some();
    Ok(ModsView {
        mods: list.iter().map(|m| row(m, &dir)).collect(),
        nexus: user,
        vortex: modlist::vortex_manages(&dir),
        running,
        sso,
    })
}

/// The application name Nexus registered for the launcher's SSO: from the
/// server's mods.json, or built in with AD_NEXUS_APP.
async fn nexus_app(state: &AppState) -> Option<String> {
    let from_server = server_list(state).await.and_then(|l| l.nexus_app).filter(|a| !a.trim().is_empty());
    from_server.or_else(|| option_env!("AD_NEXUS_APP").map(str::to_string))
}

async fn keep_key(app: &AppHandle, state: &AppState, key: &str) -> CmdResult<nexus::User> {
    let version = app.package_info().version.to_string();
    let user = nexus::Client { http: &state.http, key, app_version: &version }.validate().await.map_err(|e| e.to_string())?;
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
    keep_key(&app, &state, &key).await
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
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10 * 60);
        loop {
            if stop.load(Ordering::SeqCst) {
                return Err("sign-in cancelled".to_string());
            }
            if std::time::Instant::now() > deadline {
                return Err("No key was copied. Click Sign in with Nexus to try again.".to_string());
            }
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            let text = app.clipboard().read_text().unwrap_or_default();
            let key = text.trim().to_string();
            if tried.contains(&text) || !looks_like_key(&key) {
                continue;
            }
            tried.push(text);
            match keep_key(&app, &state, &key).await {
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
    keep_key(&app, &state, &key).await
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

/// Downloads to `path`, reporting progress. Only https addresses.
async fn download(app: &AppHandle, http: &reqwest::Client, m: &ModEntry, url: &str, path: &Path, cancel: &AtomicBool) -> Result<(), String> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    if !url.starts_with("https://") {
        return Err("the download address isn't secure".into());
    }
    let resp = http.get(url).send().await.and_then(|r| r.error_for_status()).map_err(|e| e.to_string())?;
    let total = resp.content_length().unwrap_or(0);
    if let Some(p) = path.parent() {
        tokio::fs::create_dir_all(p).await.map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("part");
    let mut f = tokio::fs::File::create(&tmp).await.map_err(|e| e.to_string())?;
    let mut got = 0u64;
    let mut last = std::time::Instant::now();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::SeqCst) {
            drop(f);
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err("cancelled".into());
        }
        let chunk = chunk.map_err(|e| e.to_string())?;
        f.write_all(&chunk).await.map_err(|e| e.to_string())?;
        got += chunk.len() as u64;
        if last.elapsed().as_millis() > 250 {
            emit(app, m, "download", got, total, "");
            last = std::time::Instant::now();
        }
    }
    f.flush().await.map_err(|e| e.to_string())?;
    drop(f);
    tokio::fs::rename(&tmp, path).await.map_err(|e| e.to_string())?;
    emit(app, m, "download", got, got.max(total), "");
    Ok(())
}

enum Outcome {
    Installed,
    /// Made for a newer Skyrim than the game's masters.
    TooNew(Vec<String>),
    /// Has an SKSE DLL built for another Skyrim.
    WrongBuild(Vec<String>),
}

/// Unpacks, checks and installs one downloaded archive.
async fn install(m: &ModEntry, archive: &Path, game_dir: &Path, file_id: Option<u64>, version: Option<String>, allow_too_new: bool) -> Result<Outcome, String> {
    let (m, archive, game_dir) = (m.clone(), archive.to_path_buf(), game_dir.to_path_buf());
    tokio::task::spawn_blocking(move || -> Result<Outcome, Error> {
        modlist::verify(&m, &archive)?;
        let work = game_dir.join(modlist::MODS_DIR).join("unpacked").join(&m.id);
        let _ = std::fs::remove_dir_all(&work);
        modlist::extract(&archive, &work)?;
        let mut copies = modlist::plan(&m, &work)?;
        // An SKSE DLL for another Skyrim never goes in; the next file is tried.
        let mut wrong = modlist::fix_wrong_builds(&mut copies, &work);
        // The Unofficial Patch for Skyrim 1.7.99 crashes 1.6.1170.
        for c in &copies {
            if c.to.file_name().map(|n| n.to_string_lossy().eq_ignore_ascii_case(launcher_core::requirements::USSEP_PLUGIN)).unwrap_or(false) {
                if let Some(v) = launcher_core::ussep::plugin_too_new(&c.from) {
                    wrong.push((format!("Unofficial Patch {v}"), "made for Skyrim 1.7.99".into()));
                }
            }
        }
        if !wrong.is_empty() {
            let _ = std::fs::remove_dir_all(&work);
            log::line(&format!("mods: {} download has the wrong build: {}", m.name, wrong.iter().map(|(n, w)| format!("{n} ({w})")).collect::<Vec<_>>().join(", ")));
            return Ok(Outcome::WrongBuild(wrong.into_iter().map(|(n, _)| n).collect()));
        }
        let newer = modlist::too_new_plugins(&copies, &game_dir);
        if !newer.is_empty() && !allow_too_new {
            let _ = std::fs::remove_dir_all(&work);
            return Ok(Outcome::TooNew(newer));
        }
        let rec = modlist::apply(&m, &copies, &game_dir, file_id, version)?;
        let _ = std::fs::remove_dir_all(&work);
        // Only call it installed when the files really are in Data.
        if !m.installed(&game_dir) {
            let gone: Vec<&str> = m.check.iter().map(String::as_str).filter(|c| !m.clone_with_check(c).installed(&game_dir)).collect();
            log::line(&format!("mods: {} unpacked but {} isn't in the game folder (copied {}, left {})", m.name, gone.join(", "), rec.files.join(", "), rec.skipped.join(", ")));
            return Err(Error::Game(format!("{} downloaded, but {} didn't end up in your Skyrim folder", m.name, gone.join(" and "))));
        }
        let _ = std::fs::remove_file(&archive);
        // And switch its plugins on, as Vortex would.
        if let Some(txt) = plugins_txt() {
            let names: Vec<String> = m.check.iter().filter_map(|c| c.strip_prefix("Data/")).filter(|n| !n.contains('/') && [".esp", ".esm", ".esl"].iter().any(|x| n.to_ascii_lowercase().ends_with(x))).map(str::to_string).collect();
            match launcher_core::loadorder::switch_on(&txt, &names) {
                Ok(on) if !on.is_empty() => log::line(&format!("mods: switched on in plugins.txt: {}", on.join(", "))),
                Ok(_) => {}
                Err(e) => log::line(&format!("mods: couldn't switch {} on in plugins.txt: {e}", names.join(", "))),
            }
        }
        log::line(&format!("mods: installed {} ({} files, {} left to Vortex or kept)", m.name, rec.files.len(), rec.skipped.len()));
        Ok(Outcome::Installed)
    })
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
async fn premium_one(app: &AppHandle, api: &nexus::Client<'_>, m: &ModEntry, game_dir: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let n = m.nexus.as_ref().unwrap();
    let files = api.files(modlist::NEXUS_GAME, n.mod_id).await.map_err(|e| e.to_string())?;
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
    for f in cands.iter().take(8) {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        emit(app, m, "download", 0, f.size_in_bytes.unwrap_or(0), f.name.clone());
        let url = api.download_link(modlist::NEXUS_GAME, n.mod_id, f.file_id, None).await.map_err(|e| e.to_string())?;
        let path = archive_path(game_dir, m, &f.file_name);
        download(app, api.http, m, &url, &path, cancel).await?;
        emit(app, m, "install", 0, 0, "");
        match install(m, &path, game_dir, Some(f.file_id), f.version.clone(), n.file.is_some()).await? {
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
    if !wrong.is_empty() && too_new.is_empty() {
        let tried: Vec<String> = cands.iter().take(8).map(|f| format!("{} {}", f.name, f.version.as_deref().unwrap_or(""))).collect();
        log::line(&format!("mods: {} files tried, none works on Skyrim 1.6.1170: {}", m.name, tried.join("; ")));
        return Err(format!("none of the files on its Nexus page that the launcher could get works on Skyrim 1.6.1170 (it needs {})", m.hint.as_deref().unwrap_or("an older version")));
    }
    Err(format!("every recent file is made for a newer Skyrim ({})", too_new.join(", ")))
}

/// Free: open the mod's page and wait for the player to press "Mod manager
/// download"; Nexus then hands the launcher an nxm:// link for that file.
async fn free_one(app: &AppHandle, api: &nexus::Client<'_>, m: &ModEntry, game_dir: &Path, cancel: &AtomicBool, rx: &mut tokio::sync::mpsc::UnboundedReceiver<nexus::Nxm>) -> Result<(), String> {
    let n = m.nexus.as_ref().unwrap();
    while rx.try_recv().is_ok() {}
    open_url(app, &m.page().unwrap())?;
    let mut note = format!("On the Nexus page, press Mod manager download{}.", m.hint.as_ref().map(|h| format!(" for {h}")).unwrap_or_default());
    loop {
        emit(app, m, "waiting", 0, 0, note.clone());
        let deadline = tokio::time::Instant::now() + WAIT_FOR_CLICK;
        let link = loop {
            if cancel.load(Ordering::SeqCst) {
                return Err("cancelled".into());
            }
            match tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await {
                Ok(Some(l)) if l.mod_id == n.mod_id && l.game == modlist::NEXUS_GAME => break l,
                Ok(Some(l)) => log::line(&format!("mods: ignored a link for mod {} while waiting for {}", l.mod_id, m.name)),
                Ok(None) => return Err("stopped waiting".into()),
                Err(_) if tokio::time::Instant::now() > deadline => return Err("no download was pressed on Nexus".into()),
                Err(_) => {}
            }
        };
        let url = api.download_link(modlist::NEXUS_GAME, n.mod_id, link.file_id, Some(&link)).await.map_err(|e| e.to_string())?;
        let path = archive_path(game_dir, m, url.split('?').next().unwrap_or(""));
        download(app, api.http, m, &url, &path, cancel).await?;
        emit(app, m, "install", 0, 0, "");
        match install(m, &path, game_dir, Some(link.file_id), None, n.file.is_some()).await? {
            Outcome::Installed => return Ok(()),
            Outcome::TooNew(p) => {
                let _ = std::fs::remove_file(&path);
                note = format!("That file is for a newer Skyrim ({}). Open Files, pick an older version for Skyrim 1.6.1170 and press Mod manager download.", p.join(", "));
            }
            Outcome::WrongBuild(p) => {
                let _ = std::fs::remove_file(&path);
                note = format!("That file has the old-Skyrim build of {}. Open Files, pick the one for Anniversary Edition (1.6.640 or newer) and press Mod manager download.", p.join(", "));
            }
        }
    }
}

async fn direct_one(app: &AppHandle, http: &reqwest::Client, m: &ModEntry, game_dir: &Path, cancel: &AtomicBool) -> Result<(), String> {
    let url = m.url.as_deref().unwrap();
    let path = archive_path(game_dir, m, url.split('?').next().unwrap_or(""));
    download(app, http, m, url, &path, cancel).await?;
    emit(app, m, "install", 0, 0, "");
    match install(m, &path, game_dir, None, None, true).await? {
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
    let mut todo: Vec<ModEntry> = modlist::missing(&list, &dir).into_iter().cloned().collect();
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

    // Free members: hold nxm:// while waiting, then give it back.
    let mut rx = None;
    let free_nexus = needs_nexus && !premium;
    if free_nexus {
        let claimed = std::env::current_exe().map_err(Error::from).and_then(|exe| nexus::claim_nxm_handler(&exe));
        match claimed {
            Ok(prev) => {
                if let Some(p) = previous_handler_path(&app) {
                    let _ = std::fs::create_dir_all(p.parent().unwrap());
                    let _ = std::fs::write(p, prev.unwrap_or_default());
                }
                log::line("mods: the launcher takes Nexus download links until the mods are in");
            }
            Err(e) => log::line(&format!("mods: couldn't take nxm:// links: {e}")),
        }
        let (tx, r) = tokio::sync::mpsc::unbounded_channel();
        *state.mods.nxm_tx.lock().unwrap() = Some(tx);
        rx = Some(r);
    }

    let mut result = RunResult::default();
    for m in &todo {
        if cancel.load(Ordering::SeqCst) {
            result.cancelled = true;
            break;
        }
        let r = if m.nexus.is_some() {
            if premium {
                premium_one(&app, &api, m, &dir, &cancel).await
            } else {
                free_one(&app, &api, m, &dir, &cancel, rx.as_mut().unwrap()).await
            }
        } else {
            direct_one(&app, &state.http, m, &dir, &cancel).await
        };
        match r {
            Ok(()) if m.installed(&dir) => {
                emit(&app, m, "done", 0, 0, "Installed");
                result.installed.push(m.name.clone());
            }
            Ok(()) => {
                let msg = format!("installed, but {} still isn't there", m.check.join(", "));
                log::line(&format!("mods: {}: {msg}", m.name));
                emit(&app, m, "failed", 0, 0, msg.clone());
                result.failed.push((m.name.clone(), msg));
            }
            Err(e) if e == "cancelled" => {
                result.cancelled = true;
                emit(&app, m, "failed", 0, 0, "Stopped");
                break;
            }
            Err(e) => {
                log::line(&format!("mods: {} failed: {e}", m.name));
                emit(&app, m, "failed", 0, 0, e.clone());
                result.failed.push((m.name.clone(), e));
            }
        }
    }

    if free_nexus {
        *state.mods.nxm_tx.lock().unwrap() = None;
        restore_left_handler(&app);
    }
    *state.mods.cancel.lock().unwrap() = None;
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
