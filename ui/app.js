// Launcher UI. Talks to the Rust side in src-tauri/src/main.rs through invoke().
(() => {
  const T = window.__TAURI__;
  const invoke = (cmd, args) => T.core.invoke(cmd, args);
  const $ = id => document.getElementById(id);

  const ICON_OK = '<path d="M5 12.5l4.5 4.5L19 7.5"/>';
  const ICON_BAD = '<circle cx="12" cy="12" r="9"/><path d="M12 7.5v5.5M12 16.5v.01"/>';
  const ICON_BUSY = '<path d="M20 12a8 8 0 1 1-2.3-5.7"/><path d="M20 4v5h-5"/>';
  const THUMBS = ['art/thumb-castle.jpg', 'art/thumb-peak.jpg', 'art/thumb-lake.jpg', 'art/thumb-city.jpg'];

  let state = null;       // get_state
  let pending = null;     // check
  let status = null;      // status.json
  let busy = false;
  let playMode = 'wait';  // wait | play | update | retry
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
      const on = status ? status.online : !!pending;
      const who = status && typeof status.players === 'number' ? ` · ${status.players}${status.maxPlayers ? '/' + status.maxPlayers : ''} players` : '';
      parts.push(`<span><i class="dot${on ? '' : ' off'}"></i>${on ? 'Online' : 'Offline'}${who}</span>`);
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
    if (id === 'settings') {
      for (const nav of Object.values(PAGES)) $(nav).removeAttribute('aria-current');
      $('nav-settings').setAttribute('aria-current', 'page');
    }
  }
  const ready_ = () => state.game && state.game.hasSkse;
  function leaveSheet() {
    if (!ready_()) { renderGame(); showSheet('first'); return; }
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
    renderRow($('c-game'), ...gameRow); renderRow($('c-skse'), ...skseRow);
    $('c-game-pick').textContent = g ? 'Change' : 'Choose folder';
    $('c-skse-recheck').hidden = !!(g && g.hasSkse);
    $('f-go').disabled = !ready_();
  }

  async function refreshState() {
    state = await invoke('get_state');
    $('set-path').value = state.config.gameDir || '';
    $('set-close').setAttribute('aria-checked', state.config.closeOnLaunch);
    $('set-bg').setAttribute('aria-checked', state.config.backgroundUpdates);
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
        setStatus("Couldn't reach the server. Checking again in a minute. " + e, true);
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
      setStatus('The update stopped. ' + e, true);
      pending = null;
    } finally {
      off();
      $('progress').hidden = true;
      busy = false;
    }
  }

  function ready() {
    pending = { ...pending, files: 0, remove: 0 };
    setPlay('play', 'PLAY');
    setChip('ok', 'Up to date');
    setStatus(null);
    loadFiles();
  }

  async function onPlay() {
    if (busy) return;
    if (playMode === 'retry') return check();
    if (playMode === 'update') return update();
    if (playMode !== 'play') return;
    setPlay('wait', 'LAUNCHING');
    setStatus('Starting Skyrim through SKSE…');
    try {
      await invoke('play');
      setTimeout(() => { if (playMode === 'wait' && !busy) ready(); }, 8000);
    } catch (e) {
      setPlay('play', 'PLAY');
      setStatus("Skyrim didn't start. " + e, true);
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
    if (typeof status.players === 'number') $('srv-players').textContent = status.maxPlayers ? `${status.players} / ${status.maxPlayers}` : status.players;
    if (status.sinceReset) $('srv-reset').textContent = status.sinceReset;
    if (Array.isArray(status.news) && status.news.length) {
      $('news').innerHTML = newsHtml(status.news.slice(0, 3));
      $('news-full').innerHTML = newsHtml(status.news.slice(0, 20), true);
    }
  }

  // ---------- launcher self-update ----------
  async function checkSelfUpdate() {
    try {
      const upd = await T.updater.check();
      if (!upd) return;
      $('self-update-text').textContent = `Launcher ${upd.version} is ready to install.`;
      $('self-update').hidden = false;
      $('self-update-go').onclick = async () => {
        $('self-update-go').disabled = true;
        $('self-update-text').textContent = 'Downloading the new launcher…';
        await upd.downloadAndInstall();
        await T.process.relaunch();
      };
    } catch (e) {
      console.warn('launcher update check failed', e);
    }
  }

  // ---------- wiring ----------
  const win = T.window.getCurrentWindow();
  $('w-min').onclick = () => win.minimize();
  $('w-close').onclick = () => win.close();
  $('w-settings').onclick = $('nav-settings').onclick = $('t-settings').onclick = () => showSheet('settings');
  $('play').onclick = onPlay;
  for (const [name, nav] of Object.entries(PAGES)) $(nav).onclick = () => { if (ready_()) showPage(name); else leaveSheet(); };
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
    const prefs = { closeOnLaunch: $('set-close').getAttribute('aria-checked') === 'true', backgroundUpdates: $('set-bg').getAttribute('aria-checked') === 'true' };
    await invoke('set_prefs', { prefs });
    Object.assign(state.config, prefs);
  });
  $('c-game-pick').onclick = pickFolder;
  $('c-skse-recheck').onclick = refreshState;
  $('f-go').onclick = () => { showPage('home'); check(); };

  (async () => {
    await refreshState();
    loadStatus();
    if (!ready_()) {
      showSheet('first');
      setStatus('Finish setup to play');
    } else {
      await check();
      loadStatus();
    }
    checkSelfUpdate();
  })();
})();
