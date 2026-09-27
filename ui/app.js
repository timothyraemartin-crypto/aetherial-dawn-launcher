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
      // Players see plain words; the raw text is in the log above.
      if (cmd === 'log_ui' || cmd === 'plain_error') throw e;
      let shown = e;
      try { shown = await T.core.invoke('plain_error', { text: String(e) }); } catch {}
      throw shown;
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
  let toolRunning = false;
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
    if (toolRunning) parts.push(`<button class="linkish" id="tool-skip">Skip for now</button>`);
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
    $('reqs').hidden = id !== 'reqs';
    if (id === 'settings') {
      for (const nav of Object.values(PAGES)) $(nav).removeAttribute('aria-current');
      $('nav-settings').setAttribute('aria-current', 'page');
    }
  }
  // SKSE, Crash Logger and the right game version are the launcher's job, so
  // a Skyrim folder is all a player needs to get going.
  const ready_ = () => !!state.game;
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
    const skseRow = [!!(g && g.hasSkse), g && g.hasSkse ? 'SKSE installed' : 'SKSE is missing', g && g.hasSkse ? 'skse64_loader.exe' : 'The launcher installs it for you when you press Play.'];
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
    $('set-music').setAttribute('aria-checked', state.config.music !== false);
    $('set-only').setAttribute('aria-checked', state.config.onlyServerMods !== false);
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
        setStatus(`Couldn't reach the Aetherial Dawn server. Check your internet; the launcher tries again every minute.` + HELP, true);
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
      if (c.canDowngrade) { setPlay('downgrade', 'PLAY'); setStatus(`${c.reason} Press Play: the launcher changes Skyrim to ${shortVer(c.target)} first, then starts the game.`); }
      else { setPlay('wait', 'WRONG VERSION'); setStatus(c.reason, true); }
      return;
    }
    if (!signedIn()) { setPlay('signin', 'SIGN IN'); setStatus('Sign in with Discord to play.', true); return; }
    if (auth.locked) { setPlay('wait', 'OFFLINE'); setStatus(auth.message, true); return; }
    setPlay('play', 'PLAY');
    if (c && c.warning) setStatus(c.warning, true);
    if (c && c.target && !c.skseOk) setStatus(`The launcher installs SKSE ${c.skseVersion || ''} for you when you press Play.`);
    else if (!(c && c.warning)) setStatus(null);
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
        bringToFront();
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
    $('dg-lead').textContent = `${c.reason || c.warning || ''} The launcher changes your game files into Skyrim ${shortVer(c.target)} itself, then checks them.`;
    $('dg-error').hidden = true;
    $('dg-progress').hidden = true;
    skipArmed = false;
    if (verifyWake) { stopVerifyWait(); $('dg-go').disabled = false; }
    $('dg-skip').textContent = 'My game is already on this version';
    $('dg-skip').hidden = !c.needed;
    dgMode();
    showSheet('downgrade');
  }
  function dgMode() {
    $('dg-go').hidden = false;
    document.querySelector('#downgrade .choice').hidden = false;
    $('dg-notes').hidden = false;
  }
  const gb = n => n >= 1073741824 ? (n / 1073741824).toFixed(1) + ' GB' : mb(n);
  function dgBusy(on) { for (const id of ['dg-go', 'dg-cancel', 'dg-skip']) $(id).disabled = on; busy = on; }
  // Set when Play started the version fix: Play carries on once it's done.
  let playAfterPatch = false;
  function dgDone(c) {
    gameCheck = c;
    dgBusy(false);
    showPage(page);
    ready();
    if (!gameCheck.needed && !statusMsg) setStatus(`Skyrim ${shortVer(gameCheck.installed)} is ready for Aetherial Dawn.`);
    const resume = playAfterPatch;
    playAfterPatch = false;
    if (resume && playMode === 'play') onPlay();
  }
  function dgFail(e) {
    playAfterPatch = false;
    dgBusy(false);
    $('dg-progress').hidden = true;
    $('dg-bar').hidden = true;
    $('dg-error').textContent = String(e);
    $('dg-error').hidden = false;
  }
  // ---------- patching the player's own files (no Steam) ----------
  const PATCH_STAGES = {
    check: (p) => `Checking your game files… ${p.done} of ${p.total}${p.file ? ' · ' + p.file : ''}`,
    download: (p) => `Downloading the patch for ${p.file} (${p.done + 1} of ${p.total})…`,
    apply: (p) => `Patching ${p.file} (${p.done + 1} of ${p.total})…`,
    verify: () => 'Checking your game…',
    build: (p) => `Building patches… ${p.file}`,
    fetch: (p) => `Downloading the patches (${p.file})… ${p.total ? Math.round(100 * p.done / p.total) + '%' : gb(p.done)}`,
    unpack: () => 'Unpacking the patches…',
    swap: () => 'Putting the new files in place…',
  };
  let verifyRun = 0, verifyWake = null;
  function stopVerifyWait() { verifyRun++; if (verifyWake) { verifyWake(); verifyWake = null; } }
  async function patchGame() {
    dgBusy(true);
    $('dg-error').hidden = true;
    $('dg-progress').hidden = false;
    $('dg-bar').hidden = false;
    $('dg-bar-i').style.width = '0%';
    $('dg-stage').textContent = 'Getting the patch list…';
    const off = await T.event.listen('patch-progress', ({ payload: p }) => {
      $('dg-stage').textContent = (PATCH_STAGES[p.stage] || (() => ''))(p);
      const f = p.done / Math.max(1, p.total);
      const share = { check: 0.3 * f, fetch: 0.6 * f, unpack: 0.6, apply: 0.6 + 0.35 * f, swap: 0.95 + 0.05 * f, verify: 1 }[p.stage] ?? 0.3 + 0.7 * f;
      $('dg-bar-i').style.width = Math.round(share * 100) + '%';
    });
    try { dgDone(await invoke('patch_game')); }
    catch (e) {
      let msg = String(e);
      // The patches start from Steam's own files: have Steam repair them
      // (Verify integrity, never a downgrade through Steam), then carry on.
      if (msg.startsWith('NO_PATCH_FILES:')) {
        const steam = await invoke('repair_game_files').catch(() => false);
        if (steam) {
          // The wait can be cancelled: Cancel bumps the run token and wakes
          // the sleep; nothing else is blocked while Steam works.
          const run = ++verifyRun;
          $('dg-bar').hidden = true;
          $('dg-stage').textContent = "Steam is checking Skyrim's files. The launcher carries on by itself when Steam is done.";
          busy = false;
          $('dg-cancel').disabled = false;
          $('dg-skip').disabled = false;
          const until = Date.now() + 30 * 60 * 1000;
          while (Date.now() < until && run === verifyRun) {
            await new Promise(r => { verifyWake = r; setTimeout(r, 30000); });
            if (run !== verifyRun) break;
            try { const c = await invoke('patch_game'); verifyWake = null; verifyRun++; dgDone(c); return; }
            catch (e2) { msg = String(e2); if (!msg.startsWith('NO_PATCH_FILES:')) break; }
          }
          verifyWake = null;
          if (run !== verifyRun) { $('dg-progress').hidden = true; $('dg-go').disabled = false; return; }
          verifyRun++;
          if (msg.startsWith('NO_PATCH_FILES:')) msg = 'NO_PATCH:Steam\'s check didn\'t bring back the files the patches need. Press this button to try again.';
        } else {
          msg = 'NO_PATCH:' + msg.slice(15) + ' Only the Steam copy of Skyrim Special Edition can be changed to the server\'s version.';
        }
      }
      if (msg.startsWith('NO_PATCH:')) dgFail(msg.slice(9));
      else dgFail(msg);
    }
    finally { off(); $('dg-bar').hidden = true; }
  }
  // Only the patch route is in the player build: no Steam sign-in, no QR
  // code (Timothy's decisions).
  function runDowngrade() { return patchGame(); }

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

  // ---------- mods: the server's list, Nexus sign-in and Download all ----------
  let modsRunning = false;
  let useKey = false;
  let ssoReady = false;
  let modsOff = null;
  const rqError = (e) => { $('rq-error').textContent = e ? String(e) : ''; $('rq-error').hidden = !e; };

  function renderMods(view) {
    const nx = view.nexus;
    $('rq-nx-out').hidden = !!nx;
    $('rq-nx-in').hidden = !nx;
    $('rq-nx-who').textContent = nx ? `Signed in to Nexus as ${nx.name} (${nx.is_premium ? 'Premium' : 'free account'}).` : '';
    $('rq-nx-free').hidden = !nx || nx.is_premium;
    ssoReady = !!view.sso;
    $('rq-sso').hidden = false;
    $('rq-keybox').hidden = !useKey;
    $('rq-vortex').hidden = !view.vortex;
    const list = $('rq-list');
    list.replaceChildren();
    for (const m of view.mods) {
      const row = document.createElement('div');
      row.className = 'rq-row';
      const text = document.createElement('div');
      const name = document.createElement('b');
      name.textContent = m.name;
      const sub = document.createElement('div');
      sub.className = 'rq-sub';
      sub.id = `rq-st-${m.id}`;
      sub.textContent = m.installed ? 'Installed' : (m.hint ? `Needs: ${m.hint}` : 'Not installed');
      text.append(name, sub);
      row.append(text);
      if (m.page && !m.installed) {
        const open = document.createElement('button');
        open.className = 'btn';
        open.textContent = 'Open';
        open.onclick = () => invoke('open_mod_page', { url: m.page }).catch(rqError);
        row.append(open);
      }
      if (m.installed) row.classList.add('ok');
      list.append(row);
    }
    const missing = view.mods.filter(m => !m.installed).length;
    $('rq-all').disabled = modsRunning || missing === 0;
    $('rq-all').textContent = missing === 0 ? 'ALL INSTALLED' : `DOWNLOAD ALL (${missing})`;
    $('rq-stop').hidden = !modsRunning;
  }

  async function refreshMods() {
    try { renderMods(await invoke('mods_state')); } catch (e) { rqError(e); }
  }

  async function showRequiredMods() {
    rqError(null);
    showSheet('reqs');
    await refreshMods();
  }

  const MOD_STAGE = { queued: 'Waiting its turn', waiting: '', download: 'Downloading', install: 'Installing', done: 'Installed', failed: '' };
  function modProgress(p) {
    const el = $(`rq-st-${p.id}`);
    if (!el) return;
    let t = MOD_STAGE[p.stage] ?? p.stage;
    if (p.stage === 'download' && p.total > 0) t += ` ${Math.floor(p.done * 100 / p.total)}%`;
    if (p.stage === 'waiting') t = p.message;
    if (p.stage === 'failed') t = `Didn't install: ${p.message}`;
    el.textContent = t;
    el.parentElement.parentElement.classList.toggle('bad', p.stage === 'failed');
  }

  async function downloadAll() {
    if (modsRunning) return;
    rqError(null);
    modsRunning = true;
    $('rq-all').disabled = true;
    $('rq-stop').hidden = false;
    if (!modsOff) modsOff = await T.event.listen('mods-progress', ({ payload }) => modProgress(payload));
    let ok = false;
    try {
      const r = await invoke('download_all_mods');
      if (r.failed.length) rqError(`Not installed yet: ${r.failed.map(f => f[0]).join(', ')}. The launcher tries again the next time you press Play.`);
      else if (r.cancelled) rqError('Stopped. Click Download all to carry on.');
      else ok = true;
    } catch (e) {
      if (String(e) === 'NEEDS_NEXUS_SIGN_IN') { rqError('Sign in to Nexus first (above).'); $('rq-key').focus(); }
      else rqError(e);
    } finally {
      modsRunning = false;
      await refreshMods();
    }
    return ok;
  }

let autoMods = false;
  // Play is waiting for Nexus sign-in; it carries on by itself after it.
  let playAfterNexus = false;
  async function onPlay(auto = false) {
    if (busy) return;
    if (playMode === 'strays') return openStrays();
    if (playMode === 'retry') return check();
    if (playMode === 'update') return update();
    // Play fixes the game version by itself, then starts the game.
    if (playMode === 'downgrade') { openDowngrade(); playAfterPatch = true; return patchGame(); }
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
      if (msg.startsWith('NEEDS_NEXUS_MODS:')) {
        setPlay('play', 'PLAY');
        let mods = [];
        try { mods = JSON.parse(msg.slice(17)); } catch (_) {}
        await showRequiredMods();
        // Play installs what's missing by itself, then carries on (once, so a
        // mod that won't install can't loop).
        if (!auto && !autoMods && !$('rq-nx-in').hidden) {
          autoMods = true;
          setStatus(`Installing ${mods.length === 1 ? mods[0].name : `${mods.length} required mods`}, then starting Skyrim…`);
          const ok = await downloadAll();
          autoMods = false;
          // auto: a mod that "installed" but still counts as missing can't loop.
          if (ok) { showSheet(null); return onPlay(true); }
          setStatus('A required mod isn\'t in yet. The launcher tries again the next time you press Play.', true);
          return;
        }
        if (auto) {
          setStatus(`${mods.length === 1 ? mods[0].name : 'A required mod'} installed but isn't detected yet. Press Play to try again; if it repeats, send Copy diagnostics to staff.`, true);
          return;
        }
        playAfterNexus = true;
        setStatus(`Sign in to Nexus in your browser; the launcher then installs ${mods.length === 1 ? mods[0].name : `${mods.length} required mods`} and starts Skyrim by itself.`);
        // Play is the click: Nexus opens now, with nothing more to press here.
        if (!auto && !$('rq-sso-go').disabled) $('rq-sso-go').click();
        return;
      }
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
  // minute, never while Skyrim is running or a download is in progress.
  let gameRunning = false, updating = false;
  let lastUpToDateLog = 0;
  async function checkSelfUpdate(byHand) {
    if (updating || gameRunning || busy || modsRunning) return byHand ? 'busy' : undefined;
    try {
      const upd = await T.updater.check();
      if (!upd) {
        if (byHand || Date.now() - lastUpToDateLog > 30 * 60 * 1000) { logUi('launcher is up to date'); lastUpToDateLog = Date.now(); }
        return 'latest';
      }
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
      return 'failed';
    }
  }
  setInterval(() => checkSelfUpdate(false), 60 * 1000);
  $('set-update').onclick = async () => {
    const b = $('set-update');
    b.disabled = true;
    b.textContent = 'Checking…';
    const r = await checkSelfUpdate(true);
    b.disabled = false;
    b.textContent = r === 'latest' ? 'Up to date' : r === 'busy' ? 'Try again after the game or download' : r === 'failed' ? "Couldn't check, try again" : 'Check for updates';
    setTimeout(() => { b.textContent = 'Check for updates'; }, 4000);
  };

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
  // After a sign-in in the browser, the launcher comes back by itself.
  function bringToFront() { win.unminimize().catch(() => {}); win.setFocus().catch(() => {}); }
  $('w-min').onclick = () => win.minimize();
  $('w-close').onclick = () => win.close();
  $('w-settings').onclick = $('nav-settings').onclick = $('t-settings').onclick = () => showSheet('settings');
  $('play').onclick = () => onPlay();
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
  const setMusic = async on => {
    $('set-music').setAttribute('aria-checked', on);
    $('music-ask').hidden = true;
    state.config.music = on;
    await invoke('set_music', { on }).catch(() => {});
  };
  $('set-music').onclick = () => setMusic($('set-music').getAttribute('aria-checked') !== 'true');
  $('mu-keep').onclick = () => setMusic(true);
  $('mu-mute').onclick = () => setMusic(false);
  document.querySelectorAll('.switch:not(#set-anim):not(#set-music)').forEach(s => s.onclick = async () => {
    s.setAttribute('aria-checked', s.getAttribute('aria-checked') !== 'true');
    const prefs = { closeOnLaunch: $('set-close').getAttribute('aria-checked') === 'true', backgroundUpdates: $('set-bg').getAttribute('aria-checked') === 'true', shareHealth: $('set-share').getAttribute('aria-checked') === 'true', onlyServerMods: $('set-only').getAttribute('aria-checked') === 'true' };
    await invoke('set_prefs', { prefs });
    Object.assign(state.config, prefs);
  });
  $('c-game-pick').onclick = pickFolder;
  $('g-downgrade').onclick = openDowngrade;
  $('dg-go').onclick = runDowngrade;
  $('dg-cancel').onclick = () => {
    playAfterPatch = false;
    stopVerifyWait(); dgMode(); showPage(page);
  };
  $('st-move').onclick = moveStrays;
  // ---------- other mods set aside before Play ----------
  const asideNote = n => {
    $('aside-note').textContent = n ? `Last time you pressed Play, ${n} file${n === 1 ? '' : 's'} from other mods ${n === 1 ? 'was' : 'were'} set aside (in your Skyrim folder under .aetherial-dawn\\disabled).` : 'Nothing has been set aside.';
  };
  try { asideNote(+localStorage.getItem('ad-set-aside') || 0); } catch { asideNote(0); }
  // What Play is doing, step by step.
  T.event.listen('play-step', ({ payload: text }) => { if (text) setStatus(text); });
  // A mod's tool running before the game starts ("Fitting armor to bodies").
  T.event.listen('tool-running', ({ payload: label }) => {
    toolRunning = !!label;
    if (label) setStatus(`${label}. This can take a few minutes the first time.`);
    else renderStatus();
  });
  $('status').addEventListener('click', e => {
    if (e.target && e.target.id === 'tool-skip') {
      e.target.disabled = true;
      invoke('skip_tool').catch(() => {});
    }
  });
  T.event.listen('mods-set-aside', ({ payload: n }) => {
    try { localStorage.setItem('ad-set-aside', String(n)); } catch {}
    asideNote(n);
    logUi(`set aside ${n} file(s) from other mods before Play`);
  });
  $('aside-restore').onclick = async () => {
    const b = $('aside-restore');
    b.disabled = true;
    try {
      // Turn the switch off first, or the next Play would set them aside again.
      $('set-only').setAttribute('aria-checked', 'false');
      const prefs = { closeOnLaunch: $('set-close').getAttribute('aria-checked') === 'true', backgroundUpdates: $('set-bg').getAttribute('aria-checked') === 'true', shareHealth: $('set-share').getAttribute('aria-checked') === 'true', onlyServerMods: false };
      await invoke('set_prefs', { prefs });
      Object.assign(state.config, prefs);
      const n = await invoke('restore_set_aside');
      try { localStorage.setItem('ad-set-aside', '0'); } catch {}
      $('aside-note').textContent = n ? `Put ${n} file${n === 1 ? '' : 's'} back. "Only the server's mods" is now off.` : `Nothing needed putting back. "Only the server's mods" is now off.`;
    } catch (e) { $('aside-note').textContent = String(e); }
    b.disabled = false;
  };
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
    if (Date.now() - crashFiledAt > 60000) {
      staff.hidden = !g.reportId;
      staff.textContent = g.reportId
        ? `Staff already have this report as ${g.reportId}.` + (g.likelyCause ? ` Likely cause: ${g.likelyCause}` : '') + ' Mention the number if you ask for help.'
        : '';
    }
    showSheet('crash');
    invoke('game_check').then(c => { gameCheck = c; renderVersion(); ready(); }).catch(() => {});
    ready();
    setStatus('Skyrim closed unexpectedly. Copy diagnostics in Settings includes the crash report.', true);
  });
  let crashFiledAt = 0;
  T.event.listen('crash-filed', ({ payload: f }) => {
    crashFiledAt = Date.now();
    const staff = $('cr-staff');
    staff.textContent = `Staff already have this report as ${f.reportId}.` + (f.likelyCause ? ` Likely cause: ${f.likelyCause}` : '') + ' Mention the number if you ask for help.';
    staff.hidden = false;
  });
  $('cr-copy').onclick = async () => {
    try { await navigator.clipboard.writeText(lastReport); $('cr-note').textContent = 'Copied. Paste it with Ctrl+V.'; }
    catch { $('cr-note').textContent = "Couldn't copy. Open the log folder and send the newest game-….txt file."; }
    $('cr-note').hidden = false;
  };
  $('rq-close').onclick = () => { playAfterNexus = false; showPage(page); };
  $('rq-all').onclick = () => downloadAll();
  // After Nexus sign-in, the Play that asked for it carries on by itself.
  async function resumePlay() {
    if (!playAfterNexus || $('rq-nx-in').hidden) return;
    // Wait out a check that is running, so the resumed Play isn't dropped.
    for (let i = 0; busy && i < 120; i++) await new Promise(r => setTimeout(r, 500));
    if (!playAfterNexus) return;
    playAfterNexus = false;
    onPlay();
  }
  $('rq-stop').onclick = () => invoke('cancel_mods');
  $('rq-getkey').onclick = () => invoke('open_nexus_key_page').catch(rqError);
  $('rq-signin').onclick = async () => {
    rqError(null);
    $('rq-signin').disabled = true;
    try { await invoke('nexus_sign_in', { key: $('rq-key').value }); $('rq-key').value = ''; await refreshMods(); resumePlay(); }
    catch (e) { rqError(e); }
    finally { $('rq-signin').disabled = false; }
  };
  $('rq-usekey').onclick = () => { useKey = true; $('rq-keybox').hidden = false; };
  $('rq-sso-stop').onclick = () => invoke('nexus_sso_cancel');
  $('rq-sso-go').onclick = async () => {
    rqError(null);
    $('rq-sso-go').disabled = true;
    $('rq-sso-go').textContent = 'Waiting for Nexus…';
    $('rq-sso-stop').hidden = false;
    $('rq-sso-note').innerHTML = ssoReady
      ? 'Nexus opened in your browser. Click <b>Authorise</b> there and come back.'
      : 'Nexus opened your API keys page in your browser. Copy your <i>Personal API Key</i> at the bottom (its Copy button, or select it and press Ctrl+C) and the launcher signs you in by itself.';
    try { await invoke(ssoReady ? 'nexus_sso' : 'nexus_copy_sign_in'); bringToFront(); await refreshMods(); resumePlay(); }
    catch (e) { if (!String(e).includes('cancelled')) rqError(e); }
    finally { $('rq-sso-go').disabled = false; $('rq-sso-go').textContent = 'Sign in with Nexus'; $('rq-sso-stop').hidden = true; }
  };
  $('rq-signout').onclick = async () => { await invoke('nexus_sign_out').catch(rqError); await refreshMods(); };
  $('files-mods').onclick = () => showRequiredMods();
  $('rq-again').onclick = () => { showPage(page); onPlay(); };
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
    stopVerifyWait();
    dgBusy(true);
    try { dgDone(await invoke('mark_game_ok')); } catch (e) { dgFail(e); }
  };
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
    // Music plays unless it was switched off in Settings; nothing to answer.
    invoke('music_start').catch(() => {});
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
