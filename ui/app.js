// Launcher UI. Talks to the Rust side in src-tauri/src/main.rs through invoke().
(() => {
  const T = window.__TAURI__;
  // Every command's failure (and the outcome of the important ones) goes to the
  // launcher log, so Copy diagnostics shows what happened. Nothing secret
  // reaches the UI, so nothing secret can be logged from here.
  const QUIET = new Set(['log_ui', 'diagnostics', 'files', 'server_status', 'auth_poll']);
  const logUi = msg => { try { T.core.invoke('log_ui', { msg: String(msg) }).catch(() => {}); } catch {} };
  const invoke = async (cmd, args) => {
    const t = performance.now();
    try {
      const r = await T.core.invoke(cmd, args);
      if (!QUIET.has(cmd)) logUi(`${cmd} ok in ${Math.round(performance.now() - t)} ms${r && typeof r === 'object' ? ' ' + JSON.stringify(r).slice(0, 400) : ''}`);
      return r;
    } catch (e) {
      if (cmd !== 'log_ui') logUi(`${cmd} FAILED after ${Math.round(performance.now() - t)} ms: ${e}`);
      throw e;
    }
  };
  window.addEventListener('error', e => logUi(`script error: ${e.message} at ${e.filename}:${e.lineno}`));
  window.addEventListener('unhandledrejection', e => logUi(`unhandled: ${e.reason}`));
  const HELP = ' If it keeps happening, open Settings, click Copy diagnostics and send it to staff.';
  const $ = id => document.getElementById(id);

  const ICON_OK = '<path d="M5 12.5l4.5 4.5L19 7.5"/>';
  const ICON_BAD = '<circle cx="12" cy="12" r="9"/><path d="M12 7.5v5.5M12 16.5v.01"/>';
  const ICON_BUSY = '<path d="M20 12a8 8 0 1 1-2.3-5.7"/><path d="M20 4v5h-5"/>';
  const THUMBS = ['art/thumb-castle.jpg', 'art/thumb-peak.jpg', 'art/thumb-lake.jpg', 'art/thumb-city.jpg'];

  let state = null;       // get_state
  let pending = null;     // check
  let status = null;      // status.json
  let gameCheck = null;
  let skipArmed = false;
  let auth = null;        // auth_status: Discord sign-in   // check().game: is Skyrim the build the server needs?
  let busy = false;
  let playMode = 'wait';  // wait | play | update | retry | downgrade | signin
  let page = 'home';

  const plural = (n, w) => `${n} ${w}${n === 1 ? '' : 's'}`;
  const mb = n => (n / 1048576).toFixed(n < 10485760 ? 1 : 0) + ' MB';
  const esc = s => String(s ?? '').replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

  // ---------- play button + status line ----------
  function setPlay(mode, label) {
    playMode = mode;
    $('play-label').textContent = label;
    $('play').disabled = mode === 'wait';
    $('play-wrap').classList.toggle('off', mode === 'wait');
  }
  let statusMsg = null;
  function setStatus(msg, isError) { statusMsg = msg ? { msg, isError } : null; renderStatus(); }
  function renderStatus() {
    const parts = [];
    if (statusMsg) parts.push(`<span${statusMsg.isError ? ' class="error"' : ''}>${esc(statusMsg.msg)}</span>`);
    else {
      // Live from the login service's /health (refreshed every 30 s); grey when it can't be reached.
      const known = status && typeof status.online === 'boolean';
      const on = known && status.online;
      const who = on && typeof status.players === 'number' ? ` · ${status.maxPlayers ? `${status.players} of ${plural(status.maxPlayers, 'player')}` : plural(status.players, 'player')}` : '';
      parts.push(`<span><i class="dot${on ? '' : ' off'}"></i>${!known ? 'Server status unavailable' : on ? 'Server online' : 'Server offline'}${who}</span>`);
      if (pending) parts.push(`<span>Build ${esc(pending.build)}</span>`);
    }
    if (state) parts.push(`<span>v${esc(state.launcherVersion)}</span>`);
    $('status').innerHTML = parts.join('');
  }
  function setChip(kind, text) {
    const c = $('mods-chip');
    c.className = 'chip ' + (kind === 'ok' ? '' : kind);
    c.querySelector('svg').innerHTML = kind === 'ok' ? ICON_OK : kind === 'warn' ? ICON_BAD : ICON_BUSY;
    c.querySelector('span').textContent = text;
  }

  // ---------- pages + sheets ----------
  const PAGES = { home: 'nav-home', server: 'nav-server', mods: 'nav-mods', news: 'nav-news' };
  function showPage(p) {
    page = p;
    showSheet(null);
    for (const [name, nav] of Object.entries(PAGES)) {
      $('page-' + name).hidden = name !== p;
      if (name === p) $(nav).setAttribute('aria-current', 'page'); else $(nav).removeAttribute('aria-current');
    }
    $('nav-settings').removeAttribute('aria-current');
    $('news-box').hidden = p !== 'home';
    $('tagline').hidden = p !== 'home';
    if (p === 'mods') loadFiles();
  }
  function showSheet(id) {
    $('first').hidden = id !== 'first';
    $('settings').hidden = id !== 'settings';
    $('downgrade').hidden = id !== 'downgrade';
    $('signin').hidden = id !== 'signin';
    $('strays').hidden = id !== 'strays';
    $('crash').hidden = id !== 'crash';
    $('health').hidden = id !== 'health';
    if (id === 'settings') {
      for (const nav of Object.values(PAGES)) $(nav).removeAttribute('aria-current');
      $('nav-settings').setAttribute('aria-current', 'page');
    }
  }
  const ready_ = () => state.game && state.game.hasSkse;
  const signedIn = () => !!(auth && auth.signedIn);
  function leaveSheet() {
    if (!ready_()) { renderGame(); showSheet('first'); return; }
    if (!signedIn()) { showSignIn(); return; }
    showPage(page);
    if (!pending && !busy) check();
  }

  function renderRow(el, ok, title, detail) {
    el.classList.remove('ok', 'bad'); el.classList.add(ok ? 'ok' : 'bad');
    el.querySelector('svg').innerHTML = ok ? ICON_OK : ICON_BAD;
    el.querySelector('b').textContent = title;
    el.querySelector('small').textContent = detail;
  }
  function renderGame() {
    const g = state.game;
    const gameRow = [!!g, g ? 'Skyrim Special Edition' : 'Skyrim not found', g ? g.dir : (state.gameError || 'Pick the folder that has SkyrimSE.exe in it.')];
    const skseRow = [!!(g && g.hasSkse), g && g.hasSkse ? 'SKSE installed' : 'SKSE is missing', g && g.hasSkse ? 'skse64_loader.exe' : 'skse64_loader.exe was not found in the game folder'];
    renderRow($('g-game'), ...gameRow); renderRow($('g-skse'), ...skseRow);
    renderVersion();
    renderRow($('c-game'), ...gameRow); renderRow($('c-skse'), ...skseRow);
    $('c-game-pick').textContent = g ? 'Change' : 'Choose folder';
    $('c-skse-recheck').hidden = !!(g && g.hasSkse);
    $('f-go').disabled = !ready_();
  }

  const shortVer = v => (v || '').split('.').slice(0, 3).join('.');
  function renderVersion() {
    const c = gameCheck, g = state && state.game;
    const row = $('g-ver');
    if (!c || !g) { renderRow(row, true, 'Skyrim version', 'Checked after the server file list loads.'); $('g-downgrade').hidden = true; return; }
    const have = c.installed ? 'Skyrim ' + shortVer(c.installed) : 'Skyrim version unknown';
    renderRow(row, !c.needed, have, c.needed ? c.reason : c.target ? `Matches the server (${shortVer(c.target)}).` : 'The server accepts any version.');
    // Always reachable, so a player can re-download the right build even after
    // the check was satisfied (for example by "already on this version").
    $('g-downgrade').hidden = !c.canDowngrade;
    $('g-downgrade').textContent = c.needed || c.warning ? 'Fix version' : 'Re-download';
    if (c.warning) renderRow(row, false, have, c.warning);
    if (g.hasSkse && c.target && !c.skseOk) {
      const skse = [false, "SKSE doesn't match", `Install SKSE ${c.skseVersion || ''} for Skyrim ${shortVer(c.target)}. ${c.skseDll} is missing.`.replace('  ', ' ')];
      renderRow($('g-skse'), ...skse); renderRow($('c-skse'), ...skse);
    }
  }

  async function refreshState() {
    state = await invoke('get_state');
    $('set-path').value = state.config.gameDir || '';
    $('set-close').setAttribute('aria-checked', state.config.closeOnLaunch);
    $('set-bg').setAttribute('aria-checked', state.config.backgroundUpdates);
    $('set-share').setAttribute('aria-checked', state.config.shareHealth !== false);
    $('set-version').textContent = 'Launcher ' + state.launcherVersion;
    $('ver').textContent = 'Launcher v' + state.launcherVersion;
    renderGame();
    renderStatus();
    return state;
  }

  async function pickFolder() {
    const dir = await T.dialog.open({ directory: true, title: 'Choose your Skyrim Special Edition folder' });
    if (!dir) return;
    try {
      await invoke('set_game_dir', { dir });
      $('set-error').hidden = true;
    } catch (e) {
      $('set-error').textContent = e;
      $('set-error').hidden = false;
    }
    await refreshState();
    pending = null;
  }

  // ---------- update flow ----------
  // While the server can't be reached (or hasn't published files yet), try again every minute.
  let retryTimer = 0;
  function scheduleRetry() {
    clearTimeout(retryTimer);
    retryTimer = setTimeout(() => { if (playMode === 'retry' && !busy) check(); }, 60000);
  }
  async function check(verifyAll = false) {
    if (busy) return;
    busy = true;
    setPlay('wait', 'CHECKING');
    setChip('busy', 'Checking');
    setStatus('Checking for updates…');
    try {
      pending = await invoke('check', { verifyAll });
      gameCheck = pending.game;
      renderGame();
      $('srv-name').textContent = pending.server.name;
      $('srv-addr').textContent = `${pending.server.ip}:${pending.server.port}`;
      $('srv-build').textContent = pending.build;
      if (pending.files || pending.remove) {
        busy = false;
        if (state.config.backgroundUpdates || verifyAll) return await update(verifyAll);
        setPlay('update', 'UPDATE');
        setChip('warn', 'Update available');
        setStatus(`Build ${pending.build} available · ${plural(pending.files, 'file')} · ${mb(pending.bytes)}`);
      } else {
        ready();
      }
    } catch (e) {
      setPlay('retry', 'RETRY');
      pending = null;
      if (String(e).includes("hasn't published")) {
        setChip('warn', 'Server not ready');
        setStatus("The server is still being set up. The launcher will check again in a minute.");
      } else {
        setChip('warn', 'Not checked');
        setStatus(`Couldn't reach the Aetherial Dawn server (${e}). Check your internet; the launcher tries again every minute.` + HELP, true);
      }
      scheduleRetry();
    } finally {
      busy = false;
    }
  }

  async function update(verifyAll = false) {
    if (busy) return;
    busy = true;
    setPlay('wait', 'UPDATING');
    setChip('busy', 'Updating');
    setStatus(`Updating to build ${pending.build} · ${mb(pending.bytes)}`);
    $('progress').hidden = false;
    let last = { t: performance.now(), b: 0 };
    const off = await T.event.listen('sync-progress', ({ payload: p }) => {
      const pct = p.bytesTotal ? (p.bytesDone / p.bytesTotal) * 100 : 100;
      $('p-bar').style.width = pct.toFixed(1) + '%';
      $('p-num').textContent = `${Math.min(p.filesDone + (p.file ? 1 : 0), p.filesTotal)} / ${plural(p.filesTotal, 'file')} · ${Math.round(pct)}%`;
      $('p-file').textContent = p.file || 'All files match the server';
      const now = performance.now();
      if (now - last.t > 500) {
        $('p-speed').textContent = mb(((p.bytesDone - last.b) / (now - last.t)) * 1000) + '/s';
        last = { t: now, b: p.bytesDone };
      }
    });
    try {
      await invoke('update', { verifyAll });
      ready();
    } catch (e) {
      setPlay('retry', 'RETRY');
      setChip('warn', 'Update failed');
      setStatus(`The game file update stopped: ${e}. Click Retry.` + HELP, true);
      pending = null;
    } finally {
      off();
      $('progress').hidden = true;
      busy = false;
    }
  }

  function ready() {
    pending = { ...pending, files: 0, remove: 0 };
    setChip('ok', 'Up to date');
    loadFiles();
    renderGame();
    const c = gameCheck;
    if (c && c.needed) {
      if (c.canDowngrade) { setPlay('downgrade', 'FIX VERSION'); setStatus(c.reason + ' Click Fix version to download it from Steam.', true); }
      else { setPlay('wait', 'WRONG VERSION'); setStatus(c.reason, true); }
      return;
    }
    if (!signedIn()) { setPlay('signin', 'SIGN IN'); setStatus('Sign in with Discord to play.', true); return; }
    if (auth.locked) { setPlay('wait', 'OFFLINE'); setStatus(auth.message, true); return; }
    setPlay('play', 'PLAY');
    if (c && c.warning) setStatus(c.warning, true);
    if (c && c.target && !c.skseOk) setStatus(`SKSE for Skyrim ${shortVer(c.target)} is missing. Install SKSE ${c.skseVersion || ''} from skse.silverlock.org.`, true);
    else setStatus(null);
  }

  // ---------- discord sign-in ----------
  function paintAvatar(el, a) {
    const name = (a && a.discordUsername) || '?';
    if (a && a.discordAvatar && /^https:\/\//.test(a.discordAvatar)) {
      el.style.backgroundImage = `url("${a.discordAvatar.replace(/["\\]/g, '')}")`;
      el.textContent = '';
    } else {
      el.style.backgroundImage = '';
      el.textContent = name.slice(0, 1).toUpperCase();
    }
  }
  function renderAccount() {
    const a = signedIn() ? auth.account : null;
    $('me').hidden = !a;
    $('set-account').hidden = !a;
    if (!a) return;
    $('me-name').textContent = $('acc-name').textContent = a.discordUsername || 'Discord user';
    $('acc-note').textContent = auth.offline ? 'Signed in with Discord (not re-checked yet)' : 'Signed in with Discord';
    paintAvatar($('me-avatar'), a); paintAvatar($('acc-avatar'), a);
  }
  async function refreshAuth() {
    try { auth = await invoke('auth_status'); }
    catch (e) { auth = { signedIn: false, message: String(e) }; }
    renderAccount();
    return auth;
  }
  // Every 10 minutes: a ban or leaving the Discord signs the player out here.
  async function recheckAuth() {
    if (!signedIn() || busy) return;
    await refreshAuth();
    if (!signedIn()) { ready(); showSignIn(auth.message); }
    else if (playMode === 'play' || playMode === 'wait') ready();
  }
  function showSignIn(message) {
    $('si-error').textContent = message || '';
    $('si-error').hidden = !message;
    $('si-wait').hidden = true;
    $('si-go').disabled = false;
    showSheet('signin');
  }
  let signInRun = 0;
  async function beginSignIn() {
    const run = ++signInRun;
    $('si-error').hidden = true;
    let st;
    try { st = await invoke('auth_begin'); }
    catch (e) { showSignIn("Couldn't open your browser. " + e); return; }
    $('si-go').disabled = true;
    $('si-wait').hidden = false;
    const until = Date.now() + 5 * 60 * 1000;
    while (run === signInRun && Date.now() < until) {
      await new Promise(r => setTimeout(r, 2000));
      if (run !== signInRun) return;
      let r;
      try { r = await invoke('auth_poll', { st }); } catch (e) { r = { status: 'offline', message: String(e) }; }
      if (r.status === 'pending' || r.status === 'offline') continue;
      if (r.status === 'done') {
        auth = { signedIn: true, account: r.account };
        renderAccount();
        showPage('home');
        if (pending) ready(); else check();
        return;
      }
      showSignIn(r.message || 'Sign-in didn\'t finish. Try again.');
      return;
    }
    if (run === signInRun) showSignIn('Sign-in timed out. Try again.');
  }

  // ---------- game version ----------
  function openDowngrade() {
    const c = gameCheck || {};
    $('dg-lead').textContent = `${c.reason || c.warning || ''} The launcher downloads Skyrim ${shortVer(c.target)} from Steam with your own account, then checks it.`;
    $('dg-error').hidden = true;
    $('dg-progress').hidden = true;
    skipArmed = false;
    $('dg-skip').textContent = 'My game is already on this version';
    $('dg-skip').hidden = !c.needed;
    dgMode(false);
    showSheet('downgrade');
    invoke('steam_app_state').then(st => {
      $('dg-app-note').textContent = st.running
        ? "Uses the Steam account you're already signed into. You paste three lines into Steam's console."
        : "Steam isn't open. Start Steam and sign in to use this, or pick another option.";
    }).catch(() => {});
  }
  // ---------- downgrade through the Steam app ----------
  let steamPoll = null;
  function dgMode(steam) {
    $('dg-steam').hidden = !steam;
    $('dg-install').hidden = !steam;
    $('dg-go').hidden = steam;
    document.querySelector('#downgrade .choice').hidden = steam;
    $('dg-notes').hidden = steam;
    const how = document.querySelector('input[name="dg-login"]:checked').value;
    $('dg-user-field').hidden = steam || (how !== 'user' && how !== 'here');
    $('dg-other').hidden = steam || !document.querySelector('#downgrade .opt.other[hidden]');
    if (!steam && steamPoll) { clearInterval(steamPoll); steamPoll = null; }
  }
  const gb = n => n >= 1073741824 ? (n / 1073741824).toFixed(1) + ' GB' : mb(n);
  function renderDepots(st) {
    const box = $('dg-depots');
    if (!box.children.length) {
      box.innerHTML = st.depots.map((d, i) => `<div class="depot"><code>${esc(d.command)}</code><button class="btn" data-i="${i}">Copy</button><small id="dg-d${i}"></small></div>`).join('');
      box.querySelectorAll('button').forEach(b => b.onclick = async () => {
        const d = st.depots[+b.dataset.i];
        try { await navigator.clipboard.writeText(d.command); b.textContent = 'Copied'; setTimeout(() => { b.textContent = 'Copy'; }, 1600); }
        catch { b.textContent = 'Select it'; }
      });
    }
    let ready = st.depots.length > 0;
    st.depots.forEach((d, i) => {
      const el = $('dg-d' + i);
      if (!el) return;
      if (!d.present) { el.className = ''; el.textContent = 'Waiting for you to paste this line'; ready = false; }
      else if (d.quietSecs < 10) { el.className = 'going'; el.textContent = `Steam is downloading · ${gb(d.bytes)}`; ready = false; }
      else { el.className = 'done'; el.textContent = `Downloaded · ${plural(d.files, 'file')} · ${gb(d.bytes)}`; }
    });
    if (!st.running) { $('dg-error').textContent = 'Steam closed. Open Steam again; downloads it already finished are kept.'; $('dg-error').hidden = false; }
    $('dg-install').disabled = !ready;
  }
  async function steamBegin() {
    dgBusy(true);
    $('dg-error').hidden = true;
    try {
      const st = await invoke('steam_app_begin');
      $('dg-depots').innerHTML = '';
      dgMode(true);
      renderDepots(st);
      steamPoll = setInterval(async () => { try { renderDepots(await invoke('steam_app_state')); } catch {} }, 3000);
    } catch (e) { dgFail(e); return; }
    dgBusy(false);
  }
  async function steamInstall() {
    dgBusy(true);
    $('dg-install').disabled = true;
    $('dg-error').hidden = true;
    $('dg-progress').hidden = false;
    $('dg-stage').textContent = 'Copying the files Steam downloaded into Skyrim…';
    try { const c = await invoke('steam_app_install'); dgMode(false); dgDone(c); }
    catch (e) { dgFail(e); $('dg-install').disabled = false; }
  }
  function dgBusy(on) { for (const id of ['dg-go', 'dg-cancel', 'dg-skip', 'dg-other']) $(id).disabled = on; busy = on; }
  function dgDone(c) {
    gameCheck = c;
    dgBusy(false);
    showPage(page);
    ready();
    if (!gameCheck.needed && !statusMsg) setStatus(`Skyrim ${shortVer(gameCheck.installed)} is ready for Aetherial Dawn.`);
  }
  function dgFail(e) {
    dgBusy(false);
    $('dg-progress').hidden = true;
    $('dg-bar').hidden = true;
    $('dg-error').textContent = String(e);
    $('dg-error').hidden = false;
  }
  const STAGES = {
    tool: 'Getting the Steam download tool…',
    steam: 'Sign in to Steam in the window that just opened. The download runs there, so keep it open until it finishes.',
    verify: 'Checking your game…',
  };
  // ---------- downgrade with the Steam sign-in inside the launcher ----------
  let inlineRun = false;
  function dgAsk(label, type, note) {
    $('dg-ask-label').textContent = label;
    $('dg-answer').type = type;
    $('dg-answer').value = '';
    $('dg-ask-note').textContent = note;
    $('dg-ask').hidden = false;
    $('dg-answer').focus();
  }
  function steamEvent(ev) {
    const stage = t => { $('dg-progress').hidden = false; $('dg-stage').textContent = t; };
    switch (ev.kind) {
      case 'signingIn': stage('Signing in to Steam…'); break;
      case 'password':
        stage('Steam is asking for your password.');
        dgAsk('Steam password', 'password', "It goes straight to Steam and isn't saved. Steam remembers this PC afterwards, so next time there's nothing to type.");
        break;
      case 'guardApp':
        stage('Steam Guard is asking for a code.');
        dgAsk('Steam Guard code', 'text', 'Open the Steam app on your phone and type the code it shows.');
        break;
      case 'guardEmail':
        stage('Steam Guard is asking for a code.');
        dgAsk('Steam Guard code', 'text', `Steam emailed a code to ${ev.email || 'your email address'}.`);
        break;
      case 'confirmPhone':
        $('dg-ask').hidden = true;
        stage('Approve the sign-in in the Steam app on your phone. The download starts right after.');
        break;
      case 'progress':
        $('dg-ask').hidden = true;
        $('dg-bar').hidden = false;
        $('dg-bar-i').style.width = Math.min(100, ev.percent) + '%';
        stage(`Downloading Skyrim ${shortVer((gameCheck || {}).target)} from Steam… ${ev.percent.toFixed(0)}%`);
        break;
    }
  }
  async function sendAnswer() {
    const text = $('dg-answer').value;
    if (!text.trim()) return;
    $('dg-answer').value = '';
    $('dg-ask').hidden = true;
    $('dg-stage').textContent = 'Checking with Steam…';
    try { await invoke('steam_login_answer', { text }); } catch (e) { dgFail(e); }
  }
  async function inlineDowngrade() {
    const user = $('dg-user').value.trim();
    if (!user) { dgFail('Type your Steam account name.'); return; }
    try { localStorage.setItem('ad-steam-user', user); } catch {}
    dgBusy(true);
    inlineRun = true;
    $('dg-cancel').disabled = false;
    $('dg-cancel').textContent = 'Stop';
    $('dg-error').hidden = true;
    $('dg-notes').hidden = true;
    document.querySelector('#downgrade .choice').hidden = true;
    $('dg-other').hidden = true;
    $('dg-progress').hidden = false;
    $('dg-stage').textContent = STAGES.tool;
    const offStage = await T.event.listen('downgrade-stage', ({ payload }) => {
      $('dg-stage').textContent = payload === 'steam' ? 'Connecting to Steam…' : (STAGES[payload] || '');
    });
    const offLogin = await T.event.listen('steam-login', ({ payload }) => steamEvent(payload));
    try { dgDone(await invoke('downgrade', { username: user, inline: true })); }
    catch (e) { dgFail(e); }
    finally {
      offStage(); offLogin();
      inlineRun = false;
      $('dg-ask').hidden = true;
      $('dg-bar').hidden = true;
      $('dg-cancel').textContent = 'Not now';
      $('dg-notes').hidden = false;
      document.querySelector('#downgrade .choice').hidden = false;
      dgMode(false);
    }
  }
  async function runDowngrade() {
    const how = document.querySelector('input[name="dg-login"]:checked').value;
    if (how === 'here') return inlineDowngrade();
    if (how === 'app') return steamBegin();
    const user = how === 'user' ? $('dg-user').value.trim() : null;
    if (user === '') { dgFail('Type your Steam account name, or choose the Steam mobile app.'); return; }
    dgBusy(true);
    $('dg-error').hidden = true;
    $('dg-progress').hidden = false;
    $('dg-stage').textContent = STAGES.tool;
    const off = await T.event.listen('downgrade-stage', ({ payload }) => { $('dg-stage').textContent = STAGES[payload] || ''; });
    try { dgDone(await invoke('downgrade', { username: user })); }
    catch (e) { dgFail(e); }
    finally { off(); }
  }

  // ---------- plugins the server didn't ship ----------
  let ignoreStrays = false;
  const strays = () => (pending && pending.strays) || [];
  function openStrays() {
    $('st-list').innerHTML = strays().map(f => `<li>${esc(f)}</li>`).join('');
    $('st-error').hidden = true;
    showSheet('strays');
  }
  async function moveStrays() {
    $('st-move').disabled = true;
    try {
      const dest = await invoke('move_strays');
      pending = { ...pending, strays: [] };
      showPage(page);
      ready();
      if (playMode === 'play') setStatus(`${dest} You're ready to play.`);
    } catch (e) { $('st-error').textContent = String(e); $('st-error').hidden = false; }
    finally { $('st-move').disabled = false; }
  }

  async function onPlay() {
    if (busy) return;
    if (playMode === 'strays') return openStrays();
    if (playMode === 'retry') return check();
    if (playMode === 'update') return update();
    if (playMode === 'downgrade') return openDowngrade();
    if (playMode === 'signin') return showSignIn();
    if (playMode !== 'play') return;
    setPlay('wait', 'LAUNCHING');
    setStatus('Starting Skyrim through SKSE…');
    try {
      await invoke('play');
      gameRunning = true;
      setTimeout(() => { if (playMode === 'wait' && !busy) ready(); }, 8000);
    } catch (e) {
      const msg = String(e);
      if (msg.startsWith('SIGNED_OUT:')) {
        auth = { signedIn: false };
        renderAccount();
        ready();
        showSignIn(msg.slice(11));
        return;
      }
      setPlay('play', 'PLAY');
      setStatus(`Skyrim didn't start: ${msg}` + HELP, true);
    }
  }

  // ---------- mods page ----------
  async function loadFiles() {
    const files = await invoke('files').catch(() => null);
    if (!files) {
      $('files-body').innerHTML = '<tr><td colspan="2">The file list loads after the launcher reaches the server.</td></tr>';
      return;
    }
    const total = files.reduce((n, f) => n + f.size, 0);
    $('mods-summary').textContent = `Build ${pending ? pending.build : ''} · ${plural(files.length, 'file')} · ${mb(total)}. The server decides which files every player needs.`;
    $('files-body').innerHTML = files.map(f => `<tr><td>${esc(f.path)}</td><td>${mb(f.size)}</td></tr>`).join('');
  }

  // ---------- server status.json (optional) ----------
  function newsHtml(items, full) {
    return items.map((n, i) =>
      `<article class="news-item"><img src="${THUMBS[i % THUMBS.length]}" alt="">
         <div><h3>${esc(n.title)}</h3><time>${esc(n.date)}</time><p>${esc(n.body)}</p></div></article>`).join('');
  }
  async function loadStatus() {
    status = await invoke('server_status').catch(() => null);
    renderStatus();
    const online = $('srv-online');
    if (!status) {
      online.className = 'online' + (pending ? '' : ' off');
      online.querySelector('span').textContent = pending ? 'Reachable' : 'Status unavailable';
      return;
    }
    online.className = 'online' + (status.online ? '' : ' off');
    online.querySelector('span').textContent = status.online ? 'Online' : 'Offline';
    $('srv-players').textContent = status.online && typeof status.players === 'number'
      ? (status.maxPlayers ? `${status.players} / ${status.maxPlayers}` : status.players) : '–';
    if (status.sinceReset) $('srv-reset').textContent = status.sinceReset;
    if (Array.isArray(status.news) && status.news.length) {
      $('news').innerHTML = newsHtml(status.news.slice(0, 3));
      $('news-full').innerHTML = newsHtml(status.news.slice(0, 20), true);
    }
  }

  // ---------- launcher self-update ----------
  // Installs every new launcher release by itself: on start and every
  // 15 minutes, never while Skyrim is running or a download is in progress.
  let gameRunning = false, updating = false;
  async function checkSelfUpdate() {
    if (updating || gameRunning || busy) return;
    try {
      const upd = await T.updater.check();
      if (!upd) { logUi('launcher is up to date'); return; }
      updating = true;
      $('self-update-text').textContent = `Updating the launcher to ${upd.version}…`;
      $('self-update').hidden = false;
      $('self-update-go').hidden = true;
      logUi(`installing launcher ${upd.version} automatically`);
      try {
        await upd.downloadAndInstall();
        await T.process.relaunch();
      } catch (e) {
        updating = false;
        logUi('launcher self-update failed: ' + e);
        $('self-update-go').hidden = false;
        $('self-update-go').disabled = false;
        $('self-update-text').textContent = "The launcher update didn't install. Click to try again.";
        $('self-update-go').onclick = () => { $('self-update-go').disabled = true; checkSelfUpdate(); };
      }
    } catch (e) {
      logUi('launcher self-update check failed: ' + e);
    }
  }
  setInterval(checkSelfUpdate, 15 * 60 * 1000);

  // ---------- game health ----------
  const HL_TAG = { ok: 'OK', info: 'INFO', warn: 'WARN', fail: 'FAIL' };
  let healthText = '';
  async function openHealth() {
    showSheet('health');
    $('hl-title').textContent = 'Checking your game…';
    $('hl-list').innerHTML = '';
    $('hl-note').hidden = true;
    $('hl-again').disabled = true;
    try {
      const h = await invoke('health_check');
      healthText = h.text;
      const bad = h.report.checks.filter(c => c.status === 'warn' || c.status === 'fail').length;
      $('hl-title').textContent = bad ? `${plural(bad, 'thing')} to look at` : 'Your game looks healthy';
      $('hl-list').innerHTML = h.report.checks.map(c => `<li><span class="hl-tag ${c.status}">${HL_TAG[c.status]}</span><div><b>${esc(c.title)}</b><small>${esc(c.detail)}</small>${c.items && c.items.length ? `<ul>${c.items.map(i => `<li>${esc(i)}</li>`).join('')}</ul>` : ''}</div></li>`).join('');
      $('hl-sent-sum').textContent = !h.share ? 'Sending to staff is off (Settings). This is what would be sent:'
        : h.endpoint ? 'What gets sent to staff (before Play when something is wrong, and after a crash)'
        : 'What will be sent to staff once reporting is switched on at the server';
      $('hl-payload').textContent = h.payload;
    } catch (e) {
      $('hl-title').textContent = "Couldn't check your game";
      $('hl-note').textContent = String(e);
      $('hl-note').hidden = false;
    } finally { $('hl-again').disabled = false; }
  }
  $('set-health').onclick = openHealth;
  $('hl-again').onclick = openHealth;
  $('hl-close').onclick = () => showSheet('settings');
  $('hl-copy').onclick = async () => {
    try { await navigator.clipboard.writeText(healthText); $('hl-note').textContent = 'Copied. Paste it with Ctrl+V.'; }
    catch { $('hl-note').textContent = "Couldn't copy. Use Copy diagnostics instead."; }
    $('hl-note').hidden = false;
  };

  // ---------- wiring ----------
  const win = T.window.getCurrentWindow();
  $('w-min').onclick = () => win.minimize();
  $('w-close').onclick = () => win.close();
  $('w-settings').onclick = $('nav-settings').onclick = $('t-settings').onclick = () => showSheet('settings');
  $('play').onclick = onPlay;
  for (const [name, nav] of Object.entries(PAGES)) $(nav).onclick = () => { if (ready_() && signedIn()) showPage(name); else leaveSheet(); };
  $('news-all').onclick = () => showPage('news');
  $('set-done').onclick = leaveSheet;
  $('set-browse').onclick = pickFolder;
  $('g-change').onclick = async () => { await pickFolder(); if (!ready_()) leaveSheet(); else check(); };
  $('t-verify').onclick = $('files-verify').onclick = () => check(true);
  $('t-check').onclick = () => check();
  $('t-folder').onclick = () => invoke('open_game_folder').catch(e => setStatus(String(e), true));
  $('srv-copy').onclick = async () => {
    const addr = $('srv-addr').textContent, label = $('srv-copy').querySelector('span');
    try { await navigator.clipboard.writeText(addr); label.textContent = 'Copied'; }
    catch { label.textContent = addr; }
    setTimeout(() => { label.textContent = 'Copy address'; }, 1800);
  };
  $('set-anim').setAttribute('aria-checked', Ambient.enabled);
  $('set-anim').onclick = () => { Ambient.set(!Ambient.enabled); $('set-anim').setAttribute('aria-checked', Ambient.enabled); };
  document.querySelectorAll('.switch:not(#set-anim)').forEach(s => s.onclick = async () => {
    s.setAttribute('aria-checked', s.getAttribute('aria-checked') !== 'true');
    const prefs = { closeOnLaunch: $('set-close').getAttribute('aria-checked') === 'true', backgroundUpdates: $('set-bg').getAttribute('aria-checked') === 'true', shareHealth: $('set-share').getAttribute('aria-checked') === 'true' };
    await invoke('set_prefs', { prefs });
    Object.assign(state.config, prefs);
  });
  $('c-game-pick').onclick = pickFolder;
  $('g-downgrade').onclick = openDowngrade;
  $('dg-go').onclick = runDowngrade;
  $('dg-cancel').onclick = () => {
    if (inlineRun) { invoke('steam_login_cancel').catch(() => {}); return; }
    dgMode(false); showPage(page);
  };
  $('dg-send').onclick = sendAnswer;
  $('dg-answer').onkeydown = e => { if (e.key === 'Enter') sendAnswer(); };
  $('dg-other').onclick = () => {
    document.querySelectorAll('#downgrade .opt.other').forEach(o => { o.hidden = false; });
    $('dg-other').hidden = true;
  };
  try { $('dg-user').value = localStorage.getItem('ad-steam-user') || ''; } catch {}
  $('dg-install').onclick = steamInstall;
  $('st-move').onclick = moveStrays;
  // ---------- after the game closes ----------
  let lastReport = '';
  T.event.listen('game-ended', ({ payload: g }) => {
    gameRunning = false;
    lastReport = g.report;
    if (!g.crashed) { setStatus(g.summary); ready(); return; }
    $('cr-summary').textContent = g.summary;
    $('cr-report').textContent = g.report;
    $('cr-note').hidden = true;
    const staff = $('cr-staff');
    staff.hidden = !g.reportId;
    staff.textContent = g.reportId
      ? `Staff already have this report as ${g.reportId}.` + (g.likelyCause ? ` Likely cause: ${g.likelyCause}` : '') + ' Mention the number if you ask for help.'
      : '';
    showSheet('crash');
    invoke('game_check').then(c => { gameCheck = c; renderVersion(); ready(); }).catch(() => {});
    ready();
    setStatus('Skyrim closed unexpectedly. Copy diagnostics in Settings includes the crash report.', true);
  });
  $('cr-copy').onclick = async () => {
    try { await navigator.clipboard.writeText(lastReport); $('cr-note').textContent = 'Copied. Paste it with Ctrl+V.'; }
    catch { $('cr-note').textContent = "Couldn't copy. Open the log folder and send the newest game-….txt file."; }
    $('cr-note').hidden = false;
  };
  $('cr-logs').onclick = () => invoke('open_log_folder').catch(() => {});
  $('cr-close').onclick = () => showPage(page);
  $('st-cancel').onclick = () => showPage(page);
  $('st-ignore').onclick = () => { ignoreStrays = true; logUi('player chose to keep other plugins: ' + strays().join(', ')); showPage(page); ready(); };
  // Two clicks, because saying yes here when Steam has swapped the game data
  // makes Skyrim crash on start.
  $('dg-skip').onclick = async () => {
    if (!skipArmed) {
      skipArmed = true;
      $('dg-error').textContent = "Only do this if you downgraded Skyrim yourself. If Steam has updated your game, Skyrim will crash on start. Click the link again to confirm.";
      $('dg-error').hidden = false;
      $('dg-skip').textContent = 'Yes, my game is already on this version';
      return;
    }
    dgBusy(true);
    try { dgDone(await invoke('mark_game_ok')); } catch (e) { dgFail(e); }
  };
  document.querySelectorAll('input[name="dg-login"]').forEach(r => r.onchange = () => {
    const how = document.querySelector('input[name="dg-login"]:checked').value;
    $('dg-user-field').hidden = how !== 'user' && how !== 'here';
  });
  $('c-skse-recheck').onclick = refreshState;
  $('f-go').onclick = () => { if (!signedIn()) { showSignIn(); return; } showPage('home'); check(); };
  $('si-go').onclick = beginSignIn;
  $('si-cancel').onclick = () => { signInRun++; $('si-wait').hidden = true; $('si-go').disabled = false; };
  $('set-diag').onclick = async () => {
    const note = $('set-diag-note');
    try {
      const text = await invoke('diagnostics');
      try { await navigator.clipboard.writeText(text); note.textContent = 'Copied. Paste it in Discord (Ctrl+V).'; }
      catch { note.textContent = 'Couldn\'t copy automatically. Open the log folder and send launcher.log instead.'; }
    } catch (e) { note.textContent = 'Couldn\'t collect diagnostics: ' + e; }
    note.hidden = false;
  };
  $('set-logs').onclick = () => invoke('open_log_folder').catch(e => { $('set-diag-note').textContent = String(e); $('set-diag-note').hidden = false; });
  $('acc-signout').onclick = async () => {
    await invoke('auth_sign_out');
    auth = { signedIn: false };
    renderAccount();
    showSignIn();
  };

  (async () => {
    await refreshState();
    loadStatus();
    setInterval(loadStatus, 30 * 1000);
    await refreshAuth();
    setInterval(recheckAuth, 10 * 60 * 1000);
    if (!ready_()) {
      showSheet('first');
      setStatus('Finish setup to play');
    } else if (!signedIn()) {
      showSignIn(auth && auth.message);
      setStatus('Sign in with Discord to play');
    } else {
      await check();
      loadStatus();
    }
    checkSelfUpdate();
  })();
})();
