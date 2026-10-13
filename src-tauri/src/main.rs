// Hides the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use launcher_core::{aside, auth, game, gameini, health, loadorder, manifest::Manifest, netcheck, pristine, requirements, serverorder, settings, sync, version, watch, Error};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;

mod bundled;
mod faces;
mod log;
mod music;

/// A server that goes silent mid-answer is an error, not a wait for ever.
const STALL: std::time::Duration = std::time::Duration::from_secs(30);

/// Where the server publishes the launcher files. Set AD_BASE_URL when
/// building to point a release at the real server.
const DEFAULT_BASE_URL: &str = match option_env!("AD_BASE_URL") {
    Some(u) => u,
    None => "https://vps-d38c928e.vps.ovh.us/launcher",
};

/// The Discord login service. Set AD_AUTH_URL when building to change it.
const AUTH_URL: &str = match option_env!("AD_AUTH_URL") {
    Some(u) => u,
    None => "https://vps-d38c928e.vps.ovh.us/ad",
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Config {
    game_dir: Option<PathBuf>,
    base_url: String,
    /// The signed-in Discord account, shown in the launcher. The token itself
    /// is kept separately, encrypted (see auth::save_token).
    account: Option<auth::Profile>,
    /// Unix time of the last time the login service accepted the token.
    last_auth_ok: Option<u64>,
    close_on_launch: bool,
    background_updates: bool,
    /// Send game health reports to the server's staff (players can turn it off).
    share_health: bool,
    /// Menu music: None until the player answers the Keep music / Mute
    /// question, then their answer.
    music: Option<bool>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            game_dir: None,
            base_url: DEFAULT_BASE_URL.into(),
            account: None,
            last_auth_ok: None,
            close_on_launch: true,
            background_updates: true,
            share_health: true,
            music: None,
        }
    }
}

struct AppState {
    config: Mutex<Config>,
    manifest: Mutex<Option<Manifest>>,
    /// Serializes checks, game file changes and Play preparation. A second
    /// command gets a useful error instead of racing the first one.
    game_operation: Mutex<()>,
    /// Remains set from a successful SKSE launch until its watcher finishes.
    /// SKSE can exit before SkyrimSE.exe appears, so a process lookup alone
    /// cannot prevent a second Play during that gap.
    active_session: AtomicBool,
    /// Set only during the final launcher-update install/relaunch window.
    /// Play and file operations cannot begin after the updater reserves it.
    update_installing: AtomicBool,
    http: reqwest::Client,
    /// The same, taking gzip-compressed answers: face sharing only, so every
    /// other download stays byte for byte what the server sent.
    faces_http: reqwest::Client,
    music: music::Music,
}

type CmdResult<T> = Result<T, String>;

/// Windows' process enumeration is fallible. A failed lookup cannot be
/// treated as "Skyrim is closed" when a file operation or launcher update
/// would interrupt an external Steam or SKSE launch.
fn external_game_running() -> CmdResult<bool> {
    for name in [watch::GAME_PROCESS, game::SKSE_LOADER] {
        if watch::find_process_checked(name)
            .map_err(|e| format!("Windows couldn't check whether Skyrim is running ({e}). Try again in a moment."))?
            .is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn begin_game_operation(state: &AppState) -> CmdResult<tokio::sync::MutexGuard<'_, ()>> {
    let guard = state.game_operation.try_lock().map_err(|_| "The launcher is already checking files, updating, or starting Skyrim. Wait for that step to finish.".to_string())?;
    if state.update_installing.load(Ordering::SeqCst) {
        return Err("The launcher is installing an update. Wait for it to restart.".into());
    }
    if state.active_session.load(Ordering::SeqCst) {
        return Err("Skyrim is already starting or running. Wait for it to close.".into());
    }
    if external_game_running()? {
        return Err("Close Skyrim before checking, updating, or starting another game session.".into());
    }
    Ok(guard)
}

fn err(e: Error) -> String {
    let raw = e.to_string();
    let p = launcher_core::plain(&raw);
    if p != raw {
        log::line(&format!("error: {raw}"));
    }
    p
}

fn config_path(app: &AppHandle) -> CmdResult<PathBuf> {
    app.path().app_config_dir().map(|d| d.join("config.json")).map_err(|e| e.to_string())
}

fn load_config(app: &AppHandle) -> Config {
    config_path(app)
        .ok()
        .and_then(|p| {
            let bytes = std::fs::read(&p).ok()?;
            let parsed = serde_json::from_slice::<Config>(&bytes).ok();
            if parsed.is_none() {
                // Defaults take over, so keep the damaged file for support
                // instead of letting the next save overwrite it.
                let bad = p.with_extension("json.bad");
                log::line(&format!("config: {} couldn't be read; starting from defaults, copy kept at {}", p.display(), bad.display()));
                let _ = std::fs::write(&bad, &bytes);
            }
            parsed
        })
        .map(|mut c| {
            // The server address is baked into each build, so an older saved
            // config (or one from a test build) never points somewhere stale.
            c.base_url = DEFAULT_BASE_URL.into();
            c
        })
        .unwrap_or_default()
}

fn save_config(app: &AppHandle, c: &Config) -> CmdResult<()> {
    let path = config_path(app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    launcher_core::atomicfile::safe_write(&path, &serde_json::to_vec_pretty(c).unwrap()).map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    config: Config,
    game: Option<game::GameInfo>,
    game_error: Option<String>,
    launcher_version: String,
}

/// Everything the UI needs on startup. Finds the game if no folder is saved.
#[tauri::command]
async fn get_state(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Snapshot> {
    let mut config = state.config.lock().await;
    if config.game_dir.is_none() {
        if let Some(found) = game::detect() {
            config.game_dir = Some(found.dir);
            save_config(&app, &config)?;
        }
    }
    let (game, game_error) = match &config.game_dir {
        Some(dir) => match game::inspect(dir) {
            Ok(g) => (Some(g), None),
            // The saved folder stopped being Skyrim (a drive not plugged in
            // yet, or the game moved). Another install Steam knows about is
            // only suggested: the launcher changes and may downgrade the
            // folder it uses, so it never switches to one the player didn't
            // choose.
            Err(e) => (None, Some(match game::detect().filter(|f| &f.dir != dir) {
                Some(found) => {
                    log::line(&format!("game folder {} isn't a Skyrim folder now ({e}); Steam has one at {}, left for the player to choose", dir.display(), found.dir.display()));
                    format!("{} If Skyrim is now in {}, choose that folder.", err(e), found.dir.display())
                }
                None => err(e),
            })),
        },
        None => (None, Some("Skyrim Special Edition wasn't found. Pick its folder.".into())),
    };
    Ok(Snapshot { config: config.clone(), game, game_error, launcher_version: app.package_info().version.to_string() })
}

#[tauri::command]
async fn set_game_dir(app: AppHandle, state: State<'_, AppState>, dir: PathBuf) -> CmdResult<game::GameInfo> {
    let info = game::pick(&dir).map_err(err)?;
    let mut config = state.config.lock().await;
    config.game_dir = Some(info.dir.clone());
    save_config(&app, &config)?;
    Ok(info)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Prefs {
    close_on_launch: bool,
    background_updates: bool,
    #[serde(default = "yes")]
    share_health: bool,
}

fn yes() -> bool {
    true
}

#[tauri::command]
async fn set_prefs(app: AppHandle, state: State<'_, AppState>, prefs: Prefs) -> CmdResult<()> {
    let mut config = state.config.lock().await;
    config.close_on_launch = prefs.close_on_launch;
    config.background_updates = prefs.background_updates;
    config.share_health = prefs.share_health;
    save_config(&app, &config)
}

/// Puts Skyrim.ccc back when Play stops before the game starts.
struct CccGuard {
    dir: PathBuf,
    armed: bool,
}

impl Drop for CccGuard {
    fn drop(&mut self) {
        if self.armed {
            restore_ccc(&self.dir, "play stopped before the game started");
        }
    }
}

fn restore_ccc(dir: &std::path::Path, why: &str) {
    match serverorder::restore_ccc(dir) {
        Ok(true) => log::line(&format!("play: put Skyrim.ccc back ({why})")),
        Ok(false) => {}
        Err(e) => log::line(&format!("play: couldn't put Skyrim.ccc back ({why}): {e}")),
    }
}

/// Puts the player's mods back (core/src/profile.rs) when Play stops before the game starts.
struct ProfileGuard {
    dir: PathBuf,
    armed: bool,
}

impl Drop for ProfileGuard {
    fn drop(&mut self) {
        if self.armed {
            restore_profile(&self.dir, "play stopped before the game started");
        }
    }
}

fn restore_profile(dir: &std::path::Path, why: &str) {
    if !launcher_core::profile::pending(dir) {
        return;
    }
    match launcher_core::profile::restore(dir) {
        Ok(r) if r.kept.is_empty() => log::line(&format!("profile: put {} mod file(s) back ({why}){}", r.files, if r.duplicates > 0 { format!("; dropped {} identical copy(ies) already in place", r.duplicates) } else { String::new() })),
        Ok(r) => log::line(&format!("profile: put {} mod file(s) back ({why}); {} stay in .aetherial-dawn/aside because something new sits at their place: {}", r.files, r.kept.len(), r.kept.iter().take(10).cloned().collect::<Vec<_>>().join(", "))),
        Err(e) => log::line(&format!("profile: couldn't put the mod files back ({why}): {e}")),
    }
}

/// Puts back every file the launcher set aside (other mods, old plugins).
#[tauri::command]
async fn restore_set_aside(state: State<'_, AppState>) -> CmdResult<usize> {
    let _operation = begin_game_operation(&state)?;
    let dir = game_dir(&state).await?;
    let n = aside::restore_all(&dir).map_err(|e| format!("Couldn't put the files back ({e}). Close Skyrim, then try again."))?;
    log::line(&format!("restored {n} set-aside file(s)"));
    Ok(n)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CheckResult {
    build: String,
    server: launcher_core::manifest::Server,
    files: usize,
    remove: usize,
    bytes: u64,
    game: version::GameCheck,
    /// "What's new" lines for the update bar; empty when nothing changes.
    notes: Vec<String>,
}

async fn game_dir(state: &AppState) -> CmdResult<PathBuf> {
    state.config.lock().await.game_dir.clone().ok_or_else(|| "Pick your Skyrim folder first.".to_string())
}

/// The server's file list: the one the last check fetched, or fetched now
/// (text audit C4: the player is never told to check for updates first).
async fn manifest_now(state: &AppState) -> CmdResult<Manifest> {
    if let Some(m) = state.manifest.lock().await.clone() {
        return Ok(m);
    }
    let base = state.config.lock().await.base_url.clone();
    let m = Manifest::fetch(&state.http, &base).await.map_err(|e| {
        log::line(&format!("server file list: {e}"));
        "The server's file list hasn't loaded yet. The launcher is fetching it; try again in a moment.".to_string()
    })?;
    *state.manifest.lock().await = Some(m.clone());
    Ok(m)
}

/// Downloads the server's file list and works out what needs updating.
#[tauri::command]
async fn check(state: State<'_, AppState>, verify_all: bool) -> CmdResult<CheckResult> {
    let _operation = begin_game_operation(&state)?;
    let dir = game_dir(&state).await?;
    let base = state.config.lock().await.base_url.clone();
    let m = Manifest::fetch(&state.http, &base).await.map_err(err)?;
    let plan = sync::plan(&dir, &m, verify_all).await.map_err(err)?;
    let result = CheckResult {
        build: m.build.clone(),
        server: m.server.clone(),
        files: plan.download.len(),
        remove: plan.remove.len(),
        bytes: plan.download_bytes,
        game: auto_version(&dir, m.game.as_ref()),
        notes: if plan.download.is_empty() && plan.remove.is_empty() { vec![] } else {
            let (down, gone): (Vec<String>, Vec<String>) = (plan.download.iter().map(|f| f.path.clone()).collect(), plan.remove.clone());
            launcher_core::whatsnew::lines(&m.notes, &down, &gone)
        },
    };
    *state.manifest.lock().await = Some(m);
    Ok(result)
}

/// Brings the game folder in line with the file list, sending `sync-progress`
/// events to the UI as it goes.
#[tauri::command]
async fn update(app: AppHandle, state: State<'_, AppState>, verify_all: bool) -> CmdResult<String> {
    let _operation = begin_game_operation(&state)?;
    let dir = game_dir(&state).await?;
    let base = state.config.lock().await.base_url.clone();
    // What the player did is reported once, win or lose, so the scoreboard shows how often Update works (event and result only).
    let out: CmdResult<String> = async {
        // A cached list can be obsolete by the time Update is pressed.
        let m = Manifest::fetch(&state.http, &base).await.map_err(err)?;
        let plan = sync::plan(&dir, &m, verify_all).await.map_err(err)?;
        if !plan.kept.is_empty() {
            log::line(&format!("update: the file list asks to remove files this launcher didn't install; left alone: {}", plan.kept.join(", ")));
        }
        sync::apply(&state.http, &base, &dir, &plan, |p| {
            let _ = app.emit("sync-progress", p);
        })
        .await
        .map_err(err)?;
        *state.manifest.lock().await = Some(m.clone());
        Ok(m.build)
    }
    .await;
    send_client_event(&app, "update", out.is_ok());
    out
}

/// Tells the login service, in the background, that a button outside Play finished and whether it worked
/// (POST {AUTH_URL}/api/client-event; the bot counts it for the scoreboard). Event and result only: no names, paths or accounts.
/// Nothing is awaited, so the button never waits; 404 (route not live) and any failure are silent for the player.
fn send_client_event(app: &AppHandle, event: &'static str, ok: bool) {
    let Some(token) = token(app) else { return };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let body = serde_json::json!({ "event": event, "result": if ok { "ok" } else { "error" }, "launcher": app.package_info().version.to_string() });
        let url = format!("{}/api/client-event", AUTH_URL.trim_end_matches('/'));
        // contract-method: POST
        match state.http.post(&url).header("authorization", token).json(&body).timeout(std::time::Duration::from_secs(10)).send().await {
            Ok(r) if r.status().is_success() || r.status().as_u16() == 404 => {}
            Ok(r) => log::line(&format!("{event}: the outcome report got {}", r.status().as_u16())),
            Err(e) => log::line(&format!("{event}: couldn't send the outcome report: {}", launcher_core::scrub(&e.to_string()))),
        }
    });
}

/// Gets a game session from the login service, writes the SkyMP client
/// settings and remembered login, and starts Skyrim through SKSE.
/// Errors that start with "SIGNED_OUT:" mean the player must sign in again.
#[tauri::command]
/// Returns warnings to show once the game is starting (none today; the page
/// still reads the list).
async fn play(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    trace_start();
    let out = play_run(&app, &state).await;
    // One report per attempt (the service takes one per Play): a failed one
    // tells staff where it stopped, sharing permitting.
    if out.is_err() {
        trace_fail();
    }
    let dir = state.config.lock().await.game_dir.clone();
    if let Some(dir) = dir {
        send_client_status(&app, &dir);
    }
    out
}

async fn play_run(app: &AppHandle, state: &AppState) -> CmdResult<Vec<String>> {
    let app = app.clone();
    let _operation = begin_game_operation(state)?;
    let config = state.config.lock().await.clone();
    let dir = config.game_dir.clone().ok_or("Pick your Skyrim folder first.")?;
    // Play must use the current server list, even after the launcher idled.
    let m = Manifest::fetch(&state.http, &config.base_url).await.map_err(err)?;
    *state.manifest.lock().await = Some(m.clone());
    let gc = auto_version(&dir, m.game.as_ref());
    log::line(&format!("play: game folder {}, build {}, version needed={} skseOk={}", dir.display(), m.build, gc.needed, gc.skse_ok));
    trace_game_version(!gc.needed);
    if gc.needed {
        return Err(gc.reason.unwrap_or_else(|| "Your Skyrim version doesn't match the server.".into()));
    }
    if gc.installed.is_some() {
        if !requirements::skse_ok(&dir) {
            play_step(&app, "Installing SKSE…");
        }
        install_skse(&state.http, &dir).await?;
    }
    game::inspect(&dir).map_err(err)?;
    // One masters.json per Play, fetched before anything changes on the PC:
    // its plugin names decide which light plugins run as full "<stem>.esm"
    // copies (desync/esl-on-server.md), and the load-order and health steps
    // below use the same copy. Without it Play stops here.
    let masters = fetch_masters(&state.http, &config.base_url).await
        .ok_or("Couldn't get the server's list of game files. Check your internet connection, then press Play again.")?;
    let order = serverorder::server_order(&masters);
    if !serverorder::valid_base(&order) {
        return Err("The server's list of game files is broken, so Play can't check your Skyrim against it. Staff need to fix it on the server; try again later.".into());
    }
    fetch_server_plugins(&state.http, &config.base_url, &dir, &order).await?;
    let server_names: Vec<String> = order.iter().map(|p| p.name.clone()).collect();
    // No mods: anything beyond the game, SKSE and SkyMP's own files keeps the
    // player out (core/src/modguard.rs). Checked on every Play, before the
    // launcher changes anything on the PC.
    let mut rules = server_rules(&app, &state.http, &config.base_url).await;
    let flags = rules.narrow(launcher_flags(&state.http, &config.base_url).await);
    play_step(&app, "Setting other mods aside…");
    let extras = {
        let (dir, names, files) = (dir.clone(), server_names.clone(), m.files.iter().map(|f| f.path.clone()).collect::<Vec<_>>());
        // Test builds of our own mods: exact paths listed one per line in .aetherial-dawn/local-allow.txt stay in place until the server lists them.
        let mut files = files;
        // Our own mods ship inside the launcher: written into the game on every Play and always allowed.
        files.extend(bundled::deploy(&dir));
        files.extend(rules.allow_files.iter().cloned());
        files.extend(std::fs::read_to_string(dir.join(".aetherial-dawn").join("local-allow.txt")).unwrap_or_default().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()));
        tokio::task::spawn_blocking(move || launcher_core::modguard::extras(&dir, &names, &files)).await.map_err(|e| e.to_string())?
    };
    // Another server's Skyrim Platform scripts: set aside like other mods, unless staff switched it off.
    let mut extras: Vec<_> = extras.into_iter().filter(|e| e.kind != "script" || flags.platform_guard).collect();
    // The required Nexus collection's own files are not "other mods" (core/src/collection.rs).
    extras.retain(|e| !rules.collection.owns(e.kind, &e.rel));
    // The last game ran to the main menu and never joined: this one also runs without the official
    // Creation Club files (they come back when it closes). status.json `launcher.connectLadder`: false switches it off.
    if flags.connect_ladder {
        let level = launcher_core::ladder::level(&dir);
        if level > 0 {
            let cc = launcher_core::ladder::cc_extras(&dir, &server_names);
            log::line(&format!("play: the last {level} game(s) never connected; setting {} Creation Club file(s) aside for this session", cc.len()));
            extras.extend(cc);
        }
    }
    let foreign_scripts: Vec<String> = extras.iter().filter(|e| e.kind == "script").map(|e| e.rel.clone()).collect();
    if !foreign_scripts.is_empty() {
        log::line(&format!("play: another server's Skyrim Platform script(s) found, set aside for this session: {}", foreign_scripts.join(", ")));
    }
    let mut profile_guard = ProfileGuard { dir: dir.clone(), armed: false };
    if !extras.is_empty() {
        // The player keeps their mods: they leave the Skyrim folder for this session and come back when the game closes.
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string();
        let (d, ex) = (dir.clone(), extras.clone());
        let moved = tokio::task::spawn_blocking(move || launcher_core::profile::set_aside(&d, &ex, &stamp)).await.map_err(|e| e.to_string())?;
        profile_guard.armed = true;
        match moved {
            Ok(m) if m.failed.is_empty() => log::line(&format!("play: set {} mod file(s) aside for this session", m.files)),
            Ok(m) => {
                log::line(&format!("play: couldn't set aside {} mod file(s): {}", m.failed.len(), m.failed.iter().take(10).cloned().collect::<Vec<_>>().join("; ")));
                let names: Vec<String> = m.failed.iter().take(6).map(|f| f.split(" (").next().unwrap_or(f).rsplit('/').next().unwrap_or(f).to_string()).collect();
                let found = format!("{}{}", names.join(", "), if m.failed.len() > 6 { format!(", and {} more", m.failed.len() - 6) } else { String::new() });
                return Err(format!("MODS_FOUND:{found}"));
            }
            Err(e) => return Err(format!("Couldn't set your other mods aside ({e}). Close Skyrim and Vortex, then press Play again.")),
        }
    }
    tidy_game(&app, &dir, &m, &server_names, &rules.tune)?;
    // Skyrim Platform's in-game browser draws the login box; without it nobody can connect.
    let (browser_check, browser_temp_fallback) = if flags.browser_guard {
        let (c, alt) = browser_guard(&dir);
        (Some(c), alt)
    } else {
        (None, None)
    };
    // Once the server loads more than the five base masters, this PC loads
    // exactly its plugins in its order: no Creation Club masters for the
    // session, and plugins.txt in the server's order (serverorder.rs).
    play_step(&app, "Setting the server's load order…");
    let mut ccc_guard = CccGuard { dir: dir.clone(), armed: false };
    if serverorder::beyond_base(&order) {
        // The server's launcher rules can switch the Creation Club step off (rules.rs).
        match if rules.empty_ccc { serverorder::hide_ccc(&dir) } else { Ok(Vec::new()) } {
            Ok(h) => {
                ccc_guard.armed = true;
                if !h.is_empty() {
                    log::line(&format!("play: emptied Skyrim.ccc for this session (Creation Club masters would shift the server's plugin positions); the original is put back when the game closes: {}", h.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")));
                }
            }
            Err(e) => return Err(format!("Couldn't switch Creation Club content off for this session ({e}). Close Skyrim and try again.")),
        }
        if let Some(txt) = plugins_txt(&app) {
            match serverorder::set_exact(&dir, &txt, &order) {
                Ok(true) => log::line(&format!("play: set {} to the server's {} plugins in order, then this PC's own", txt.display(), order.len())),
                Ok(false) => {}
                Err(e) => return Err(format!("Couldn't set your load order ({e}). Close Skyrim, then try again.")),
            }
            // loadorder.txt is only a record for mod tools, but it should list nothing else.
            match if rules.rewrite_loadorder { serverorder::set_loadorder_exact(&txt.with_file_name("loadorder.txt"), &order) } else { Ok(false) } {
                Ok(true) => log::line(&format!("play: loadorder.txt now lists exactly the server's {} plugins (the old list is kept beside it)", order.len())),
                Ok(false) => {}
                Err(e) => log::line(&format!("play: couldn't rewrite loadorder.txt: {e}")),
            }
        }
    }
    // The server's own plugins are not the collection's to switch on or order (collection.rs `pinned`).
    rules.collection.pinned = order.iter().map(|p| p.name.clone()).collect();
    // The collection's plugins must be on in plugins.txt (Vortex leaves some off); only when the rules say so.
    if let Some(txt) = plugins_txt(&app) {
        match launcher_core::collection::enable_plugins(&rules.collection, &dir, &txt) {
            Ok(v) if !v.is_empty() => log::line(&format!("play: switched on the collection's plugins in plugins.txt: {}", v.join(", "))),
            Ok(_) => {}
            Err(e) => log::line(&format!("play: couldn't switch on the collection's plugins: {e}")),
        }
    }
    play_step(&app, "Checking your game…");
    let mut report = run_health_with_masters(&app, &dir, Some(&m), Some(masters)).await;
    rules.adjust_order_check(&mut report);
    rules.add_collection_check(&dir, plugins_txt(&app).as_deref(), &mut report);
    // "Same game files as the server": warn-only, so it can never stop Play (core/src/parity.rs).
    if flags.parity_report {
        if let Some(txt) = plugins_txt(&app) {
            let fp = launcher_core::parity::build(&app.package_info().version.to_string(), &dir, &txt, Some(&order));
            report.checks.push(launcher_core::parity::check(&fp));
        }
    }
    log::line(&format!("health before play: worst={:?}\n{}", report.worst, report.text()));
    let base_check = report.checks.iter().find(|c| c.id == "masters")
        .ok_or("Could not finish checking the base game files. Try Play again.")?;
    if base_check.status != health::Status::Ok {
        let exe_is_servers = m.game.as_ref().and_then(|g| g.version.as_deref()).and_then(version::parse_version)
            .is_some_and(|want| version::exe_version(&dir.join(game::GAME_EXE)) == Some(want));
        return Err(health::masters_fix_message(&dir, &base_check.items, exe_is_servers));
    }
    // Skyrim's own archives missing or empty: the game starts, then crashes or shows no textures or meshes.
    if flags.archive_gate {
        if let Some(c) = report.checks.iter().find(|c| c.id == "archives").filter(|c| c.status != health::Status::Ok) {
            log::line(&format!("play: stopped, Skyrim's own archives are missing or empty: {}", c.items.join(", ")));
            return Err(format!("Skyrim's own game archives are missing or empty ({}), so the game would crash or show missing textures and meshes. In Steam, right-click Skyrim Special Edition, Properties, Installed Files, Verify integrity of game files; then press Play again.", c.items.iter().take(4).cloned().collect::<Vec<_>>().join(", ")));
        }
    }
    // The game would stop with SkyMP's "LOAD ORDER ERROR"; say it here.
    if let Some(c) = report.checks.iter().find(|c| c.id == "serverorder" && c.status == health::Status::Fail) {
        let which = c.items.iter().take(3).cloned().collect::<Vec<_>>().join("; ");
        // Sent to staff by the launcher when sharing is on (text audit C2).
        let filed = if config.share_health { send_health(&app, &state.http, &config, &m.build, "before play", None, None, None, &report).await } else { Filed::default() };
        let tell = match filed.report_id {
            Some(id) => format!("Staff have been sent a report ({id})."),
            None => "Send Copy diagnostics to staff.".into(),
        };
        return Err(rules.order_mismatch_message(&which, &tell));
    }
    // The required Nexus collection (rules `collection.level` = "block"): a missing mod or a plugin that is off stops Play.
    if let Some(c) = report.checks.iter().find(|c| c.id == "collection" && c.status == health::Status::Fail) {
        log::line(&format!("play: stopped, the required collection is incomplete: {}", c.items.join("; ")));
        return Err(launcher_core::collection::block_message(&rules.collection, &c.items));
    }
    // Skyrim that starts but never reaches the server: Windows Firewall rules
    // for SkyrimSE.exe and the server's UDP port. Advisory only: nothing here
    // stops Play. Staff switch parts off in status.json (`launcher.firewallFix`,
    // `launcher.udpProbe`).
    // Which antivirus and firewall products this PC has. When another product has switched Windows' own
    // firewall off, rules in it change nothing, so the launcher doesn't ask Windows for permission to edit them.
    let security = tokio::task::spawn_blocking(netcheck::security_summary).await.unwrap_or_default();
    log::line(&format!("play: security software: {security}"));
    let windows_firewall_off = netcheck::windows_firewall_off(&security);
    let firewall = if flags.firewall_fix && windows_firewall_off {
        netcheck::Firewall::Unknown("Windows Firewall is turned off on this PC (another security product manages it), so the launcher left its rules alone".into())
    } else if flags.firewall_fix {
        play_step(&app, "Checking Windows Firewall for Skyrim…");
        let (exe, marker) = (dir.join(game::GAME_EXE), app.path().app_local_data_dir().ok().map(|d| d.join("firewall-fix.txt")));
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let notify = app.clone();
        tokio::task::spawn_blocking(move || {
            netcheck::ensure_firewall(&exe, marker.as_deref(), now, &|| {
                log::line("play: Windows Firewall blocks or doesn't allow Skyrim; asking Windows for permission to fix it");
                let _ = notify.emit("play-step", "Windows will ask permission to let Skyrim through its firewall. Press Yes…");
            })
        })
        .await
        .unwrap_or_else(|e| netcheck::Firewall::Unknown(e.to_string()))
    } else {
        netcheck::Firewall::Off
    };
    log::line(&format!("play: firewall check: {firewall:?}"));
    let udp = if flags.udp_probe {
        play_step(&app, "Checking the connection to the server…");
        netcheck::probe_udp(&m.server.ip, m.server.port, 3, std::time::Duration::from_millis(1500)).await
    } else {
        netcheck::Probe::Off
    };
    log::line(&format!("play: server UDP probe {}:{}: {udp:?}", m.server.ip, m.server.port));
    report.checks.push(netcheck::firewall_check(&firewall));
    report.checks.push(netcheck::platform_check(&foreign_scripts));
    report.checks.push(netcheck::udp_check(&udp, m.server.port));
    if rules.ping_stats && matches!(udp, netcheck::Probe::Answered(_)) {
        if let Some(st) = netcheck::ping_stats(&m.server.ip, m.server.port, 10, std::time::Duration::from_millis(200)).await {
            log::line(&format!("play: server ping {st:?}"));
            report.checks.push(netcheck::ping_check(&st));
        }
    }
    if let Some(c) = browser_check.clone() {
        report.checks.push(c);
    }
    report.worst = report.checks.iter().map(|c| c.status).max().unwrap_or(health::Status::Ok);
    if report.worst >= health::Status::Warn && config.share_health {
        // Nobody needs the report number here, and the staff service can
        // rate-limit for 90 s or more, so the upload runs beside Play.
        let (app, http, config, build, report) = (app.clone(), state.http.clone(), config.clone(), m.build.clone(), report.clone());
        launcher_core::detach::detach(async move {
            send_health(&app, &http, &config, &build, "before play", None, None, None, &report).await;
        });
    }
    let token = token(&app).ok_or("SIGNED_OUT:Sign in with Discord to play.")?;
    play_step(&app, "Getting your game session…");
    let session = match auth::play(&state.http, AUTH_URL, &token).await {
        auth::Answer::Ok(p) => {
            log::line("play: login service gave a game session");
            p.session
        }
        auth::Answer::SignedOut(msg) => {
            sign_out(&app, state).await;
            return Err(format!("SIGNED_OUT:{msg}"));
        }
        auth::Answer::Refused { message, .. } => {
            sign_out(&app, state).await;
            return Err(format!("SIGNED_OUT:{message}"));
        }
        auth::Answer::Offline(msg) => return Err(format!("Couldn't get you into the game: {msg}. Try again in a minute.")),
        auth::Answer::Pending => return Err("The login service didn't answer. Try again.".into()),
    };
    settings::write(
        &dir,
        &settings::ClientSettings {
            server_ip: &m.server.ip,
            server_port: m.server.port,
            master: AUTH_URL,
            server_master_key: auth::SERVER_KEY,
            session: &session,
        },
    )
    .map_err(err)?;
    settings::write_auth_data(&dir, &session, &config.account.clone().unwrap_or_default()).map_err(err)?;
    log::line("play: wrote skymp5-client-settings.txt and auth data");
    // Read the settings back the way the SkyMP client will: where it dials, the login session, and that its script is there.
    let bad = settings::verify(&dir, &m.server.ip, m.server.port, &session);
    if bad.is_empty() {
        log::line(&format!("play: settings checked: the client will dial {}:{} with the login session just issued", m.server.ip, m.server.port));
    } else {
        log::line(&format!("play: settings check failed: {}", bad.join("; ")));
        return Err("Couldn't set up your connection to the server. Press Play again; if it keeps happening, use Report a problem.".into());
    }
    state.music.stop();
    // Last look before the game starts: Steam can put Skyrim.ccc back after the
    // first set-up, which would put Creation Club masters ahead of the server's
    // plugins. Empty it again, set plugins.txt again, and stop with a clear
    // message instead of starting a game the server would turn away.
    if serverorder::beyond_base(&order) {
        if let Some(txt) = plugins_txt(&app) {
            match serverorder::hide_ccc(&dir) {
                Ok(h) if !h.is_empty() => {
                    ccc_guard.armed = true;
                    log::line(&format!("play: Skyrim.ccc had been refilled since set-up; emptied it again right before launch: {}", h.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")));
                }
                Ok(_) => {}
                Err(e) => log::line(&format!("play: couldn't re-check Skyrim.ccc before launch: {e}")),
            }
            if let Err(e) = serverorder::set_exact(&dir, &txt, &order) {
                log::line(&format!("play: couldn't re-check plugins.txt before launch: {e}"));
            }
            let bad = serverorder::mismatches_with(&dir, &txt, &order, true);
            if !bad.is_empty() {
                return Err(format!("Your plugins don't match the server's order, so the game would refuse to connect: {}. Close Steam's Creation Club downloads and press Play again.", bad.iter().take(3).cloned().collect::<Vec<_>>().join("; ")));
            }
            log::line(&format!("play: load order verified right before launch: the game's first {} plugins are the server's", order.len()));
        }
    }
    // What this PC will start with, for the server to compare with its own order. Background, best effort.
    if flags.parity_report {
        send_client_parity(&app, &dir, &order, false, 0);
    }
    play_step(&app, "Starting Skyrim through SKSE…");
    // The game gets its own TEMP (core/src/isolate.rs) so nothing another SkyMP client left in %TEMP%
    // reaches it; if that cannot be made, the writable-fallback folder from the browser check is used.
    let game_temp = if flags.isolate_temp {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string();
        match launcher_core::isolate::prepare(&dir, &stamp) {
            Ok(t) => {
                log::line(&format!("play: the game gets its own temp folder {} (other Skyrim servers' temp files stay out)", t.display()));
                Some(t)
            }
            Err(e) => {
                log::line(&format!("play: couldn't make the game's own temp folder ({e}); using the system temp"));
                browser_temp_fallback.clone()
            }
        }
    } else {
        browser_temp_fallback.clone()
    };
    *GAME_TEMP.lock().unwrap_or_else(|p| p.into_inner()) = game_temp.clone();
    let launched = game::launch(&dir, game_temp.as_deref());
    launched.map_err(err)?;
    state.active_session.store(true, Ordering::SeqCst);
    let google = game::google_env_present();
    if !google.is_empty() {
        log::line(&format!("play: left Google sign-in settings out of the game's environment: {}", google.join(", ")));
    }
    log::line("play: started skse64_loader.exe");
    // Skyrim.ccc goes back when the game closes (watch_game).
    ccc_guard.armed = false;
    // The game is up: the mods stay aside until it closes (watch_game).
    profile_guard.armed = false;
    let started = std::time::SystemTime::now();
    if config.close_on_launch {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.hide();
        }
    }
    // Read the order again while the game runs: plugins.txt was seen rewritten ~12 s after the start, after the launch reading matched.
    if flags.parity_report {
        for secs in [45, 180] {
            send_client_parity(&app, &dir, &order, true, secs);
        }
    }
    tauri::async_runtime::spawn(watch_game(app.clone(), dir.clone(), started, config.close_on_launch));
    Ok(Vec::new())
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CrashFiled {
    report_id: Option<String>,
    likely_cause: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GameEnded {
    crashed: bool,
    summary: String,
    report: String,
    /// Staff's report number and likely cause, when the report was sent.
    report_id: Option<String>,
    likely_cause: Option<String>,
}

fn reports_dir() -> Option<PathBuf> {
    log::path().and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

/// Whether Skyrim Platform's browser helper was running 30 s into the last
/// session: 1 yes, 0 no, -1 not known.
fn browser_probe() -> &'static std::sync::atomic::AtomicI8 {
    static B: std::sync::atomic::AtomicI8 = std::sync::atomic::AtomicI8::new(-1);
    &B
}

/// Closing Skyrim with its X (or Alt+F4) can leave SkyrimSE.exe and Skyrim
/// Platform's browser helper running with no window, so Steam still shows the
/// game as running, and Play refuses to start another session until it is gone.
/// Once Skyrim has had a window, and then has none for a few seconds while the
/// process is still alive, the launcher ends it and the helpers, so a player
/// can rejoin straight away.
const WINDOWLESS_GRACE_SECS: u32 = 4;
async fn end_when_window_closed(pid: u32) {
    let alive = || watch::find_process(watch::GAME_PROCESS) == Some(pid);
    let mut seen = false;
    let mut gone: u32 = 0;
    while alive() {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if watch::has_visible_window(pid) {
            seen = true;
            gone = 0;
        } else if seen {
            gone += 1;
            if gone >= WINDOWLESS_GRACE_SECS && alive() {
                log::line(&format!("game: Skyrim's window closed but the game kept running for {WINDOWLESS_GRACE_SECS} seconds; ending it so Steam sees it closed"));
                watch::terminate(pid);
                for helper in watch::BROWSER_HELPERS {
                    let ended = tokio::task::spawn_blocking(move || watch::end_all(helper)).await.unwrap_or_default();
                    if !ended.is_empty() {
                        log::line(&format!("game: ended {} leftover {helper} process(es)", ended.len()));
                    }
                }
                return;
            }
        }
    }
}

/// A Skyrim left behind with no window (force-closed, or started by an earlier
/// launcher) would make Play refuse until the player found and ended it in Task
/// Manager. While no session of this launcher is running, a SkyrimSE.exe that
/// has been up for at least 30 seconds and has had no window for 8 seconds in a
/// row is ended, with Skyrim Platform's helpers, so Play works straight away.
async fn end_leftover_skyrim(app: AppHandle) {
    let mut gone = 0u32;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if app.state::<AppState>().active_session.load(Ordering::SeqCst) {
            gone = 0;
            continue;
        }
        let Some(pid) = watch::find_process(watch::GAME_PROCESS) else {
            gone = 0;
            continue;
        };
        if watch::has_visible_window(pid) || !watch::process_age_secs(pid).is_some_and(|a| a >= 30) {
            gone = 0;
            continue;
        }
        gone += 1;
        if gone >= 8 {
            gone = 0;
            // Staff can switch this off (status.json launcher.leftoverKill=false): a Skyrim started from Steam that is
            // still loading, or whose window is hidden, looks the same as a left-behind one.
            let (http, base_url) = {
                let state = app.state::<AppState>();
                let http = state.http.clone();
                let base_url = state.config.lock().await.base_url.clone();
                (http, base_url)
            };
            let status = get_json(&http, &format!("{}/status.json", base_url.trim_end_matches('/'))).await;
            if !netcheck::leftover_kill_on(status.as_ref()) {
                continue;
            }
            log::line("game: a Skyrim with no window was left running (force-closed, or from an earlier launcher); ending it so Play works");
            watch::terminate(pid);
            for helper in watch::BROWSER_HELPERS {
                let ended = tokio::task::spawn_blocking(move || watch::end_all(helper)).await.unwrap_or_default();
                if !ended.is_empty() {
                    log::line(&format!("game: ended {} leftover {helper} process(es)", ended.len()));
                }
            }
        }
    }
}

/// Follows Skyrim from launch to exit. Every session leaves a report in the
/// log folder; a crash brings the launcher back with that report on screen.
/// The TEMP folder the running game was given (None: the system's).
static GAME_TEMP: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

fn game_temp_dir() -> std::path::PathBuf {
    GAME_TEMP.lock().unwrap_or_else(|p| p.into_inner()).clone().unwrap_or_else(std::env::temp_dir)
}

async fn watch_game(app: AppHandle, game_dir: std::path::PathBuf, started: std::time::SystemTime, close_on_launch: bool) {
    let close = watch_game_inner(app.clone(), &game_dir, started, close_on_launch).await;
    app.state::<AppState>().active_session.store(false, Ordering::SeqCst);
    restore_ccc(&game_dir, "the game closed");
    restore_profile(&game_dir, "the game closed");
    if close {
        app.exit(0);
    }
}

/// True when the launcher should close now (the game ended normally and the
/// player chose to close it on launch).
async fn watch_game_inner(app: AppHandle, game_dir: &std::path::Path, started: std::time::SystemTime, close_on_launch: bool) -> bool {
    let game_dir = game_dir.to_path_buf();
    // skse64_loader starts SkyrimSE.exe and exits, so look for the game
    // itself; while the loader still runs (Steam starting or updating), keep
    // waiting rather than calling a slow start a crash.
    let pid = watch::wait_for_game(watch::find_process, std::time::Duration::from_secs(1), watch::START_WAIT, watch::START_WAIT_MAX).await;
    let (code, ran, summary) = match pid {
        None => (None, std::time::Duration::ZERO, "Skyrim didn't start: SkyrimSE.exe never appeared within 90 seconds of SKSE's loader finishing.".to_string()),
        Some(pid) => {
            log::line(&format!("game: SkyrimSE.exe running as process {pid}"));
            tokio::spawn(end_when_window_closed(pid));
            // Skyrim Platform's in-game browser draws the SkyMP login box (the
            // way into the server). Whether its helper process is up 30 s in,
            // for the report: no helper means no login box (overlay, graphics
            // card or antivirus trouble).
            let (ccc_dir, ccc_txt) = (game_dir.clone(), plugins_txt(&app));
            let browser_seen = browser_probe();
            browser_seen.store(-1, Ordering::SeqCst);
            tokio::spawn(async move {
              // At 12 s too: players often close a game that sits at the main menu before 30 s.
              let mut waited = 0u64;
              for at in [12u64, 30] {
                tokio::time::sleep(std::time::Duration::from_secs(at - waited)).await;
                waited = at;
                if watch::find_process(watch::GAME_PROCESS) != Some(pid) {
                    return;
                }
                // Evidence for the load order: what Skyrim.ccc and plugins.txt hold while the game runs.
                let ccc_sizes: Vec<String> = ["Skyrim.ccc", "Data/Skyrim.ccc"].iter().filter_map(|f| std::fs::metadata(ccc_dir.join(f)).ok().map(|m| format!("{f} {} bytes", m.len()))).collect();
                let entries: Vec<String> = ccc_txt.as_ref().and_then(|t| std::fs::read_to_string(t).ok()).unwrap_or_default().lines().filter(|l| !l.trim_start().starts_with('#') && !l.trim().is_empty()).map(|l| l.trim().to_string()).collect();
                let head: Vec<String> = entries.iter().take(8).cloned().collect();
                log::line(&format!("game: {at} seconds in, {}; plugins.txt starts with: {}", if ccc_sizes.is_empty() { "no Skyrim.ccc".to_string() } else { ccc_sizes.join(", ") }, head.join(", ")));
                // A "*" means switched on. The game may list Creation Club plugins itself, switched off; only a switched-on one can load.
                let cc_on: Vec<&str> = entries.iter().filter(|l| l.starts_with('*') && l[1..].trim_start().to_ascii_lowercase().starts_with("cc")).map(|l| l.as_str()).collect();
                log::line(&format!("game: {at} seconds in, plugins.txt has {} Creation Club plugin(s) listed, {} switched on{}", entries.iter().filter(|l| l.trim_start_matches('*').trim_start().to_ascii_lowercase().starts_with("cc")).count(), cc_on.len(), if cc_on.is_empty() { String::new() } else { format!(": {}", cc_on.join(", ")) }));
                // The browser creates its profile folder in TEMP when it starts; no folder means its code never got that far.
                let temp_made = game_temp_dir().join("Skyrim Platform").exists();
                log::line(&format!("game: {at} seconds in, the browser's temp folder {} exists", if temp_made { "DOES" } else { "does NOT" }));
                let up = watch::BROWSER_HELPERS.iter().any(|h| watch::find_process(h).is_some());
                browser_probe().store(up as i8, Ordering::SeqCst);
                log::line(&format!("game: Skyrim Platform's in-game browser (SkyrimPlatformCEF.exe) {} {at} seconds after the game started", if up { "is running" } else { "is NOT running" }));
              }
            });
            // Face sharing for as long as the game runs.
            let stop = std::sync::Arc::new(AtomicBool::new(false));
            let sharing = token(&app).map(|t| {
                let http = app.state::<AppState>().faces_http.clone();
                tokio::spawn(faces::run(http, AUTH_URL.to_string(), t, game_dir.clone(), stop.clone()))
            });
            let code = tokio::task::spawn_blocking(move || watch::wait_exit(pid)).await.ok().flatten();
            stop.store(true, Ordering::SeqCst);
            let last = match sharing {
                Some(h) => h.await.ok().flatten(),
                None => None,
            };
            faces::tidy(&game_dir, last);
            let ran = started.elapsed().unwrap_or_default();
            let summary = match code {
                None => format!("Skyrim closed after {} seconds (Windows gave no exit code).", ran.as_secs()),
                Some(_) => format!("Skyrim {} after {} seconds.", watch::describe(code), ran.as_secs()),
            };
            (code, ran, summary)
        }
    };
    let crashed = pid.is_none() || watch::crashed(code, ran);
    log::line(&format!("game: {summary}{}", if crashed { " Treated as a crash." } else { "" }));
    let mut summary = summary;
    if crashed && version::forget_manual(&game_dir) {
        // A wrong game build is the usual reason Skyrim dies within seconds.
        log::line("game: removed the hand-set \"already on this version\" mark so the version check runs again");
        summary.push_str(" Your game files may not be the version Aetherial Dawn needs, so the launcher is checking them again.");
    }
    let docs = app
        .path()
        .document_dir()
        .or_else(|_| app.path().home_dir().map(|h| h.join("Documents")))
        .unwrap_or_default();
    let skse_logs = docs.join("My Games").join("Skyrim Special Edition").join("SKSE");
    let mut report = format!(
        "Aetherial Dawn game session report\nLauncher {} · {}\n{summary}\n",
        app.package_info().version,
        log::timestamp()
    );
    let (http, config, build, manifest) = {
        let st = app.state::<AppState>();
        let m = st.manifest.lock().await.clone();
        let c = st.config.lock().await.clone();
        (st.http.clone(), c, m.as_ref().map(|m| m.build.clone()).unwrap_or_default(), m)
    };
    let mut health = run_health(&app, &http, &config.base_url, &game_dir, manifest.as_ref()).await;
    // Skyrim ran a while, did not crash, and never loaded a world: it most likely
    // never connected (switch off with status.json `launcher.connectWatch`: false).
    let flags = server_rules(&app, &http, &config.base_url).await.narrow(launcher_flags(&http, &config.base_url).await);
    let skse_text = std::fs::read_to_string(skse_logs.join("skse64.log")).unwrap_or_default();
    let never_connected = !crashed && pid.is_some() && flags.connect_watch && netcheck::never_connected(&skse_text, ran.as_secs());
    if flags.connect_ladder && !crashed && pid.is_some() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let level = launcher_core::ladder::record(&game_dir, never_connected, netcheck::world_loaded(&skse_text), now);
        log::line(&format!("game: connect ladder level is now {level} (0 = the last game got into a world; the next Play sets Creation Club files aside from 1)"));
    }
    if never_connected {
        log::line(&format!("game: ran {} seconds without crashing but never loaded into the world; it most likely never connected to the server", ran.as_secs()));
        let mut connect = netcheck::connect_check(ran.as_secs());
        if browser_probe().load(Ordering::SeqCst) == 0 {
            connect.items.push("Skyrim Platform's in-game browser was not running, so the login box could not show. Try closing Medal, Discord and Xbox Game Bar overlays, and check antivirus.".into());
            summary.push_str(" The in-game browser that draws the login box was not running; closing overlays such as Medal, Discord and Xbox Game Bar often fixes that.");
        }
        health.checks.push(connect);
        health.worst = health.checks.iter().map(|c| c.status).max().unwrap_or(health::Status::Ok);
        summary.push_str(" Skyrim never loaded into the world, so it most likely never connected to the server. Staff can see why if you leave sharing on; a firewall, router or VPN blocking UDP is the usual cause.");
    }
    report.push_str(&format!("\n===== game health =====\n{}", health.text()));
    // The player's own folders stay private in what staff get (quality L1).
    let private: Vec<PathBuf> = [app.path().home_dir().ok(), app.path().document_dir().ok()].into_iter().flatten().collect();
    let private: Vec<&std::path::Path> = private.iter().map(|p| p.as_path()).collect();
    let mut staff_summary = watch::redact_paths(&summary, &private);
    let crash_log = if crashed { watch::crash_log_for_staff(&skse_logs, started, &private) } else { None };
    if crashed {
        if let Some(cl) = watch::crash_logger_summary(&skse_logs, started) {
            log::line(&format!("game: crash logger says:\n{cl}"));
            staff_summary.push_str(&format!("\n\nCrash logger:\n{}", watch::redact_paths(&cl, &private)));
        }
    }
    // ENB, ReShade and other injectors go to the backup folder after a
    // crash, so the next Play starts without them (text audit A7); "Put my
    // other mods back" in Settings brings them back.
    if crashed {
        let inj = health::injector_files(&game_dir);
        if !inj.is_empty() {
            let stamp = format!("{}-injectors", log::timestamp().replace([':', ' '], "-"));
            match aside::move_aside(&game_dir, &inj, &stamp) {
                Ok(dest) => log::line(&format!("game: set the injectors aside after the crash, in {}: {}", dest.display(), inj.join(", "))),
                Err(e) => log::line(&format!("game: couldn't set the injectors aside ({e}): {}", inj.join(", "))),
            }
        }
    }
    // Sent in the background: the staff service can ask for a short wait
    // (one report per 20 s), and the crash window shouldn't wait for it.
    // Where the player was and who was near, from the log ad-report.js writes (names and numbers only).
    let position = if crashed { launcher_core::adreport::position_text(&platform_log_text(&app)) } else { None };
    if crashed && config.share_health {
        let (app2, http2, health2, config2, build2, staff2) = (app.clone(), http.clone(), health.clone(), config.clone(), build.clone(), staff_summary.clone());
        tauri::async_runtime::spawn(async move {
            let f = send_health(&app2, &http2, &config2, &build2, "after a crash", Some(&staff2), crash_log, position, &health2).await;
            if f.report_id.is_some() {
                let _ = app2.emit("crash-filed", CrashFiled { report_id: f.report_id, likely_cause: f.likely_cause });
            }
        });
    }
    let filed = Filed::default();
    report.push_str(&format!("\n===== game data =====\n{}{}", bundled::report_line(&game_dir, plugins_txt(&app).as_deref()), game_data_report(&app, &game_dir, flags.asset_report)));
    let session_logs = watch::collect(&skse_logs, &game_temp_dir(), started);
    let browser_text = {
        let t = launcher_core::browsercheck::report_text(&game_dir);
        let defender = tokio::task::spawn_blocking(netcheck::defender_history).await.unwrap_or_default();
        let security = tokio::task::spawn_blocking(netcheck::security_summary).await.unwrap_or_default();
        format!("{t}===== security software =====\n{security}\n===== Windows Defender detections naming Skyrim, cef or Platform =====\n{defender}\n")
    };
    report.push_str(&session_logs);
    report.push_str(&format!("\n===== launcher log (last 40 lines) =====\n{}\n", log::tail(40)));
    // Staff can read what happened on the next attempt: the whole session report
    // (health, SKSE and Skyrim Platform log tails, the launcher log) goes up with
    // the "never connected" health report, paths left out. The bot keeps it in
    // the crash log field and the size limit trims it. Switch: launcher.sessionUpload=false.
    if never_connected && config.share_health {
        let (app2, http2, health2, summary2, config2, build2) = (app.clone(), http.clone(), health.clone(), staff_summary.clone(), config.clone(), build.clone());
        let session_text = if flags.session_upload { Some(watch::redact_paths(&format!("Session logs after a game that never connected (browser helper at 30 s: {})\n{session_logs}\n{browser_text}\n===== launcher log (last 40 lines) =====\n{}", match browser_probe().load(Ordering::SeqCst) { 1 => "running", 0 => "NOT running", _ => "not known" }, log::tail(40)), &private)) } else { None };
        tauri::async_runtime::spawn(async move {
            send_health(&app2, &http2, &config2, &build2, "game ran but never connected", None, session_text, Some(summary2), &health2).await;
        });
    }
    if let Some(dir) = reports_dir() {
        let name = format!("game-{}.txt", log::timestamp().replace([':', ' '], "-"));
        if std::fs::write(dir.join(&name), &report).is_ok() {
            log::line(&format!("game: saved session report {name}"));
        }
    }
    if crashed {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
        }
        let _ = app.emit("game-ended", GameEnded { crashed, summary: summary.clone(), report: report.clone(), report_id: filed.report_id.clone(), likely_cause: filed.likely_cause.clone() });
        false
    } else if close_on_launch {
        true
    } else {
        let _ = app.emit("game-ended", GameEnded { crashed, summary: summary.clone(), report: report.clone(), report_id: filed.report_id.clone(), likely_cause: filed.likely_cause.clone() });
        false
    }
}

// ---------- menu music ----------

/// Starts the menu music unless the player muted it. Returns whether the
/// player has answered the Keep music / Mute question yet.
#[tauri::command]
async fn music_start(state: State<'_, AppState>) -> CmdResult<bool> {
    let c = state.config.lock().await.clone();
    if c.music != Some(false) && watch::find_process("SkyrimSE.exe").is_none() {
        if let Some(dir) = c.game_dir {
            state.music.play(dir);
        }
    }
    Ok(c.music.is_some())
}

#[tauri::command]
async fn set_music(app: AppHandle, state: State<'_, AppState>, on: bool) -> CmdResult<()> {
    let mut c = state.config.lock().await;
    c.music = Some(on);
    save_config(&app, &c)?;
    if on {
        if let Some(dir) = c.game_dir.clone() {
            state.music.play(dir);
        }
    } else {
        state.music.stop();
    }
    log::line(&format!("music: turned {}", if on { "on" } else { "off" }));
    Ok(())
}

/// What's in the game folder that can crash Skyrim before the main menu:
/// every master, plugin and archive in Data (Creation Club content included),
/// the Creation Club list, the player's load order, and DLLs next to the exe.
fn game_data_report(app: &AppHandle, dir: &std::path::Path, assets: bool) -> String {
    let mut o = String::new();
    let mut files: Vec<(String, u64, String)> = std::fs::read_dir(dir.join("Data"))
        .map(|r| {
            r.flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    let l = n.to_ascii_lowercase();
                    if !(l.ends_with(".esm") || l.ends_with(".esl") || l.ends_with(".esp") || l.ends_with(".bsa") || l == "skyrim.ccc") {
                        return None;
                    }
                    let md = e.metadata().ok()?;
                    Some((n, md.len(), md.modified().map(log::stamp).unwrap_or_default()))
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort_by_key(|f| f.0.to_ascii_lowercase());
    let cc = files.iter().filter(|f| f.0.to_ascii_lowercase().starts_with("cc")).count();
    o.push_str(&format!("Data folder: {} masters/plugins/archives, {cc} of them Creation Club (cc*)\n", files.len()));
    for (n, len, when) in &files {
        o.push_str(&format!("  {n}  {len} bytes  {when}\n"));
    }
    let ccc = std::fs::read_to_string(dir.join("Data").join("Skyrim.ccc")).or_else(|_| std::fs::read_to_string(dir.join("Skyrim.ccc")));
    match ccc {
        Ok(t) => {
            let listed: Vec<&str> = t.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            let missing: Vec<&str> = listed.iter().copied().filter(|l| !files.iter().any(|f| f.0.eq_ignore_ascii_case(l))).collect();
            o.push_str(&format!("Skyrim.ccc lists {} Creation Club files; {} of them are not in Data (the game skips those)\n", listed.len(), missing.len()));
        }
        Err(_) => o.push_str("Skyrim.ccc: not found\n"),
    }
    let plugins = app.path().local_data_dir().ok().map(|d| d.join("Skyrim Special Edition").join("plugins.txt"));
    match plugins.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
        Some(t) => {
            let on: Vec<&str> = t.lines().map(str::trim).filter(|l| l.starts_with('*')).collect();
            o.push_str(&format!("plugins.txt: {} enabled: {}\n", on.len(), if on.is_empty() { "none".into() } else { on.join(", ") }));
        }
        None => o.push_str("plugins.txt: not found (only the base game and Creation Club load)\n"),
    }
    let dlls: Vec<String> = std::fs::read_dir(dir)
        .map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.to_ascii_lowercase().ends_with(".dll")).collect())
        .unwrap_or_default();
    o.push_str(&format!("DLLs next to SkyrimSE.exe: {}\n", if dlls.is_empty() { "none".into() } else { dlls.join(", ") }));
    // Texture check (switch off: status.json launcher.assetReport=false).
    if assets {
        let docs = app.path().document_dir().or_else(|_| app.path().home_dir().map(|h| h.join("Documents"))).ok();
        let text = launcher_core::visuals::report(dir, docs.as_deref());
        for l in text.lines().filter(|l| l.contains("MISSING OR EMPTY")) {
            log::line(&format!("game: {}", l.trim()));
        }
        o.push_str(&text);
    }
    o
}

/// The newest game session report, for the crash screen and diagnostics.
fn last_report() -> Option<(String, String)> {
    let dir = reports_dir()?;
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("game-") && n.ends_with(".txt"))
        .collect();
    names.sort();
    let name = names.pop()?;
    Some((name.clone(), std::fs::read_to_string(dir.join(name)).ok()?))
}

// ---------- Discord sign-in ----------

fn token_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("session.bin"))
}

fn token(app: &AppHandle) -> Option<String> {
    token_path(app).and_then(|p| auth::load_token(&p))
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// A loggable summary of a login service answer. Never includes the token.
fn answer_kind<T>(a: &auth::Answer<T>) -> String {
    match a {
        auth::Answer::Ok(_) => "accepted".into(),
        auth::Answer::Pending => "waiting for the browser".into(),
        auth::Answer::SignedOut(m) => format!("signed out ({m})"),
        auth::Answer::Refused { error, message } => format!("refused {error} ({message})"),
        auth::Answer::Offline(m) => format!("service unreachable ({m})"),
    }
}

async fn sign_out(app: &AppHandle, state: &AppState) {
    log::line("signing out and clearing the saved login");
    if let Some(p) = token_path(app) {
        auth::forget_token(&p);
    }
    let mut config = state.config.lock().await;
    config.account = None;
    config.last_auth_ok = None;
    if let Some(dir) = &config.game_dir {
        settings::clear_login(dir);
    }
    let _ = save_config(app, &config);
}

async fn signed_in(app: &AppHandle, state: &AppState, profile: auth::Profile) -> CmdResult<()> {
    let mut config = state.config.lock().await;
    config.account = Some(profile);
    config.last_auth_ok = Some(now());
    save_config(app, &config)
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct AuthStatus {
    signed_in: bool,
    account: Option<auth::Profile>,
    /// Signed in, but the service couldn't be reached to confirm it.
    offline: bool,
    /// Play is locked (offline for too long).
    locked: bool,
    message: Option<String>,
}

/// Re-checks the saved sign-in with the login service. A ban or leaving the
/// Discord signs the player out here.
#[tauri::command]
async fn auth_status(app: AppHandle, state: State<'_, AppState>) -> CmdResult<AuthStatus> {
    let Some(token) = token(&app) else {
        return Ok(AuthStatus::default());
    };
    let answer = auth::me(&state.http, AUTH_URL, &token).await;
    log::line(&format!("sign-in check: {}", answer_kind(&answer)));
    match answer {
        auth::Answer::Ok(profile) => {
            signed_in(&app, &state, profile.clone()).await?;
            Ok(AuthStatus { signed_in: true, account: Some(profile), ..Default::default() })
        }
        auth::Answer::SignedOut(msg) => {
            sign_out(&app, &state).await;
            Ok(AuthStatus { message: Some(msg), ..Default::default() })
        }
        auth::Answer::Refused { message, .. } => {
            sign_out(&app, &state).await;
            Ok(AuthStatus { message: Some(message), ..Default::default() })
        }
        auth::Answer::Offline(_) | auth::Answer::Pending => {
            let config = state.config.lock().await;
            Ok(AuthStatus {
                signed_in: true,
                account: config.account.clone(),
                offline: true,
                // Every Play requests a fresh server session. A remembered
                // login cannot make that request while the service is down.
                locked: true,
                message: Some("Couldn't reach the login service, so a new game session cannot start. Your saved sign-in is kept; try again when it responds.".into()),
            })
        }
    }
}

/// Opens the Discord sign-in in the browser. Returns the state to poll with.
#[tauri::command]
async fn auth_begin(app: AppHandle) -> CmdResult<String> {
    use tauri_plugin_opener::OpenerExt;
    let st = auth::new_state();
    let verifier = auth::new_verifier();
    let challenge = auth::challenge(&verifier);
    let (listener, port) = auth::listen().await.map_err(err)?;
    // One sign-in at a time; the verifier stays in this process.
    {
        let mut all = logins().lock().unwrap();
        all.clear();
        all.insert(st.clone(), None);
    }
    let (st2, http) = (st.clone(), app.state::<AppState>().http.clone());
    let task = tauri::async_runtime::spawn(async move {
        let answer = match tokio::time::timeout(LOGIN_WAIT, auth::wait_for_code(listener, &st2)).await {
            Ok(Some(ret)) => {
                let answer = auth::exchange(&http, AUTH_URL, &st2, &ret.code, &verifier).await;
                // The browser tab says whose sign-in it was, or why it failed.
                let line = match &answer {
                    auth::Answer::Ok(done) => format!("Signed in to Aetherial Dawn as {}", done.profile.discord_username.as_deref().unwrap_or("your Discord account")),
                    auth::Answer::Refused { message, .. } => message.clone(),
                    _ => "That sign-in didn't go through. Try again from the launcher.".into(),
                };
                ret.finish(&line).await;
                answer
            }
            _ => auth::Answer::SignedOut("Sign-in timed out. Try again.".into()),
        };
        if let Some(slot) = logins().lock().unwrap().get_mut(&st2) {
            *slot = Some(answer);
        }
    });
    // A new press gets a new state and verifier; the earlier sign-in stops
    // listening at once.
    if let Some(old) = login_task().lock().unwrap().replace(task) {
        old.abort();
    }
    log::line(&format!("sign-in: opened the browser, waiting on 127.0.0.1:{port}"));
    app.opener().open_url(auth::login_url(AUTH_URL, &st, &challenge, port), None::<&str>).map_err(|e| e.to_string())?;
    Ok(st)
}

const LOGIN_WAIT: std::time::Duration = std::time::Duration::from_secs(5 * 60);

fn login_task() -> &'static std::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>> {
    static T: std::sync::OnceLock<std::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>>> = std::sync::OnceLock::new();
    T.get_or_init(Default::default)
}

/// The sign-in in progress, by state, and its answer once the browser came
/// back and the code was traded (kept only in memory).
type Logins = std::collections::HashMap<String, Option<auth::Answer<auth::SignedIn>>>;
fn logins() -> &'static std::sync::Mutex<Logins> {
    static V: std::sync::OnceLock<std::sync::Mutex<Logins>> = std::sync::OnceLock::new();
    V.get_or_init(Default::default)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PollResult {
    /// pending | done | refused | expired | offline | save_failed
    status: &'static str,
    message: Option<String>,
    account: Option<auth::Profile>,
    /// For save_failed: no_place | denied | full | io | protect | other.
    kind: Option<&'static str>,
}

#[tauri::command]
async fn auth_poll(app: AppHandle, state: State<'_, AppState>, st: String) -> CmdResult<PollResult> {
    let r = |status, message: Option<String>| PollResult { status, message, account: None, kind: None };
    let answer = {
        let mut all = logins().lock().unwrap();
        match all.get_mut(&st) {
            None => return Ok(r("expired", Some("The sign-in link expired. Try again.".into()))),
            Some(slot) => slot.take(),
        }
    };
    let answer = match answer {
        // Removed only once it is used, so a failed save can be tried again.
        Some(auth::Answer::Ok(done)) => {
            let saved = match token_path(&app) {
                None => Err(("no_place", "there's no folder to save it in".to_string())),
                Some(path) => auth::save_token(&path, &done.token).map_err(|e| (auth::save_failure_kind(&e), err(e))),
            };
            if let Err((kind, e)) = saved {
                log::line(&format!("sign-in: couldn't save it ({kind}), will try again: {e}"));
                if let Some(slot) = logins().lock().unwrap().get_mut(&st) {
                    *slot = Some(auth::Answer::Ok(done));
                }
                return Ok(PollResult { status: "save_failed", message: Some(format!("Couldn't save your sign-in: {e}")), account: None, kind: Some(kind) });
            }
            logins().lock().unwrap().remove(&st);
            auth::Answer::Ok(done)
        }
        Some(a) => {
            logins().lock().unwrap().remove(&st);
            a
        }
        None => auth::Answer::Pending,
    };
    if !matches!(answer, auth::Answer::Pending) {
        log::line(&format!("sign-in: {}", answer_kind(&answer)));
    }
    Ok(match answer {
        auth::Answer::Pending => r("pending", None),
        auth::Answer::Ok(done) => {
            signed_in(&app, &state, done.profile.clone()).await?;
            PollResult { status: "done", message: None, account: Some(done.profile), kind: None }
        }
        auth::Answer::Refused { message, .. } => r("refused", Some(message)),
        auth::Answer::SignedOut(message) => r("expired", Some(message)),
        // The code is traded once, so a failed trade means starting again.
        auth::Answer::Offline(message) => r("expired", Some(format!("Couldn't finish signing in: {message}. Try again."))),
    })
}

/// Asks Steam to repair Skyrim's own files (Verify integrity), which puts
/// back the files the server's patches start from. Never a downgrade through
/// Steam. Returns false when the game isn't a Steam copy.
#[tauri::command]
async fn repair_game_files(app: AppHandle, state: State<'_, AppState>) -> CmdResult<bool> {
    use tauri_plugin_opener::OpenerExt;
    let dir = game_dir(&state).await?;
    if !dir.to_string_lossy().to_ascii_lowercase().contains("steamapps") {
        return Ok(false);
    }
    log::line("patch: asking Steam to verify Skyrim's files (steam://validate/489830)");
    app.opener().open_url("steam://validate/489830", None::<&str>).map_err(|e| e.to_string())?;
    Ok(true)
}

/// A command's error in plain words for the player (the UI's invoke
/// wrapper asks for every failure); the raw text is already in the log.
#[tauri::command]
fn plain_error(text: String) -> String {
    launcher_core::plain_ui(&text)
}

#[tauri::command]
async fn auth_sign_out(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    sign_out(&app, &state).await;
    Ok(())
}


#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PatchProgress {
    stage: &'static str,
    file: String,
    done: usize,
    total: usize,
}

/// Puts the server's Skyrim build in place with patches from the server,
/// on the player's own files: no Steam and no sign-in. Sends `patch-progress`
/// events. Errors starting with "NO_PATCH:" mean the player's copy is a
/// build no patch was made from yet.
/// No patch fits this copy. When SkyrimSE.exe isn't the server's build, the
/// answer is NO_PATCH_FILES, so the UI has Steam repair the game to Steam's
/// current build (which MulderLoad's patches start from) and tries again.
fn not_patchable(dir: &std::path::Path, spec: &launcher_core::manifest::GameSpec) -> String {
    let have = version::exe_version(&dir.join(game::GAME_EXE));
    let want = spec.version.as_deref().and_then(version::parse_version);
    match have {
        Some(h) if Some(h) == want => "NO_PATCH:Your Skyrim is already the server's version.".into(),
        Some(h) => format!("NO_PATCH_FILES:Your Skyrim is version {}, which the launcher can't change directly.", version::short(h)),
        None => "NO_PATCH_FILES:The launcher couldn't read your Skyrim's version.".into(),
    }
}

/// Whether Skyrim or a game-file operation is active. The self-update waits
/// while either one is happening, including the gap before SkyrimSE starts.
#[tauri::command]
fn game_running(state: State<'_, AppState>) -> CmdResult<bool> {
    let held = state.game_operation.try_lock().is_err()
        || state.active_session.load(Ordering::SeqCst)
        || state.update_installing.load(Ordering::SeqCst);
    if held { return Ok(true) }
    external_game_running()
}

/// Reserve the final install/relaunch so Play cannot start in the interval
/// between the UI's process check and the updater's install call.
#[tauri::command]
fn self_update_begin(state: State<'_, AppState>) -> CmdResult<bool> {
    let Ok(_operation) = state.game_operation.try_lock() else { return Ok(false) };
    if state.active_session.load(Ordering::SeqCst) {
        return Ok(false);
    }
    if external_game_running()? { return Ok(false) }
    Ok(state.update_installing.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_ok())
}

/// A launcher update needs Skyrim closed. Ends the game and Skyrim Platform's
/// helpers (the UI sends this only after its on-screen countdown). Returns
/// whether no Skyrim process is left; the session watcher then clears itself.
#[tauri::command]
async fn close_game_for_update() -> CmdResult<bool> {
    log::line("update: ending Skyrim so the launcher update can install");
    tokio::task::spawn_blocking(|| {
        for name in [watch::GAME_PROCESS, game::SKSE_LOADER] {
            watch::end_all(name);
        }
        for helper in watch::BROWSER_HELPERS {
            watch::end_all(helper);
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(!external_game_running()?)
}

/// Brings the launcher window in front of the game (top = true), so the
/// countdown before a forced close is seen, and puts it back (top = false).
#[tauri::command]
fn raise_launcher(app: AppHandle, top: bool) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.set_always_on_top(top);
        if top {
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
        }
    }
}

#[tauri::command]
fn self_update_end(state: State<'_, AppState>) {
    state.update_installing.store(false, Ordering::SeqCst);
}

#[tauri::command]
async fn patch_game(app: AppHandle, state: State<'_, AppState>) -> CmdResult<version::GameCheck> {
    use launcher_core::{community, patcher};
    let _operation = begin_game_operation(&state)?;
    let dir = game_dir(&state).await?;
    let spec = game_spec(&state).await?;
    // A version fix cut short (launcher closed, power lost) is put back first,
    // or the half-changed game reads as neither build.
    if community::needs_recovery(&dir) {
        if watch::find_process(watch::GAME_PROCESS).is_some() {
            return Err("Close Skyrim first.".into());
        }
        log::line("patch: an earlier version fix was cut short, putting the game's files back");
        community::recover(&dir).map_err(|e| {
            log::line(&format!("patch: putting back failed: {e}"));
            format!("Couldn't change Skyrim to the server's version ({e}). Close Skyrim and Steam, then press Fix version again.")
        })?;
    }
    // Steam's current build: MulderLoad's public patches, no Steam sign-in.
    if spec.version.as_deref() == Some(community::TARGET) && community::supported(&dir) {
        if watch::find_process(watch::GAME_PROCESS).is_some() {
            return Err("Close Skyrim first.".into());
        }
        let (lang, _) = community::language(&dir);
        log::line(&format!("patch: Steam {} ({lang}) found, using the MulderLoad patches", community::FROM_VERSION));
        let app2 = app.clone();
        let mut report = move |stage: &str, file: &str, done: u64, total: u64| {
            let stage = match stage {
                "download" => "fetch",
                "unpack" => "unpack",
                "patch" => "apply",
                _ => "swap",
            };
            let _ = app2.emit("patch-progress", PatchProgress { stage, file: file.to_string(), done: done as usize, total: total as usize });
        };
        let changed = community::downgrade(&state.http, &dir, &mut report).await.map_err(|e| {
            log::line(&format!("patch: MulderLoad patches failed: {e}"));
            // A missing game file keeps its prefix, so the UI has Steam repair it.
            let e = e.to_string();
            match e.strip_prefix(community::MISSING_FILE) {
                Some(rest) => format!("{}{rest}", community::MISSING_FILE),
                None => format!("Couldn't change Skyrim to the server's version ({e}). Close Skyrim and Steam, then press Fix version again."),
            }
        })?;
        log::line(&format!("patch: {} file(s) changed: {}", changed.len(), changed.join(", ")));
        let _ = app.emit("patch-progress", PatchProgress { stage: "verify", file: String::new(), done: 1, total: 1 });
        let gc = finish_downgrade(&dir, &spec, false)?;
        skse_after_patch(&state.http, &dir).await;
        return Ok(gc);
    }
    let base = state.config.lock().await.base_url.clone();
    let url = format!("{}/{}", base.trim_end_matches('/'), patcher::INDEX);
    // Patches made on this PC (staff, see build_patches) are used directly.
    let local_dir = patch_build_dir(&dir).join("out");
    let local: Option<patcher::Index> = std::fs::read(local_dir.join("index.json")).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let index: patcher::Index = if let Some(ix) = local.filter(|ix| Some(ix.target.as_str()) == spec.version.as_deref()) {
        log::line(&format!("patch: using patches made on this PC in {}", local_dir.display()));
        ix
    } else {
        state
        .http
        .get(&url)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| {
            log::line(&format!("patch: patch list not available: {e}"));
            not_patchable(&dir, &spec)
        })?
        .json()
        .await
        .map_err(|e| {
            log::line(&format!("patch: patch list unreadable: {e}"));
            not_patchable(&dir, &spec)
        })?
    };
    if Some(index.target.as_str()) != spec.version.as_deref() {
        log::line(&format!("patch: the server's patches make {}, but it needs {}", index.target, spec.version.as_deref().unwrap_or("?")));
        return Err(not_patchable(&dir, &spec));
    }
    if watch::find_process(watch::GAME_PROCESS).is_some() {
        return Err("Close Skyrim first.".into());
    }
    log::line(&format!("patch: checking {} game files against {url}", index.files.len()));
    let _ = app.emit("patch-progress", PatchProgress { stage: "check", file: String::new(), done: 0, total: index.files.len() });
    let (dir2, ix2, app2) = (dir.clone(), index.clone(), app.clone());
    let steps = tokio::task::spawn_blocking(move || {
        let mut n = 0;
        let total = ix2.files.len();
        patcher::plan(&dir2, &ix2, |p| {
            n += 1;
            let _ = app2.emit("patch-progress", PatchProgress { stage: "check", file: p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(), done: n.min(total), total });
            patcher::sha256_file(p).ok()
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    let missing: Vec<&str> = steps.iter().filter(|s| s.patch.is_none()).map(|s| s.file.path.as_str()).collect();
    if !missing.is_empty() {
        log::line(&format!("patch: no patch for this copy of: {}", missing.join(", ")));
        return Err(format!("NO_PATCH_FILES:Your copy of {} is a Skyrim build the server has no patch for yet.", missing.join(", ")));
    }
    log::line(&format!("patch: {} file(s) to patch: {}", steps.len(), steps.iter().map(|s| s.file.path.as_str()).collect::<Vec<_>>().join(", ")));
    let tmp = app.path().app_local_data_dir().map_err(|e| e.to_string())?.join("patches");
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let total = steps.len();
    // Every patched file is made first; the game only changes in the one
    // journalled swap after the loop, so a failure here leaves it as it was.
    let stage = patcher::stage_dir(&dir);
    let _ = std::fs::remove_dir_all(&stage);
    let made = server_patch_files(&app, &state, &dir, &base, &local_dir, &tmp, &stage, &steps).await;
    let swaps = match made {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&stage);
            return Err(e);
        }
    };
    let _ = app.emit("patch-progress", PatchProgress { stage: "swap", file: String::new(), done: 0, total });
    let (dir2, app2) = (dir.clone(), app.clone());
    let swapped = tokio::task::spawn_blocking(move || {
        let mut report = move |_: &str, file: &str, done: u64, total: u64| {
            let _ = app2.emit("patch-progress", PatchProgress { stage: "swap", file: file.to_string(), done: done as usize, total: total as usize });
        };
        community::swap_in(&dir2, &swaps, &mut report)
    })
    .await
    .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(&stage);
    swapped.map_err(|e| format!("Couldn't change Skyrim to the server's version ({e}). Close Skyrim and Steam, then press Fix version again."))?;
    log::line(&format!("patch: swapped in {total} patched file(s)"));
    let _ = app.emit("patch-progress", PatchProgress { stage: "verify", file: String::new(), done: total, total });
    let gc = finish_downgrade(&dir, &spec, false)?;
    skse_after_patch(&state.http, &dir).await;
    Ok(gc)
}

/// Downloads each needed server patch and makes its patched file in `stage`.
/// Nothing in the game changes here.
#[allow(clippy::too_many_arguments)]
async fn server_patch_files(
    app: &AppHandle,
    state: &AppState,
    dir: &std::path::Path,
    base: &str,
    local_dir: &std::path::Path,
    tmp: &std::path::Path,
    stage: &std::path::Path,
    steps: &[launcher_core::patcher::Step],
) -> CmdResult<Vec<launcher_core::community::Swap>> {
    use launcher_core::patcher;
    let total = steps.len();
    let mut swaps = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        let patch = step.patch.as_ref().expect("checked");
        // plan() only picks plainly named patches; checked again before any
        // path is built from the name.
        if !patcher::plain_file_name(&patch.file) {
            return Err(format!("The server's patch list names a patch file the launcher won't use ({}). Staff need to fix it on the server; try again later.", patch.file));
        }
        let _ = app.emit("patch-progress", PatchProgress { stage: "download", file: step.file.path.clone(), done: i, total });
        let purl = format!("{}/patches/{}", base.trim_end_matches('/'), patch.file);
        let made_here = local_dir.join(&patch.file);
        let local = if made_here.is_file() { made_here } else { tmp.join(&patch.file) };
        let have = local.is_file() && patcher::sha256_file(&local).ok().is_some_and(|h| h.eq_ignore_ascii_case(&patch.sha256));
        if !have {
            let bytes = state.http.get(&purl).send().await.and_then(|r| r.error_for_status()).map_err(|e| format!("Couldn't download the patch for {} ({e}).", step.file.path))?.bytes().await.map_err(|e| e.to_string())?;
            let got = patcher::sha256_bytes(&bytes);
            if !got.eq_ignore_ascii_case(&patch.sha256) {
                return Err(format!("The patch for {} was damaged while downloading. Try again.", step.file.path));
            }
            std::fs::write(&local, &bytes).map_err(|e| e.to_string())?;
        }
        let _ = app.emit("patch-progress", PatchProgress { stage: "apply", file: step.file.path.clone(), done: i, total });
        let (dir2, stage2, step2, local2) = (dir.to_path_buf(), stage.to_path_buf(), step.clone(), local.clone());
        let swap = tokio::task::spawn_blocking(move || patcher::make_patched(&dir2, &stage2, &step2, &local2))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("Couldn't patch {} ({e}). Nothing in the game was changed. Close Skyrim and Steam, then try again.", step.file.path))?;
        if local.starts_with(tmp) {
            let _ = std::fs::remove_file(&local);
        }
        log::line(&format!("patch: made {}", step.file.path));
        swaps.push(swap);
    }
    Ok(swaps)
}

/// Installs SKSE once the game is on the server's version. The version fix
/// itself has worked by then, so a failure here doesn't report it as failed
/// (running it again would only say the game is already on that version);
/// Play installs it again and stops with the reason if it still can't be
/// installed.
async fn skse_after_patch(http: &reqwest::Client, dir: &std::path::Path) {
    if let Err(e) = install_skse(http, dir).await {
        log::line(&format!("patch: the game is on the server's version, but SKSE didn't install yet ({e}); Play tries again"));
    }
}

fn patch_build_dir(game_dir: &std::path::Path) -> PathBuf {
    game_dir.join(".aetherial-dawn").join("patch-build")
}


async fn game_spec(state: &AppState) -> CmdResult<launcher_core::manifest::GameSpec> {
    let m = manifest_now(state).await?;
    m.game.ok_or_else(|| "The server doesn't ask for a particular Skyrim version.".into())
}

/// Before every Play: the light plugins the server runs as full ones get
/// their "<stem>.esm" copy, plugins the SkyMP client can't load are switched
/// off in plugins.txt, plugins made for a newer Skyrim (Creation Club
/// downloads Steam updated after a downgrade) and broken stub plugins are put
/// aside, and the Skyrim Platform cache, graphics preference and Skyrim.ini
/// archive list are tidied. Nothing is deleted.
fn tidy_game(app: &AppHandle, dir: &std::path::Path, m: &Manifest, server_names: &[String], tunes: &[gameini::Tune]) -> CmdResult<()> {
    // Light plugins the server's list runs as full plugins get a "<stem>.esm"
    // copy with the light flag cleared, the same bytes as the server's.
    let full = launcher_core::aliases::light_as_full(dir, server_names);
    if !full.is_empty() {
        log::line(&format!("play: light plugins the server runs as full plugins: {}", full.join(", ")));
    }
    match launcher_core::aliases::ensure_full(dir, plugins_txt(app).as_deref(), &full) {
        Ok(v) if !v.is_empty() => log::line(&format!("play: plugins loading under the name the server uses: {}", v.iter().map(|(a, b)| format!("{a} as {b}")).collect::<Vec<_>>().join(", "))),
        Ok(_) => {}
        // A light plugin left light would shift every record after it
        // against the server's, so that one stops Play.
        Err(e) if !full.is_empty() => return Err(format!("Couldn't prepare the server's plugins ({e}). Close Skyrim, then try again.")),
        Err(e) => log::line(&format!("play: couldn't give plugins the name the server uses: {e}")),
    }
    if let Some(txt) = plugins_txt(app) {
        let extras = loadorder::extras(dir, &txt, m);
        if !extras.is_empty() {
            let names: Vec<String> = extras.iter().map(|e| e.name.clone()).collect();
            loadorder::switch_off(&txt, &names).map_err(|e| format!("Couldn't change your load order ({e}). Close Skyrim, then try again."))?;
            log::line(&format!("play: switched off in {}: {}", txt.display(), extras.iter().map(|e| e.describe()).collect::<Vec<_>>().join(", ")));
        }
    }
    // Plugins made for a newer Skyrim (Creation Club downloads Steam updated
    // after the downgrade) and broken stub plugins crash the game while it
    // loads data, even switched off. The game skips Creation Club files that
    // aren't in Data, so they go to the backup folder with their archives.
    let mut moved: Vec<String> = Vec::new();
    for (n, why) in loadorder::too_new(dir) {
        log::line(&format!("play: {n} is for a newer Skyrim ({why})"));
        moved.extend(loadorder::with_archives(dir, &n));
    }
    if let Ok(rd) = std::fs::read_dir(dir.join("Data")) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            let l = n.to_ascii_lowercase();
            if (l.ends_with(".esp") || l.ends_with(".esm") || l.ends_with(".esl")) && loadorder::broken(&e.path()).is_some() {
                moved.push(format!("Data/{n}"));
            }
        }
    }
    if !moved.is_empty() {
        let stamp = format!("{}-plugins", log::timestamp().replace([':', ' '], "-"));
        let dest = aside::move_aside(dir, &moved, &stamp).map_err(|e| format!("Couldn't move plugins out of the way ({e}). Close Skyrim, then try again."))?;
        log::line(&format!("play: moved {} plugin file(s) to {}: {}", moved.len(), dest.display(), moved.join(", ")));
        if let Some(spec) = m.game.as_ref() {
            match version::refresh(dir, spec) {
                Ok(true) => {
                    if version::made_by_launcher(dir) {
                        keep_copy(dir, spec);
                    }
                }
                Ok(false) => {}
                Err(e) => log::line(&format!("play: couldn't update the version record: {e}")),
            }
        }
    }
    // loadorder.txt starts with the five base masters.
    if let Some(txt) = plugins_txt(app) {
        match loadorder::fix_order(&txt.with_file_name("loadorder.txt")) {
            Ok(true) => log::line("play: put the five base masters first in loadorder.txt"),
            Ok(false) => {}
            Err(e) => log::line(&format!("play: couldn't fix loadorder.txt: {e}")),
        }
    }
    clear_browser_cache();
    match game::ensure_platform_folders(dir) {
        Ok(made) if !made.is_empty() => log::line(&format!("play: made missing Skyrim Platform plugin folder(s): {}", made.join(", "))),
        Ok(_) => {}
        Err(e) => log::line(&format!("play: couldn't make Skyrim Platform's plugin folders: {e}")),
    }
    match game::prefer_fast_gpu(dir) {
        Ok(true) => log::line(&format!("play: set Windows to run {} on the high-performance graphics card", game::gpu_pref_path(dir))),
        Ok(false) => {}
        Err(e) => log::line(&format!("play: couldn't set the graphics card preference: {e}")),
    }
    let docs = app.path().document_dir().or_else(|_| app.path().home_dir().map(|h| h.join("Documents"))).unwrap_or_default();
    for ini in gameini::ini_paths(&docs) {
        match gameini::repair(&ini, &dir.join("Data")) {
            Ok(r) if !r.is_empty() => log::line(&format!(
                "play: cleaned {}: dropped {} missing archive(s) [{}] and {} repeat(s) [{}]",
                ini.display(),
                r.missing.len(),
                r.missing.join(", "),
                r.duplicates.len(),
                r.duplicates.join(", ")
            )),
            Ok(r) if r.skipped => log::line(&format!("play: left {} alone: the Data folder could not be read or holds no .bsa archive", ini.display())),
            Ok(_) => {}
            Err(e) => log::line(&format!("play: couldn't clean {}: {e}", ini.display())),
        }
    }
    // The game keeps running when its window loses focus (Timothy: never pause).
    for ini in gameini::skyrim_ini_paths(&docs) {
        match gameini::ensure_always_active(&ini) {
            Ok(true) => log::line(&format!("play: set bAlwaysActive=1 in {}", ini.display())),
            Ok(false) => {}
            Err(e) => log::line(&format!("play: couldn't set bAlwaysActive in {}: {e}", ini.display())),
        }
    }
    // Staff-set ini limits from launcher-rules.json (gameini::TUNABLE keys only; each change is logged).
    if !tunes.is_empty() {
        // The frame cap lives in the game folder; set it first so fMaxTime below follows it.
        let tweaks = dir.join("Data/SKSE/Plugins/SSEDisplayTweaks.ini");
        match gameini::apply_tunes(&tweaks, tunes) {
            Ok(ch) if !ch.is_empty() => log::line(&format!("play: tuned {}: {}", tweaks.display(), ch.join("; "))),
            Ok(_) => {}
            Err(e) => log::line(&format!("play: couldn't tune {}: {e}", tweaks.display())),
        }
        let cap = std::fs::read_to_string(&tweaks).ok().and_then(|t| gameini::framerate_limit(&t));
        let tunes = gameini::resolve_framerate(tunes, cap);
        for ini in gameini::ini_paths(&docs) {
            match gameini::apply_tunes(&ini, &tunes) {
                Ok(ch) if !ch.is_empty() => log::line(&format!("play: tuned {}: {}", ini.display(), ch.join("; "))),
                Ok(_) => {}
                Err(e) => log::line(&format!("play: couldn't tune {}: {e}", ini.display())),
            }
        }
    }
    Ok(())
}

/// Installs SKSE 2.2.6 from its official GitHub release when it is missing.
/// SKSE is needed to join at all, so one that can't be installed stops Play
/// with the reason. It is the only thing the launcher installs.
/// The server's own plugins beyond the five base masters (masters.json) come from the feed at <base>/<name>,
/// checked against the listed size and sha256, and replace a local copy that differs.
async fn fetch_server_plugins(http: &reqwest::Client, base: &str, dir: &std::path::Path, order: &[serverorder::ServerPlugin]) -> Result<(), String> {
    for p in order.iter().skip(serverorder::BASE.len()) {
        let (Some(size), Some(sha)) = (p.size, p.sha256.as_deref()) else { continue };
        if !serverorder::plain_plugin_name(&p.name) || p.name.to_ascii_lowercase().starts_with("cc") {
            continue;
        }
        let path = dir.join("Data").join(&p.name);
        if std::fs::metadata(&path).is_ok_and(|m| m.len() == size) && sync::sha256_file(&path).await.is_ok_and(|h| h.eq_ignore_ascii_case(sha)) {
            continue;
        }
        let url = format!("{}/{}", base.trim_end_matches('/'), p.name);
        let fail = |why: String| if why.contains("404") {
            format!("The server is still being updated: its plugin {} isn't available yet. Wait a minute, then press Play again.", p.name)
        } else {
            format!("Couldn't get the server's plugin {} ({why}). Check your internet connection, then press Play again.", p.name)
        };
        let bytes = http.get(&url).timeout(std::time::Duration::from_secs(60)).send().await.and_then(|r| r.error_for_status()).map_err(|e| fail(e.to_string()))?
            .bytes().await.map_err(|e| fail(e.to_string()))?;
        let got = launcher_core::sync::sha256_bytes(&bytes);
        if bytes.len() as u64 != size || !got.eq_ignore_ascii_case(sha) {
            return Err(fail("it didn't match what the server lists".into()));
        }
        loadorder::atomic_write(&path, &bytes).map_err(|e| fail(e.to_string()))?;
        log::line(&format!("play: downloaded the server's plugin {} ({size} bytes, sha256 checked)", p.name));
    }
    Ok(())
}

async fn install_skse(http: &reqwest::Client, dir: &std::path::Path) -> CmdResult<()> {
    let mut line = |l: &str| log::line(l);
    requirements::ensure_skse(http, dir, &requirements::Sources::official(), &mut line).await.map_err(|f| {
        log::line(&format!("couldn't install {}: {}", f.name, f.cause));
        f.message()
    })
}

/// What Play is doing now, for the status line under the Play button, so a
/// long step (a first SKSE download, the pre-Play health check) never looks
/// like the launcher hung.
fn play_step(app: &AppHandle, text: &str) {
    trace_enter(text);
    let _ = app.emit("play-step", text);
}

/// The Play attempt in progress, for the install report (clientstatus.rs).
fn play_trace() -> &'static std::sync::Mutex<launcher_core::clientstatus::PlayTrace> {
    static T: std::sync::OnceLock<std::sync::Mutex<launcher_core::clientstatus::PlayTrace>> = std::sync::OnceLock::new();
    T.get_or_init(|| std::sync::Mutex::new(launcher_core::clientstatus::PlayTrace::new()))
}

fn with_trace(f: impl FnOnce(&mut launcher_core::clientstatus::PlayTrace)) {
    f(&mut play_trace().lock().unwrap_or_else(|e| e.into_inner()));
}

fn trace_start() {
    with_trace(|t| *t = launcher_core::clientstatus::PlayTrace::new());
}

fn trace_enter(text: &str) {
    with_trace(|t| t.enter(text.trim_end_matches(['…', '.'])));
}

fn trace_game_version(ok: bool) {
    with_trace(|t| t.game_version(ok));
}

fn trace_fail() {
    with_trace(|t| t.fail());
}

/// Tells the login service, in the background, what SkyMP needs that this PC
/// has (names only; see clientstatus.rs). Nothing is awaited here, so Play
/// never waits. A 404 (route not live yet) is silent.
fn send_client_status(app: &AppHandle, dir: &std::path::Path) {
    let Some(token) = token(app) else { return };
    let (app, dir) = (app.clone(), dir.to_path_buf());
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let version = state.manifest.lock().await.as_ref().and_then(|m| m.game.as_ref()).and_then(|g| g.version.clone());
        let launcher = app.package_info().version.to_string();
        // Where the attempt got to goes only to players who share health reports.
        let play = state.config.lock().await.share_health.then(|| play_trace().lock().unwrap_or_else(|e| e.into_inner()).snapshot());
        let report = tauri::async_runtime::spawn_blocking(move || {
            let required = launcher_core::clientstatus::required(&dir, version.as_deref());
            launcher_core::clientstatus::report(&launcher, &required)
        })
        .await;
        let Ok(report) = report else { return };
        let report = match play { Some(trace) => report.with_play(trace), None => report };
        let (http, url) = (state.http.clone(), format!("{}/api/client-status", AUTH_URL.trim_end_matches('/')));
        match http.post(&url).header("authorization", token).json(&report).timeout(std::time::Duration::from_secs(10)).send().await {
            Ok(r) if r.status().is_success() || r.status().as_u16() == 404 => {}
            Ok(r) => log::line(&format!("play: the install report got {}", r.status().as_u16())),
            Err(e) => log::line(&format!("play: couldn't send the install report: {}", launcher_core::scrub(&e.to_string()))),
        }
    });
}

/// Tells the login service, in the background, what this PC will start the game with: game version,
/// the plugin order, Creation Club files and archive names (core/src/parity.rs; names and hashes only).
/// Nothing is awaited, so Play never waits; 404 (route not live) and any failure are silent for the player.
fn send_client_parity(app: &AppHandle, dir: &std::path::Path, order: &[launcher_core::serverorder::ServerPlugin], running: bool, after_secs: u64) {
    let (Some(token), Some(txt)) = (token(app), plugins_txt(app)) else { return };
    let (app, dir, order) = (app.clone(), dir.to_path_buf(), order.to_vec());
    tauri::async_runtime::spawn(async move {
        if after_secs > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(after_secs)).await;
        }
        let state = app.state::<AppState>();
        // A reading after the game closed would see the order the launcher put back, not the one the game ran with.
        if running && !state.active_session.load(Ordering::SeqCst) {
            return;
        }
        let launcher = app.package_info().version.to_string();
        let fp = tauri::async_runtime::spawn_blocking(move || {
            let fp = launcher_core::parity::build(&launcher, &dir, &txt, Some(&order));
            if running { fp.running() } else { fp }
        })
        .await;
        let Ok(fp) = fp else { return };
        log::line(&format!(
            "play: {} fingerprint: game {} ({}), {} Creation Club file(s) present, {} switched on, matches the server: {}{}",
            fp.phase, fp.game_version.as_deref().unwrap_or("unknown"), fp.edition_guess, fp.cc_present_count, fp.cc_active.len(),
            fp.server_match.map_or("no list".to_string(), |m| m.to_string()),
            fp.server_diff.first().map(|d| format!(" ({d})")).unwrap_or_default()
        ));
        let url = format!("{}/api/client-parity", AUTH_URL.trim_end_matches('/'));
        match state.http.post(&url).header("authorization", token).json(&fp).timeout(std::time::Duration::from_secs(10)).send().await {
            Ok(r) if r.status().is_success() || r.status().as_u16() == 404 => {}
            Ok(r) => log::line(&format!("play: the {} fingerprint got {}", fp.phase, r.status().as_u16())),
            Err(e) => log::line(&format!("play: couldn't send the {} fingerprint: {}", fp.phase, launcher_core::scrub(&e.to_string()))),
        }
    });
}

/// Before Play: what Skyrim Platform's in-game browser needs (core/src/browsercheck.rs).
/// Puts a browser backend that cannot work (off, nirnlab) back to auto, checks the
/// temp folder the browser keeps its profile in (a folder under the game's
/// .aetherial-dawn is used for the game's TEMP when it isn't writable), and lists the
/// browser's files. Switch: status.json `launcher.browserGuard`: false.
fn browser_guard(dir: &std::path::Path) -> (health::Check, Option<std::path::PathBuf>) {
    let fixed = match launcher_core::browsercheck::apply_ini_fix(dir) {
        Ok(Some((old, new))) => {
            log::line(&format!("play: SkyrimPlatform.ini had the in-game browser set to \"{old}\"; set it to \"{new}\" (the built-in browser) so the login box can show"));
            Some((old, new))
        }
        Ok(None) => None,
        Err(e) => {
            log::line(&format!("play: couldn't fix the browser setting in SkyrimPlatform.ini: {e}"));
            None
        }
    };
    let backend = std::fs::read_to_string(launcher_core::browsercheck::ini_path(dir)).ok().and_then(|t| launcher_core::browsercheck::backend(&t));
    let mut temp = launcher_core::browsercheck::temp_probe(&std::env::temp_dir().join("Skyrim Platform"));
    let mut fallback = None;
    if let Err(e) = &temp {
        log::line(&format!("play: the in-game browser's temp folder is not usable ({e})"));
        let alt = dir.join(".aetherial-dawn").join("tmp");
        match launcher_core::browsercheck::temp_probe(&alt.join("Skyrim Platform")) {
            Ok(()) => {
                log::line(&format!("play: the game will use {} for its temp files this session", alt.display()));
                fallback = Some(alt);
                temp = Ok(());
            }
            Err(e2) => log::line(&format!("play: no fallback temp folder either ({e2})")),
        }
    }
    let inv = launcher_core::browsercheck::inventory(dir);
    log::line(&format!(
        "play: browser check: backend {}, {} browser file(s): {}",
        backend.as_deref().unwrap_or("not set"),
        inv.len(),
        inv.iter().take(12).map(|(n, s)| format!("{n} ({s} B)")).collect::<Vec<_>>().join(", ")
    ));
    (launcher_core::browsercheck::check(&inv, backend.as_deref(), &temp, fixed.as_ref()), fallback)
}

/// Skyrim Platform's built-in browser (CEF) keeps its profile in
/// %TEMP%\Skyrim Platform and reuses it on every start. A profile left by a
/// crashed run or another Skyrim Platform build crashed libcef.dll 5 seconds
/// in, while loading its settings (first tester, 2026-09-26). It is only a
/// cache, so it's cleared before every Play and rebuilt by the game.
fn clear_browser_cache() {
    let dir = std::env::temp_dir().join("Skyrim Platform");
    if !dir.exists() {
        return;
    }
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => log::line(&format!("play: cleared Skyrim Platform's browser cache at {}", dir.display())),
        Err(e) => log::line(&format!("play: couldn't clear Skyrim Platform's browser cache at {} ({e}); it may be in use", dir.display())),
    }
}

/// Staff reports go to the login service's crash-report endpoint, signed
/// with the player's launcher token (live on the server since 2026-09-26).
/// Set to false to only show and log reports.
const HEALTH_REPORTS_ON: bool = true;
/// The endpoint takes at most 60 KB.
const HEALTH_REPORT_MAX: usize = 58_000;

fn health_report_url() -> Option<String> {
    HEALTH_REPORTS_ON.then(|| format!("{AUTH_URL}/api/crash-reports")) // contract-method: POST (sent by post_report)
}

/// What staff sent back: the report number and the likely cause.
#[derive(Clone, Default)]
struct Filed {
    report_id: Option<String>,
    likely_cause: Option<String>,
}

/// The server's masters.json (its plugins and their fingerprints).
async fn fetch_masters(http: &reqwest::Client, base: &str) -> Option<serde_json::Value> {
    let url = format!("{}/masters.json", base.trim_end_matches('/'));
    match http.get(&url).timeout(std::time::Duration::from_secs(8)).send().await.and_then(|r| r.error_for_status()) {
        Ok(r) => r.json::<serde_json::Value>().await.ok(),
        Err(e) => {
            log::line(&format!("health: couldn't load {url}: {e}"));
            None
        }
    }
}

async fn run_health(app: &AppHandle, http: &reqwest::Client, base: &str, dir: &std::path::Path, m: Option<&Manifest>) -> health::Report {
    let masters = fetch_masters(http, base).await;
    run_health_with_masters(app, dir, m, masters).await
}

async fn run_health_with_masters(app: &AppHandle, dir: &std::path::Path, m: Option<&Manifest>, masters: Option<serde_json::Value>) -> health::Report {
    let appdata = app.path().local_data_dir().ok().map(|d| d.join("Skyrim Special Edition"));
    let docs = app.path().document_dir().or_else(|_| app.path().home_dir().map(|h| h.join("Documents"))).ok();
    let cache = app.path().app_local_data_dir().ok().map(|d| health::cache_path(&d));
    let home = app.path().home_dir().ok();
    let (dir, m) = (dir.to_path_buf(), m.cloned());
    tauri::async_runtime::spawn_blocking(move || {
        health::run(&health::Inputs {
            game_dir: &dir,
            manifest: m.as_ref(),
            masters: masters.as_ref(),
            appdata: appdata.as_deref(),
            documents: docs.as_deref(),
            hash_cache: cache.as_deref(),
            home: home.as_deref(),
        })
    })
    .await
    .unwrap_or_else(|_| health::Report { checks: vec![], worst: health::Status::Info })
}

/// What staff receive. Same text the player sees; never the settings file,
/// tokens or sessions.
fn health_payload(app: &AppHandle, config: &Config, build: &str, when: &str, crash: Option<&str>, r: &health::Report) -> serde_json::Value {
    serde_json::json!({
        "kind": if crash.is_some() { "crash" } else { "health" },
        "when": when,
        "launcher": app.package_info().version.to_string(),
        "build": build,
        "discordId": config.account.as_ref().and_then(|a| a.discord_id.clone()),
        "discordUsername": config.account.as_ref().and_then(|a| a.discord_username.clone()),
        "crash": crash,
        "worst": r.worst,
        // The launcher's own guess, from the checks, for staff to prefer.
        "likelyCause": health::likely_cause(r),
        "checks": r.checks,
        "text": r.text(),
    })
}

/// Never names the client settings file or PluginsNoLoad (the endpoint
/// refuses those), and stays under the size limit.
fn fit_report(mut body: serde_json::Value) -> serde_json::Value {
    let bad = |t: &str| {
        let l = t.to_ascii_lowercase();
        l.contains("skymp5-client-settings") || l.contains("pluginsnoload")
    };
    if let Some(checks) = body["checks"].as_array_mut() {
        for c in checks {
            if c["detail"].as_str().is_some_and(bad) {
                c["detail"] = "(detail left out: it named a private file)".into();
            }
            if let Some(items) = c["items"].as_array_mut() {
                items.retain(|i| !i.as_str().is_some_and(bad));
                if items.len() > 40 {
                    let more = items.len() - 40;
                    items.truncate(40);
                    items.push(format!("…and {more} more").into());
                }
            }
        }
    }
    for key in ["text", "crashLog"] {
        if let Some(t) = body[key].as_str() {
            let lines: Vec<&str> = t.lines().filter(|l| !bad(l)).collect();
            body[key] = lines.join("\n").into();
        }
    }
    // The crash log gives way first, down to 12000 characters, so the
    // checks' text still fits beside it.
    let size = |b: &serde_json::Value| serde_json::to_vec(b).map(|v| v.len()).unwrap_or(0);
    let over = size(&body).saturating_sub(HEALTH_REPORT_MAX);
    if over > 0 {
        if let Some(cl) = body["crashLog"].as_str().map(str::to_string) {
            let keep = cl.chars().count().saturating_sub(over + 200).max(12_000);
            body["crashLog"] = watch::cut_chars(&cl, keep).into();
        }
    }
    let mut n = 0;
    while serde_json::to_vec(&body).map(|v| v.len()).unwrap_or(0) > HEALTH_REPORT_MAX && n < 20 {
        n += 1;
        let t = body["text"].as_str().unwrap_or("").to_string();
        let keep = t.len() / 2;
        let cut = (0..=keep).rev().find(|i| t.is_char_boundary(*i)).unwrap_or(0);
        body["text"] = format!("{}\n…(cut to fit)", &t[..cut]).into();
        if n > 6 {
            if let Some(checks) = body["checks"].as_array_mut() {
                for c in checks {
                    if let Some(items) = c["items"].as_array_mut() {
                        items.truncate(5);
                    }
                }
            }
        }
    }
    body
}

#[allow(clippy::too_many_arguments)]
async fn send_health(
    app: &AppHandle,
    http: &reqwest::Client,
    config: &Config,
    build: &str,
    when: &str,
    crash: Option<&str>,
    crash_log: Option<String>,
    extra: Option<String>,
    r: &health::Report,
) -> Filed {
    let Some(url) = health_report_url() else {
        log::line(&format!("health: report {when} not sent: the staff endpoint isn't set up yet"));
        return Filed::default();
    };
    let Some(token) = token(app) else {
        log::line(&format!("health: report {when} not sent: not signed in"));
        return Filed::default();
    };
    let mut body = health_payload(app, config, build, when, crash, r);
    body["consent"] = true.into();
    if let Some(extra) = extra {
        let text = format!("{}

{extra}", body["text"].as_str().unwrap_or(""));
        body["text"] = text.into();
    }
    if let Some(cl) = crash_log {
        // The newest crash logger log as is (optional crashLog, bot CONTRACT.md).
        body["crashLog"] = cl.into();
    }
    let body = fit_report(body);
    for attempt in 0..2 {
        match post_report(http, &url, &token, &body, when).await {
            Posted::Filed(mut f) => {
                // The checks know more than the staff service's guess (it
                // blamed plugins for a Menu Framework crash on 2026-09-26).
                if let Some(c) = health::likely_cause(r) {
                    f.likely_cause = Some(c);
                }
                return f;
            }
            Posted::RetryAfter(secs) if attempt == 0 && secs <= 90 => {
                log::line(&format!("health: sending report {when} again in {secs}s"));
                tokio::time::sleep(std::time::Duration::from_secs(secs + 1)).await;
            }
            _ => return Filed::default(),
        }
    }
    Filed::default()
}

enum Posted {
    Filed(Filed),
    RetryAfter(u64),
    Failed,
}

async fn post_report(http: &reqwest::Client, url: &str, token: &str, body: &serde_json::Value, when: &str) -> Posted {
    let res = http.post(url).header("authorization", token).json(body).timeout(std::time::Duration::from_secs(10)).send().await;
    let res = match res {
        Ok(r) => r,
        Err(e) => {
            log::line(&format!("health: report {when} not sent: {e}"));
            return Posted::Failed;
        }
    };
    let status = res.status();
    let v: serde_json::Value = res.json().await.unwrap_or_default();
    match status.as_u16() {
        200..=299 => {
            let f = Filed {
                report_id: v["reportId"].as_str().map(str::to_string),
                likely_cause: v["likelyCause"].as_str().filter(|s| !s.is_empty()).map(str::to_string),
            };
            log::line(&format!("health: report {when} sent to staff as {} (likely cause: {})", f.report_id.as_deref().unwrap_or("?"), f.likely_cause.as_deref().unwrap_or("none given")));
            Posted::Filed(f)
        }
        404 => {
            log::line(&format!("health: report {when} not sent: the staff endpoint isn't on the server yet"));
            Posted::Failed
        }
        429 => {
            log::line(&format!("health: report {when} not sent yet: too many reports, retry after {}s", v["retryAfter"]));
            Posted::RetryAfter(v["retryAfter"].as_u64().or_else(|| v["retryAfter"].as_f64().map(|f| f.ceil() as u64)).unwrap_or(20))
        }
        code => {
            log::line(&format!("health: report {when} not sent: server answered {code} {}", v["error"].as_str().or(v["message"].as_str()).unwrap_or("")));
            Posted::Failed
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthOut {
    report: health::Report,
    text: String,
    /// Exactly what would be sent to staff, as JSON.
    payload: String,
    endpoint: bool,
    share: bool,
}

/// Sends staff a health report for a problem the player just hit, when
/// sharing is on, so the player isn't asked to copy diagnostics (text audit
/// C2). Returns the report's number, or nothing when it wasn't sent.
#[tauri::command]
async fn report_problem(app: AppHandle, state: State<'_, AppState>, what: String) -> CmdResult<Option<String>> {
    let config = state.config.lock().await.clone();
    if !config.share_health {
        return Ok(None);
    }
    let Some(dir) = config.game_dir.clone() else { return Ok(None) };
    let m = state.manifest.lock().await.clone();
    let report = run_health(&app, &state.http, &config.base_url, &dir, m.as_ref()).await;
    let what: String = what.chars().filter(|c| !c.is_control()).take(200).collect();
    log::line(&format!("health: sending staff a report for: {what}"));
    let filed = send_health(&app, &state.http, &config, m.as_ref().map(|m| m.build.as_str()).unwrap_or(""), &what, None, None, None, &report).await;
    Ok(filed.report_id)
}

/// Runs the checks for the Settings screen.
/// The tail of Skyrim Platform's log, where ad-report.js writes where the player was.
fn platform_log_text(app: &AppHandle) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let docs = app.path().document_dir().or_else(|_| app.path().home_dir().map(|h| h.join("Documents"))).unwrap_or_default();
    let path = docs.join("My Games").join("Skyrim Special Edition").join("SKSE").join("skyrim-platform.log");
    let Ok(mut f) = std::fs::File::open(path) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if len > launcher_core::adreport::MAX_LOG_BYTES {
        let _ = f.seek(SeekFrom::Start(len - launcher_core::adreport::MAX_LOG_BYTES));
    }
    let mut bytes = Vec::new();
    let _ = f.read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn bug_sent_path() -> Option<PathBuf> {
    reports_dir().map(|d| d.join("bug-sent.txt"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BugState {
    /// A button press (F10 in game) that has not been sent yet.
    available: bool,
    /// What was recorded, for the player to read before sending.
    summary: String,
}

/// Whether the player pressed F10 in game since the last bug report was sent.
#[tauri::command]
async fn bug_state(app: AppHandle) -> CmdResult<BugState> {
    let log = tokio::task::spawn_blocking({
        let app = app.clone();
        move || platform_log_text(&app)
    })
    .await
    .map_err(|e| e.to_string())?;
    let at = launcher_core::adreport::newest_bug_at(&log);
    let sent = bug_sent_path().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let collected = launcher_core::adreport::collect(&log);
    Ok(BugState { available: at.as_deref().is_some_and(|a| a != sent.trim()), summary: launcher_core::adreport::describe(collected.last_bug.as_ref()) })
}

/// Sends a bug report to staff: the player's note and where they were. The
/// live bot takes it as a "health" report with a note (its report kinds are
/// crash and health), so it needs no server change.
#[tauri::command]
async fn send_bug(app: AppHandle, state: State<'_, AppState>, note: String) -> CmdResult<Option<String>> {
    let config = state.config.lock().await.clone();
    let build = state.manifest.lock().await.as_ref().map(|m| m.build.clone()).unwrap_or_default();
    let Some(url) = health_report_url() else { return Err("The staff report service isn't set up yet.".into()) };
    let Some(token) = token(&app) else { return Err("SIGNED_OUT:Sign in with Discord to send a report.".into()) };
    let log = tokio::task::spawn_blocking({
        let app = app.clone();
        move || platform_log_text(&app)
    })
    .await
    .map_err(|e| e.to_string())?;
    let note: String = note.chars().filter(|c| !c.is_control() || *c == '\n').take(1000).collect();
    let body = serde_json::json!({
        "kind": "health",
        "when": "player bug report",
        "launcher": app.package_info().version.to_string(),
        "build": build,
        "discordId": config.account.as_ref().and_then(|a| a.discord_id.clone()),
        "discordUsername": config.account.as_ref().and_then(|a| a.discord_username.clone()),
        "crash": null,
        "worst": "ok",
        "likelyCause": null,
        "checks": [],
        "note": note.trim(),
        "text": launcher_core::adreport::bug_text(&note, &log),
        "consent": true,
    });
    match post_report(&state.http, &url, &token, &fit_report(body), "bug report").await {
        Posted::Filed(f) => {
            if let (Some(p), Some(at)) = (bug_sent_path(), launcher_core::adreport::newest_bug_at(&log)) {
                let _ = std::fs::write(p, at);
            }
            Ok(f.report_id)
        }
        Posted::RetryAfter(s) => Err(format!("Staff are getting a lot of reports right now. Try again in {s} seconds.")),
        Posted::Failed => Err("Couldn't send the report. Check your internet connection and try again.".into()),
    }
}

#[tauri::command]
async fn health_check(app: AppHandle, state: State<'_, AppState>) -> CmdResult<HealthOut> {
    let config = state.config.lock().await.clone();
    let dir = config.game_dir.clone().ok_or("Pick your Skyrim folder first.")?;
    let m = state.manifest.lock().await.clone();
    let report = run_health(&app, &state.http, &config.base_url, &dir, m.as_ref()).await;
    log::line(&format!("health (settings): worst={:?}", report.worst));
    let payload = serde_json::to_string_pretty(&health_payload(&app, &config, m.as_ref().map(|m| m.build.as_str()).unwrap_or(""), "example", None, &report)).unwrap_or_default();
    Ok(HealthOut { text: report.text(), report, payload, endpoint: HEALTH_REPORTS_ON, share: config.share_health })
}

/// The player's load order file (the game uses it).
fn plugins_txt(app: &AppHandle) -> Option<PathBuf> {
    app.path().local_data_dir().ok().map(|d| d.join("Skyrim Special Edition").join("plugins.txt"))
}

/// For players who already put the right build in place themselves.
#[tauri::command]
async fn mark_game_ok(state: State<'_, AppState>) -> CmdResult<version::GameCheck> {
    let _operation = begin_game_operation(&state)?;
    let dir = game_dir(&state).await?;
    let m = manifest_now(&state).await?;
    let spec = m.game.clone().ok_or("The server doesn't ask for a particular Skyrim version.")?;
    let before = version::check(&dir, Some(&spec));
    log::line(&format!("player says Skyrim is already on the server's version (check said: {})", before.reason.as_deref().unwrap_or("ok")));
    finish_downgrade(&dir, &spec, true)
}

fn finish_downgrade(dir: &std::path::Path, spec: &launcher_core::manifest::GameSpec, manual: bool) -> CmdResult<version::GameCheck> {
    let want = spec.version.as_deref().and_then(version::parse_version);
    let have = version::exe_version(&dir.join(game::GAME_EXE));
    if want.is_some() && have != want {
        return Err(format!(
            "SkyrimSE.exe is still {}, not {}.",
            have.map(version::short).unwrap_or_else(|| "unreadable".into()),
            want.map(version::short).unwrap_or_default()
        ));
    }
    version::record(dir, spec, manual).map_err(err)?;
    // Every route (patch, MulderLoad, "already on this version"): a newer
    // game's ContentCatalog.txt crashes 1.6.1170 at the logo.
    log::line(&format!("version: {}", launcher_core::community::set_aside_content_catalog()));
    if !manual {
        match version::hold_updates(dir, spec.app) {
            Ok(p) => log::line(&format!("downgrade: set Steam to update Skyrim only when launched and made {} read-only", p.display())),
            Err(e) => log::line(&format!("downgrade: couldn't stop Steam updating Skyrim: {e}")),
        }
        keep_copy(dir, spec);
    }
    Ok(version::check(dir, Some(spec)))
}

fn keep_copy(dir: &std::path::Path, spec: &launcher_core::manifest::GameSpec) {
    match pristine::save(dir, spec) {
        Ok(n) => log::line(&format!("version: kept a linked copy of {n} Steam files in {} to put back if Steam updates the game", pristine::DIR)),
        Err(e) => log::line(&format!("version: couldn't keep a copy of the game files, so a Steam update will need a new download: {e}")),
    }
}

/// The version check, fixing what it can by itself: when Steam has updated
/// or repaired the game since the launcher downgraded it, the kept copy is
/// put back with no download and nothing for the player to do. A good build
/// with no kept copy yet (downgraded before 0.1.21) gets one now.
fn auto_version(dir: &std::path::Path, spec: Option<&launcher_core::manifest::GameSpec>) -> version::GameCheck {
    let gc = version::check(dir, spec);
    let Some(spec) = spec else { return gc };
    if !gc.needed {
        if gc.warning.is_none() && version::made_by_launcher(dir) && !pristine::available(dir, spec) {
            keep_copy(dir, spec);
        }
        return gc;
    }
    if !pristine::available(dir, spec) || watch::find_process(watch::GAME_PROCESS).is_some() {
        return gc;
    }
    log::line(&format!("version: {} Putting the kept copy back.", gc.reason.as_deref().unwrap_or("Game files changed.")));
    match pristine::restore(dir, spec) {
        Ok(Some(files)) => {
            log::line(&format!("version: put back {} file(s): {}", files.len(), files.join(", ")));
            let want = spec.version.as_deref().and_then(version::parse_version);
            if want.is_none() || version::exe_version(&dir.join(game::GAME_EXE)) == want {
                // The Steam files are the saved build again; anything else
                // that changed (a mod's archive) isn't the game version.
                if let Err(e) = version::record(dir, spec, false) {
                    log::line(&format!("version: couldn't record the build: {e}"));
                }
                log::line(&format!("version: {}", launcher_core::community::set_aside_content_catalog()));
                match version::hold_updates(dir, spec.app) {
                    Ok(_) => log::line("version: Steam set to update Skyrim only when launched from Steam, again"),
                    Err(e) => log::line(&format!("version: couldn't stop Steam updating Skyrim: {e}")),
                }
            }
            let again = version::check(dir, Some(spec));
            log::line(&format!("version: after putting files back, needed={}", again.needed));
            again
        }
        Ok(None) => gc,
        Err(e) => {
            log::line(&format!("version: couldn't put the kept copy back: {e}"));
            gc
        }
    }
}

/// The client files in the current server build, for the Game files page.
#[tauri::command]
async fn files(state: State<'_, AppState>) -> CmdResult<Vec<launcher_core::manifest::FileEntry>> {
    let m = state.manifest.lock().await;
    m.as_ref().map(|m| m.files.clone()).ok_or_else(|| "The file list hasn't loaded yet.".to_string())
}

/// Which listed files are current, changed or missing on this PC, for the
/// per-row marks on the Game files page. Changes nothing.
#[tauri::command]
async fn files_state(state: State<'_, AppState>) -> CmdResult<Vec<sync::FileState>> {
    let dir = game_dir(&state).await?;
    let m = state.manifest.lock().await.clone().ok_or_else(|| "The file list hasn't loaded yet.".to_string())?;
    sync::file_states(&dir, &m).await.map_err(err)
}

#[tauri::command]
async fn open_game_folder(state: State<'_, AppState>) -> CmdResult<()> {
    let dir = game_dir(&state).await?;
    let opener = if cfg!(windows) { "explorer" } else { "xdg-open" };
    std::process::Command::new(opener).arg(dir).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// The first-run checklist on Home (core/src/setup.rs), from checks the
/// launcher already makes. Nothing here changes anything on the PC.
#[tauri::command]
async fn setup_state(app: AppHandle, state: State<'_, AppState>) -> CmdResult<Vec<launcher_core::setup::Step>> {
    let dir = state.config.lock().await.game_dir.clone();
    let spec = state.manifest.lock().await.as_ref().and_then(|m| m.game.clone());
    let signed_in = token(&app).is_some();
    let inputs = match dir {
        Some(dir) => tokio::task::spawn_blocking(move || {
            let version = spec.as_ref().map(|s| !version::check(&dir, Some(s)).needed);
            let game_dir_ok = game::inspect(&dir).is_ok();
            let req = launcher_core::clientstatus::required(&dir, None);
            let has = |name: &str| req.iter().find(|(n, _)| *n == name).is_some_and(|(_, ok)| *ok);
            launcher_core::setup::Inputs {
                game_folder: game_dir_ok,
                game_version_ok: version,
                skse: has("SKSE64"),
                signed_in,
            }
        })
        .await
        .map_err(|e| e.to_string())?,
        None => launcher_core::setup::Inputs { signed_in, ..Default::default() },
    };
    Ok(launcher_core::setup::steps(&inputs))
}

/// Server state for the home screen and the server page. Up/down and players
/// come live from the login service's public /health (the game server checks
/// in there every 5 s); the optional `status.json` adds news and the last reset.
/// Either can be missing; None only when both are.
#[tauri::command]
async fn server_status(state: State<'_, AppState>) -> CmdResult<Option<serde_json::Value>> {
    let base = state.config.lock().await.base_url.clone();
    let file_url = format!("{}/status.json", base.trim_end_matches('/'));
    let health_url = format!("{}/health", AUTH_URL.trim_end_matches('/'));
    let (file, health) = tokio::join!(get_json(&state.http, &file_url), get_json(&state.http, &health_url));
    let file_ok = matches!(&file, Some(serde_json::Value::Object(_)));
    let mut out = match file {
        Some(serde_json::Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    };
    // Tells the UI the status file itself was read, so a missing `launcher` block means staff removed it
    // and not that the file couldn't be fetched just now. A file can't set this itself.
    out.remove("fromFile");
    if file_ok { out.insert("fromFile".into(), true.into()); }
    let seen = health.as_ref().and_then(|h| h.get("gameServerSeen")).and_then(|v| v.as_bool());
    // Log only changes, so a 30-second poll doesn't flood the log.
    if HEALTH_OK.swap(seen.is_some(), Ordering::Relaxed) != seen.is_some() {
        log::line(&match seen {
            Some(_) => format!("server status: {health_url} answering again"),
            None => format!("server status: no answer from {health_url}"),
        });
    }
    // The Discord invite for the sign-in window ("Not a member yet?").
    out.remove("discordInvite");
    if let Some(invite) = health.as_ref().and_then(|h| h.get("discordInvite")).and_then(|v| v.as_str()).and_then(launcher_core::discord_invite) {
        out.insert("discordInvite".into(), invite.into());
    }
    if let (Some(seen), Some(h)) = (seen, health.as_ref()) {
        out.insert("online".into(), seen.into());
        for key in ["players", "maxPlayers"] {
            match h.get(key) {
                Some(v) if v.is_number() => { out.insert(key.into(), v.clone()); }
                _ => { out.remove(key); }
            }
        }
    }
    Ok(if out.is_empty() { None } else { Some(serde_json::Value::Object(out)) })
}

static HEALTH_OK: AtomicBool = AtomicBool::new(true);

/// The parts of the connection help staff left switched on (status.json
/// `launcher.firewallFix`, `udpProbe`, `connectWatch`). An unreadable status
/// file leaves them on.
async fn launcher_flags(http: &reqwest::Client, base_url: &str) -> netcheck::Flags {
    let url = format!("{}/status.json", base_url.trim_end_matches('/'));
    netcheck::Flags::from_status(get_json(http, &url).await.as_ref())
}

/// This Play's launcher rules (core/src/rules.rs): the server's signed
/// `launcher-rules.json`, else the last good copy, else the built-in defaults.
async fn server_rules(app: &AppHandle, http: &reqwest::Client, base_url: &str) -> launcher_core::rules::Rules {
    use launcher_core::rules::Source;
    let dir = app.path().app_local_data_dir().ok();
    let (rules, source, note) = launcher_core::rules::load(http, base_url, dir.as_deref(), launcher_core::feedsig::trust()).await;
    let from = match source { Source::Server => "the server", Source::Cache => "the saved copy", Source::BuiltIn => "the built-in defaults" };
    log::line(&format!("rules: launcher rules {} from {from}{}", rules.rev, note.map(|n| format!(" ({n})")).unwrap_or_default()));
    rules
}

async fn get_json(http: &reqwest::Client, url: &str) -> Option<serde_json::Value> {
    let resp = http.get(url).timeout(std::time::Duration::from_secs(8)).send().await.ok()?.error_for_status().ok()?;
    resp.json().await.ok()
}

/// Records something from the UI (a failed command, a script error).
#[tauri::command]
fn log_ui(msg: String) {
    let mut msg = msg;
    msg.truncate(2000);
    log::line(&format!("ui: {msg}"));
}

#[tauri::command]
fn open_log_folder() -> CmdResult<()> {
    let dir = log::path().and_then(|p| p.parent().map(|d| d.to_path_buf())).ok_or("The log isn't set up.")?;
    let opener = if cfg!(windows) { "explorer" } else { "xdg-open" };
    std::process::Command::new(opener).arg(dir).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// A plain-text report to paste to staff when something goes wrong.
/// Never includes the Discord token or the game session.
#[tauri::command]
async fn diagnostics(app: AppHandle, state: State<'_, AppState>) -> CmdResult<String> {
    use std::fmt::Write;
    let config = state.config.lock().await.clone();
    let manifest = state.manifest.lock().await.clone();
    let mut o = String::new();
    let _ = writeln!(o, "Aetherial Dawn launcher diagnostics");
    let _ = writeln!(o, "Time: {}", log::timestamp());
    let _ = writeln!(o, "Launcher: {} on {} {}", app.package_info().version, std::env::consts::OS, std::env::consts::ARCH);
    let _ = writeln!(o, "Server files: {}", config.base_url);
    let _ = writeln!(o, "Login service: {AUTH_URL}");
    let _ = writeln!(o, "\n[Skyrim]");
    match &config.game_dir {
        None => {
            let _ = writeln!(o, "Folder: not set");
        }
        Some(dir) => {
            let _ = writeln!(o, "Folder: {}", dir.display());
            let exe = version::exe_version(&dir.join(game::GAME_EXE));
            let _ = writeln!(o, "SkyrimSE.exe: {}", exe.map(version::show).unwrap_or_else(|| "missing or unreadable".into()));
            let _ = writeln!(o, "skse64_loader.exe: {}", if dir.join(game::SKSE_LOADER).exists() { "present" } else { "MISSING" });
            let dlls: Vec<String> = std::fs::read_dir(dir)
                .map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.to_lowercase().starts_with("skse64_") && n.to_lowercase().ends_with(".dll")).collect())
                .unwrap_or_default();
            let _ = writeln!(o, "SKSE dlls: {}", if dlls.is_empty() { "none".into() } else { dlls.join(", ") });
            if let Some(acf) = version::acf_path(dir, 489830) {
                let depots = std::fs::read_to_string(&acf).map(|t| version::installed_depots(&t)).unwrap_or_default();
                let mut d: Vec<String> = depots.iter().map(|(k, v)| format!("{k}:{v}")).collect();
                d.sort();
                let _ = writeln!(o, "Steam depots: {}", d.join(" "));
            }
            let marker = dir.join(".aetherial-dawn").join("game.json");
            let _ = writeln!(o, "Downgrade marker: {}", if marker.exists() { "present" } else { "none" });
            let gc = version::check(dir, manifest.as_ref().and_then(|m| m.game.as_ref()));
            let _ = writeln!(o, "Version check: {}", serde_json::to_string(&gc).unwrap_or_default());
            let settings = dir.join(settings::SETTINGS_PATH);
            let _ = writeln!(o, "skymp5-client-settings.txt: {}", if settings.exists() { "present" } else { "not written yet" });
            o.push_str(&game_data_report(&app, dir, true));
        }
    }
    let _ = writeln!(o, "\n[Server]");
    match &manifest {
        Some(m) => {
            let _ = writeln!(o, "Build: {}  Address: {}:{}  Files: {}", m.build, m.server.ip, m.server.port, m.files.len());
        }
        None => {
            let _ = writeln!(o, "File list: not loaded");
        }
    }
    let _ = writeln!(o, "\n[Discord sign-in]");
    match &config.account {
        Some(a) => {
            let _ = writeln!(
                o,
                "Account: {} (Discord id {}, player id {})",
                a.discord_username.as_deref().unwrap_or("?"),
                a.discord_id.as_deref().unwrap_or("?"),
                a.master_api_id.map(|i| i.to_string()).unwrap_or_else(|| "?".into())
            );
        }
        None => {
            let _ = writeln!(o, "Account: not signed in");
        }
    }
    let _ = writeln!(o, "Saved login: {}", if token(&app).is_some() { "yes" } else { "no" });
    let _ = writeln!(o, "Last confirmed: {}", config.last_auth_ok.map(|t| format!("{} min ago", now().saturating_sub(t) / 60)).unwrap_or_else(|| "never".into()));
    match last_report() {
        Some((name, text)) => {
            let _ = writeln!(o, "\n[Last game session: {name}]\n{}", text.lines().take(150).collect::<Vec<_>>().join("\n"));
        }
        None => {
            let _ = writeln!(o, "\n[Last game session: none recorded]");
        }
    }
    let _ = writeln!(o, "\n[Log: {}]", log::path().map(|p| p.display().to_string()).unwrap_or_default());
    let _ = writeln!(o, "{}", log::tail(80));
    Ok(o)
}

/// `--make-patches <from> <to> <out> [version]`: builds game patches from
/// the Skyrim folder `from` (newer Steam build) to `to` (the server's build)
/// into `out`, then exits. Progress goes to `<out>/make-patches.log`.
fn make_patches_cli(args: &[String]) -> Option<i32> {
    let i = args.iter().position(|a| a == "--make-patches")?;
    let (Some(from), Some(to), Some(out)) = (args.get(i + 1), args.get(i + 2), args.get(i + 3)) else {
        eprintln!("usage: --make-patches <from game folder> <to game folder> <out folder> [version]");
        return Some(2);
    };
    let version = args.get(i + 4).cloned().unwrap_or_else(|| "1.6.1170.0".into());
    let out = PathBuf::from(out);
    let _ = std::fs::create_dir_all(&out);
    let log_path = out.join("make-patches.log");
    let mut logf = std::fs::OpenOptions::new().create(true).append(true).open(&log_path).ok();
    let mut say = |m: &str| {
        use std::io::Write;
        println!("{m}");
        if let Some(f) = logf.as_mut() {
            let _ = writeln!(f, "[{}] {m}", log::timestamp());
        }
    };
    say(&format!("making patches from {from} to {to} ({version}) in {}", out.display()));
    match launcher_core::patcher::build(std::path::Path::new(from), std::path::Path::new(to), &version, &out, &mut say) {
        Ok(ix) => {
            let n: usize = ix.files.iter().map(|f| f.patches.len()).sum();
            say(&format!("done: {} files listed, {n} patches, index.json written", ix.files.len()));
            Some(0)
        }
        Err(e) => {
            say(&format!("failed: {e}"));
            Some(1)
        }
    }
}

/// The window starts hidden (tauri.conf.json) so the player never sees a
/// blank or half-drawn frame; the page shows it once its first screen is
/// drawn. If the page never says so, it is shown anyway after 3 s.
static WINDOW_SHOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn show_window_once(app: &tauri::AppHandle, by: &str) {
    if WINDOW_SHOWN.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
    if by != "page" {
        log::line(&format!("window: shown by the {by}, the page didn't draw in time"));
    }
}

/// Opens the Discord invite in the browser; nothing but a plain invite link.
#[tauri::command]
fn open_invite(app: tauri::AppHandle, url: String) -> CmdResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let invite = launcher_core::discord_invite(&url).ok_or("That isn't a Discord invite.")?;
    app.opener().open_url(invite, None::<&str>).map_err(|e| e.to_string())?;
    Ok(())
}

/// Shrinks the still-hidden window to the usable part of its screen, so the
/// frameless window and its PLAY button never open under the taskbar.
fn fit_window(app: &tauri::AppHandle) {
    let Some(w) = app.get_webview_window("main") else { return };
    // A window that opens off every screen goes on the main one.
    let Some(monitor) = w.current_monitor().ok().flatten().or_else(|| w.primary_monitor().ok().flatten()) else { return };
    let Ok(size) = w.inner_size() else { return };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let work = area.size.to_logical::<f64>(scale);
    let want = size.to_logical::<f64>(scale);
    let (width, height) = launcher_core::window::fit((want.width, want.height), (work.width, work.height));
    if (width, height) != (want.width, want.height) {
        log::line(&format!("window: {}x{} doesn't fit the screen's {}x{}, opening at {width}x{height}", want.width, want.height, work.width, work.height));
        let _ = w.set_size(tauri::LogicalSize::new(width, height));
    }
    // Centred in the usable area in physical pixels, so a taskbar (at any
    // edge) never covers the PLAY button.
    let (x, y) = launcher_core::window::centre_in(
        (area.position.x as f64, area.position.y as f64, area.size.width as f64, area.size.height as f64),
        (width * scale, height * scale),
    );
    let _ = w.set_position(tauri::PhysicalPosition::new(x.round() as i32, y.round() as i32));
}

#[tauri::command]
fn window_ready(app: tauri::AppHandle) {
    show_window_once(&app, "page");
}

/// A second launch brings the running launcher forward.
fn bring_forward(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = make_patches_cli(&args) {
        std::process::exit(code);
    }
    tauri::Builder::default()
        // A second launch (Windows runs one for each nxm:// link) brings this one forward.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| bring_forward(app)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            if let Ok(dir) = app.path().app_local_data_dir() {
                log::init(dir.join("logs"));
                // Remembers which server files came signed (feedsig.rs).
                launcher_core::feedsig::init(&dir);
            }
            let config = load_config(app.handle());
            log::line(&format!(
                "launcher {} starting on {} {}; server files {}; login service {AUTH_URL}; Skyrim folder {}",
                app.package_info().version,
                std::env::consts::OS,
                std::env::consts::ARCH,
                config.base_url,
                config.game_dir.as_ref().map(|d| d.display().to_string()).unwrap_or_else(|| "not set".into())
            ));
            save_config(app.handle(), &config)?;
            // The bot counts launcher versions from this header.
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert("x-launcher-version", reqwest::header::HeaderValue::from_static(env!("CARGO_PKG_VERSION")));
            let client = |gzip: bool| {
                reqwest::Client::builder()
                    .user_agent(concat!("AetherialDawnLauncher/", env!("CARGO_PKG_VERSION")))
                    .default_headers(headers.clone())
                    .connect_timeout(std::time::Duration::from_secs(10))
                    // A server that goes silent mid-answer is an error, not
                    // a wait for ever (sync, file list, sign-in, session).
                    .read_timeout(STALL)
                    .gzip(gzip)
                    .build()
            };
            let (http, faces_http) = (client(false)?, client(true)?);
            app.manage(AppState { config: Mutex::new(config), manifest: Mutex::new(None), game_operation: Mutex::new(()), active_session: AtomicBool::new(false), update_installing: AtomicBool::new(false), http, faces_http, music: music::Music::new() });
            tauri::async_runtime::spawn(end_leftover_skyrim(app.handle().clone()));
            // A session that ended while the launcher was closed.
            if let Some(dir) = app.state::<AppState>().config.try_lock().ok().and_then(|c| c.game_dir.clone()) {
                if watch::find_process(watch::GAME_PROCESS).is_none() {
                    restore_ccc(&dir, "the launcher started and the game isn't running");
                    restore_profile(&dir, "the launcher started and the game isn't running");
                    faces::tidy(&dir, None);
                }
            }
            fit_window(app.handle());
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(3));
                show_window_once(&handle, "fallback");
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![plain_error, repair_game_files, get_state, set_game_dir, set_prefs, check, update, play, files, files_state, open_game_folder, server_status, setup_state, mark_game_ok, auth_status, auth_begin, auth_poll, auth_sign_out, log_ui, open_log_folder, diagnostics, health_check, bug_state, send_bug, report_problem, patch_game, game_running, self_update_begin, self_update_end, close_game_for_update, raise_launcher, music_start, set_music, restore_set_aside, window_ready, open_invite])
        .build(tauri::generate_context!())
        .expect("error while running the launcher")
        .run(|_, _| {});
}
