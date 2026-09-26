// Launcher UI. Talks to the Rust side in src-tauri/src/main.rs through invoke().
(() => {
  const T = window.__TAURI__;
  const invoke = (cmd, args) => T.core.invoke(cmd, args);
  const $ = id => document.getElementById(id);

  const ICON_OK = '<path d="M5 12.5l4.5 4.5L19 7.5"/>';
  const ICON_BAD = '<circle cx="12" cy="12" r="9"/><path d="M12 7.5v5.5M12 16.5v.01"/>';

  let state = null;       // result of get_state
  let pending = null;     // result of check
  let busy = false;

  const mb = n => (n / 1048576).toFixed(n < 10485760 ? 1 : 0) + ' MB';
  const esc = s => String(s).replace(/[&<>"]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

  function setMeta(title, sub, isError) {
    $('meta').innerHTML = `<span${isError ? ' class="error"' : ''}>${title}</span>` + (sub ? `<span class="mono">${esc(sub)}</span>` : '');
  }
  function setPlay(label, enabled) {
    $('play').textContent = label;
    $('play').disabled = !enabled;
  }

  // ---- sheets ----
  function showSheet(id) {
    $('first').hidden = id !== 'first';
    $('settings').hidden = id !== 'settings';
    $('r-play').setAttribute('aria-current', id !== 'settings');
    $('r-settings').setAttribute('aria-current', id === 'settings');
  }

  function renderCheck(el, ok, title, detail) {
    el.className = 'check ' + (ok ? 'ok' : 'bad');
    el.querySelector('svg').innerHTML = ok ? ICON_OK : ICON_BAD;
    el.querySelector('b').textContent = title;
    el.querySelector('small').textContent = detail;
  }

  function renderFirstRun() {
    const g = state.game;
    renderCheck($('c-game'), !!g, g ? 'Skyrim Special Edition found' : 'Skyrim Special Edition not found',
      g ? g.dir : (state.gameError || 'Pick the folder that has SkyrimSE.exe in it.'));
    $('c-game-pick').textContent = g ? 'Change' : 'Choose folder';
    renderCheck($('c-skse'), !!(g && g.hasSkse), g && g.hasSkse ? 'SKSE installed' : 'SKSE is missing',
      g && g.hasSkse ? 'skse64_loader.exe found' : 'skse64_loader.exe was not found in the game folder');
    $('c-skse').querySelector('button').hidden = !!(g && g.hasSkse);
    $('f-go').disabled = !(g && g.hasSkse);
  }

  async function refreshState() {
    state = await invoke('get_state');
    $('set-path').value = state.config.gameDir || '';
    $('set-close').setAttribute('aria-checked', state.config.closeOnLaunch);
    $('set-bg').setAttribute('aria-checked', state.config.backgroundUpdates);
    $('set-version').textContent = 'Launcher ' + state.launcherVersion;
    return state;
  }

  async function pickFolder() {
    const dir = await T.dialog.open({ directory: true, title: 'Choose your Skyrim Special Edition folder' });
    if (!dir) return false;
    try {
      await invoke('set_game_dir', { dir });
      $('set-error').hidden = true;
    } catch (e) {
      $('set-error').textContent = e;
      $('set-error').hidden = false;
      state.gameError = String(e);
    }
    await refreshState();
    return true;
  }

  // ---- update flow ----
  async function check(verifyAll = false) {
    busy = true;
    setPlay('CHECKING', false);
    setMeta('Checking for updates…');
    try {
      pending = await invoke('check', { verifyAll });
      $('s-addr').textContent = `${pending.server.ip} : ${pending.server.port}`;
      $('s-build').textContent = pending.build;
      if (pending.files || pending.remove) {
        if (state.config.backgroundUpdates || verifyAll) return await update(verifyAll);
        setPlay('UPDATE', true);
        setMeta(`<b>Build ${esc(pending.build)}</b> is available`, `${pending.files} changed files · ${mb(pending.bytes)}`);
      } else {
        ready(pending.build);
      }
    } catch (e) {
      setPlay('RETRY', true);
      setMeta("Couldn't reach the server", String(e), true);
      pending = null;
    } finally {
      busy = false;
    }
  }

  async function update(verifyAll = false) {
    busy = true;
    setPlay('UPDATING', false);
    setMeta(`Updating to <b>build ${esc(pending.build)}</b>`, `${pending.files} files · ${mb(pending.bytes)}`);
    $('progress').hidden = false;
    let last = { t: performance.now(), b: 0 };
    const off = await T.event.listen('sync-progress', ({ payload: p }) => {
      const pct = p.bytesTotal ? (p.bytesDone / p.bytesTotal) * 100 : 100;
      $('p-bar').style.width = pct.toFixed(1) + '%';
      $('p-num').textContent = `${Math.min(p.filesDone + (p.file ? 1 : 0), p.filesTotal)} / ${p.filesTotal} files · ${Math.round(pct)}%`;
      $('p-file').textContent = p.file || 'All files match the server';
      const now = performance.now();
      if (now - last.t > 500) {
        $('p-speed').textContent = mb(((p.bytesDone - last.b) / (now - last.t)) * 1000) + '/s';
        last = { t: now, b: p.bytesDone };
      }
    });
    try {
      const build = await invoke('update', { verifyAll });
      $('progress').hidden = true;
      ready(build, 'Updated');
    } catch (e) {
      setPlay('RETRY', true);
      setMeta('The update stopped', String(e), true);
      pending = null;
    } finally {
      off();
      busy = false;
    }
  }

  function ready(build, word = 'Up to date') {
    pending = { ...pending, files: 0, remove: 0 };
    setPlay('PLAY', true);
    setMeta(`<b>${word}</b> · build ${esc(build)}`, pending && pending.server ? `${pending.server.name} · ${pending.server.ip}:${pending.server.port}` : '');
    Sky.setDawn(0.15);
  }

  async function onPlay() {
    if (busy) return;
    const label = $('play').textContent;
    if (label === 'RETRY') return check();
    if (label === 'UPDATE') return update();
    setPlay('LAUNCHING', false);
    setMeta('<b>Starting Skyrim</b> through SKSE', pending ? `Connecting to ${pending.server.ip}:${pending.server.port}` : '');
    Sky.setDawn(1);
    try {
      await invoke('play');
      setTimeout(() => { setPlay('PLAY', true); Sky.setDawn(0.15); }, 8000);
    } catch (e) {
      setPlay('PLAY', true);
      setMeta("Skyrim didn't start", String(e), true);
      Sky.setDawn(0.15);
    }
  }

  // ---- side panel: optional status.json from the server ----
  async function loadStatus() {
    const s = await invoke('server_status').catch(() => null);
    if (!s) {
      $('s-text').textContent = pending ? 'Server reachable' : 'Server status unavailable';
      $('s-dot').classList.toggle('off', !pending);
      return;
    }
    $('s-dot').classList.toggle('off', !s.online);
    $('s-text').textContent = s.online ? 'Online' : 'Offline';
    if (typeof s.players === 'number') {
      $('s-stats').classList.remove('off');
      $('s-players').textContent = s.players;
      $('s-max').textContent = s.maxPlayers ? `of ${s.maxPlayers} players` : 'players';
      $('s-uptime').textContent = s.sinceReset || '–';
    }
    if (Array.isArray(s.news) && s.news.length) {
      $('news').innerHTML = s.news.slice(0, 4).map(n =>
        `<article><time>${esc(n.date || '')}</time><h3>${esc(n.title || '')}</h3><p>${esc(n.body || '')}</p></article>`).join('');
      $('news-wrap').hidden = false;
    }
  }

  // ---- launcher self-update ----
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

  // ---- wiring ----
  const win = T.window.getCurrentWindow();
  $('w-min').onclick = () => win.minimize();
  $('w-close').onclick = () => win.close();
  $('play').onclick = onPlay;
  $('r-play').onclick = () => { if (state.game && state.game.hasSkse) showSheet(null); };
  $('r-settings').onclick = () => showSheet('settings');
  $('set-done').onclick = () => { if (state.game && state.game.hasSkse) { showSheet(null); if (!pending && !busy) check(); } else { renderFirstRun(); showSheet('first'); } };
  $('set-browse').onclick = pickFolder;
  $('set-verify').onclick = () => { if (!busy) { showSheet(null); check(true); } };
  document.querySelectorAll('.switch').forEach(s => s.onclick = async () => {
    s.setAttribute('aria-checked', s.getAttribute('aria-checked') !== 'true');
    const prefs = { closeOnLaunch: $('set-close').getAttribute('aria-checked') === 'true', backgroundUpdates: $('set-bg').getAttribute('aria-checked') === 'true' };
    await invoke('set_prefs', { prefs });
    Object.assign(state.config, prefs);
  });
  $('c-game-pick').onclick = async () => { await pickFolder(); renderFirstRun(); };
  $('c-skse-recheck').onclick = async () => { await refreshState(); renderFirstRun(); };
  $('f-go').onclick = () => { showSheet(null); check(); };

  (async () => {
    await refreshState();
    if (!state.game || !state.game.hasSkse) {
      renderFirstRun();
      showSheet('first');
    } else {
      await check();
    }
    loadStatus();
    checkSelfUpdate();
  })();
})();
