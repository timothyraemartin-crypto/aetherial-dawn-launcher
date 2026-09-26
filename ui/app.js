// Launcher UI. Talks to the Rust side in src-tauri/src/main.rs through invoke().
(() => {
  const T = window.__TAURI__;
  const invoke = (cmd, args) => T.core.invoke(cmd, args);
  const $ = id => document.getElementById(id);

  const ICON_OK = '<path d="M5 12.5l4.5 4.5L19 7.5"/>';
  const ICON_BAD = '<circle cx="12" cy="12" r="9"/><path d="M12 7.5v5.5M12 16.5v.01"/>';
  const ICON_BUSY = '<path d="M20 12a8 8 0 1 1-2.3-5.7"/><path d="M20 4v5h-5"/>';

  let state = null;     // get_state
  let pending = null;   // check
  let busy = false;
  let playMode = 'wait'; // wait | play | update | retry

  const files_ = n => `${n} file${n === 1 ? '' : 's'}`;
  const mb = n => (n / 1048576).toFixed(n < 10485760 ? 1 : 0) + ' MB';
  const esc = s => String(s ?? '').replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

  // ---------- scenery ----------
  // Real key art dropped into ui/art/hero.jpg replaces the painted hero.
  Scenery.mount($('hero-art'), { seed: 11, castle: 0.74, sunX: 0.55 });
  Scenery.mount($('side-art'), { seed: 5, layers: 4, tall: 1.3, mist: false });
  Scenery.mount($('server-art'), { seed: 23, castle: 0.5, sunX: 0.4, mist: false });
  Scenery.mount($('pack-art'), { seed: 41, layers: 4, sunX: 0.7, mist: false });
  Scenery.mount($('band-art'), { seed: 77, castle: 0.3, tall: 0.8, sunX: 0.2 });
  const hero = $('hero-img');
  hero.onload = () => { hero.hidden = false; };
  hero.src = 'art/hero.jpg';

  // ---------- play button + status line ----------
  function setPlay(mode, label) {
    playMode = mode;
    $('play-label').textContent = label;
    $('play').disabled = mode === 'wait';
    $('play-wrap').classList.toggle('off', mode === 'wait');
  }
  function setStatus(parts, isError) {
    $('play-status').innerHTML = parts.filter(Boolean).map(p => `<span${isError ? ' class="error"' : ''}>${esc(p)}</span>`).join('');
  }
  function setChip(kind, text) {
    const c = $('pack-chip');
    c.className = 'chip ' + (kind === 'ok' ? '' : kind);
    c.querySelector('svg').innerHTML = kind === 'ok' ? ICON_OK : kind === 'warn' ? ICON_BAD : ICON_BUSY;
    c.querySelector('span').textContent = text;
  }

  // ---------- pages + sheets ----------
  function showPage(page) {
    $('page-home').hidden = page !== 'home';
    $('page-files').hidden = page !== 'files';
    for (const [id, p] of [['nav-home', 'home'], ['nav-files', 'files'], ['nav-settings', 'settings']]) {
      if (p === page) $(id).setAttribute('aria-current', 'page'); else $(id).removeAttribute('aria-current');
    }
    if (page === 'files') loadFiles();
  }
  function showSheet(id) {
    $('first').hidden = id !== 'first';
    $('settings').hidden = id !== 'settings';
    if (id === 'settings') { $('nav-settings').setAttribute('aria-current', 'page'); $('nav-home').removeAttribute('aria-current'); $('nav-files').removeAttribute('aria-current'); }
  }
  function closeSheet() {
    if (!(state.game && state.game.hasSkse)) { renderFirstRun(); showSheet('first'); return; }
    showSheet(null);
    showPage($('page-files').hidden ? 'home' : 'files');
    if (!pending && !busy) check();
  }

  function renderRow(el, ok, title, detail) {
    el.className = el.className.replace(/\b(ok|bad)\b/g, '').trim() + (ok ? ' ok' : ' bad');
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
    $('f-go').disabled = !(g && g.hasSkse);
  }
  const renderFirstRun = renderGame;

  async function refreshState() {
    state = await invoke('get_state');
    $('set-path').value = state.config.gameDir || '';
    $('set-close').setAttribute('aria-checked', state.config.closeOnLaunch);
    $('set-bg').setAttribute('aria-checked', state.config.backgroundUpdates);
    $('set-version').textContent = 'Launcher ' + state.launcherVersion;
    $('side-ver').textContent = 'Launcher v' + state.launcherVersion;
    renderGame();
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
  function showServer(m) {
    $('srv-name').textContent = m.server.name;
    $('srv-addr').textContent = `${m.server.ip}:${m.server.port}`;
  }

  async function check(verifyAll = false) {
    if (busy) return;
    busy = true;
    setPlay('wait', 'CHECKING');
    setChip('busy', 'Checking');
    setStatus(['Checking for updates…']);
    try {
      pending = await invoke('check', { verifyAll });
      showServer(pending);
      $('pack-meta').textContent = `Build ${pending.build}`;
      if (pending.files || pending.remove) {
        busy = false;
        if (state.config.backgroundUpdates || verifyAll) return await update(verifyAll);
        setPlay('update', 'UPDATE');
        setChip('warn', 'Update available');
        setStatus([`Build ${pending.build} available`, files_(pending.files), mb(pending.bytes)]);
      } else {
        ready(pending.build, 'Ready to play');
      }
    } catch (e) {
      setPlay('retry', 'RETRY');
      setChip('warn', 'Not checked');
      setStatus(["Couldn't reach the server"], true);
      $('band-cite').textContent = String(e);
      pending = null;
    } finally {
      busy = false;
    }
  }

  async function update(verifyAll = false) {
    if (busy) return;
    busy = true;
    setPlay('wait', 'UPDATING');
    setChip('busy', 'Updating');
    setStatus([`Updating to build ${pending.build}`, mb(pending.bytes)]);
    $('band-quote').hidden = true; $('band-cite').hidden = true; $('progress').hidden = false;
    let last = { t: performance.now(), b: 0 };
    const off = await T.event.listen('sync-progress', ({ payload: p }) => {
      const pct = p.bytesTotal ? (p.bytesDone / p.bytesTotal) * 100 : 100;
      $('p-bar').style.width = pct.toFixed(1) + '%';
      $('p-num').textContent = `${Math.min(p.filesDone + (p.file ? 1 : 0), p.filesTotal)} / ${files_(p.filesTotal)} · ${Math.round(pct)}%`;
      $('p-file').textContent = p.file || 'All files match the server';
      const now = performance.now();
      if (now - last.t > 500) {
        $('p-speed').textContent = mb(((p.bytesDone - last.b) / (now - last.t)) * 1000) + '/s';
        last = { t: now, b: p.bytesDone };
      }
    });
    try {
      const build = await invoke('update', { verifyAll });
      ready(build, verifyAll ? 'All files verified' : 'Updated');
    } catch (e) {
      setPlay('retry', 'RETRY');
      setChip('warn', 'Update failed');
      setStatus(['The update stopped: ' + e], true);
      pending = null;
    } finally {
      off();
      $('progress').hidden = true; $('band-quote').hidden = false; $('band-cite').hidden = false;
      busy = false;
    }
  }

  function ready(build, word) {
    pending = { ...pending, files: 0, remove: 0 };
    setPlay('play', 'PLAY');
    setChip('ok', 'Up to date');
    setStatus([word, `Build ${build}`, state.game && state.game.hasSkse ? 'SKSE ready' : '']);
    $('band-cite').textContent = '— AETHERIAL DAWN';
    loadFiles();
  }

  async function onPlay() {
    if (busy) return;
    if (playMode === 'retry') return check();
    if (playMode === 'update') return update();
    if (playMode !== 'play') return;
    setPlay('wait', 'LAUNCHING');
    setStatus(['Starting Skyrim through SKSE', pending ? `${pending.server.ip}:${pending.server.port}` : '']);
    try {
      await invoke('play');
      setTimeout(() => { if (playMode === 'wait' && !busy) ready(pending.build, 'Ready to play'); }, 8000);
    } catch (e) {
      setPlay('play', 'PLAY');
      setStatus(["Skyrim didn't start: " + e], true);
    }
  }

  // ---------- files page ----------
  async function loadFiles() {
    const files = await invoke('files').catch(() => null);
    if (!files) {
      $('files-summary').textContent = '';
      $('files-body').innerHTML = '<tr><td colspan="2">The file list loads after the launcher reaches the server.</td></tr>';
      return;
    }
    const total = files.reduce((n, f) => n + f.size, 0);
    $('files-summary').textContent = `${files_(files.length)} · ${mb(total)}`;
    $('files-body').innerHTML = files.map(f => `<tr><td>${esc(f.path)}</td><td>${mb(f.size)}</td></tr>`).join('');
    $('pack-meta').textContent = `Build ${pending ? pending.build : ''} · ${files_(files.length)} · ${mb(total)}`;
  }

  // ---------- server status.json (optional) ----------
  async function loadStatus() {
    const s = await invoke('server_status').catch(() => null);
    const online = $('srv-online');
    if (!s) {
      online.className = 'online' + (pending ? '' : ' off');
      online.querySelector('span').textContent = pending ? 'Reachable' : 'Status unavailable';
      return;
    }
    online.className = 'online' + (s.online ? '' : ' off');
    online.querySelector('span').textContent = s.online ? 'Online' : 'Offline';
    if (s.description) $('srv-desc').textContent = s.description;
    if (typeof s.players === 'number') $('srv-players').textContent = s.maxPlayers ? `${s.players} / ${s.maxPlayers}` : `${s.players} online`;
    if (s.sinceReset) $('srv-reset').textContent = `Reset ${s.sinceReset} ago`;
    if (Array.isArray(s.news) && s.news.length) {
      $('news').innerHTML = s.news.slice(0, 5).map((n, i) =>
        `<article class="news-item"><div class="nthumb"><canvas data-seed="${101 + i * 17}"></canvas></div>
           <div><h3>${esc(n.title)}</h3><time>${esc(n.date)}</time><p>${esc(n.body)}</p></div></article>`).join('');
      $('news').querySelectorAll('canvas').forEach(c => Scenery.mount(c, { seed: +c.dataset.seed, layers: 4, mist: false, sunX: 0.3 + (c.dataset.seed % 5) / 10 }));
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
  $('play').onclick = onPlay;
  $('nav-home').onclick = () => { showSheet(null); if (state.game && state.game.hasSkse) showPage('home'); else closeSheet(); };
  $('nav-files').onclick = () => { showSheet(null); if (state.game && state.game.hasSkse) showPage('files'); else closeSheet(); };
  $('nav-settings').onclick = () => showSheet('settings');
  $('t-settings').onclick = () => showSheet('settings');
  $('set-done').onclick = closeSheet;
  $('set-browse').onclick = pickFolder;
  $('g-change').onclick = async () => { await pickFolder(); if (!(state.game && state.game.hasSkse)) closeSheet(); else check(); };
  $('t-verify').onclick = $('files-verify').onclick = () => check(true);
  $('t-check').onclick = () => check();
  $('t-folder').onclick = () => invoke('open_game_folder').catch(e => setStatus([String(e)], true));
  $('pack-view').onclick = () => showPage('files');
  $('srv-copy').onclick = async () => {
    const addr = $('srv-addr').textContent;
    try { await navigator.clipboard.writeText(addr); $('srv-copy').querySelector('span').textContent = 'Copied'; }
    catch { $('srv-copy').querySelector('span').textContent = addr; }
    setTimeout(() => { $('srv-copy').querySelector('span').textContent = 'Copy address'; }, 1800);
  };
  document.querySelectorAll('.switch').forEach(s => s.onclick = async () => {
    s.setAttribute('aria-checked', s.getAttribute('aria-checked') !== 'true');
    const prefs = { closeOnLaunch: $('set-close').getAttribute('aria-checked') === 'true', backgroundUpdates: $('set-bg').getAttribute('aria-checked') === 'true' };
    await invoke('set_prefs', { prefs });
    Object.assign(state.config, prefs);
  });
  $('c-game-pick').onclick = pickFolder;
  $('c-skse-recheck').onclick = refreshState;
  $('f-go').onclick = () => { showSheet(null); check(); };

  (async () => {
    await refreshState();
    loadStatus();
    if (!state.game || !state.game.hasSkse) {
      showSheet('first');
      setStatus(['Finish setup to play']);
    } else {
      await check();
      loadStatus();
    }
    checkSelfUpdate();
  })();
})();
