// Hides the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use launcher_core::{auth, downgrade, game, manifest::Manifest, settings, sync, version, Error};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;

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
    };
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
    if gc.needed {
        return Err(gc.reason.unwrap_or_else(|| "Your Skyrim version doesn't match the server.".into()));
    }
    game::inspect(&dir).map_err(err)?;
    let token = token(&app).ok_or("SIGNED_OUT:Sign in with Discord to play.")?;
    let session = match auth::play(&state.http, AUTH_URL, &token).await {
        auth::Answer::Ok(p) => p.session,
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
    game::launch(&dir).map_err(err)?;
    if config.close_on_launch {
        app.exit(0);
    }
    Ok(())
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

async fn sign_out(app: &AppHandle, state: &AppState) {
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
    match auth::me(&state.http, AUTH_URL, &token).await {
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
    Ok(match auth::poll(&state.http, AUTH_URL, &st).await {
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
    let tools = app.path().app_local_data_dir().map_err(|e| e.to_string())?.join("tools");
    let _ = app.emit("downgrade-stage", "tool");
    let tool = downgrade::ensure_tool(&state.http, &tools, spec.tool.as_ref()).await.map_err(err)?;
    let _ = app.emit("downgrade-stage", "steam");
    downgrade::run(&tool, &args, &tools).await.map_err(err)?;
    let _ = app.emit("downgrade-stage", "verify");
    finish_downgrade(&dir, &spec)
}

/// For players who already put the right build in place themselves.
#[tauri::command]
async fn mark_game_ok(state: State<'_, AppState>) -> CmdResult<version::GameCheck> {
    let dir = game_dir(&state).await?;
    let m = state.manifest.lock().await.clone().ok_or("Check for updates first.")?;
    let spec = m.game.clone().ok_or("The server doesn't ask for a particular Skyrim version.")?;
    finish_downgrade(&dir, &spec)
}

fn finish_downgrade(dir: &std::path::Path, spec: &launcher_core::manifest::GameSpec) -> CmdResult<version::GameCheck> {
    let want = spec.version.as_deref().and_then(version::parse_version);
    let have = version::exe_version(&dir.join(game::GAME_EXE));
    if want.is_some() && have != want {
        return Err(format!(
            "SkyrimSE.exe is still {}, not {}.",
            have.map(version::short).unwrap_or_else(|| "unreadable".into()),
            want.map(version::short).unwrap_or_default()
        ));
    }
    version::record(dir, spec).map_err(err)?;
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

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config = load_config(app.handle());
            save_config(app.handle(), &config)?;
            let http = reqwest::Client::builder()
                .user_agent(concat!("AetherialDawnLauncher/", env!("CARGO_PKG_VERSION")))
                .connect_timeout(std::time::Duration::from_secs(10))
                .build()?;
            app.manage(AppState { config: Mutex::new(config), manifest: Mutex::new(None), http });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_state, set_game_dir, set_prefs, check, update, play, files, open_game_folder, server_status, game_check, downgrade, mark_game_ok, auth_status, auth_begin, auth_poll, auth_sign_out])
        .run(tauri::generate_context!())
        .expect("error while running the launcher");
}
