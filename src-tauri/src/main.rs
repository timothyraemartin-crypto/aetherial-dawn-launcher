// Hides the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use launcher_core::{auth, downgrade, game, manifest::Manifest, settings, steamapp, strays, sync, version, watch, Error};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;

mod log;

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
        }
    }
}

struct AppState {
    config: Mutex<Config>,
    manifest: Mutex<Option<Manifest>>,
    http: reqwest::Client,
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
}

#[tauri::command]
async fn set_prefs(app: AppHandle, state: State<'_, AppState>, prefs: Prefs) -> CmdResult<()> {
    let mut config = state.config.lock().await;
    config.close_on_launch = prefs.close_on_launch;
    config.background_updates = prefs.background_updates;
    save_config(&app, &config)
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
async fn check(state: State<'_, AppState>, verify_all: bool) -> CmdResult<CheckResult> {
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
        game: version::check(&dir, m.game.as_ref()),
        strays: strays::find(&dir, &m),
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
    let gc = version::check(&dir, m.game.as_ref());
    log::line(&format!("play: game folder {}, build {}, version needed={} skseOk={}", dir.display(), m.build, gc.needed, gc.skse_ok));
    if gc.needed {
        return Err(gc.reason.unwrap_or_else(|| "Your Skyrim version doesn't match the server.".into()));
    }
    game::inspect(&dir).map_err(err)?;
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
    game::launch(&dir).map_err(err)?;
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
struct GameEnded {
    crashed: bool,
    summary: String,
    report: String,
}

fn reports_dir() -> Option<PathBuf> {
    log::path().and_then(|p| p.parent().map(|d| d.to_path_buf()))
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
        let _ = app.emit("game-ended", GameEnded { crashed, summary, report });
    } else if close_on_launch {
        app.exit(0);
    } else {
        let _ = app.emit("game-ended", GameEnded { crashed, summary, report });
    }
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
    Ok(version::check(&dir, m.as_ref().and_then(|m| m.game.as_ref())))
}

/// Downloads the server's Skyrim build from Steam with the player's own
/// account, through DepotDownloader in its own window. Sends
/// `downgrade-stage` events ("tool", "steam", "verify") to the UI.
#[tauri::command]
async fn downgrade(app: AppHandle, state: State<'_, AppState>, username: Option<String>) -> CmdResult<version::GameCheck> {
    let dir = game_dir(&state).await?;
    let m = state.manifest.lock().await.clone().ok_or("Check for updates first.")?;
    let spec = m.game.clone().ok_or("The server doesn't ask for a particular Skyrim version.")?;
    let login = match username.as_deref().map(str::trim) {
        Some(u) if !u.is_empty() => downgrade::Login::User(u.to_string()),
        _ => downgrade::Login::Qr,
    };
    let args = downgrade::args(&spec, &dir, &login).map_err(err)?;
    log::line(&format!("downgrade: to {} in {}, DepotDownloader {}", spec.version.as_deref().unwrap_or("?"), dir.display(), args.join(" ")));
    let tools = app.path().app_local_data_dir().map_err(|e| e.to_string())?.join("tools");
    let _ = app.emit("downgrade-stage", "tool");
    let tool = downgrade::ensure_tool(&state.http, &tools, spec.tool.as_ref()).await.map_err(err)?;
    log::line(&format!("downgrade: tool ready at {}", tool.display()));
    let _ = app.emit("downgrade-stage", "steam");
    downgrade::run(&tool, &args, &tools).await.map_err(err)?;
    log::line("downgrade: Steam download window closed");
    let _ = app.emit("downgrade-stage", "verify");
    finish_downgrade(&dir, &spec, false)
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

/// Moves plugins the server didn't ship into .aetherial-dawn/disabled/<time>/
/// in the game folder, so they can be put back by hand.
#[tauri::command]
async fn move_strays(state: State<'_, AppState>) -> CmdResult<String> {
    let dir = game_dir(&state).await?;
    let m = state.manifest.lock().await.clone().ok_or("Check for updates first.")?;
    let list = strays::find(&dir, &m);
    let stamp = log::timestamp().replace([':', ' '], "-");
    let dest = strays::move_aside(&dir, &list, &stamp).map_err(|e| format!("Couldn't move the plugins ({e}). Close Skyrim and try again."))?;
    log::line(&format!("moved {} plugin(s) to {}: {}", list.len(), dest.display(), list.join(", ")));
    Ok(dest.display().to_string())
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
    }
    Ok(version::check(dir, Some(spec)))
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

/// Optional `status.json` for the side panel. Missing or broken is fine.
#[tauri::command]
async fn server_status(state: State<'_, AppState>) -> CmdResult<Option<serde_json::Value>> {
    let base = state.config.lock().await.base_url.clone();
    let url = format!("{}/status.json", base.trim_end_matches('/'));
    let Ok(resp) = state.http.get(url).send().await.and_then(|r| r.error_for_status()) else {
        return Ok(None);
    };
    Ok(resp.json().await.ok())
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
                let st = strays::find(dir, m);
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

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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
            app.manage(AppState { config: Mutex::new(config), manifest: Mutex::new(None), http });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_state, set_game_dir, set_prefs, check, update, play, files, open_game_folder, server_status, game_check, downgrade, mark_game_ok, auth_status, auth_begin, auth_poll, auth_sign_out, log_ui, open_log_folder, diagnostics, steam_app_state, steam_app_begin, steam_app_install, move_strays, last_game_report])
        .run(tauri::generate_context!())
        .expect("error while running the launcher");
}
