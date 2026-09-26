// Hides the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use launcher_core::{auth, downgrade, game, gameini, health, loadorder, manifest::Manifest, pristine, requirements, settings, steamapp, strays, sync, version, watch, Error};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;

mod log;
mod mods;
mod music;

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

/// How long the launcher trusts a sign-in it couldn't re-check (service down).
const OFFLINE_GRACE_SECS: u64 = 24 * 3600;

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
    /// The Nexus Mods account the player signed in with (the key itself is
    /// kept separately, encrypted).
    nexus_user: Option<launcher_core::nexus::User>,
    /// Menu music: None until the player answers the Keep music / Mute
    /// question, then their answer.
    music: Option<bool>,
    /// Only the server's mods: other mods' files Vortex put in Data are set
    /// aside before Play (Timothy, 2026-09-26). Players can turn it off.
    only_server_mods: bool,
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
            nexus_user: None,
            music: None,
            only_server_mods: true,
        }
    }
}

struct AppState {
    config: Mutex<Config>,
    manifest: Mutex<Option<Manifest>>,
    http: reqwest::Client,
    /// The Steam download running inside the launcher, and its input for the
    /// player's password or Steam Guard code.
    steam_child: std::sync::Arc<std::sync::Mutex<Option<std::process::Child>>>,
    steam_input: std::sync::Mutex<Option<std::process::ChildStdin>>,
    mods: mods::ModsState,
    music: music::Music,
}

type CmdResult<T> = Result<T, String>;

fn err(e: Error) -> String {
    e.to_string()
}

fn config_path(app: &AppHandle) -> CmdResult<PathBuf> {
    app.path().app_config_dir().map(|d| d.join("config.json")).map_err(|e| e.to_string())
}

fn load_config(app: &AppHandle) -> Config {
    config_path(app)
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<Config>(&b).ok())
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
    std::fs::write(path, serde_json::to_vec_pretty(c).unwrap()).map_err(|e| e.to_string())
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
            Err(e) => (None, Some(e.to_string())),
        },
        None => (None, Some("Skyrim Special Edition wasn't found. Pick its folder.".into())),
    };
    Ok(Snapshot { config: config.clone(), game, game_error, launcher_version: app.package_info().version.to_string() })
}

#[tauri::command]
async fn set_game_dir(app: AppHandle, state: State<'_, AppState>, dir: PathBuf) -> CmdResult<game::GameInfo> {
    let info = game::inspect(&dir).map_err(err)?;
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
    #[serde(default = "yes")]
    only_server_mods: bool,
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
    config.only_server_mods = prefs.only_server_mods;
    save_config(&app, &config)
}

/// Puts back every file the launcher set aside (other mods, old plugins).
#[tauri::command]
async fn restore_set_aside(state: State<'_, AppState>) -> CmdResult<usize> {
    let dir = game_dir(&state).await?;
    let n = launcher_core::allowlist::restore_all(&dir).map_err(|e| format!("Couldn't put the files back ({e}). Close Skyrim and Vortex, then try again."))?;
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
    /// Plugins in the game folder that the server didn't ship.
    strays: Vec<String>,
}

async fn game_dir(state: &AppState) -> CmdResult<PathBuf> {
    state.config.lock().await.game_dir.clone().ok_or_else(|| "Pick your Skyrim folder first.".to_string())
}

/// Downloads the server's file list and works out what needs updating.
#[tauri::command]
async fn check(app: AppHandle, state: State<'_, AppState>, verify_all: bool) -> CmdResult<CheckResult> {
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
        strays: all_strays(&app, &dir, &m),
    };
    if !result.strays.is_empty() {
        log::line(&format!("check: plugins not from the server: {}", result.strays.join(", ")));
    }
    *state.manifest.lock().await = Some(m);
    Ok(result)
}

/// Brings the game folder in line with the file list, sending `sync-progress`
/// events to the UI as it goes.
#[tauri::command]
async fn update(app: AppHandle, state: State<'_, AppState>, verify_all: bool) -> CmdResult<String> {
    let dir = game_dir(&state).await?;
    let base = state.config.lock().await.base_url.clone();
    let m = match state.manifest.lock().await.clone() {
        Some(m) => m,
        None => Manifest::fetch(&state.http, &base).await.map_err(err)?,
    };
    let plan = sync::plan(&dir, &m, verify_all).await.map_err(err)?;
    sync::apply(&state.http, &base, &dir, &plan, |p| {
        let _ = app.emit("sync-progress", p);
    })
    .await
    .map_err(err)?;
    Ok(m.build)
}

/// Gets a game session from the login service, writes the SkyMP client
/// settings and remembered login, and starts Skyrim through SKSE.
/// Errors that start with "SIGNED_OUT:" mean the player must sign in again.
#[tauri::command]
async fn play(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let config = state.config.lock().await.clone();
    let dir = config.game_dir.clone().ok_or("Pick your Skyrim folder first.")?;
    let m = state.manifest.lock().await.clone().ok_or("Check for updates before playing.")?;
    let gc = auto_version(&dir, m.game.as_ref());
    log::line(&format!("play: game folder {}, build {}, version needed={} skseOk={}", dir.display(), m.build, gc.needed, gc.skse_ok));
    if gc.needed {
        return Err(gc.reason.unwrap_or_else(|| "Your Skyrim version doesn't match the server.".into()));
    }
    if gc.installed.is_some() {
        install_missing_mods(&state.http, &dir).await?;
    }
    game::inspect(&dir).map_err(err)?;
    tidy_game(&app, &dir, &m, config.only_server_mods)?;
    let report = run_health(&app, &state.http, &config.base_url, &dir, Some(&m)).await;
    log::line(&format!("health before play: worst={:?}\n{}", report.worst, report.text()));
    if report.worst >= health::Status::Warn && config.share_health {
        send_health(&app, &state.http, &config, &m.build, "before play", None, &report).await;
    }
    ensure_requirements(&state, &dir).await?;
    let token = token(&app).ok_or("SIGNED_OUT:Sign in with Discord to play.")?;
    let session = match auth::play(&state.http, AUTH_URL, &token).await {
        auth::Answer::Ok(p) => {
            log::line("play: login service gave a game session");
            p.session
        }
        auth::Answer::SignedOut(msg) => {
            sign_out(&app, &state).await;
            return Err(format!("SIGNED_OUT:{msg}"));
        }
        auth::Answer::Refused { message, .. } => {
            sign_out(&app, &state).await;
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
    state.music.stop();
    game::launch(&dir).map_err(err)?;
    let google = game::google_env_present();
    if !google.is_empty() {
        log::line(&format!("play: left Google sign-in settings out of the game's environment: {}", google.join(", ")));
    }
    log::line("play: started skse64_loader.exe");
    let started = std::time::SystemTime::now();
    if config.close_on_launch {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.hide();
        }
    }
    tauri::async_runtime::spawn(watch_game(app.clone(), dir.clone(), started, config.close_on_launch));
    Ok(())
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

/// Closing Skyrim with its X (or Alt+F4) can leave SkyrimSE.exe and Skyrim
/// Platform's browser helper running with no window, so Steam still shows the
/// game as running. Once Skyrim has had a window, and then has none for 15
/// seconds while the process is still alive, the launcher ends it and the
/// helpers.
async fn end_when_window_closed(pid: u32) {
    let alive = || watch::find_process(watch::GAME_PROCESS) == Some(pid);
    let mut seen = false;
    let mut gone = 0;
    while alive() {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if watch::has_visible_window(pid) {
            seen = true;
            gone = 0;
        } else if seen {
            gone += 1;
            if gone >= 15 && alive() {
                log::line("game: Skyrim's window closed but the game kept running for 15 seconds; ending it so Steam sees it closed");
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

/// Follows Skyrim from launch to exit. Every session leaves a report in the
/// log folder; a crash brings the launcher back with that report on screen.
async fn watch_game(app: AppHandle, game_dir: std::path::PathBuf, started: std::time::SystemTime, close_on_launch: bool) {
    // skse64_loader starts SkyrimSE.exe and exits, so look for the game itself.
    let mut pid = None;
    for _ in 0..90 {
        if let Some(p) = watch::find_process(watch::GAME_PROCESS) {
            pid = Some(p);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    let (code, ran, summary) = match pid {
        None => (None, std::time::Duration::ZERO, "Skyrim didn't start: SKSE's loader ran, but SkyrimSE.exe never appeared within 90 seconds.".to_string()),
        Some(pid) => {
            log::line(&format!("game: SkyrimSE.exe running as process {pid}"));
            tokio::spawn(end_when_window_closed(pid));
            let code = tokio::task::spawn_blocking(move || watch::wait_exit(pid)).await.ok().flatten();
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
    let health = run_health(&app, &http, &config.base_url, &game_dir, manifest.as_ref()).await;
    report.push_str(&format!("\n===== game health =====\n{}", health.text()));
    let mut staff_summary = summary.clone();
    if crashed {
        if let Some(cl) = watch::crash_logger_summary(&skse_logs, started) {
            log::line(&format!("game: crash logger says:\n{cl}"));
            staff_summary.push_str(&format!("\n\nCrash logger:\n{cl}"));
        }
    }
    // Sent in the background: the staff service can ask for a short wait
    // (one report per 20 s), and the crash window shouldn't wait for it.
    if crashed && config.share_health {
        let (app2, http2, health2) = (app.clone(), http.clone(), health.clone());
        tauri::async_runtime::spawn(async move {
            let f = send_health(&app2, &http2, &config, &build, "after a crash", Some(&staff_summary), &health2).await;
            if f.report_id.is_some() {
                let _ = app2.emit("crash-filed", CrashFiled { report_id: f.report_id, likely_cause: f.likely_cause });
            }
        });
    }
    let filed = Filed::default();
    report.push_str(&format!("\n===== game data =====\n{}", game_data_report(&app, &game_dir)));
    report.push_str(&watch::collect(&skse_logs, &std::env::temp_dir(), started));
    report.push_str(&format!("\n===== launcher log (last 40 lines) =====\n{}\n", log::tail(40)));
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
    } else if close_on_launch {
        app.exit(0);
    } else {
        let _ = app.emit("game-ended", GameEnded { crashed, summary: summary.clone(), report: report.clone(), report_id: filed.report_id.clone(), likely_cause: filed.likely_cause.clone() });
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
fn game_data_report(app: &AppHandle, dir: &std::path::Path) -> String {
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

#[tauri::command]
fn last_game_report() -> CmdResult<String> {
    last_report().map(|(_, text)| text).ok_or_else(|| "No game session has been recorded yet.".into())
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
            let fresh = config.last_auth_ok.is_some_and(|t| now().saturating_sub(t) < OFFLINE_GRACE_SECS);
            Ok(AuthStatus {
                signed_in: true,
                account: config.account.clone(),
                offline: true,
                locked: !fresh,
                message: Some(if fresh {
                    "Couldn't reach the login service. You can still play for now.".into()
                } else {
                    "Couldn't confirm your Discord sign-in for over a day. Connect to the internet and try again.".into()
                }),
            })
        }
    }
}

/// Opens the Discord sign-in in the browser. Returns the state to poll with.
#[tauri::command]
async fn auth_begin(app: AppHandle) -> CmdResult<String> {
    use tauri_plugin_opener::OpenerExt;
    let st = auth::new_state();
    app.opener().open_url(auth::login_url(AUTH_URL, &st), None::<&str>).map_err(|e| e.to_string())?;
    Ok(st)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PollResult {
    /// pending | done | refused | expired | offline
    status: &'static str,
    message: Option<String>,
    account: Option<auth::Profile>,
}

#[tauri::command]
async fn auth_poll(app: AppHandle, state: State<'_, AppState>, st: String) -> CmdResult<PollResult> {
    let r = |status, message: Option<String>| PollResult { status, message, account: None };
    let answer = auth::poll(&state.http, AUTH_URL, &st).await;
    if !matches!(answer, auth::Answer::Pending) {
        log::line(&format!("sign-in: {}", answer_kind(&answer)));
    }
    Ok(match answer {
        auth::Answer::Pending => r("pending", None),
        auth::Answer::Ok(done) => {
            let path = token_path(&app).ok_or("No place to save the sign-in.")?;
            auth::save_token(&path, &done.token).map_err(err)?;
            signed_in(&app, &state, done.profile.clone()).await?;
            PollResult { status: "done", message: None, account: Some(done.profile) }
        }
        auth::Answer::Refused { message, .. } => r("refused", Some(message)),
        auth::Answer::SignedOut(message) => r("expired", Some(message)),
        auth::Answer::Offline(message) => r("offline", Some(message)),
    })
}

#[tauri::command]
async fn auth_sign_out(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    sign_out(&app, &state).await;
    Ok(())
}

/// Is the player's Skyrim the build the server needs?
#[tauri::command]
async fn game_check(state: State<'_, AppState>) -> CmdResult<version::GameCheck> {
    let dir = game_dir(&state).await?;
    let m = state.manifest.lock().await.clone();
    Ok(auto_version(&dir, m.as_ref().and_then(|m| m.game.as_ref())))
}

/// Downloads the server's Skyrim build from Steam with the player's own
/// account through DepotDownloader. Sends `downgrade-stage` events ("tool",
/// "steam", "verify") to the UI. With `inline`, the sign-in happens inside
/// the launcher: `steam-login` events say what Steam asks for, and the UI
/// answers through `steam_login_answer`. Otherwise DepotDownloader opens its
/// own window.
#[tauri::command]
async fn downgrade(app: AppHandle, state: State<'_, AppState>, username: Option<String>, inline: Option<bool>) -> CmdResult<version::GameCheck> {
    let dir = game_dir(&state).await?;
    let m = state.manifest.lock().await.clone().ok_or("Check for updates first.")?;
    let spec = m.game.clone().ok_or("The server doesn't ask for a particular Skyrim version.")?;
    let inline = inline.unwrap_or(false);
    let login = match username.as_deref().map(str::trim) {
        Some(u) if !u.is_empty() => downgrade::Login::User(u.to_string()),
        _ if inline => return Err("Type your Steam account name.".into()),
        _ => downgrade::Login::Qr,
    };
    let args = downgrade::args(&spec, &dir, &login).map_err(err)?;
    log::line(&format!("downgrade: to {} in {}{}, DepotDownloader {}", spec.version.as_deref().unwrap_or("?"), dir.display(), if inline { " (sign-in inside the launcher)" } else { "" }, args.join(" ")));
    let tools = app.path().app_local_data_dir().map_err(|e| e.to_string())?.join("tools");
    let _ = app.emit("downgrade-stage", "tool");
    let tool = downgrade::ensure_tool(&state.http, &tools, spec.tool.as_ref()).await.map_err(err)?;
    log::line(&format!("downgrade: tool ready at {}", tool.display()));
    let _ = app.emit("downgrade-stage", "steam");
    if inline {
        run_inline(&app, &state, &tool, &args, &tools).await?;
        log::line("downgrade: Steam download finished");
    } else {
        downgrade::run(&tool, &args, &tools).await.map_err(err)?;
        log::line("downgrade: Steam download window closed");
    }
    let _ = app.emit("downgrade-stage", "verify");
    finish_downgrade(&dir, &spec, false)
}

async fn run_inline(app: &AppHandle, state: &AppState, tool: &std::path::Path, args: &[String], tools: &std::path::Path) -> CmdResult<()> {
    use std::io::Read;
    if state.steam_child.lock().unwrap().is_some() {
        return Err("A Steam download is already running.".into());
    }
    let mut child = downgrade::spawn_piped(tool, args, tools).map_err(err)?;
    let failed: std::sync::Arc<std::sync::Mutex<Option<String>>> = Default::default();
    let pipes: Vec<Box<dyn Read + Send>> = [child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>), child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>)]
        .into_iter()
        .flatten()
        .collect();
    *state.steam_input.lock().unwrap() = child.stdin.take();
    *state.steam_child.lock().unwrap() = Some(child);
    let mut readers = Vec::new();
    for mut pipe in pipes {
        let (app, failed) = (app.clone(), failed.clone());
        readers.push(std::thread::spawn(move || {
            let mut scanner = downgrade::Scanner::default();
            let mut buf = [0u8; 4096];
            let mut last_logged = -10.0f32;
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                for ev in scanner.feed(&String::from_utf8_lossy(&buf[..n])) {
                    match &ev {
                        downgrade::Event::Progress { percent } => {
                            if *percent >= last_logged + 10.0 {
                                last_logged = *percent;
                                log::line(&format!("steam: {percent:.0}%"));
                            }
                        }
                        downgrade::Event::Line { text } => log::line(&format!("steam: {text}")),
                        downgrade::Event::LoginFailed { message } => {
                            log::line(&format!("steam: sign-in refused: {message}"));
                            *failed.lock().unwrap() = Some(message.clone());
                        }
                        other => log::line(&format!("steam: asks {other:?}")),
                    }
                    let _ = app.emit("steam-login", &ev);
                }
            }
        }));
    }
    let status = loop {
        let done = {
            let mut guard = state.steam_child.lock().unwrap();
            match guard.as_mut() {
                None => None,
                Some(c) => match c.try_wait() {
                    Ok(Some(s)) => {
                        guard.take();
                        Some(Some(s))
                    }
                    Ok(None) => Some(None),
                    Err(_) => {
                        guard.take();
                        None
                    }
                },
            }
        };
        match done {
            None => break None,
            Some(Some(s)) => break Some(s),
            Some(None) => tokio::time::sleep(std::time::Duration::from_millis(400)).await,
        }
    };
    state.steam_input.lock().unwrap().take();
    for r in readers {
        let _ = tokio::task::spawn_blocking(move || r.join()).await;
    }
    match status {
        Some(s) if s.success() => Ok(()),
        None => Err("The Steam download was stopped. Nothing was changed that a second try won't fix.".into()),
        Some(s) => {
            log::line(&format!("downgrade: DepotDownloader ended with {s}"));
            match failed.lock().unwrap().take() {
                Some(m) => Err(format!("Steam didn't accept the sign-in ({m}). Check your account name and password, then try again.")),
                None => Err("The Steam download didn't finish. Nothing was changed that a second try won't fix.".into()),
            }
        }
    }
}

/// Passes the player's answer (password or Steam Guard code) to Steam. The
/// text is never logged or kept.
#[tauri::command]
fn steam_login_answer(state: State<'_, AppState>, text: String) -> CmdResult<()> {
    use std::io::Write;
    let mut guard = state.steam_input.lock().unwrap();
    let input = guard.as_mut().ok_or("The Steam download isn't running.")?;
    let line = format!("{}\n", text.trim_end_matches(['\r', '\n']));
    input.write_all(line.as_bytes()).and_then(|_| input.flush()).map_err(|e| format!("Couldn't pass that to Steam: {e}"))?;
    log::line("steam: passed the player's answer to Steam");
    Ok(())
}

/// Stops a Steam download running inside the launcher.
#[tauri::command]
fn steam_login_cancel(state: State<'_, AppState>) {
    if let Some(mut c) = state.steam_child.lock().unwrap().take() {
        let _ = c.kill();
        let _ = c.wait();
        log::line("steam: download stopped by the player");
    }
    state.steam_input.lock().unwrap().take();
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
#[tauri::command]
async fn patch_game(app: AppHandle, state: State<'_, AppState>) -> CmdResult<version::GameCheck> {
    use launcher_core::{community, patcher};
    let dir = game_dir(&state).await?;
    let spec = game_spec(&state).await?;
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
            format!("Couldn't patch the game: {e}")
        })?;
        log::line(&format!("patch: {} file(s) changed: {}", changed.len(), changed.join(", ")));
        let _ = app.emit("patch-progress", PatchProgress { stage: "verify", file: String::new(), done: 1, total: 1 });
        let gc = finish_downgrade(&dir, &spec, false)?;
        install_missing_mods(&state.http, &dir).await?;
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
        .map_err(|e| format!("NO_PATCH:The server's patches aren't available ({e})."))?
        .json()
        .await
        .map_err(|e| format!("NO_PATCH:The server's patch list is damaged ({e})."))?
    };
    if Some(index.target.as_str()) != spec.version.as_deref() {
        return Err(format!("NO_PATCH:The server's patches make {}, but it needs {}.", index.target, spec.version.as_deref().unwrap_or("?")));
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
        return Err(format!("NO_PATCH:Your copy of {} is a Skyrim build the server has no patch for yet. Staff have been told.", missing.join(", ")));
    }
    log::line(&format!("patch: {} file(s) to patch: {}", steps.len(), steps.iter().map(|s| s.file.path.as_str()).collect::<Vec<_>>().join(", ")));
    let tmp = app.path().app_local_data_dir().map_err(|e| e.to_string())?.join("patches");
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    let total = steps.len();
    for (i, step) in steps.iter().enumerate() {
        let patch = step.patch.as_ref().expect("checked");
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
        let (dir2, step2, local2) = (dir.clone(), step.clone(), local.clone());
        tokio::task::spawn_blocking(move || patcher::apply_step(&dir2, &step2, &local2))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| format!("Couldn't patch {} ({e}). Close Skyrim, Steam and Vortex, then try again.", step.file.path))?;
        if local.starts_with(&tmp) {
            let _ = std::fs::remove_file(&local);
        }
        log::line(&format!("patch: patched {}", step.file.path));
    }
    let _ = app.emit("patch-progress", PatchProgress { stage: "verify", file: String::new(), done: total, total });
    let gc = finish_downgrade(&dir, &spec, false)?;
    install_missing_mods(&state.http, &dir).await?;
    Ok(gc)
}

fn patch_build_dir(game_dir: &std::path::Path) -> PathBuf {
    game_dir.join(".aetherial-dawn").join("patch-build")
}

/// Staff, once per Steam build: downloads the server's build with Steam into
/// a separate folder (never the game folder), then makes patches from the
/// game as Steam has it now to that build. The patches are used on this PC
/// right away, and the folder `<game>\.aetherial-dawn\patch-build\out` is
/// what goes on the server at launcher/patches/. Sends the same events as
/// the Steam sign-in, then `patch-progress` with stage "build".
#[tauri::command]
async fn build_patches(app: AppHandle, state: State<'_, AppState>, username: String) -> CmdResult<String> {
    let dir = game_dir(&state).await?;
    let spec = game_spec(&state).await?;
    let version = spec.version.clone().ok_or("The server doesn't name a Skyrim version.")?;
    let build = patch_build_dir(&dir);
    let target = build.join(&version);
    let out = build.join("out");
    let login = downgrade::Login::User(username.trim().to_string());
    let args = downgrade::args(&spec, &target, &login).map_err(err)?;
    log::line(&format!("patch build: downloading {version} into {} to make patches", target.display()));
    let tools = app.path().app_local_data_dir().map_err(|e| e.to_string())?.join("tools");
    let _ = app.emit("downgrade-stage", "tool");
    let tool = downgrade::ensure_tool(&state.http, &tools, spec.tool.as_ref()).await.map_err(err)?;
    let _ = app.emit("downgrade-stage", "steam");
    run_inline(&app, &state, &tool, &args, &tools).await?;
    log::line("patch build: download finished, making patches");
    let (from, app2, out2, target2) = (dir.clone(), app.clone(), out.clone(), target.clone());
    let index = tokio::task::spawn_blocking(move || {
        launcher_core::patcher::build(&from, &target2, &version, &out2, |m| {
            log::line(&format!("patch build: {m}"));
            let _ = app2.emit("patch-progress", PatchProgress { stage: "build", file: m.to_string(), done: 0, total: 0 });
        })
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("Couldn't make the patches ({e})."))?;
    let n: usize = index.files.iter().map(|f| f.patches.len()).sum();
    log::line(&format!("patch build: {n} patches for {} files in {}", index.files.len(), out.display()));
    Ok(out.display().to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SteamApp {
    running: bool,
    depots: Vec<steamapp::DepotState>,
}

async fn game_spec(state: &AppState) -> CmdResult<launcher_core::manifest::GameSpec> {
    let m = state.manifest.lock().await.clone().ok_or("Check for updates first.")?;
    m.game.ok_or_else(|| "The server doesn't ask for a particular Skyrim version.".into())
}

fn steam_root() -> CmdResult<PathBuf> {
    steamapp::steam_root().ok_or_else(|| "Steam wasn't found on this PC. Use the Steam mobile app option instead.".into())
}

/// Is Steam open, and how far along is each depot download?
#[tauri::command]
async fn steam_app_state(state: State<'_, AppState>) -> CmdResult<SteamApp> {
    let spec = game_spec(&state).await?;
    let depots = steamapp::steam_root().map(|r| steamapp::state(&r, &spec)).unwrap_or_default();
    Ok(SteamApp { running: steamapp::steam_running(), depots })
}

/// Clears old depot downloads and opens Steam's console, where the player
/// pastes the download lines.
#[tauri::command]
async fn steam_app_begin(app: AppHandle, state: State<'_, AppState>) -> CmdResult<SteamApp> {
    use tauri_plugin_opener::OpenerExt;
    let spec = game_spec(&state).await?;
    let root = steam_root()?;
    if !steamapp::steam_running() {
        return Err("Steam isn't open. Start Steam, sign in, then try again.".into());
    }
    steamapp::clear(&root, &spec);
    log::line(&format!("downgrade (Steam app): opening the console, Steam at {}, lines: {}", root.display(), steamapp::commands(&spec).join(" / ")));
    app.opener().open_url("steam://open/console", None::<&str>).map_err(|e| format!("Couldn't open Steam's console ({e}). Press Windows+R, type steam://open/console and press Enter."))?;
    Ok(SteamApp { running: true, depots: steamapp::state(&root, &spec) })
}

/// Copies what Steam downloaded into the game folder and checks the version.
#[tauri::command]
async fn steam_app_install(state: State<'_, AppState>) -> CmdResult<version::GameCheck> {
    let dir = game_dir(&state).await?;
    let spec = game_spec(&state).await?;
    let root = steam_root()?;
    let (dir2, spec2) = (dir.clone(), spec.clone());
    let n = tokio::task::spawn_blocking(move || steamapp::install(&root, &spec2, &dir2)).await.map_err(|e| e.to_string())?.map_err(err)?;
    log::line(&format!("downgrade (Steam app): copied {n} files into {}", dir.display()));
    finish_downgrade(&dir, &spec, false)
}

/// Before every Play: puts leftovers from other mod setups out of the game's
/// way, so players never have to. SKSE/Platform plugins and loose Interface
/// files the server didn't ship go to .aetherial-dawn/disabled/<time>/,
/// extra plugins are switched off in plugins.txt, and archives Skyrim.ini
/// names but that no longer exist are dropped (the ini is backed up first).
/// Nothing is deleted.
fn tidy_game(app: &AppHandle, dir: &std::path::Path, m: &Manifest, only_server_mods: bool) -> CmdResult<()> {
    let list = strays::find(dir, m);
    if !list.is_empty() {
        let stamp = log::timestamp().replace([':', ' '], "-");
        let dest = strays::move_aside(dir, &list, &stamp).map_err(|e| format!("Couldn't move old mod files out of the way ({e}). Close Skyrim and try again."))?;
        log::line(&format!("play: moved {} file(s) from other mods to {}: {}", list.len(), dest.display(), list.join(", ")));
    }
    let mut set_aside = list.len();
    // Files of mods it must keep that 0.1.38's sweep moved go back first.
    match launcher_core::allowlist::restore_kept(dir) {
        Ok(back) if !back.is_empty() => log::line(&format!("play: put back {} file(s) of required or listed mods that were set aside: {}", back.len(), back.join(", "))),
        Ok(_) => {}
        Err(e) => log::line(&format!("play: couldn't put set-aside files back: {e}")),
    }
    if only_server_mods {
        let others = launcher_core::allowlist::unlisted_vortex_files(dir, m);
        if !others.is_empty() {
            let stamp = format!("{}-other-mods", log::timestamp().replace([':', ' '], "-"));
            let dest = strays::move_aside(dir, &others, &stamp).map_err(|e| format!("Couldn't set your other mods aside ({e}). Close Skyrim and Vortex, then try again."))?;
            log::line(&format!("play: server's mods only, set aside {} file(s) from other Vortex mods to {}: {}", others.len(), dest.display(), others.join(", ")));
            set_aside += others.len();
        }
    }
    if set_aside > 0 {
        let _ = app.emit("mods-set-aside", set_aside);
    }
    if let Some(txt) = plugins_txt(app) {
        let extras = loadorder::extras(dir, &txt, m);
        if !extras.is_empty() {
            let names: Vec<String> = extras.iter().map(|e| e.name.clone()).collect();
            loadorder::switch_off(&txt, &names).map_err(|e| format!("Couldn't change your load order ({e}). Close Skyrim and Vortex, then try again."))?;
            log::line(&format!("play: switched off in {}: {}", txt.display(), extras.iter().map(|e| e.describe()).collect::<Vec<_>>().join(", ")));
        }
    }
    // Plugins made for a newer Skyrim (Creation Club downloads Steam updated
    // after the downgrade) and broken stub plugins crash the game while it
    // loads data, even switched off. The game skips Creation Club files that
    // aren't in Data, so they go to the backup folder with their archives.
    let mut aside: Vec<String> = Vec::new();
    for (n, why) in loadorder::too_new(dir) {
        log::line(&format!("play: {n} is for a newer Skyrim ({why})"));
        aside.extend(loadorder::with_archives(dir, &n));
    }
    if let Ok(rd) = std::fs::read_dir(dir.join("Data")) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            let l = n.to_ascii_lowercase();
            if (l.ends_with(".esp") || l.ends_with(".esm") || l.ends_with(".esl")) && loadorder::broken(&e.path()).is_some() {
                aside.push(format!("Data/{n}"));
            }
        }
    }
    if !aside.is_empty() {
        let stamp = format!("{}-plugins", log::timestamp().replace([':', ' '], "-"));
        let dest = strays::move_aside(dir, &aside, &stamp).map_err(|e| format!("Couldn't move plugins out of the way ({e}). Close Skyrim and Vortex, then try again."))?;
        log::line(&format!("play: moved {} plugin file(s) to {}: {}", aside.len(), dest.display(), aside.join(", ")));
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
    // Required and listed mods' plugins (SkyUI, the Unofficial Patch) only
    // work switched on; Vortex does this, the launcher's own installs don't.
    if let Some(txt) = plugins_txt(app) {
        match loadorder::switch_on(&txt, &loadorder::wanted(dir)) {
            Ok(on) if !on.is_empty() => log::line(&format!("play: switched on in {}: {}", txt.display(), on.join(", "))),
            Ok(_) => {}
            Err(e) => log::line(&format!("play: couldn't switch plugins on: {e}")),
        }
        match loadorder::fix_order(&txt.with_file_name("loadorder.txt")) {
            Ok(true) => log::line("play: put the five base masters first in loadorder.txt"),
            Ok(false) => {}
            Err(e) => log::line(&format!("play: couldn't fix loadorder.txt: {e}")),
        }
    }
    restore_crash_logger(dir);
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
            Ok(_) => {}
            Err(e) => log::line(&format!("play: couldn't clean {}: {e}", ini.display())),
        }
    }
    Ok(())
}

/// Required mods (Timothy, 2026-09-26): the ones on GitHub are installed here
/// when missing; the ones only on Nexus Mods stop Play with
/// "NEEDS_NEXUS_MODS:<json list>" and the UI walks the player through them.
/// Installs SKSE 2.2.6 and Crash Logger from their official GitHub releases
/// when they're missing.
async fn install_missing_mods(http: &reqwest::Client, dir: &std::path::Path) -> CmdResult<()> {
    let cleaned = requirements::clean_partials(dir);
    if !cleaned.is_empty() {
        log::line(&format!("removed half-written mod files: {}", cleaned.join(", ")));
    }
    if !requirements::skse_ok(dir) {
        match requirements::install_skse(http, dir).await {
            Ok(()) => log::line(&format!("installed SKSE {}", requirements::SKSE_VERSION)),
            Err(e) => {
                log::line(&format!("couldn't install SKSE: {e}"));
                return Err(format!("Couldn't install SKSE {} ({e}). Check your internet connection and try again.", requirements::SKSE_VERSION));
            }
        }
    }
    if !requirements::crash_logger_ok(dir) {
        match requirements::install_crash_logger(http, dir).await {
            Ok(()) => log::line(&format!("installed Crash Logger {}", requirements::CRASH_LOGGER_VERSION)),
            Err(e) => log::line(&format!("couldn't install Crash Logger: {e}")),
        }
    }
    if !requirements::souls_ok(dir) {
        match requirements::install_souls(http, dir).await {
            Ok(()) => log::line(&format!("installed Skyrim Souls RE {}", requirements::SOULS_VERSION)),
            Err(e) => log::line(&format!("couldn't install Skyrim Souls RE: {e}")),
        }
    }
    Ok(())
}

async fn ensure_requirements(state: &AppState, dir: &std::path::Path) -> CmdResult<()> {
    install_missing_mods(&state.http, dir).await?;
    let list = mods::full_list(state).await;
    let missing: Vec<mods::Row> = launcher_core::modlist::missing(&list, dir).into_iter().map(|m| mods::row(m, dir)).collect();
    if !missing.is_empty() {
        log::line(&format!("play: stopped, {} mod(s) from the mod list are missing", missing.len()));
        return Err(format!("NEEDS_NEXUS_MODS:{}", serde_json::to_string(&missing).unwrap_or_default()));
    }
    Ok(())
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

/// Launchers before 0.1.20 moved crash loggers aside with other SKSE plugins.
/// Puts the newest one back, so the next crash names the module that failed.
fn restore_crash_logger(dir: &std::path::Path) {
    let plugins = dir.join("Data").join("SKSE").join("Plugins");
    if strays::CRASH_LOGGERS.iter().any(|n| plugins.join(n).is_file()) {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir.join(strays::DISABLED_DIR)) else { return };
    let mut stamps: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    stamps.sort();
    for stamp in stamps.iter().rev() {
        let from = stamp.join("Data").join("SKSE").join("Plugins").join("CrashLogger.dll");
        if from.is_file() {
            match std::fs::rename(&from, plugins.join("CrashLogger.dll")) {
                Ok(()) => log::line(&format!("play: put the crash logger back from {}", stamp.display())),
                Err(e) => log::line(&format!("play: couldn't put the crash logger back: {e}")),
            }
            return;
        }
    }
}

/// Staff reports go to the login service's crash-report endpoint, signed
/// with the player's launcher token (live on the server since 2026-09-26).
/// Set to false to only show and log reports.
const HEALTH_REPORTS_ON: bool = true;
/// The endpoint takes at most 60 KB.
const HEALTH_REPORT_MAX: usize = 58_000;

fn health_report_url() -> Option<String> {
    HEALTH_REPORTS_ON.then(|| format!("{AUTH_URL}/api/crash-reports"))
}

/// What staff sent back: the report number and the likely cause.
#[derive(Clone, Default)]
struct Filed {
    report_id: Option<String>,
    likely_cause: Option<String>,
}

async fn run_health(app: &AppHandle, http: &reqwest::Client, base: &str, dir: &std::path::Path, m: Option<&Manifest>) -> health::Report {
    let url = format!("{}/masters.json", base.trim_end_matches('/'));
    let masters = match http.get(&url).timeout(std::time::Duration::from_secs(8)).send().await.and_then(|r| r.error_for_status()) {
        Ok(r) => r.json::<serde_json::Value>().await.ok(),
        Err(e) => {
            log::line(&format!("health: couldn't load {url}: {e}"));
            None
        }
    };
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
    if let Some(t) = body["text"].as_str() {
        let lines: Vec<&str> = t.lines().filter(|l| !bad(l)).collect();
        body["text"] = lines.join("\n").into();
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

async fn send_health(app: &AppHandle, http: &reqwest::Client, config: &Config, build: &str, when: &str, crash: Option<&str>, r: &health::Report) -> Filed {
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

/// Runs the checks for the Settings screen.
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

/// The player's load order file (Vortex and the game both use it).
fn plugins_txt(app: &AppHandle) -> Option<PathBuf> {
    app.path().local_data_dir().ok().map(|d| d.join("Skyrim Special Edition").join("plugins.txt"))
}

/// SKSE/Platform plugins the server didn't ship, then plugins switched on in
/// the load order that aren't base game, Creation Club or the server's.
fn all_strays(app: &AppHandle, dir: &std::path::Path, m: &Manifest) -> Vec<String> {
    let mut list = strays::find(dir, m);
    if let Some(txt) = plugins_txt(app) {
        list.extend(loadorder::extras(dir, &txt, m).iter().map(|e| e.describe()));
    }
    list
}

/// Moves plugins the server didn't ship into .aetherial-dawn/disabled/<time>/
/// in the game folder, so they can be put back by hand, and switches extra
/// plugins off in plugins.txt (the files stay in Data).
#[tauri::command]
async fn move_strays(app: AppHandle, state: State<'_, AppState>) -> CmdResult<String> {
    let dir = game_dir(&state).await?;
    let m = state.manifest.lock().await.clone().ok_or("Check for updates first.")?;
    let list = strays::find(&dir, &m);
    let mut said = Vec::new();
    if !list.is_empty() {
        let stamp = log::timestamp().replace([':', ' '], "-");
        let dest = strays::move_aside(&dir, &list, &stamp).map_err(|e| format!("Couldn't move the plugins ({e}). Close Skyrim and try again."))?;
        log::line(&format!("moved {} plugin(s) to {}: {}", list.len(), dest.display(), list.join(", ")));
        said.push(format!("Moved to {}", dest.display()));
    }
    if let Some(txt) = plugins_txt(&app) {
        let extras = loadorder::extras(&dir, &txt, &m);
        if !extras.is_empty() {
            let names: Vec<String> = extras.iter().map(|e| e.name.clone()).collect();
            loadorder::switch_off(&txt, &names).map_err(|e| format!("Couldn't change your load order ({e}). Close Skyrim and Vortex, then try again."))?;
            log::line(&format!("switched off in {}: {}", txt.display(), extras.iter().map(|e| e.describe()).collect::<Vec<_>>().join(", ")));
            said.push(format!("Switched off {} in your load order. If you use Vortex, switch {} off in its Plugins tab too, or it switches {} back on.", names.join(", "), if names.len() == 1 { "it" } else { "them" }, if names.len() == 1 { "it" } else { "them" }));
        }
    }
    Ok(said.join(" "))
}

/// For players who already put the right build in place themselves.
#[tauri::command]
async fn mark_game_ok(state: State<'_, AppState>) -> CmdResult<version::GameCheck> {
    let dir = game_dir(&state).await?;
    let m = state.manifest.lock().await.clone().ok_or("Check for updates first.")?;
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

#[tauri::command]
async fn open_game_folder(state: State<'_, AppState>) -> CmdResult<()> {
    let dir = game_dir(&state).await?;
    let opener = if cfg!(windows) { "explorer" } else { "xdg-open" };
    std::process::Command::new(opener).arg(dir).spawn().map(|_| ()).map_err(|e| e.to_string())
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
    let mut out = match file {
        Some(serde_json::Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    };
    let seen = health.as_ref().and_then(|h| h.get("gameServerSeen")).and_then(|v| v.as_bool());
    // Log only changes, so a 30-second poll doesn't flood the log.
    if HEALTH_OK.swap(seen.is_some(), Ordering::Relaxed) != seen.is_some() {
        log::line(&match seen {
            Some(_) => format!("server status: {health_url} answering again"),
            None => format!("server status: no answer from {health_url}"),
        });
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
            if let Some(m) = &manifest {
                let st = all_strays(&app, dir, m);
                let _ = writeln!(o, "Plugins not from the server: {}", if st.is_empty() { "none".into() } else { st.join(", ") });
            }
            let settings = dir.join(settings::SETTINGS_PATH);
            let _ = writeln!(o, "skymp5-client-settings.txt: {}", if settings.exists() { "present" } else { "not written yet" });
            o.push_str(&game_data_report(&app, dir));
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = make_patches_cli(&args) {
        std::process::exit(code);
    }
    tauri::Builder::default()
        // Windows starts the launcher again for each nxm:// link; hand the
        // link to the running one.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| mods::on_second_launch(app, &args)))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            if let Ok(dir) = app.path().app_local_data_dir() {
                log::init(dir.join("logs"));
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
            let http = reqwest::Client::builder()
                .user_agent(concat!("AetherialDawnLauncher/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(std::time::Duration::from_secs(10))
                .build()?;
            app.manage(AppState { config: Mutex::new(config), manifest: Mutex::new(None), http, steam_child: Default::default(), steam_input: Default::default(), mods: Default::default(), music: music::Music::new() });
            mods::restore_left_handler(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_state, set_game_dir, set_prefs, check, update, play, files, open_game_folder, server_status, game_check, downgrade, mark_game_ok, auth_status, auth_begin, auth_poll, auth_sign_out, log_ui, open_log_folder, diagnostics, steam_app_state, steam_app_begin, steam_app_install, move_strays, last_game_report, health_check, steam_login_answer, steam_login_cancel, patch_game, build_patches, music_start, set_music, mods::open_mod_page, mods::mods_state, mods::nexus_sign_in, mods::nexus_sso, mods::nexus_copy_sign_in, mods::nexus_sso_cancel, mods::nexus_sign_out, mods::open_nexus_key_page, mods::cancel_mods, mods::download_all_mods, restore_set_aside])
        .run(tauri::generate_context!())
        .expect("error while running the launcher");
}
