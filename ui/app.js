// Launcher UI. Talks to the Rust side in src-tauri/src/main.rs through invoke().
(() => {
  const T = window.__TAURI__;
  // Every command's failure (and the outcome of the important ones) goes to the
  // launcher log, so Copy diagnostics shows what happened. Nothing secret
  // reaches the UI, so nothing secret can be logged from here.
  const QUIET = new Set(['log_ui', 'diagnostics', 'files', 'server_status', 'setup_state', 'auth_poll', 'game_running']);
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
  // An error with HELP on it: when health sharing is on, the launcher sends
  // staff a report itself and says so instead (text audit C2).
  function helpStatus(msg, what) {
    setStatus(msg + HELP, true);
    invoke('report_problem', { what }).then(id => {
      if (id && statusMsg && statusMsg.msg === msg + HELP) setStatus(`${msg} Staff have been sent a report (${id}).`, true);
    }).catch(() => {});
  }
  const $ = id => document.getElementById(id);

  const ICON_OK = '<path d="M5 12.5l4.5 4.5L19 7.5"/>';
  const ICON_BAD = '<circle cx="12" cy="12" r="9"/><path d="M12 7.5v5.5M12 16.5v.01"/>';
  const ICON_INFO = '<circle cx="12" cy="12" r="9"/><path d="M12 11v5.5M12 7.5v.01"/>';
  const ICON_BUSY = '<path d="M20 12a8 8 0 1 1-2.3-5.7"/><path d="M20 4v5h-5"/>';
  const THUMBS = ['art/thumb-castle.jpg', 'art/thumb-peak.jpg', 'art/thumb-lake.jpg', 'art/thumb-city.jpg'];

  let state = null;       // get_state
  let pending = null;     // check
  let status = null;      // status.json
  let gameCheck = null;
  let skipArmed = false;
  let auth = null;        // auth_status: Discord sign-in   // check().game: is Skyrim the build the server needs?
  let busy = false;
  let playInFlight = false;
  let playMode = 'wait';  // wait | play | mods | update | retry | auth-retry | downgrade | signin
  let modsCheckSeq = 0;
  let page = 'home';

  const plural = (n, w) => `${n} ${w}${n === 1 ? '' : 's'}`;
  const timeLeft = sec => sec < 90 ? `${Math.max(1, Math.round(sec))} sec` : `${Math.round(sec / 60)} min`;
  const mb = n => (n / 1048576).toFixed(n < 10485760 ? 1 : 0) + ' MB';
  const esc = s => String(s ?? '').replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

  // ---------- play button + status line ----------
  function setPlay(mode, label) {
    playMode = mode;
    $('play-label').textContent = label;
    $('play').disabled = mode === 'wait';
    $('play-wrap').classList.toggle('off', mode === 'wait');
    // For styling only: busy = the launcher is working, ready = PLAY starts the game,
    // anything else is a step the player takes first (sign in, fix mods, retry).
    const busy = mode === 'wait' && /ING$|CHECKING/.test(label);
    $('play-wrap').dataset.state = busy ? 'busy' : mode === 'play' || mode === 'downgrade' ? 'ready' : mode === 'wait' ? 'off' : 'action';
    showHint();
  }
  // First-time players are told where the menu is, until they have started the game once.
  const HINT_KEY = 'ad.f3hint';
  const hintSeen = () => { try { return localStorage.getItem(HINT_KEY) === '1'; } catch (_) { return false; } };
  function showHint() {
    const hint = $('play-hint');
    if (!hint || hintSeen()) { if (hint) hint.hidden = true; return; }
    hint.hidden = false;
    hint.style.visibility = $('play-wrap').dataset.state === 'ready' ? 'visible' : 'hidden';
  }
  function markHintSeen() { try { localStorage.setItem(HINT_KEY, '1'); } catch (_) {} showHint(); setupWanted = false; applySetup(); }
  let statusMsg = null;
  function setStatus(msg, isError) {
    statusMsg = msg ? { msg, isError } : null;
    renderStatus();
    const live = $('status-live');
    if (live && live.textContent !== (msg || '')) live.textContent = msg || '';
  }
  let toolRunning = false;
  function renderStatus() {
    const parts = [];
    if (statusMsg) parts.push(`<span${statusMsg.isError ? ' class="error"' : ''}>${esc(statusMsg.msg)}</span>`);
    else {
      // Only a current /health answer can claim the server is online.
      const st = status;
      const known = st && typeof st.online === 'boolean';
      const on = known && st.online;
      const who = on && typeof st.players === 'number' ? ` · ${typeof st.maxPlayers === 'number' && st.maxPlayers > 0 ? `${st.players} of ${plural(st.maxPlayers, 'player')}` : plural(st.players, 'player')}` : '';
      parts.push(`<span><i class="dot${on ? '' : ' off'}"></i>${!known ? (statusAsked ? 'Server status unavailable' : 'Connecting to the server') : on ? 'Server online' : 'Server offline'}${who}</span>`);
      if (st && st.maintenance) parts.push(`<span class="error">${esc(typeof st.maintenance === 'string' ? st.maintenance : 'Server maintenance')}</span>`);
      const build = pending && pending.build;
      if (build) parts.push(`<span>Build ${esc(build)}</span>`);
    }
    if (toolRunning) parts.push(`<button class="linkish" id="tool-skip">Skip for now</button>`);
    if (state) parts.push(`<span>v${esc(state.launcherVersion)}</span>`);
    const html = parts.join('');
    if (html !== shownStatus) $('status').innerHTML = shownStatus = html;
  }
  let shownStatus = '';
  // Saved news and Discord invite can fill their panels while current checks
  // run. Saved readiness and server health are never shown as current.
  const LAST = 'ad.lastReady';
  const lastSeen = (() => { try { return JSON.parse(localStorage.getItem(LAST)) || {}; } catch (_) { return {}; } })();
  function remember(patch) {
    Object.assign(lastSeen, patch);
    try { localStorage.setItem(LAST, JSON.stringify(lastSeen)); } catch (_) {}
  }
  if (lastSeen.play) remember({ play: false });
  let statusAsked = false;
  function setChip(kind, text) {
    const c = $('mods-chip');
    c.className = 'chip ' + (kind === 'ok' ? '' : kind);
    c.querySelector('svg').innerHTML = kind === 'ok' ? ICON_OK : kind === 'warn' ? ICON_BAD : ICON_BUSY;
    c.querySelector('span').textContent = text;
  }

  // ---------- pages + sheets ----------
  const PAGES = { home: 'nav-home', server: 'nav-server', mods: 'nav-mods', news: 'nav-news' };
  const SHEETS = ['first', 'settings', 'downgrade', 'signin', 'strays', 'crash', 'health', 'reqs'];
  let activeSheet = null, sheetReturnFocus = null;
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
    applySetup();
    if (p === 'mods') loadFiles();
  }
  function showSheet(id) {
    const previous = activeSheet;
    if (id && !previous) sheetReturnFocus = document.activeElement;
    for (const name of SHEETS) $(name).hidden = name !== id;
    activeSheet = id;
    // Keep background controls out of the keyboard and screen-reader path
    // until the dialog closes. The window is frameless, so the title bar
    // (minimise, close, dragging), the logo plate (also a drag region) and
    // the restart-to-update banner stay live behind every sheet.
    const STAY_LIVE = ['sheet', 'titlebar', 'plate', 'banner'];
    for (const child of document.querySelector('.app').children) {
      if (!STAY_LIVE.some(c => child.classList.contains(c))) child.inert = !!id;
    }
    if (id === 'settings') {
      for (const nav of Object.values(PAGES)) $(nav).removeAttribute('aria-current');
      $('nav-settings').setAttribute('aria-current', 'page');
    }
    if (id) {
      $(id).focus();
    } else if (previous) {
      const target = sheetReturnFocus && sheetReturnFocus.isConnected && sheetReturnFocus.tabIndex >= 0
        && !sheetReturnFocus.disabled && !sheetReturnFocus.closest('[hidden]')
        ? sheetReturnFocus : $(PAGES[page]);
      sheetReturnFocus = null;
      target?.focus();
    }
  }
  document.addEventListener('keydown', e => {
    if (!activeSheet || e.key !== 'Tab') return;
    const sheet = $(activeSheet);
    const focusables = [...sheet.querySelectorAll('button:not([disabled]), input:not([disabled]), a[href], select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])')]
      .filter(el => !el.closest('[hidden]') && el.getClientRects().length);
    if (!focusables.length) { e.preventDefault(); sheet.focus(); return; }
    const first = focusables[0], last = focusables[focusables.length - 1];
    if (e.shiftKey && (document.activeElement === first || document.activeElement === sheet || !sheet.contains(document.activeElement))) {
      e.preventDefault(); last.focus();
    } else if (!e.shiftKey && (document.activeElement === last || document.activeElement === sheet || !sheet.contains(document.activeElement))) {
      e.preventDefault(); first.focus();
    }
  });
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

  // ok: true (done), false (needs the player), null (the launcher does it).
  function renderRow(el, ok, title, detail) {
    el.classList.remove('ok', 'bad', 'info'); el.classList.add(ok === null ? 'info' : ok ? 'ok' : 'bad');
    el.querySelector('svg').innerHTML = ok === null ? ICON_INFO : ok ? ICON_OK : ICON_BAD;
    el.querySelector('b').textContent = title;
    el.querySelector('small').textContent = detail;
  }
  function renderGame() {
    const g = state.game;
    // A folder just picked that wasn't Skyrim: said here too, since the
    // first-run sheet has no Settings error line.
    const gameRow = [!!g, g ? 'Skyrim Special Edition' : 'Skyrim not found', g ? (pickError ? `${pickError} Still using ${g.dir}.` : g.dir) : (pickError || state.gameError || 'Pick the folder that has SkyrimSE.exe in it.')];
    // SKSE is the launcher's job: nothing here asks the player to get it.
    const skseRow = g && g.hasSkse ? [true, 'SKSE installed', ''] : [null, 'SKSE', 'Installed for you when you press Play.'];
    renderRow($('g-game'), ...gameRow); renderRow($('g-skse'), ...skseRow);
    renderVersion();
    renderRow($('c-game'), ...gameRow); renderRow($('c-skse'), ...skseRow);
    $('c-game-pick').textContent = g ? 'Change' : 'Choose folder';
    $('c-skse-recheck').hidden = true;
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

  let pickError = null;
  // One folder pick at a time: a second press while the launcher is still
  // checking the first folder is ignored, so an older answer can never land
  // after a newer one (on screen or in the saved settings).
  let picking = false;
  async function pickFolder() {
    if (picking) return;
    picking = true;
    try { await pickFolderOnce(); } finally { picking = false; }
  }
  async function pickFolderOnce() {
    const dir = await T.dialog.open({ directory: true, title: 'Choose your Skyrim Special Edition folder' });
    if (!dir) return;
    try {
      await invoke('set_game_dir', { dir });
      $('set-error').hidden = true;
      pickError = null;
    } catch (e) {
      $('set-error').textContent = e;
      $('set-error').hidden = false;
      pickError = String(e);
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
  // signIn: the Discord check running alongside, needed before PLAY is final.
  function check(verifyAll = false, opt = {}) {
    if (gameRunning || playInFlight || updating) {
      setStatus('Finish the current game or download before checking game files.');
      return Promise.resolve(false);
    }
    if (busy) return Promise.resolve();
    return checkNow(verifyAll, opt);
  }
  async function checkNow(verifyAll, { signIn } = {}) {
    // A Vortex answer from an older manifest must not enable Play here.
    modsCheckSeq++;
    busy = true;
    setPlay('wait', 'CHECKING');
    setChip('busy', 'Checking');
    setStatus('Checking for updates…');
    try {
      pending = await invoke('check', { verifyAll });
      if (signIn) await signIn;
      if (signIn && !signedIn()) {
        // Signed out since last time: nothing is downloaded before sign-in.
        busy = false;
        pending = null;
        setPlay('signin', 'SIGN IN');
        setChip('busy', 'Client files ready · mods not checked yet');
        setStatus('Sign in with Discord to play');
        return;
      }
      gameCheck = pending.game;
      renderGame();
      $('srv-name').textContent = pending.server.name;
      $('srv-addr').textContent = `${pending.server.ip}:${pending.server.port}`;
      $('srv-build').textContent = pending.build;
      showWhatsNew(pending.files || pending.remove ? pending.notes : null);
      if (pending.files || pending.remove) {
        busy = false;
        if (state.config.backgroundUpdates || verifyAll) return await update(verifyAll);
        setPlay('update', 'UPDATE');
        setChip('warn', 'Update available');
        setStatus(`Build ${pending.build} available · ${plural(pending.files, 'file')} · ${mb(pending.bytes)}`);
      } else {
        await ready();
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

  // What's new beside the Update button: the lines staff wrote, or the files
  // about to change. Never a dialog, and gone once there is nothing to update.
  function showWhatsNew(lines) {
    const box = $('whatsnew');
    if (!box) return;
    const list = Array.isArray(lines) ? lines.filter(l => typeof l === 'string' && l) : [];
    box.hidden = !list.length;
    $('whatsnew-list').innerHTML = list.map(l => `<li>${esc(l)}</li>`).join('');
  }

  async function update(verifyAll = false) {
    if (gameRunning || playInFlight || updating) {
      setStatus('Finish the current game or download before updating game files.');
      return;
    }
    if (busy) return;
    busy = true;
    setPlay('wait', 'UPDATING');
    setChip('busy', 'Updating');
    setStatus(`Updating to build ${pending.build} · ${mb(pending.bytes)}`);
    $('progress').hidden = false;
    $('p-progress').setAttribute('aria-valuenow', '0');
    let last = { t: performance.now(), b: 0 }, speed = 0;
    const began = last.t;
    let lastEvent = began;
    const stallTimer = setInterval(() => { if (performance.now() - lastEvent > 3000) $('p-speed').textContent = 'Waiting for the server…'; }, 1000);
    const off = await T.event.listen('sync-progress', ({ payload: p }) => {
      const pct = p.bytesTotal ? (p.bytesDone / p.bytesTotal) * 100 : 100;
      $('p-bar').style.width = pct.toFixed(1) + '%';
      $('p-progress').setAttribute('aria-valuenow', String(Math.round(Math.min(100, pct))));
      $('p-num').textContent = `${Math.min(p.filesDone + (p.file ? 1 : 0), p.filesTotal)} / ${plural(p.filesTotal, 'file')} · ${Math.round(pct)}%`;
      $('p-file').textContent = p.file ? p.file.split(/[\\/]/).pop() : 'All files match the server';
      $('p-file').title = p.file || '';
      const now = performance.now();
      lastEvent = now;
      if (now - last.t > 500) {
        const rate = ((p.bytesDone - last.b) / (now - last.t)) * 1000;
        // Weighted by time, so a long gap counts for more than a burst of quick events.
        const alpha = 1 - Math.exp(-(now - last.t) / 5000);
        speed = speed ? speed + alpha * (rate - speed) : rate;
        const left = speed > 0 && p.bytesTotal > p.bytesDone ? (p.bytesTotal - p.bytesDone) / speed : 0;
        // Not a guess before three seconds of samples.
        const guess = left > 0 && now - began >= 3000 ? ` · ${left > 99 * 60 ? 'over 99 min' : 'about ' + timeLeft(left)} left` : '';
        $('p-speed').textContent = mb(speed) + '/s' + guess;
        last = { t: now, b: p.bytesDone };
      }
    });
    try {
      await invoke('update', { verifyAll });
      // The update command completed and verified the pending file changes.
      pending = { ...pending, files: 0, remove: 0 };
      showWhatsNew(null);
      await ready();
    } catch (e) {
      setPlay('retry', 'RETRY');
      setChip('warn', 'Update failed');
      helpStatus(`The game file update stopped: ${e}. Click Retry.`, `update stopped: ${e}`);
      pending = null;
    } finally {
      off();
      clearInterval(stallTimer);
      $('progress').hidden = true;
      busy = false;
    }
  }

  let helperWarning = null;
  // The first-run checklist on Home: shown until the player has started the
  // game once, or while every step is done there is nothing to show.
  let setupSeq = 0, setupWanted = false;
  // The checklist belongs to Home only.
  function applySetup() { const box = $('setup'); if (box) box.hidden = !(setupWanted && page === 'home'); }
  async function refreshSetup() {
    const box = $('setup'), seq = ++setupSeq;
    if (!box || hintSeen()) { setupWanted = false; applySetup(); return; }
    const steps = await invoke('setup_state').catch(() => null);
    if (seq !== setupSeq) return;
    if (!Array.isArray(steps) || steps.every(s => s.done)) { setupWanted = false; applySetup(); return; }
    $('setup-list').innerHTML = steps.map(s => `<li class="${s.done ? 'done' : 'todo'}"><i aria-hidden="true"></i><b>${esc(s.title)}${s.done ? ' (done)' : ''}</b><small>${esc(s.hint)}</small></li>`).join('');
    setupWanted = true;
    applySetup();
  }
  function ready() {
    refreshSetup();
    // Only a current check or a completed update can claim file readiness.
    if (!pending || pending.files || pending.remove) {
      setPlay('retry', 'RECHECK');
      setChip('warn', 'Game files need checking');
      setStatus('Check the current game files before Play.', true);
      return;
    }
    setChip('busy', 'Client files ready · mods not checked yet');
    loadFiles();
    renderGame();
    if (gameRunning || playInFlight) {
      setPlay('wait', gameRunning ? 'IN GAME' : 'LAUNCHING');
      return;
    }
    const c = gameCheck;
    if (c && c.needed) {
      if (c.canDowngrade) { setPlay('downgrade', 'PLAY'); setStatus(`${c.reason} Press Play: the launcher changes Skyrim to ${shortVer(c.target)} first, then starts the game.`); }
      else { setPlay('wait', 'WRONG VERSION'); setStatus(c.reason, true); }
      return;
    }
    if (!signedIn()) { setPlay('signin', 'SIGN IN'); setStatus('Sign in with Discord to play.', true); return; }
    if (auth.locked) {
      setPlay('auth-retry', 'RETRY SIGN-IN');
      setChip('warn', 'Sign-in unavailable');
      setStatus(auth.message, true);
      return;
    }
    setPlay('wait', 'CHECKING MODS');
    setStatus('Checking your mods…');
    setChip('busy', 'Client files ready · checking mods');
    return checkModReadiness();
  }

  function applyModReadiness(view) {
    if (gameRunning || playInFlight || updating || !['wait', 'play', 'mods'].includes(playMode)) return;
    const missingDirect = Array.isArray(view.mods)
      ? view.mods.filter(m => m.from === 'direct' && !m.installed).map(m => m.name)
      : [];
    const missingVortex = Array.isArray(view.mods)
      ? view.mods.filter(m => m.from === 'nexus' && m.in_vortex === false).map(m => m.name)
      : [];
    const shortList = (names, label) => {
      const shown = names.slice(0, 3).join(', ');
      const rest = names.length > 3 ? `, +${names.length - 3} more` : '';
      return `${plural(names.length, 'required mod')} ${label}: ${shown}${rest}. Open Requirements for the full list.`;
    };
    // With the server's Vortex gate off (vortex_required false), PLAY needs
    // only every listed mod present, as Play itself then checks.
    const missingAny = Array.isArray(view.mods) ? view.mods.filter(m => !m.installed).map(m => m.name) : [];
    const ready = view.vortex_required === false
      ? Array.isArray(view.mods) && missingAny.length === 0
      : view.vortex_ready === true && missingDirect.length === 0;
    if (ready) {
      setPlay('play', 'PLAY');
      if (gameCheck && gameCheck.warning) setStatus(gameCheck.warning, true);
      else if (gameCheck && gameCheck.target && !gameCheck.skseOk) setStatus(`The launcher installs SKSE ${gameCheck.skseVersion || ''} for you when you press Play.`);
      else setStatus(null);
      // A helper mod that couldn't be installed: Play still starts, and the
      // reason stays on the status line.
      if (helperWarning && !(gameCheck && gameCheck.warning)) setStatus(helperWarning, true);
      setChip('ok', view.vortex_required === false ? 'Ready to play' : 'Client and Vortex ready');
    } else {
      setPlay('mods', 'MODS NEEDED');
      setChip('warn', 'Mods need attention');
      const missingFiles = view.vortex_required === false ? missingAny : missingDirect;
      setStatus(missingFiles.length
        ? shortList(missingFiles, missingFiles.length === 1 ? 'still needs game files' : 'still need game files')
        : missingVortex.length && /^(Aetherial Dawn profile:|Vortex: \d+ required mods?)/.test(view.vortex_line || '')
          ? shortList(missingVortex, missingVortex.length === 1 ? 'needs attention in Vortex' : 'need attention in Vortex')
          : view.vortex_line || 'Connect Vortex and check the required mods before Play.', true);
    }
  }

  async function checkModReadiness() {
    const seq = ++modsCheckSeq;
    try {
      const view = await invoke('mods_state');
      if (seq === modsCheckSeq) applyModReadiness(view);
    } catch (e) {
      if (seq === modsCheckSeq) applyModReadiness({ vortex_ready: false, vortex_line: `Could not check required mods: ${e}` });
    }
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
    $('acc-note').textContent = auth.offline ? 'Login service unavailable; saved sign-in kept' : 'Signed in with Discord';
    paintAvatar($('me-avatar'), a); paintAvatar($('acc-avatar'), a);
  }
  // One question to the login service at a time: a Retry press, the
  // minute retry and the 10-minute check share the answer in flight.
  let authAsk = null;
  function refreshAuth() {
    authAsk = authAsk || (async () => {
      try { auth = await invoke('auth_status'); }
      catch (e) { auth = { signedIn: false, message: String(e) }; }
      renderAccount();
      return auth;
    })().finally(() => { authAsk = null; });
    return authAsk;
  }
  // Every 10 minutes, and sooner during an outage: a ban or leaving the
  // Discord signs the player out; a recovered service restores Play.
  let authRechecking = false;
  async function recheckAuth() {
    if (!signedIn() || busy || authRechecking) return;
    authRechecking = true;
    try {
      await refreshAuth();
      if (!signedIn()) {
        modsCheckSeq++;
        pending = null;
        setPlay('signin', 'SIGN IN');
        setChip('warn', 'Sign-in needed');
        showSignIn(auth.message);
      }
      else if (pending && ['play', 'wait', 'auth-retry'].includes(playMode)) await check();
    } finally { authRechecking = false; }
  }
  // The invite the login service publishes; the last one seen stands in.
  // A refusal that carries its own invite link (not a member) shows it as a
  // link in its text instead, so it isn't said twice.
  const INVITE = /https:\/\/discord\.gg\/[A-Za-z0-9-]{2,32}/;
  let signInHasInvite = false;
  function renderInvite() {
    const invite = (status && status.discordInvite) || lastSeen.invite;
    $('si-join').hidden = !invite || signInHasInvite;
    if (invite) $('si-join-go').textContent = invite.replace('https://', '');
  }
  function showSignIn(message) {
    const el = $('si-error');
    const found = message ? INVITE.exec(message) : null;
    signInHasInvite = !!found;
    renderInvite();
    el.textContent = '';
    if (found) {
      const link = document.createElement('button');
      link.className = 'linkish';
      link.textContent = found[0].replace('https://', '');
      link.onclick = () => invoke('open_invite', { url: found[0] }).catch(() => {});
      el.append(message.slice(0, found.index), link, message.slice(found.index + found[0].length));
    } else el.textContent = message || '';
    el.hidden = !message;
    $('si-wait').hidden = true;
    $('si-go').disabled = false;
    showSheet('signin');
  }
  async function signedInNow() {
    bringToFront();
    renderAccount();
    showPage('home');
    // A previous manifest may now require an update; never reuse it as
    // readiness after the browser sign-in completes.
    while (busy) await new Promise(r => setTimeout(r, 100));
    await check();
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
    // The launcher gives up on the browser after 5 minutes and says so; this
    // later limit only covers a launcher that stops answering, so a sign-in
    // finished just before 5 minutes isn't dropped here.
    const until = Date.now() + 6 * 60 * 1000;
    let lastError = '';
    while (run === signInRun && Date.now() < until) {
      await new Promise(r => setTimeout(r, 2000));
      if (run !== signInRun) return;
      let r;
      try { r = await invoke('auth_poll', { st }); lastError = ''; } catch (e) { r = { status: 'offline' }; lastError = String(e); }
      // Cancelled or started again while this answer was on its way. A
      // finished one is already saved by the launcher, so the page asks it
      // who is signed in rather than showing signed out; a sign-in started
      // since goes on and its answer wins.
      if (run !== signInRun) {
        if (r.status === 'done') {
          await refreshAuth();
          if (signedIn() && $('si-wait').hidden) signedInNow();
        }
        return;
      }
      // The launcher has the sign-in but couldn't save it yet; it tries again
      // on the next ask.
      if (r.status === 'save_failed') { lastError = r.message || "Couldn't save your sign-in"; continue; }
      if (r.status === 'pending' || r.status === 'offline') continue;
      if (r.status === 'done') {
        auth = { signedIn: true, account: r.account };
        await signedInNow();
        return;
      }
      showSignIn(r.message || 'Sign-in didn\'t finish. Try again.');
      return;
    }
    if (run === signInRun) showSignIn(lastError ? lastError + '. Try again.' : 'Sign-in timed out. Try again.');
  }

  // ---------- game version ----------
  function openDowngrade() {
    const c = gameCheck || {};
    $('dg-lead').textContent = `${c.reason || c.warning || ''} The launcher changes your game files into Skyrim ${shortVer(c.target)} itself, then checks them.`;
    $('dg-error').hidden = true;
    $('dg-progress').hidden = true;
    skipArmed = false;
    // Only the Play that opens this sheet sets it again (onPlay).
    playAfterPatch = false;
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
  async function dgDone(c) {
    gameCheck = c;
    dgBusy(false);
    showPage(page);
    await ready();
    if (!gameCheck.needed && !statusMsg) setStatus(`Skyrim ${shortVer(gameCheck.installed)} matches the server version.`);
    const resume = playAfterPatch;
    playAfterPatch = false;
    if (resume && playMode === 'play') await onPlay();
    else if (resume && playMode === 'signin') showSignIn('Your game is ready. Sign in with Discord, then press Play.');
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
    try { await dgDone(await invoke('patch_game')); }
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
            try { const c = await invoke('patch_game'); verifyWake = null; verifyRun++; await dgDone(c); return; }
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

  // ---------- required mods: manual installation through Vortex ----------
  const rqError = (e) => { $('rq-error').textContent = e ? String(e) : ''; $('rq-error').hidden = !e; };

  // Vortex's three answers for one mod, each named; a step Vortex hasn't
  // answered is left out rather than guessed.
  function vortexSteps(m) {
    const parts = [
      [m.vortex_installed, 'installed', 'not installed'],
      [m.vortex_enabled, 'switched on', 'not switched on'],
      [m.vortex_deployed, 'deployed', 'not deployed'],
    ].filter(([v]) => v === true || v === false).map(([v, yes, no]) => (v ? yes : no));
    return parts.length ? ` · In Vortex: ${parts.join(', ')}` : '';
  }

  function renderMods(view, fromPlay = false) {
    $('rq-summary').textContent = fromPlay
      ? `Play stopped because these ${plural(view.mods.length, 'required mod')} need attention in Vortex.`
      : (view.counts_text || `Showing ${plural(view.mods.length, 'required mod')}. Check each one in Vortex before Play.`);
    $('rq-vortex-step').hidden = !view.vortex_line;
    $('rq-vortex-step').textContent = view.vortex_line || '';
    // Mods the launcher placed itself, and whether Vortex also lists their files.
    $('rq-ownership').hidden = !view.ownership_text;
    $('rq-ownership').textContent = view.ownership_text || '';
    $('rq-vortex-connect').hidden = fromPlay || !!view.vortex_paired;
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
      sub.textContent = fromPlay
        ? (m.looks_for ? `Missing files: ${m.looks_for}` : 'Required mod files are missing')
        : m.from === 'direct'
        ? (m.installed ? 'Game files found' : m.looks_for ? `Missing files: ${m.looks_for}` : 'Game files not found')
        : m.in_vortex === true ? 'Vortex profile, deployment, and game files confirmed'
        : m.in_vortex === false ? (m.looks_for ? `Vortex deployment or game files need attention: ${m.looks_for}` : 'Vortex deployment needs attention') + vortexSteps(m)
        : m.installed ? 'Game files found — waiting for Vortex profile check'
        : m.looks_for ? `Missing files: ${m.looks_for}` : 'Waiting for Vortex profile check';
      text.append(name, sub);
      if (m.hint && !m.installed) {
        const hint = document.createElement('div');
        hint.className = 'rq-sub';
        hint.textContent = m.hint;
        text.append(hint);
      }
      row.append(text);
      if (m.page) {
        const open = document.createElement('button');
        open.className = 'btn';
        open.textContent = 'Open mod page';
        open.onclick = () => invoke('open_mod_page', { url: m.page }).catch(rqError);
        row.append(open);
      }
      if (!fromPlay && (m.in_vortex === true || (m.from === 'direct' && m.installed))) row.classList.add('ok');
      list.append(row);
    }
  }

  async function refreshMods() {
    try {
      const view = await invoke('mods_state');
      renderMods(view);
    } catch (e) { rqError(e); }
  }

  async function showRequiredMods(missing = null) {
    rqError(null);
    showSheet('reqs');
    if (Array.isArray(missing)) renderMods({ mods: missing }, true);
    else {
      $('rq-summary').textContent = 'Loading required mods…';
      $('rq-list').replaceChildren();
      await refreshMods();
    }
  }

  async function invokePlay() {
    playInFlight = true;
    try { return await invoke('play'); }
    finally { playInFlight = false; }
  }
  // ---------- server check before launch ----------
  // status.json may carry `maintenance` (true or a message): Play is blocked
  // until it is gone. An offline or full server is a warning the player can
  // override (Play anyway) or sit out (Wait and join: the 30-second status
  // poll starts the game when the server is open). An unknown status never blocks.
  function gateReason(st) {
    if (!st) return null;
    if (st.maintenance) return { kind: 'maintenance', msg: typeof st.maintenance === 'string' ? st.maintenance : 'The server is down for maintenance.' };
    if (st.online === false) return { kind: 'offline', msg: 'The server is offline right now.' };
    if (typeof st.players === 'number' && typeof st.maxPlayers === 'number' && st.maxPlayers > 0 && st.players >= st.maxPlayers) return { kind: 'full', msg: `The server is full (${st.players} of ${st.maxPlayers}).` };
    return null;
  }
  let gateWaiting = false, gateChecking = false;
  function showGate(reason) {
    $('play-gate-msg').textContent = gateWaiting ? `${reason.msg} Waiting for it to open; checking every 30 seconds.` : reason.msg;
    $('gate-anyway').hidden = reason.kind === 'maintenance';
    $('gate-wait').hidden = gateWaiting;
    $('play-gate').hidden = false;
    setPlay('play', 'PLAY');
    const live = $('status-live');
    if (live) live.textContent = $('play-gate-msg').textContent;
  }
  function hideGate() { gateWaiting = false; $('play-gate').hidden = true; }
  // Called after every status refresh while the player waits.
  function gateTick() {
    if (!gateWaiting) return;
    const reason = gateReason(status);
    if (reason) return showGate(reason);
    // Only a real answer that says the server is up starts the game. No
    // answer at all (the status call failed) is not "open": keep waiting.
    if (!status || status.online !== true) return showGate({ kind: 'offline', msg: 'Cannot reach the server, still waiting.' });
    hideGate();
    onPlay(true, true);
  }
  async function onPlay(checked = false, gateOk = false) {
    if (gameRunning || playInFlight || updating) return;
    if (busy) return;
    // Installing the launcher update closes the launcher; Play waits for it.
    if (updating) { setStatus('The launcher is updating itself. Play is ready again once it restarts.'); return; }
    if (playMode === 'strays') return openStrays();
    if (playMode === 'mods') return showRequiredMods();
    if (playMode === 'retry') return check();
    if (playMode === 'auth-retry') {
      setPlay('wait', 'CHECKING SIGN-IN');
      return recheckAuth();
    }
    if (playMode === 'update') return update();
    // Play fixes the game version by itself, then starts the game.
    if (playMode === 'downgrade') { openDowngrade(); playAfterPatch = true; return patchGame(); }
    if (playMode === 'signin') return showSignIn(auth && auth.message);
    if (playMode !== 'play') return;
    if (!checked) {
      // The manifest may have changed while the launcher sat open or Skyrim ran.
      const didCheck = await check();
      return didCheck !== false && playMode === 'play' ? onPlay(true) : undefined;
    }
    if (!gateOk) {
      // A healthy last answer starts the game at once. A problem is re-read first,
      // so an old answer never holds a player back.
      if (gateReason(status)) {
        if (gateChecking) return;
        gateChecking = true;
        try { await loadStatus(); } finally { gateChecking = false; }
        const reason = gateReason(status);
        if (reason) return showGate(reason);
      }
    }
    hideGate();
    setPlay('wait', 'LAUNCHING');
    setStatus('Starting Skyrim through SKSE…');
    playing = true;
    try {
      // Helper mods that couldn't be installed: the game starts without
      // them, and the reason stays on the status line.
      const warns = await invokePlay();
      helperWarning = Array.isArray(warns) && warns.length ? warns.join(' ') : null;
      gameRunning = true;
      markHintSeen();
      setPlay('wait', 'IN GAME');
      setStatus(helperWarning || 'Skyrim is running.', !!helperWarning);
    } catch (e) {
      const msg = String(e);
      if (msg.startsWith('NEEDS_NEXUS_MODS:')) {
        modsCheckSeq++;
        setPlay('mods', 'MODS NEEDED');
        setChip('warn', 'Mods need attention');
        let missing;
        try { missing = JSON.parse(msg.slice('NEEDS_NEXUS_MODS:'.length)); }
        catch (_) { missing = null; }
        await showRequiredMods(missing);
        if (!Array.isArray(missing)) rqError('Could not read the missing-mod list. Press Play again or send launcher.log to staff.');
        setStatus('Some required mods are missing. Install them in Vortex (switch them on and deploy), then press Play again.', true);
        return;
      }
      if (msg.startsWith('VORTEX_NOT_READY:')) {
        modsCheckSeq++;
        setPlay('mods', 'MODS NEEDED');
        setChip('warn', 'Mods need attention');
        await showRequiredMods();
        const reason = msg.slice('VORTEX_NOT_READY:'.length);
        rqError(reason);
        setStatus(reason, true);
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
      helpStatus(`Skyrim didn't start: ${msg}`, `game didn't start: ${msg}`);
    } finally {
      playing = false;
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
  let shownNews = '';
  async function loadStatus() {
    status = await invoke('server_status').catch(() => null);
    statusAsked = true;
    if (status) remember({ news: Array.isArray(status.news) ? status.news.slice(0, 20) : lastSeen.news, invite: status.discordInvite || lastSeen.invite });
    renderInvite();
    renderStatus();
    gateTick();
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
      showNews(status.news);
    }
  }
  // News changes the height of the box above Play, so it is only redrawn when
  // it changed, and the last session's news is drawn at start-up.
  function showNews(items) {
    const key = JSON.stringify(items.slice(0, 20));
    if (key === shownNews) return;
    shownNews = key;
    $('news').innerHTML = newsHtml(items.slice(0, 3));
    $('news-full').innerHTML = newsHtml(items.slice(0, 20), true);
  }

  // ---------- launcher self-update ----------
  // Installs every new launcher release by itself: on start and every
  // minute, never while Skyrim is running or a download is in progress.
  // playing: Play has been pressed and hasn't finished starting the game.
  // Installing an update closes the launcher, so it waits for all of these.
  let gameRunning = false, updating = false, playing = false;
  let lastUpToDateLog = 0;
  // An update downloaded while Play was starting, installed once it's safe.
  let downloaded = null;
  const updateWaits = () => gameRunning || playing || playInFlight || busy;
  // gameRunning only knows a game Play started; Skyrim started from Steam,
  // Vortex or MO2 is found by asking Windows. When that question fails, the
  // game might be running, so the update waits (installing closes the
  // launcher) and the next minute's check asks again.
  let gameCheckFailed = false;
  const skyrimUp = async () => {
    try { const up = !!(await T.core.invoke('game_running')); gameCheckFailed = false; return up; }
    catch (e) { gameCheckFailed = true; logUi('game_running failed, the launcher update waits: ' + e); return true; }
  };
  const waitReason = () => gameCheckFailed ? 'unknown' : 'busy';
  async function checkSelfUpdate(byHand) {
    if (updating || updateWaits()) return byHand ? 'busy' : undefined;
    if (await skyrimUp()) return byHand ? waitReason() : undefined;
    try {
      const upd = downloaded || await T.updater.check();
      if (!upd) {
        if (byHand || Date.now() - lastUpToDateLog > 30 * 60 * 1000) { logUi('launcher is up to date'); lastUpToDateLog = Date.now(); }
        return 'latest';
      }
      // Play may have started while the check was out.
      if (updating || updateWaits()) return byHand ? 'busy' : undefined;
      updating = true;
      $('self-update-text').textContent = `Updating the launcher to ${upd.version}…`;
      $('self-update').hidden = false;
      $('self-update-go').hidden = true;
      logUi(`installing launcher ${upd.version} automatically`);
      let reserved = false;
      try {
        if (!downloaded) {
          await upd.download();
          downloaded = upd;
        }
        // And again after the download: installing closes the launcher.
        const held = updateWaits() || await skyrimUp();
        if (held) {
          updating = false;
          $('self-update-text').textContent = gameCheckFailed
            ? `Launcher ${upd.version} is ready. It installs once the launcher can check that Skyrim isn't running.`
            : `Launcher ${upd.version} is ready. It installs once you're done playing.`;
          logUi(`launcher ${upd.version} downloaded; install waits for Play and the game`);
          return byHand ? waitReason() : undefined;
        }
        // Reserve the final install with the game launcher itself. This
        // closes the gap between the last process check and install().
        reserved = !!(await T.core.invoke('self_update_begin'));
        if (!reserved) {
          updating = false;
          $('self-update-text').textContent = `Launcher ${upd.version} is ready. It installs when Skyrim and file checks finish.`;
          return byHand ? 'busy' : undefined;
        }
        await upd.install();
        await T.process.relaunch();
      } catch (e) {
        if (reserved) await T.core.invoke('self_update_end').catch(() => {});
        updating = false;
        downloaded = null;
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
    b.textContent = r === 'latest' ? 'Up to date' : r === 'busy' ? 'Try again after the game or download' : r === 'unknown' ? "Couldn't check the game, try again" : r === 'failed' ? "Couldn't check, try again" : 'Check for updates';
    setTimeout(() => { b.textContent = 'Check for updates'; }, 4000);
  };

  // ---------- game health ----------
  const HL_TAG = { ok: 'OK', info: 'INFO', warn: 'WARN', fail: 'FAIL' };
  let healthText = '';
  async function openHealth() {
    // Set up before it opens, so focus doesn't land on a button that is
    // about to be disabled.
    $('hl-title').textContent = 'Checking your game…';
    $('hl-list').innerHTML = '';
    $('hl-note').hidden = true;
    $('hl-again').disabled = true;
    showSheet('health');
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
  $('play').onclick = () => { hideGate(); onPlay(); };
  $('gate-anyway').onclick = () => { hideGate(); onPlay(true, true); };
  $('gate-wait').onclick = () => { gateWaiting = true; const r = gateReason(status); if (r) showGate(r); };
  $('gate-cancel').onclick = hideGate;
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
    state.config.music = on;
    await invoke('set_music', { on }).catch(() => {});
  };
  $('set-music').onclick = () => setMusic($('set-music').getAttribute('aria-checked') !== 'true');
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
  // ---------- staff: Nexus key for the server-mod export ----------
  const xkShow = (saved, note) => {
    $('xk-forget').hidden = !saved;
    $('xk-key').value = '';
    $('xk-note').textContent = note || (saved ? 'A key is saved on this PC.' : 'No key saved.');
  };
  const xkRefresh = () => invoke('export_key_saved').then(v => xkShow(!!v)).catch(() => {});
  $('xk').addEventListener('toggle', () => { if ($('xk').open) xkRefresh(); });
  $('xk-save').onclick = async () => {
    const b = $('xk-save');
    b.disabled = true;
    $('xk-note').textContent = 'Checking the key with Nexus…';
    try { xkShow(true, await invoke('export_key_save', { key: $('xk-key').value })); }
    catch (e) { $('xk-note').textContent = String(e); }
    b.disabled = false;
  };
  $('xk-forget').onclick = async () => {
    try { await invoke('export_key_forget'); xkShow(false, 'Key removed.'); }
    catch (e) { $('xk-note').textContent = String(e); }
  };
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
    if (!g.crashed) {
      check().then(() => { if (playMode === 'play') setStatus(g.summary); });
      return;
    }
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
    // Staff already have it: no need to ask the player to paste it.
    $('cr-ask').hidden = !staff.hidden;
    showSheet('crash');
    check();
  });
  let crashFiledAt = 0;
  T.event.listen('crash-filed', ({ payload: f }) => {
    crashFiledAt = Date.now();
    const staff = $('cr-staff');
    staff.textContent = `Staff already have this report as ${f.reportId}.` + (f.likelyCause ? ` Likely cause: ${f.likelyCause}` : '') + ' Mention the number if you ask for help.';
    staff.hidden = false;
    $('cr-ask').hidden = true;
  });
  $('cr-copy').onclick = async () => {
    try { await navigator.clipboard.writeText(lastReport); $('cr-note').textContent = 'Copied. Paste it with Ctrl+V.'; }
    catch { $('cr-note').textContent = "Couldn't copy. Open the log folder and send the newest game-….txt file."; }
    $('cr-note').hidden = false;
  };
  $('rq-close').onclick = () => { showPage(page); if (!gameRunning && !busy) check(); };
  $('files-mods').onclick = () => showRequiredMods();
  $('rq-vortex-go').onclick = async () => {
    rqError(null);
    try {
      $('rq-vortex-said').textContent = await invoke('vortex_connect');
      $('rq-vortex-said').hidden = false;
      await refreshMods();
    } catch (e) { rqError(e); }
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
    stopVerifyWait();
    dgBusy(true);
    try { dgDone(await invoke('mark_game_ok')); } catch (e) { dgFail(e); }
  };
  $('c-skse-recheck').onclick = refreshState;
  $('si-join-go').onclick = () => invoke('open_invite', { url: (status && status.discordInvite) || lastSeen.invite }).catch(() => {});
  $('f-go').onclick = () => { if (!signedIn()) { showSignIn(); return; } showPage('home'); check(); };
  $('f-retry').onclick = () => window.location.reload();
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

  // The window opens hidden (tauri.conf.json) and is shown here, once the
  // first screen is drawn with its fonts, so no blank or half-drawn frame shows.
  let windowShown = false;
  function showWindow() {
    if (windowShown) return;
    windowShown = true;
    invoke('window_ready').catch(() => {});
  }
  setTimeout(showWindow, 1500);

  (async () => {
    try {
      await refreshState();
    } catch (e) {
      $('c-game').querySelector('b').textContent = 'Launcher settings unavailable';
      $('c-game').querySelector('small').textContent = String(e);
      $('c-game-pick').hidden = true;
      $('c-skse').hidden = true;
      $('f-go').disabled = true;
      $('f-retry').hidden = false;
      $('first-error').textContent = 'The launcher could not load or save its settings. Check the app settings folder, then try again. If it keeps happening, send launcher.log to staff.';
      $('first-error').hidden = false;
      setPlay('wait', 'SETUP ERROR');
      setChip('warn', 'Not checked');
      setStatus('Launcher settings could not load. Use Try again after fixing the settings folder.', true);
      showSheet('first');
      showWindow();
      return;
    }
    // Music plays unless it was switched off in Settings; nothing to answer.
    invoke('music_start').catch(() => {});
    loadStatus();
    setInterval(loadStatus, 30 * 1000);
    const signIn = refreshAuth();
    setInterval(recheckAuth, 10 * 60 * 1000);
    setInterval(() => { if (auth && auth.offline) recheckAuth(); }, 60 * 1000);
    if (Array.isArray(lastSeen.news) && lastSeen.news.length) showNews(lastSeen.news);
    // Shown after a drawn frame; a hidden window may not draw, so not later than 150 ms.
    const drawn = () => new Promise(r => { requestAnimationFrame(() => requestAnimationFrame(r)); setTimeout(r, 150); });
    Promise.race([document.fonts.ready, new Promise(r => setTimeout(r, 400))]).then(drawn).then(showWindow);
    if (!ready_()) {
      await signIn;
      showSheet('first');
      setStatus('Finish setup to play');
    } else {
      // The game check and the Discord check run side by side.
      signIn.then(() => { if (!signedIn()) showSignIn(auth && auth.message); });
      await check(false, { signIn });
    }
    checkSelfUpdate();
  })();
})();
