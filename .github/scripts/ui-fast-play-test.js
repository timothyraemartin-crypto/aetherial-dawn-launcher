// Proves the launcher Play path: a returning player waits for fresh game,
// and Discord checks before PLAY is enabled. The launcher's page runs
// in headless Chrome with a fake launcher
// back end: a fake Discord answer, a fake game-files answer and a fake
// latest.json. No packages needed: Chrome's --dump-dom prints the result.
//
//   node .github/scripts/ui-fast-play-test.js [path to chrome]
'use strict';
const fs = require('fs'), os = require('os'), path = require('path');
const { execFileSync } = require('child_process');

const UI = path.join(__dirname, '..', '..', 'ui');
const VERSION = '9.9.9';
const DIR = 'C:\\Games\\Skyrim Special Edition';

function findChrome() {
  const given = process.argv[2] || process.env.CHROME;
  const candidates = [given,
    'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
    'C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe',
    '/usr/bin/google-chrome', '/usr/bin/chromium', '/usr/bin/chromium-browser'].filter(Boolean);
  const found = candidates.find(c => fs.existsSync(c));
  if (!found) throw new Error('no Chrome found; pass its path');
  return found;
}

// The fake back end, run in the page before app.js. Each scenario sets
// window.__S: what the last session left, and how each fake answer looks.
function fakeBackEnd() {
  const S = window.__S;
  const t0 = performance.now();
  const log = window.__T = { invokes: [], labels: [], statuses: [], lastReady: null, newsHeights: [], actions: [] };
  const listeners = {};
  if (S.pageSize) Object.assign(document.documentElement.style, { width: S.pageSize[0] + 'px', height: S.pageSize[1] + 'px' });
  const at = () => Math.round(performance.now() - t0);
  if (S.authIntervalMs || S.statusPollMs) {
    const every = window.setInterval.bind(window);
    window.setInterval = (fn, ms, ...args) => every(fn, ms === 10 * 60 * 1000 && S.authIntervalMs ? S.authIntervalMs : ms === 30 * 1000 && S.statusPollMs ? S.statusPollMs : ms, ...args);
  }
  try { localStorage.clear(); if (S.seed) localStorage.setItem('ad.lastReady', JSON.stringify(S.seed)); if (S.hintSeen) localStorage.setItem('ad.f3hint', '1'); } catch (_) {}
  const game = Object.assign({ needed: false, installed: '1.6.1170.0', target: '1.6.1170.0', skseOk: true, canDowngrade: true }, S.game || {});
  if (S.clipboard) {
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText: async t => { if (S.clipboard !== 'ok') throw new Error('denied'); log.copied = t; } } });
    document.execCommand = cmd => { if (cmd === 'copy' && S.clipboard === 'fallback') { log.copied = document.activeElement && document.activeElement.value; return true; } return false; };
  }
  const answers = {
    get_state: () => ({ launcherVersion: S.version, config: { gameDir: S.dir, closeOnLaunch: false, backgroundUpdates: !!S.backgroundUpdates, shareHealth: true, music: false, onlyServerMods: true }, game: S.gameGone ? null : { dir: S.dir, hasSkse: S.hasSkse !== false }, gameError: S.gameGone || null }),
    auth_status: () => (S.authSequence && S.authSequence.shift()) || ((S.authAfter && (S.authCalls = (S.authCalls || 0) + 1) > 1) ? S.authAfter : S.auth),
    check: () => Object.assign({ build: 'B2', server: { name: 'Aetherial Dawn', ip: '127.0.0.1', port: 7777 }, files: 0, remove: 0, bytes: 0, game }, (S.checkSequence && S.checkSequence.shift()) || S.check || {}),
    update: () => null,
    server_status: () => ({ online: true, players: 1, maxPlayers: 50, ...(typeof S.statusNow === 'object' ? S.statusNow : {}), ...(S.statusFileDown ? { launcher: undefined } : { fromFile: true }), discordInvite: S.invite, news: [{ title: 'News', date: '28 Sep', body: 'Body.' }, { title: 'More', date: '27 Sep', body: 'Body.' }] }),
    setup_state: () => S.setup || [{ id: 'folder', title: 'Choose your Skyrim folder', hint: 'x', done: true }],
    bug_state: () => ({ available: true, summary: 't2 pos 4,5,6 cell Whiterun world Tamriel nearby: Dovah (120)' }),
    send_bug: a => { (window.__T.bugs ||= []).push(a.note); return 'R-test'; },
    files: () => [], play: () => S.playWarnings || null,
    patch_game: () => S.patchResult || { ...game, needed: false },
    game_running: () => !!S.outsideGame,
    self_update_begin: () => !S.updateReservationFails,
    self_update_end: () => null,
    close_game_for_update: () => { S.outsideGame = false; return true; },
    raise_launcher: () => null,
    export_key_saved: () => !!S.keySaved,
    export_key_save: a => { log.keySent = a.key; S.keySaved = true; return 'Saved for Staffer (Premium).'; },
    export_key_forget: () => { S.keySaved = false; return null; },
    plain_error: a => a.text,
    log_ui: a => { (log.uiLogs = log.uiLogs || []).push(String(a.msg)); return null; },
    auth_begin: () => 'st',
    // S.signIn: when the launcher has the answer, and an error each poll gives.
    auth_poll: () => !S.signIn ? { status: 'done', account: { discordUsername: 'Player' } } : S.signIn.error ? { status: 'save_failed', kind: 'denied', message: S.signIn.error } : S.signIn.refuseFirst && !S.refused ? (S.refused = true, { status: 'refused', message: 'An old refusal.' })
      // A finished sign-in is saved by the launcher, so auth_status says so from then on.
      : at() - (S.signInAt || 0) >= S.signIn.doneAfter ? (S.auth = { signedIn: true, account: { discordUsername: 'Player' } }, { status: 'done', account: { discordUsername: 'Player' } }) : { status: 'pending' },
  };
  const delay = Object.assign({ get_state: 20, auth_status: 300, check: 900, update: 600, server_status: 300, play: 50, default: 10 }, S.delay || {});
  window.__TAURI__ = {
    core: { invoke: (cmd, args) => new Promise((res, rej) => {
      log.invokes.push([at(), 'ask', cmd]);
      const pickDelay = cmd === 'set_game_dir' && S.picks && (S.picks.find(p => p.dir === (args || {}).dir) || {}).delay;
      setTimeout(() => {
        log.invokes.push([at(), 'answer', cmd]);
        if (cmd === 'get_state' && S.stateFails) return rej('settings are not writable');
        if (cmd === 'server_status' && S.statusNow === 'down') return rej('no answer');
        if (cmd === 'send_bug' && S.bugError) return rej(S.bugError);
        if (cmd === 'auth_status' && S.authFails) return rej('network down');
        if (cmd === 'game_running' && (S.gameCheckFails || (S.gameCheckFailsFrom && (S.gameChecks = (S.gameChecks || 0) + 1) >= S.gameCheckFailsFrom))) return rej('process list unavailable');
        if (cmd === 'export_key_save' && S.keyError) { log.keySent = args.key; return rej(S.keyError); }
        if (cmd === 'set_music' && S.setMusicError) return rej(S.setMusicError);
        if (cmd === 'set_prefs' && S.setPrefsError) return rej(S.setPrefsError);
        if (cmd === 'set_game_dir' && S.setDirError) return rej(S.setDirError);
        if (cmd === 'set_game_dir' && S.picks && S.picks.find(p => p.dir === args.dir && p.error)) return rej(S.picks.find(p => p.dir === args.dir).error);
        if (cmd === 'update' && S.updateError) return rej(S.updateError);
        if (cmd === 'check' && S.checkError) return rej(S.checkError);
        if (cmd === 'files_state' && S.fileStatesError) return rej(S.fileStatesError);
        if (cmd === 'diagnostics') return res('diagnostics text');
        if (cmd === 'play' && S.playError && !S.played) { S.played = true; return rej(S.playError); }
        res(answers[cmd] ? answers[cmd](args || {}) : null);
      }, pickDelay || (delay[cmd] ?? delay.default));
    }) },
    event: { listen: async (name, callback) => {
      (listeners[name] ||= []).push(callback);
      return () => { listeners[name] = listeners[name].filter(fn => fn !== callback); };
    } },
    window: { getCurrentWindow: () => ({ minimize: async () => {}, close: async () => {}, unminimize: async () => {}, setFocus: async () => {}, show: async () => {} }) },
    // S.picks: what each press of "Choose folder" returns in turn (dir null = cancelled).
    dialog: { open: async () => S.picks ? (S.picks[(S.picked = (S.picked || 0) + 1) - 1] || {}).dir || null : S.pickDir || null },
    // The fake latest.json: this launcher is the newest, unless S.update names a newer one.
    updater: { check: () => new Promise((r, no) => setTimeout(() => S.updaterFails ? no(new Error('offline')) : r(S.update ? {
      version: S.update,
      download: () => { log.invokes.push([at(), 'ask', 'updater_download']); return new Promise(res => setTimeout(res, 200)); },
      install: async () => { log.invokes.push([at(), 'ask', 'updater_install']); },
      downloadAndInstall: async () => { log.invokes.push([at(), 'ask', 'updater_install']); },
    } : null), S.updaterDelay ?? 100)) },
    process: { relaunch: async () => {} },
  };
  document.addEventListener('DOMContentLoaded', () => {
    const label = document.getElementById('play-label'), btn = document.getElementById('play');
    const note = () => log.labels.push([at(), label.textContent + (btn.disabled ? ' [off]' : '')]);
    note();
    new MutationObserver(note).observe(label, { childList: true, characterData: true, subtree: true });
    new MutationObserver(note).observe(btn, { attributes: true, attributeFilter: ['disabled'] });
    const status = document.getElementById('status');
    const noteStatus = () => { log.statuses.push([at(), status.textContent]); (log.statusH ||= []).push([at(), status.offsetHeight, !!document.getElementById('status-copy')]); };
    noteStatus();
    new MutationObserver(noteStatus).observe(status, { childList: true, characterData: true, subtree: true });
    const box = () => log.newsHeights.push([at(), document.getElementById('news-box').offsetHeight]);
    box();
    setInterval(box, 100);
    // Scenarios can press before or after current checks complete.
    // The player presses Play as soon as it shows enabled (and once more later).
    // The player picks a folder in the first-run sheet.
    if (S.pickDir) setTimeout(() => document.getElementById('c-game-pick').click(), 3000);
    for (const p of S.picks || []) setTimeout(() => document.getElementById('c-game-pick').click(), p.at);
    if (S.seps) {
      const states = [
        '<span><i class="dot"></i>Server online · 12 of 100 players</span><span>Build B2</span><span>v9.9.9</span>',
        '<span>Skyrim 1.6.1170 matches the server version. You are ready to play and the launcher checked every file.</span><span>Build B2</span><span>v9.9.9</span>',
        '<span><i class="dot"></i>Server online</span><span class="error">Server maintenance: back at 18:00 UTC, thanks for waiting here</span><span>Build B2</span><span>v9.9.9</span>',
        '<span><i class="dot"></i>Server online · 1 of 50 players</span><span class="error">Maintenance in 5 min</span><span>Build B2</span><span>v9.9.9</span>',
        '<span><i class="dot"></i>Server online · 1 of 50 players</span><span class="error">Maintenance at 18:00 UTC</span><span>Build B2</span><span>v9.9.9</span>',
        '<span><i class="dot"></i>Server online · 1 of 50 players</span><span class="error">Server maintenance 18:00 UTC</span><span>Build B2</span><span>v9.9.9</span>',
        '<span><i class="dot"></i>Server online · 1 of 50 players</span><span class="error">Server maintenance at 18:00 UTC</span><span>Build B2</span><span>v9.9.9</span>',
      ];
      log.seps = [];
      states.forEach((html, n) => {
        setTimeout(() => { document.getElementById('status').innerHTML = html; }, 1300 + n * 500);
        setTimeout(() => {
          const kids = [...document.getElementById('status').children].filter(el => el.tagName === 'SPAN' && !el.classList.contains('copy-row'));
          let prevTop = null;
          log.seps.push(kids.map((el, i) => {
            const top = el.getBoundingClientRect().top, rowStart = prevTop === null || top > prevTop + 2;
            if (prevTop === null || top > prevTop) prevTop = top;
            const c = getComputedStyle(el, '::before').content;
            return { text: el.textContent.slice(0, 14), rowStart, sep: c === '"|"', error: el.classList.contains('error') };
          }));
        }, 1300 + n * 500 + 250);
      });
    }
    if (S.signIn) setTimeout(() => { S.signInAt = at(); const pn = document.querySelector('#signin .panel'); log.siBefore = pn.getBoundingClientRect().height; document.getElementById('si-go').click(); }, 1500);
    // Cancel while the first answer is on its way, then sign in again.
    if (S.signIn && S.signIn.cancelAt) setTimeout(() => document.getElementById('si-cancel').click(), S.signIn.cancelAt);
    if (S.signIn && S.signIn.restartAt) setTimeout(() => { document.getElementById('si-cancel').click(); document.getElementById('si-go').click(); }, S.signIn.restartAt);
    for (const t of S.clicks || []) setTimeout(() => { log.invokes.push([at(), 'click', label.textContent]); btn.click(); }, t);
    for (const action of S.actions || []) setTimeout(() => {
      if (action.kind === 'click') document.getElementById(action.id).click();
      if (action.kind === 'event') for (const callback of listeners[action.name] || []) callback({ payload: action.payload });
      if (action.kind === 'status') S.statusNow = action.value;
      if (action.kind === 'statusFile') S.statusFileDown = action.value;
      if (action.kind === 'type') document.getElementById(action.id).value = action.text;
      if (action.kind === 'open') document.getElementById(action.id).open = true;
      if (action.kind === 'focus') document.getElementById(action.id).focus();
      if (action.kind === 'key') document.dispatchEvent(new KeyboardEvent('keydown', { key: action.key, shiftKey: !!action.shiftKey, bubbles: true, cancelable: true }));
      (log.setupHidden ||= []).push(document.getElementById('setup').hidden);
      log.actions.push([at(), action.kind, action.id || action.name || action.key, document.activeElement && document.activeElement.id]);
    }, action.at);
    setTimeout(() => {
      try { log.lastReady = JSON.parse(localStorage.getItem('ad.lastReady')); } catch (_) {}
      log.signinShown = !document.getElementById('signin').hidden;
      const stepsOf = id => { const ol = document.querySelector('#' + id + ' .steps'); return ol ? [...ol.children].map(li => li.textContent.trim().replace(/,\s*/, ' ') + (li.getAttribute('aria-current') ? '*' : '')) : null; };
      log.steps = { first: stepsOf('first'), signin: stepsOf('signin') };
      log.siAfter = document.querySelector('#signin .panel').getBoundingClientRect().height;
      { const o = document.querySelector('#signin .steps'); const l = o && o.children; log.stepColors = l ? { other: getComputedStyle(l[2]).color, now: getComputedStyle(l[1]).color, muted: getComputedStyle(document.documentElement).getPropertyValue('--muted').trim() } : null; log.slotRole = (document.querySelector('#signin .si-slot') || {}).getAttribute ? document.querySelector('#signin .si-slot').getAttribute('role') : null; }
      if (S.jank) {
        const wrap = document.getElementById('play-wrap'), top = () => Math.round(wrap.getBoundingClientRect().top * 10) / 10;
        const ids = ['play-hint', 'whatsnew', 'play-gate', 'progress', 'setup'];
        const before = ids.map(i => document.getElementById(i)).filter(Boolean).map(el => [el, el.hidden]);
        before.forEach(([el]) => { el.hidden = true; });
        const base0 = top();
        log.jank = { base: base0 };
        for (const [el, was] of before) { el.hidden = false; log.jank[el.id] = Math.round((top() - base0) * 10) / 10; el.hidden = true; }
        before.forEach(([el, was]) => { el.hidden = was; });
        // The status line under the button changes with state: a short line, an error with the Copy button, maintenance, a long list.
        const st = document.getElementById('status'), keep = st.innerHTML;
        const samples = {
          'a short status line': '<span><i class="dot"></i>Server online</span><span>v0.1.0</span>',
          'an error with Copy details': '<span class="error">Couldn\'t reach the Aetherial Dawn server. Check your internet; the launcher tries again every minute.</span><span class="copy-row"><button class="btn small" id="x">Copy details for staff</button><span class="copy-note"></span></span><span>v0.1.0</span>',
          'maintenance': '<span><i class="dot"></i>Server online</span><span class="error">Server maintenance: back at 18:00 UTC, thanks for waiting</span><span>v0.1.0</span>',
          'a long missing-mods line': '<span>Skyrim Script Extender, Unofficial Skyrim Special Edition Patch, SkyUI, Address Library for SKSE Plugins, powerofthree\'s Tweaks and 6 more still need game files</span><span>v0.1.0</span>',
        };
        log.jankStatus = {};
        st.innerHTML = samples['a short status line'];
        const sBase = top();
        for (const [name, html] of Object.entries(samples)) { st.innerHTML = html; log.jankStatus[name] = Math.round((top() - sBase) * 10) / 10; }
        st.innerHTML = keep;
      }
      { const cs = getComputedStyle(document.documentElement), g = n => cs.getPropertyValue(n).trim().toLowerCase(); log.tokens = { text: g('--text'), muted: g('--muted'), faint: g('--faint'), ok: g('--ok'), warn: g('--warn'), gold: g('--gold'), frost: g('--frost'), glow: g('--glow'), danger: g('--danger') }; }
      { const probe = c => { const d = document.createElement('i'); d.style.color = c; document.body.appendChild(d); const v = getComputedStyle(d).color; d.remove(); return v; }; log.okColor = probe('var(--ok)'); log.goldColor = probe('var(--gold)'); { const e = document.createElement('span'); e.className = 'error'; document.body.appendChild(e); log.errorColor = getComputedStyle(e).color; e.remove(); const t = document.createElement('div'); t.className = 'toast bad'; document.body.appendChild(t); log.badToastColor = getComputedStyle(t).color; t.remove(); } log.faintColor = probe('var(--faint)'); }
      log.siGoLabel = document.getElementById('si-go').textContent.trim();
      log.siGoDisabled = document.getElementById('si-go').disabled;
      log.status = document.getElementById('status').textContent;
      const live = document.getElementById('status-live');
      log.announced = live ? live.textContent : null;
      // Before the focus-ring check below moves focus to PLAY.
      log.focus = document.activeElement && document.activeElement.id;
      const playBtn = document.getElementById('play');
      playBtn.focus({ focusVisible: true });
      log.playRing = playBtn.matches(':focus-visible') ? getComputedStyle(document.getElementById('play-wrap')).outlineStyle : 'not focus-visible';
      const wrap = document.getElementById('play-wrap'), hint = document.getElementById('play-hint');
      log.playState = wrap.dataset.state || null;
      log.hint = hint && !hint.hidden && getComputedStyle(hint).visibility !== 'hidden' ? hint.textContent : null;
      log.hintBox = hint && !hint.hidden ? hint.getBoundingClientRect().height > 0 : false;
      try { log.hintStored = localStorage.getItem('ad.f3hint'); } catch (_) {}
      log.progress = { file: document.getElementById('p-file').textContent, speed: document.getElementById('p-speed').textContent, num: document.getElementById('p-num').textContent };
      log.readyAnim = getComputedStyle(document.getElementById('play-wrap')).animationName;
      log.copied = log.copied || null;
      log.statusUi = { text: status.textContent, copy: !!document.getElementById('status-copy'), errorClass: !!document.querySelector('#status .error'), minH: parseFloat(getComputedStyle(status).minHeight) || 0, lineH: parseFloat(getComputedStyle(status).lineHeight) || 0, dividerAfterError: (() => { const e = document.querySelector('#status .error'); const n = e && e.nextElementSibling && e.nextElementSibling.tagName === 'SPAN' ? e.nextElementSibling : null; return n ? getComputedStyle(n, '::before').content : null; })() };
      { const n = document.getElementById('min-notice'); log.minNotice = n && !n.hidden ? n.textContent : null; }
      const groupOf = id => { const el = document.getElementById(id); const hs = [...document.querySelectorAll('#settings h3')]; let found = null; for (const h of hs) if (h.compareDocumentPosition(el) & Node.DOCUMENT_POSITION_FOLLOWING) found = h.textContent; return found; };
      log.settingsGroups = { heads: [...document.querySelectorAll('#settings h3')].map(h => h.textContent), close: groupOf('set-close'), bg: groupOf('set-bg'), share: groupOf('set-share'), folder: groupOf('set-path') };
      const saved = document.getElementById('set-saved'), failed = document.getElementById('set-failed');
      log.saved = (failed && failed.textContent) || (saved ? saved.textContent : null);
      const okEl = document.getElementById('set-saved'), badEl = document.getElementById('set-failed');
      const toast = badEl && badEl.textContent ? badEl : okEl;
      log.toastRoles = { ok: okEl.getAttribute('role'), bad: badEl ? badEl.getAttribute('role') : null, emptyDisplay: getComputedStyle(badEl || okEl).display, emptyOpacity: getComputedStyle(badEl || okEl).opacity };
      log.toastInModal = !!okEl.closest('[aria-modal="true"]') && !!badEl.closest('[aria-modal="true"]');
      { const sc = document.querySelector('#settings .sheet-scroll'); log.toastOutsideBlur = !sc || (!sc.contains(okEl) && !sc.contains(badEl)); }
      if (S.scrollSettings) { const sh = document.querySelector('#settings .sheet-scroll'); sh.querySelector('.panel').style.minHeight = '2000px'; sh.scrollTop = sh.scrollHeight; log.settingsScrolled = sh.scrollTop; }
      const tr = toast.getBoundingClientRect();
      log.toast = { top: Math.round(tr.top), bottom: Math.round(tr.bottom), fixed: getComputedStyle(toast).position, color: getComputedStyle(toast).color, cls: toast.className, shown: tr.width > 0 && tr.height > 0 };
      log.musicChecked = document.getElementById('set-music').getAttribute('aria-checked');
      log.bgChecked = document.getElementById('set-bg').getAttribute('aria-checked');
      log.gameRow = document.querySelector('#c-game small').textContent;
      const join = document.getElementById('si-join');
      log.join = join.hidden ? null : join.textContent;
      const errLink = document.querySelector('#si-error button');
      log.errorLink = errLink ? errLink.textContent : null;
      log.skse = ['g-skse', 'c-skse'].map(id => { const el = document.getElementById(id); return el.querySelector('b').textContent + el.querySelector('small').textContent; });
      log.skseRecheck = !document.getElementById('c-skse-recheck').hidden;
      log.playDisabled = btn.disabled;
      log.bug = { bugs: log.bugs || [], shown: !document.getElementById('bug').hidden, summary: document.getElementById('bg-summary').textContent, result: document.getElementById('bg-result').hidden ? null : document.getElementById('bg-result').textContent, sendOff: document.getElementById('bg-send').disabled };
      const wn = document.getElementById('whatsnew');
      log.whatsNew = wn.hidden ? null : { lines: [...document.querySelectorAll('#whatsnew-list li')].map(li => li.textContent), open: wn.open };
      const setupBox = document.getElementById('setup');
      { const b = setupBox.getBoundingClientRect(); log.setupInView = !setupBox.hidden && b.width > 100 && b.left >= 0 && b.top >= 0 && b.right <= innerWidth && b.bottom <= innerHeight; log.setupClear = !setupBox.hidden && document.elementFromPoint(b.left + 20, b.top + 20)?.closest('#setup') !== null; }
      log.setup = setupBox.hidden ? null : [...document.querySelectorAll('#setup-list li')].map(li => li.className + ':' + li.querySelector('b').textContent);
      const gate = document.getElementById('play-gate');
      log.gate = gate.hidden ? null : { msg: document.getElementById('play-gate-msg').textContent, anyway: !document.getElementById('gate-anyway').hidden, wait: !document.getElementById('gate-wait').hidden };
      log.modal = [...document.querySelectorAll('.sheet')].find(el => !el.hidden)?.id || null;
      log.navInert = document.getElementById('nav-home').closest('.nav').inert;
      log.titlebarInert = document.querySelector('.titlebar').inert;
      log.firstError = document.getElementById('first-error').hidden ? null : document.getElementById('first-error').textContent;
      log.retryShown = !document.getElementById('f-retry').hidden;
      log.statusRole = document.getElementById('status-live').getAttribute('role');
      log.progressRole = document.getElementById('p-progress').getAttribute('role');
      const rect = sel => { const b = document.querySelector(sel).getBoundingClientRect(); return [Math.round(b.top), Math.round(b.bottom)]; };
      log.layout = { height: document.querySelector('.app').offsetHeight, width: document.querySelector('.app').offsetWidth, titlebar: rect('.titlebar'), dock: rect('.dock'), play: rect('#play') };
      log.signInError = document.getElementById('si-error').hidden ? null : document.getElementById('si-error').textContent;
      log.me = document.getElementById('me').hidden ? null : document.getElementById('me-name').textContent;
      const pre = document.createElement('pre');
      pre.id = 'ui-test-result';
      pre.textContent = JSON.stringify(log);
      document.body.appendChild(pre);
    }, S.end || 6000);
  });
}

const seed = { dir: DIR, build: 'B1', version: VERSION, play: true, status: { online: true, players: 1, maxPlayers: 50 } };
const ok = { signedIn: true, account: { discordUsername: 'Player' }, offline: false, locked: false };
const base = { version: VERSION, dir: DIR, auth: ok, seed, clicks: [1250] };

const played = r => r.invokes.some(i => i[1] === 'ask' && i[2] === 'play');
const askedAt = (r, cmd) => (r.invokes.find(i => i[1] === 'ask' && i[2] === cmd) || [null])[0];
const answeredAt = (r, cmd) => (r.invokes.find(i => i[1] === 'answer' && i[2] === cmd) || [null])[0];
const firstLabel = (r, text) => (r.labels.find(l => l[1] === text) || [null])[0];
const lastLabel = r => r.labels[r.labels.length - 1][1];

const scenarios = [
  { name: 'Bug report: the saved F10 snapshot is shown and Send needs consent', s: { ...base, clicks: [], end: 3500, actions: [{ at: 1500, kind: 'click', id: 'w-settings' }, { at: 1800, kind: 'click', id: 'set-bug' }] }, expect: r => [
    ['the sheet shows what the game saved', r.bug.shown && /pos 4,5,6 cell Whiterun/.test(r.bug.summary), r.bug.summary],
    ['Send is off until the player agrees', r.bug.sendOff === true],
  ] },
  { name: 'Bug report: with consent the note is sent and the player is thanked', s: { ...base, clicks: [], end: 4500, actions: [{ at: 1500, kind: 'click', id: 'w-settings' }, { at: 1800, kind: 'click', id: 'set-bug' }, { at: 2300, kind: 'type', id: 'bg-note', text: 'door stuck' }, { at: 2400, kind: 'click', id: 'bg-consent' }, { at: 2500, kind: 'click', id: 'bg-send' }] }, expect: r => [
    ['the note was sent once', JSON.stringify(r.bugs) === '["door stuck"]', JSON.stringify(r.bugs)],
    ['the player is told it went to staff with the number', /Sent to staff as R-test/.test(r.bug.result || ''), r.bug.result],
  ] },
  { name: 'Bug report: a failed send says so and Send is offered again', s: { ...base, clicks: [], bugError: 'offline', end: 4500, actions: [{ at: 1500, kind: 'click', id: 'w-settings' }, { at: 1800, kind: 'click', id: 'set-bug' }, { at: 2400, kind: 'click', id: 'bg-consent' }, { at: 2500, kind: 'click', id: 'bg-send' }] }, expect: r => [
    ['the player sees an error', /offline/.test(r.bug.result || ''), r.bug.result],
    ['Send can be pressed again', r.bug.sendOff === false],
  ] },
  { name: 'Other mods installed: Play stops and names them, and the game is not started', s: { ...base, playError: 'MODS_FOUND:SkyUI_SE.esp, CrashLogger.dll, textures (212 files), and 3 more' }, expect: r => [
    ['the game was asked to start once and refused', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'play').length === 1],
    ['the status says other mods block joining and names them', /can't join with other mods/.test(r.status) && r.status.includes('SkyUI_SE.esp, CrashLogger.dll, textures (212 files), and 3 more'), r.status],
    ['Play is offered again', lastLabel(r) === 'PLAY'],
  ] },
  { name: 'update available: the notes are listed under the update, closed, with no dialog', s: { ...base, clicks: [], check: { files: 2, bytes: 5000000, notes: ['New tavern in Whiterun', 'Fixed <b>crash</b> at the docks'] }, end: 4000 }, expect: r => [
    ['the lines are listed', !!r.whatsNew && r.whatsNew.lines.length === 2 && r.whatsNew.lines[0] === 'New tavern in Whiterun'],
    ['markup in a note is shown as text', !!r.whatsNew && r.whatsNew.lines[1].includes('<b>crash</b>')],
    ['it starts closed', !!r.whatsNew && r.whatsNew.open === false],
    ['no sheet or dialog is open', r.modal === null],
    ['the button offers UPDATE', lastLabel(r) === 'UPDATE'],
  ] },
  { name: 'update finished: the notes go away and the button returns to PLAY', s: { ...base, clicks: [1250], seedPlay: true, check: { files: 2, bytes: 5000000, notes: ['New tavern in Whiterun'] }, end: 7000 }, expect: r => [
    ['the update ran', askedAt(r, 'update') !== null],
    ['the notes are gone once it finished', r.whatsNew === null],
    ['the button returns to PLAY after the update', r.labels.some(l => l[0] > askedAt(r, 'update') && l[1].startsWith('PLAY'))],
  ] },
  { name: 'no update: the notes are not shown', s: { ...base, clicks: [], check: { files: 0, notes: ['Old note'] }, end: 4000 }, expect: r => [
    ['it is hidden', r.whatsNew === null],
  ] },
  { name: 'new player: the setup checklist lists what is still open', s: { ...base, clicks: [], seed: null, setup: [
    { id: 'folder', title: 'Choose your Skyrim folder', hint: 'Open Settings.', done: true },
    { id: 'skse', title: 'SKSE is installed', hint: 'Press Play.', done: false },
    { id: 'signin', title: 'Sign in with Discord', hint: 'Press Sign in.', done: false } ], end: 4000 }, expect: r => [
    ['the checklist is shown with each step', !!r.setup && r.setup.length === 3],
    ['the box sits inside the window, not behind other parts', r.setupInView === true && r.setupClear === true],
    ['done steps are marked and open ones are not', !!r.setup && r.setup[0].startsWith('done:') && r.setup[1].startsWith('todo:') && r.setup[2].startsWith('todo:')],
  ] },
  { name: 'setup checklist: only on Home, back when the player returns', s: { ...base, clicks: [], setup: [{ id: 'skse', title: 'SKSE is installed', hint: 'Press Play.', done: false }], end: 5000,
    actions: [{ kind: 'click', id: 'nav-server', at: 3000 }, { kind: 'click', id: 'nav-news', at: 3600 }, { kind: 'click', id: 'nav-home', at: 4200 }, { kind: 'click', id: 'nav-home', at: 4300 }] }, expect: r => [
    ['the checklist is hidden on the Server and News pages', r.setupHidden.slice(0, 2).every(h => h === true)],
    ['it is back on Home', !!r.setup && r.setup.length === 1],
  ] },
  { name: 'setup checklist: hidden once the game has been started before', s: { ...base, clicks: [], hintSeen: true, setup: [{ id: 'skse', title: 'SKSE is installed', hint: 'Press Play.', done: false }], end: 4000 }, expect: r => [
    ['the checklist is not shown', r.setup === null],
  ] },
  { name: 'setup checklist: nothing to show when every step is done', s: { ...base, clicks: [], end: 4000 }, expect: r => [
    ['the checklist is not shown', r.setup === null],
  ] },
  { name: 'server offline at Play: warned, the game waits for Play anyway', s: { ...base, statusNow: { online: false }, actions: [{ kind: 'click', id: 'gate-anyway', at: 3500 }], end: 5000 }, expect: r => [
    ['the game starts only after Play anyway', askedAt(r, 'play') > 3500],
  ] },
  { name: 'server offline at Play: the warning stays and the game never starts', s: { ...base, statusNow: { online: false }, end: 3200 }, expect: r => [
    ['the game never starts', !played(r)],
    ['the warning names the offline server and offers both choices', !!r.gate && /offline/.test(r.gate.msg) && r.gate.anyway && r.gate.wait],
  ] },
  { name: 'server full: Wait and join starts the game once a slot opens', s: { ...base, statusNow: { online: true, players: 50, maxPlayers: 50 }, statusPollMs: 1000,
    actions: [{ kind: 'click', id: 'gate-wait', at: 3000 }, { kind: 'status', value: { online: true, players: 49, maxPlayers: 50 }, at: 4000 }], end: 6500 }, expect: r => [
    ['the game starts after the slot opens', askedAt(r, 'play') > 4000],
    ['the game starts once', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'play').length === 1],
  ] },
  { name: 'waiting for a full server, then the status call fails: the game does not start', s: { ...base, statusNow: { online: true, players: 50, maxPlayers: 50 }, statusPollMs: 1000,
    actions: [{ kind: 'click', id: 'gate-wait', at: 3000 }, { kind: 'status', value: 'down', at: 4000 }], end: 7000 }, expect: r => [
    ['the game never starts', !played(r)],
    ['the notice says it cannot reach the server and is still waiting', !!r.gate && /Cannot reach the server, still waiting/.test(r.gate.msg)],
  ] },
  { name: 'server full: still waiting, the game does not start', s: { ...base, statusNow: { online: true, players: 50, maxPlayers: 50 }, statusPollMs: 1000, actions: [{ kind: 'click', id: 'gate-wait', at: 3000 }], end: 5500 }, expect: r => [
    ['the game never starts', !played(r)],
    ['the warning says it is waiting', !!r.gate && /Waiting/.test(r.gate.msg) && !r.gate.wait],
  ] },
  { name: 'maintenance: Play is blocked and Play anyway is not offered', s: { ...base, statusNow: { maintenance: 'Back at 18:00 UTC.' }, end: 3500 }, expect: r => [
    ['the game never starts', !played(r)],
    ['the message is shown and only Wait and join is offered', !!r.gate && /Back at 18:00/.test(r.gate.msg) && !r.gate.anyway && r.gate.wait],
    ['the status line carries the message', /Back at 18:00/.test(r.status)],
  ] },
  { name: 'returning player: Play waits for fresh game and Discord checks', s: base, expect: r => [
    ['PLAY is not enabled before fresh checks answer', firstLabel(r, 'PLAY') !== null && firstLabel(r, 'PLAY') >= Math.max(answeredAt(r, 'auth_status'), answeredAt(r, 'check'))],
    ['the game starts once', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'play').length === 1],
    ['the game starts only after the Discord answer', askedAt(r, 'play') >= answeredAt(r, 'auth_status')],
    ['the game starts only after the game-files answer', askedAt(r, 'play') >= answeredAt(r, 'check')],
    ['cached server health is not shown before the current answer', r.statuses.filter(s => s[0] < answeredAt(r, 'server_status')).every(s => !/Server online/.test(s[1]))],
  ] },
  { name: 'banned since last time: SIGN IN, the game never starts', s: { ...base, auth: { signedIn: false, message: 'You are banned from the Discord.' } }, expect: r => [
    ['the game never starts', !played(r)],
    ['nothing is downloaded', askedAt(r, 'update') === null],
    ['the button ends on SIGN IN', lastLabel(r) === 'SIGN IN'],
    ['the sign-in window is open', r.signinShown],
    ['next start does not open on PLAY', r.lastReady && r.lastReady.play === false],
  ] },
  { name: 'not a member: the sign-in window links the Discord invite', s: { ...base, auth: { signedIn: false }, invite: 'https://discord.gg/aetherial' }, expect: r => [
    ['the invite shows', r.join === 'Not a member yet? Join here: discord.gg/aetherial', r.join],
  ] },
  { name: 'not a member, and the refusal carries the invite: one clickable link', s: { ...base, clicks: [], auth: { signedIn: false, message: 'Join the Aetherial Dawn Discord first: https://discord.gg/aetherial. Then press Sign in with Discord again.' }, invite: 'https://discord.gg/aetherial' }, expect: r => [
    ['the refusal shows the link as a button', r.errorLink === 'discord.gg/aetherial', r.errorLink],
    ['the separate join line is not shown twice', r.join === null, r.join],
  ] },
  { name: 'not a member, the refusal without a link: the join line shows', s: { ...base, auth: { signedIn: false, message: 'Join the Aetherial Dawn Discord first, then sign in again.' }, invite: 'https://discord.gg/aetherial' }, expect: r => [
    ['the join line shows', r.join === 'Not a member yet? Join here: discord.gg/aetherial', r.join],
  ] },
  { name: 'SKSE not installed yet: nothing asks the player to get it', s: { ...base, hasSkse: false, seed: null, clicks: [] }, expect: r => [
    ['the row reads "SKSE / Installed for you when you press Play."', r.skse.every(t => t === 'SKSEInstalled for you when you press Play.'), JSON.stringify(r.skse)],
    ['no Check again button', !r.skseRecheck],
  ] },
  { name: 'SKSE installed: no file name under it', s: base, expect: r => [
    ['the row reads "SKSE installed" alone', r.skse.every(t => t === 'SKSE installed'), JSON.stringify(r.skse)],
  ] },
  { name: 'Discord cannot be reached: SIGN IN, the game never starts', s: { ...base, authFails: true }, expect: r => [
    ['the game never starts', !played(r)],
    ['the button ends on SIGN IN', lastLabel(r) === 'SIGN IN'],
  ] },
  { name: 'login service unavailable: retry is available and the game never starts', s: { ...base, auth: { ...ok, offline: true, locked: true, message: 'Login service unavailable.' } }, expect: r => [
    ['the game never starts', !played(r)],
    ['the button offers a sign-in retry', lastLabel(r) === 'RETRY SIGN-IN'],
    ['the status explains the outage', /Login service unavailable/.test(r.status)],
    ['next start does not open on PLAY', r.lastReady && r.lastReady.play === false],
  ] },
  { name: 'login service recovers when Retry sign-in is pressed', s: { ...base, clicks: [1250], authSequence: [
    { ...ok, offline: true, locked: true, message: 'Login service unavailable.' }, ok,
  ] }, expect: r => [
    ['Discord is checked twice', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'auth_status').length === 2],
    ['Play returns only after the second check', lastLabel(r) === 'PLAY' && firstLabel(r, 'PLAY') >= r.invokes.filter(i => i[1] === 'answer' && i[2] === 'auth_status')[1][0]],
    ['retry does not launch the game', !played(r)],
  ] },
  { name: 'sign-out and browser sign-in preserve a pending game update', s: { ...base, clicks: [], end: 6500,
    authIntervalMs: 1200, authSequence: [ok, { signedIn: false, message: 'Sign in again.' }],
    check: { build: 'B2', files: 3, bytes: 3000000 },
    actions: [{ at: 1700, kind: 'click', id: 'si-go' }],
  }, expect: r => [
    ['the saved sign-in was rechecked and rejected', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'auth_status').length >= 2],
    ['the browser sign-in triggered a fresh manifest check', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'check').length >= 2],
    ['Play never appears against the unapplied build', firstLabel(r, 'PLAY') === null && !played(r)],
    ['the update remains the hero action', lastLabel(r) === 'UPDATE', lastLabel(r)],
  ] },
  { name: 'an update found before Play offers the update', s: { ...base, clicks: [], check: { build: 'B2', files: 3, bytes: 3000000 } }, expect: r => [
    ['the game never starts', !played(r)],
    ['the button ends on UPDATE', lastLabel(r) === 'UPDATE'],
    ['next start does not open on PLAY', r.lastReady && r.lastReady.play === false],
  ] },
  { name: 'background update completes before Play becomes available', s: { ...base, clicks: [1900], backgroundUpdates: true, check: { build: 'B2', files: 3, bytes: 3000000 } }, expect: r => [
    ['the button shows UPDATING', firstLabel(r, 'UPDATING [off]') !== null],
    ['the game starts only after the update', askedAt(r, 'play') !== null && askedAt(r, 'play') >= answeredAt(r, 'update')],
  ] },
  { name: 'wrong game version (no fix possible): Play stops', s: { ...base, game: { needed: true, canDowngrade: false, reason: 'Skyrim is not the version the server needs.' } }, expect: r => [
    ['the game never starts', !played(r)],
    ['no version fix starts by itself', askedAt(r, 'patch_game') === null],
    ['the button ends on WRONG VERSION', lastLabel(r) === 'WRONG VERSION [off]'],
    ['next start does not open on PLAY', r.lastReady && r.lastReady.play === false],
  ] },
  { name: 'wrong game version (fixable): an early press is not taken as a yes to the fix', s: { ...base, clicks: [60], game: { needed: true, canDowngrade: true, reason: 'Skyrim needs changing.' } }, expect: r => [
    ['the game never starts', !played(r)],
    ['the fix waits for a press made after the reason shows', askedAt(r, 'patch_game') === null],
    ['the status line gives the reason', /Skyrim needs changing/.test(r.status)],
  ] },
  { name: 'Play resumes after version repair', s: { ...base,
    game: { needed: true, canDowngrade: true, reason: 'Skyrim needs changing.' },
    checkSequence: [
      { game: { needed: true, canDowngrade: true, reason: 'Skyrim needs changing.' } },
      { game: { needed: false, installed: '1.6.1170.0', target: '1.6.1170.0', skseOk: true } },
    ],
    patchResult: { needed: false, installed: '1.6.1170.0', target: '1.6.1170.0', skseOk: true },
  }, expect: r => [
    ['the game version patch ran once', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'patch_game').length === 1],
    ['Play resumed after the repair', askedAt(r, 'play') !== null],
  ] },
  { name: 'a helper mod could not be installed: the game still starts and the reason stays shown', s: { ...base, playWarnings: ["Crash Logger 1.25.0 isn't installed: the download didn't get through. Skyrim starts without it; Play tries again next time."] }, expect: r => [
    ['the game starts', played(r)],
    ['the status line keeps the warning', /Crash Logger 1\.25\.0 isn't installed/.test(r.status), r.status],
  ] },
  { name: 'a message on the status line is announced to screen readers', s: { ...base, game: { needed: true, canDowngrade: false, reason: 'Skyrim is not the version the server needs.' } }, expect: r => [
    ['screen readers get the message', /not the version the server needs/.test(r.announced || ''), r.announced],
  ] },
  { name: 'PLAY shows a keyboard focus ring', s: { ...base, clicks: [] }, expect: r => [
    ['PLAY\'s focus ring is drawn on its frame', r.playRing === 'solid', r.playRing],
  ] },
  { name: 'launcher updated since last time: no early PLAY', s: { ...base, seed: { ...seed, version: '0.0.1' }, clicks: [] }, expect: r => [
    ['PLAY is not enabled before the checks', firstLabel(r, 'PLAY') === null || firstLabel(r, 'PLAY') >= answeredAt(r, 'check')],
  ] },
  { name: 'another Skyrim folder: no early PLAY', s: { ...base, seed: { ...seed, dir: 'D:\\Other' }, clicks: [] }, expect: r => [
    ['PLAY is not enabled before the checks', firstLabel(r, 'PLAY') === null || firstLabel(r, 'PLAY') >= answeredAt(r, 'check')],
  ] },
  { name: 'newer launcher found when Play is pressed: it installs first and the game does not start yet', s: { ...base, update: '9.9.10', updaterDelay: 900, delay: { play: 3000 } }, expect: r => [
    ['the launcher installs its update', askedAt(r, 'updater_install') !== null],
    ['the game is not started by the old launcher', !played(r), JSON.stringify(r.invokes.filter(i => /updater|play/.test(i[2])))],
  ] },
  { name: 'Play with no update available: the game starts', s: { ...base, clicks: [1250] }, expect: r => [
    ['nothing is installed', askedAt(r, 'updater_install') === null],
    ['the game starts', played(r)],
  ] },
  { name: 'Play when the update check fails or is offline: the game still starts', s: { ...base, updaterFails: true, clicks: [1250] }, expect: r => [
    ['nothing is installed', askedAt(r, 'updater_install') === null],
    ['the game starts', played(r)],
  ] },
  { name: 'launcher update found while Skyrim runs from Steam or Vortex: it waits', s: { ...base, update: '9.9.10', outsideGame: true, clicks: [] }, expect: r => [
    ['the launcher asks Windows whether Skyrim runs', askedAt(r, 'game_running') !== null],
    ['the update never installs while that Skyrim runs', askedAt(r, 'updater_install') === null, JSON.stringify(r.invokes.filter(i => /updater|game_running/.test(i[2])))],
  ] },
  { name: 'newer launcher while in game: countdown, then Skyrim is closed and the update installs', s: { ...base, update: '9.9.10', outsideGame: true, statusNow: { launcher: { forceCloseSecs: 5 } }, clicks: [], end: 16000 }, expect: r => [
    ['the window is raised for the countdown', askedAt(r, 'raise_launcher') !== null],
    ['Skyrim is closed only after the countdown', askedAt(r, 'close_game_for_update') !== null && askedAt(r, 'close_game_for_update') - askedAt(r, 'updater_download') >= 4500, JSON.stringify(r.invokes.filter(i => /updater|close_game/.test(i[2])))],
    ['the update installs after the game is closed', askedAt(r, 'updater_install') > askedAt(r, 'close_game_for_update')],
  ] },
  { name: 'newer launcher while in game with the kill switch on: nothing is closed', s: { ...base, update: '9.9.10', outsideGame: true, statusNow: { launcher: { forceClose: false, forceCloseSecs: 5 } }, clicks: [], end: 12000 }, expect: r => [
    ['Skyrim is left alone', askedAt(r, 'close_game_for_update') === null],
    ['no install while the game runs', askedAt(r, 'updater_install') === null],
  ] },
  { name: 'launcher update found but the game check fails: it waits', s: { ...base, update: '9.9.10', gameCheckFails: true, clicks: [] }, expect: r => [
    ['the launcher asked whether Skyrim runs', askedAt(r, 'game_running') !== null],
    ['the update never installs while the game state is unknown', askedAt(r, 'updater_install') === null, JSON.stringify(r.invokes.filter(i => /updater|game_running/.test(i[2])))],
  ] },
  { name: 'the game check fails between download and install: it waits', s: { ...base, update: '9.9.10', gameCheckFailsFrom: 2, clicks: [] }, expect: r => [
    ['the update downloads', askedAt(r, 'updater_download') !== null],
    ['it asks again right before installing', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'game_running').length >= 2],
    ['the update never installs while the game state is unknown', askedAt(r, 'updater_install') === null, JSON.stringify(r.invokes.filter(i => /updater|game_running/.test(i[2])))],
  ] },
  { name: 'Play reserves the launch slot just before update install: updater waits', s: { ...base, update: '9.9.10', updateReservationFails: true, clicks: [] }, expect: r => [
    ['the updater attempted the final reservation', askedAt(r, 'self_update_begin') !== null],
    ['the update never installs after reservation was refused', askedAt(r, 'updater_install') === null],
  ] },
  { name: 'launcher update found with nothing running: it installs', s: { ...base, update: '9.9.10', clicks: [] }, expect: r => [
    ['the update installs', askedAt(r, 'updater_install') !== null],
  ] },
  { name: 'Skyrim running: PLAY does not come back, and a second press starts nothing', s: { ...base, clicks: [60, 9500], end: 12500 }, expect: r => [
    ['the game starts once', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'play').length === 1],
    ['the button shows IN GAME once the launch settles', firstLabel(r, 'IN GAME [off]') !== null],
    ['PLAY is never offered again while the game runs', !r.labels.some(l => l[0] > answeredAt(r, 'play') && l[1] === 'PLAY'), JSON.stringify(r.labels.slice(-4))],
  ] },
  { name: 'first start, a wrong folder, then a cancelled pick: the reason stays and nothing is saved', s: { ...base, seed: null, clicks: [], picks: [{ at: 3000, dir: 'D:\\Games', error: "SkyrimSE.exe isn't in D:\\Games." }, { at: 3500, dir: null }] }, expect: r => [
    ['the folder was sent to the launcher once', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'set_game_dir').length === 1],
    ['the reason still shows', /isn't in D:/.test(r.gameRow), r.gameRow],
  ] },
  { name: 'first start, a second pick while the first is still checked: ignored, so answers never cross', s: { ...base, seed: null, clicks: [], picks: [{ at: 3000, dir: 'D:\\Games', error: "SkyrimSE.exe isn't in D:\\Games.", delay: 1500 }, { at: 3300, dir: 'C:\\Games\\Skyrim Special Edition' }] }, expect: r => [
    ['the launcher checks one folder at a time', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'set_game_dir').length === 1],
    ['the answer shown is the one for the folder checked', /isn't in D:/.test(r.gameRow), r.gameRow],
  ] },
  { name: 'the saved folder stopped being Skyrim: Steam\'s other copy is only suggested', s: { ...base, gameGone: "E:\\Skyrim Special Edition can't be opened. If Skyrim is now in C:\\Steam\\steamapps\\common\\Skyrim Special Edition, choose that folder." }, expect: r => [
    ['the game never starts', !played(r)],
    ['the launcher never switches folder by itself', askedAt(r, 'set_game_dir') === null],
    ['the player is told where Steam has Skyrim', /choose that folder/.test(r.gameRow), r.gameRow],
  ] },
  { name: 'first start, a folder that is not Skyrim is picked: the first-run sheet says why', s: { ...base, seed: null, clicks: [], pickDir: 'D:\\Games', setDirError: "D:\\Games doesn't have SkyrimSE.exe in it." }, expect: r => [
    ['the reason shows in the first-run sheet', /doesn't have SkyrimSE\.exe/.test(r.gameRow), r.gameRow],
  ] },
  { name: 'first start on this PC: the news box keeps its size', s: { ...base, seed: null, clicks: [] }, expect: r => [
    ['PLAY is not enabled before the checks', firstLabel(r, 'PLAY') !== null && firstLabel(r, 'PLAY') >= answeredAt(r, 'check')],
    // From when the window is shown (fonts settle before that, unseen).
    ['the window is shown by the page', askedAt(r, 'window_ready') !== null],
    ['the news box never changes height once shown', new Set(r.newsHeights.filter(h => h[0] >= askedAt(r, 'window_ready')).map(h => h[1])).size === 1, JSON.stringify(r.newsHeights.slice(0, 8))],
  ] },
  { name: 'a game in progress keeps Play disabled and blocks file checks', s: { ...base, clicks: [1250, 3000], actions: [
    { at: 2600, kind: 'click', id: 'nav-server' },
    { at: 2700, kind: 'click', id: 't-check' },
    { at: 2800, kind: 'click', id: 't-verify' },
  ] }, expect: r => [
    ['Play starts once', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'play').length === 1],
    ['no extra check starts during the game', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'check').length === 2],
    ['the Play button stays disabled in game', r.playDisabled && lastLabel(r) === 'IN GAME [off]', lastLabel(r)],
  ] },
  { name: 'a file check cannot overlap the Play command', s: { ...base, delay: { play: 1500 }, actions: [
    { at: 2400, kind: 'click', id: 't-check' },
  ] }, expect: r => [
    ['Play starts once', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'play').length === 1],
    ['no extra check starts during Play', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'check').length === 2],
  ] },
  { name: 'Play checks a changed manifest before starting', s: { ...base, clicks: [2000], checkSequence: [
    { build: 'B1' }, { build: 'B2', files: 1, bytes: 1000 },
  ] }, expect: r => [
    ['a second manifest check runs', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'check').length === 2],
    ['the old build does not launch', !played(r)],
    ['the button offers the new update', lastLabel(r) === 'UPDATE', lastLabel(r)],
  ] },
  { name: 'game exit checks for a newly published build', s: { ...base, checkSequence: [
    { build: 'B1' }, { build: 'B1' }, { build: 'B2', files: 1, bytes: 1000 },
  ], actions: [{ at: 3000, kind: 'event', name: 'game-ended', payload: { crashed: false, summary: 'Skyrim closed normally.', report: '' } }] }, expect: r => [
    ['Play happened once before the new build', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'play').length === 1],
    ['the game exit triggers a new manifest check', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'check').length === 3],
    ['the button offers the new update', lastLabel(r) === 'UPDATE', lastLabel(r)],
  ] },
  { name: 'settings failure opens a recoverable first-run error', s: { ...base, stateFails: true, clicks: [] }, expect: r => [
    ['the first-run error is visible', /could not load or save its settings/i.test(r.firstError || '')],
    ['Try again is available', r.retryShown],
    ['the error dialog has focus', r.modal === 'first' && r.focus === 'first', `${r.modal}/${r.focus}`],
    ['Play stays disabled', r.playDisabled],
  ] },
  { name: 'Settings traps Tab and disables background navigation', s: { ...base, clicks: [], actions: [
    { at: 1800, kind: 'click', id: 'nav-settings' },
    { at: 1900, kind: 'focus', id: 'set-update' },
    { at: 1950, kind: 'key', key: 'Tab' },
  ] }, expect: r => [
    ['Settings dialog stays open', r.modal === 'settings'],
    ['Tab wraps from the last action to the first', r.actions.some(a => a[1] === 'key' && a[3] === 'acc-signout'), JSON.stringify(r.actions)],
    ['background navigation is inert', r.navInert],
    ['titlebar controls stay live behind the dialog (the window is frameless)', r.titlebarInert === false],
    ['status and update progress have semantic roles', r.statusRole === 'status' && r.progressRole === 'progressbar'],
  ] },
  { name: 'closing Settings restores the previous keyboard focus', s: { ...base, clicks: [], actions: [
    { at: 1700, kind: 'focus', id: 'nav-settings' },
    { at: 1800, kind: 'click', id: 'nav-settings' },
    { at: 1900, kind: 'click', id: 'set-done' },
  ] }, expect: r => [
    ['Settings closes', r.modal === null],
    ['focus returns to Settings button', r.focus === 'nav-settings', r.focus],
    ['background navigation is active again', !r.navInert],
  ] },
];

// The smallest window the launcher allows (core/src/window.rs MIN), which is
// what a 1080p screen at 150% or a 768p laptop at 125% opens at.
// Chrome's window size includes its frame and, on Windows, a scroll bar, so
// the page's size is set in the page itself.
scenarios.push({ name: 'the smallest window: PLAY and the news fit under the title bar', s: { ...base, pageSize: [1024, 560] }, expect: r => [
  ['the page is laid out at the minimum size', r.layout.width === 1024 && r.layout.height === 560, JSON.stringify(r.layout)],
  ['PLAY is fully on screen', r.layout.play[0] >= 0 && r.layout.play[1] <= r.layout.height],
  ['the dock starts below the title bar buttons', r.layout.dock[0] >= r.layout.titlebar[1]],
] });
// Signing in: the launcher can take up to 5 minutes and a few seconds to
// answer, and a sign-in it couldn't save is tried again, then told.
const signedOut = { ...base, auth: { signedIn: false }, seed: null, clicks: [] };
scenarios.push({ name: 'a sign-in the launcher finishes just after 5 minutes is kept', s: { ...signedOut, signIn: { doneAfter: 5 * 60 * 1000 + 4000 }, end: 5 * 60 * 1000 + 12000 }, expect: r => [
  ['the player ends up signed in', r.me === 'Player', JSON.stringify([r.me, r.signInError])],
] });
scenarios.push({ name: 'an answer to a cancelled sign-in is ignored', s: { ...signedOut, signIn: { doneAfter: 6000, refuseFirst: true, restartAt: 4000 }, delay: { auth_poll: 3000 }, end: 16000 }, expect: r => [
  ['the new sign-in finishes', r.me === 'Player', JSON.stringify([r.me, r.signInError])],
  ['the old refusal is never shown', r.signInError === null],
] });
scenarios.push({ name: 'Cancel pressed while a finished sign-in is on its way: the page shows the saved sign-in', s: { ...signedOut, signIn: { doneAfter: 0, cancelAt: 4500 }, delay: { auth_poll: 3000 }, end: 12000 }, expect: r => [
  ['the page asks the launcher who is signed in after that answer', r.invokes.some(i => i[1] === 'ask' && i[2] === 'auth_status' && i[0] >= answeredAt(r, 'auth_poll'))],
  ['the page shows the player signed in, as the launcher saved', r.me === 'Player', JSON.stringify([r.me, r.signInError])],
] });
scenarios.push({ name: 'a sign-in that can never be saved says why', s: { ...signedOut, signIn: { doneAfter: 0, error: "Couldn't save your sign-in: Access is denied" }, end: 6 * 60 * 1000 + 10000 }, expect: r => [
  ['the sign-in window says the save failed', /Couldn't save your sign-in: Access is denied/.test(r.signInError || ''), JSON.stringify(r.signInError)],
  ['the player is not signed in', r.me === null],
  ['the save is tried again until the wait ends', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'auth_poll').length > 100, String(r.invokes.filter(i => i[1] === 'ask' && i[2] === 'auth_poll').length)],
] });
scenarios.push({ name: 'Play states: busy while checking, ready when it can start', s: { ...base, clicks: [], end: 400 }, expect: r => [
  ['PLAY shows the busy state while the checks run', r.playState === 'busy', String(r.playState)],
] });
scenarios.push({ name: 'Play states: ready, and a first-time player is told about F3', s: { ...base, clicks: [] }, expect: r => [
  ['PLAY shows the ready state', r.playState === 'ready', String(r.playState)],
  ['the F3 hint shows', r.hint === 'In game, press F3 for the menu.', String(r.hint)],
] });
scenarios.push({ name: 'Play states: a returning player gets no F3 hint and no gap for it', s: { ...base, clicks: [], hintSeen: true }, expect: r => [
  ['no hint text', r.hint === null, String(r.hint)],
  ['no space is kept for it', r.hintBox === false],
] });
scenarios.push({ name: 'Play states: starting the game counts as having seen the F3 hint', s: base, expect: r => [
  ['the hint is remembered after Play', r.hintStored === '1', String(r.hintStored)],
] });

scenarios.push({ name: 'download progress: friendly file name, speed and time left', s: { ...base, clicks: [], backgroundUpdates: true, check: { build: 'B2', files: 3, bytes: 3000000 }, delay: { update: 20000 }, end: 5200, actions: [
  { at: 2000, kind: 'event', name: 'sync-progress', payload: { bytesDone: 0, bytesTotal: 5000000, filesDone: 1, filesTotal: 3, file: 'Data/Textures/actors/character/face.dds' } },
  { at: 3000, kind: 'event', name: 'sync-progress', payload: { bytesDone: 1000000, bytesTotal: 5000000, filesDone: 1, filesTotal: 3, file: 'Data/Textures/actors/character/face.dds' } },
  { at: 4000, kind: 'event', name: 'sync-progress', payload: { bytesDone: 2000000, bytesTotal: 5000000, filesDone: 1, filesTotal: 3, file: 'Data/Textures/actors/character/face.dds' } },
  { at: 5000, kind: 'event', name: 'sync-progress', payload: { bytesDone: 3000000, bytesTotal: 5000000, filesDone: 1, filesTotal: 3, file: 'Data/Textures/actors/character/face.dds' } },
] }, expect: r => [
  ['the file shows by its own name, not the long path', r.progress.file === 'face.dds', JSON.stringify(r.progress.file)],
  ['speed and time left show together once 3 s of samples exist', /^\d+(\.\d)? MB\/s · about \d+ (sec|min) left$/.test(r.progress.speed), JSON.stringify(r.progress.speed)],
] });
scenarios.push({ name: 'download progress: no time-left guess in the first seconds', s: { ...base, clicks: [], backgroundUpdates: true, check: { build: 'B2', files: 3, bytes: 3000000 }, delay: { update: 20000 }, end: 3300, actions: [
  { at: 2000, kind: 'event', name: 'sync-progress', payload: { bytesDone: 0, bytesTotal: 5000000, filesDone: 1, filesTotal: 3, file: 'Data/Textures/actors/character/face.dds' } },
  { at: 2700, kind: 'event', name: 'sync-progress', payload: { bytesDone: 1000000, bytesTotal: 5000000, filesDone: 1, filesTotal: 3, file: 'Data/Textures/actors/character/face.dds' } },
] }, expect: r => [
  ['the speed shows without a time left', /^\d+(\.\d)? MB\/s$/.test(r.progress.speed), JSON.stringify(r.progress.speed)],
] });
scenarios.push({ name: 'download progress: a huge time left is capped', s: { ...base, clicks: [], backgroundUpdates: true, check: { build: 'B2', files: 3, bytes: 3000000 }, delay: { update: 20000 }, end: 6300, actions: [
  { at: 2000, kind: 'event', name: 'sync-progress', payload: { bytesDone: 0, bytesTotal: 10000000000, filesDone: 1, filesTotal: 3, file: 'Data/Textures/actors/character/face.dds' } },
  { at: 3000, kind: 'event', name: 'sync-progress', payload: { bytesDone: 1000000, bytesTotal: 10000000000, filesDone: 1, filesTotal: 3, file: 'Data/Textures/actors/character/face.dds' } },
  { at: 4000, kind: 'event', name: 'sync-progress', payload: { bytesDone: 2000000, bytesTotal: 10000000000, filesDone: 1, filesTotal: 3, file: 'Data/Textures/actors/character/face.dds' } },
  { at: 5000, kind: 'event', name: 'sync-progress', payload: { bytesDone: 3000000, bytesTotal: 10000000000, filesDone: 1, filesTotal: 3, file: 'Data/Textures/actors/character/face.dds' } },
  { at: 6000, kind: 'event', name: 'sync-progress', payload: { bytesDone: 4000000, bytesTotal: 10000000000, filesDone: 1, filesTotal: 3, file: 'Data/Textures/actors/character/face.dds' } },
] }, expect: r => [
  ['the time left reads "over 99 min", not thousands of minutes', / · over 99 min left$/.test(r.progress.speed), JSON.stringify(r.progress.speed)],
] });
scenarios.push({ name: 'download progress: no news for 3 s says it is waiting for the server', s: { ...base, clicks: [], backgroundUpdates: true, check: { build: 'B2', files: 3, bytes: 3000000 }, delay: { update: 20000 }, end: 8000, actions: [
  { at: 2000, kind: 'event', name: 'sync-progress', payload: { bytesDone: 0, bytesTotal: 5000000, filesDone: 1, filesTotal: 3, file: 'Data/Textures/actors/character/face.dds' } },
  { at: 3000, kind: 'event', name: 'sync-progress', payload: { bytesDone: 1000000, bytesTotal: 5000000, filesDone: 1, filesTotal: 3, file: 'Data/Textures/actors/character/face.dds' } },
  { at: 4000, kind: 'event', name: 'sync-progress', payload: { bytesDone: 2000000, bytesTotal: 5000000, filesDone: 1, filesTotal: 3, file: 'Data/Textures/actors/character/face.dds' } },
] }, expect: r => [
  ['a stall says it is waiting, not a stale time left', r.progress.speed === 'Waiting for the server…', JSON.stringify(r.progress.speed)],
] });
scenarios.push({ name: 'Play ready glow animates opacity or transform only, so hover still brightens it', s: { ...base, clicks: [] }, expect: r => [
  ['the ready state does not animate a filter on the wrapper', r.readyAnim === 'none' || r.readyAnim === null, String(r.readyAnim)],
] });

scenarios.push({ name: 'errors: a failed update says what happened and offers Copy details, not a paragraph of steps', s: { ...base, clicks: [], backgroundUpdates: true, check: { build: 'B2', files: 3, bytes: 3000000 }, updateError: 'disk is full', end: 5000 }, expect: r => [
  ['the message says what stopped and what to click', /The game file update stopped: disk is full\. Click Retry\./.test(r.statusUi.text), JSON.stringify(r.statusUi.text)],
  ['the long "open Settings" instructions are gone', !/open Settings/.test(r.statusUi.text), JSON.stringify(r.statusUi.text)],
  ['a Copy details button is there', r.statusUi.copy],
] });
scenarios.push({ name: 'errors: Copy details copies the diagnostics and says so', s: { ...base, clicks: [], backgroundUpdates: true, check: { build: 'B2', files: 3, bytes: 3000000 }, updateError: 'disk is full', end: 7500, actions: [{ at: 3000, kind: 'click', id: 'status-copy' }], clipboard: 'ok' }, expect: r => [
  ['the diagnostics text reached the clipboard', r.copied === 'diagnostics text', String(r.copied)],
  ['the Copy button is still there to press again', r.statusUi.copy],
  ['the status area keeps one height from the error to the result', new Set(r.statusH.filter(h => h[2]).map(h => h[1])).size === 1, JSON.stringify(r.statusH.filter(h => h[2]).map(h => h[1]))],
  ['the line confirms the copy', /Copied\. Paste it in Discord/.test(r.statusUi.text) && !/Couldn't copy/.test(r.statusUi.text), JSON.stringify(r.statusUi.text)],
  ['no divider bar is left before the version', r.statusUi.dividerAfterError === null || r.statusUi.dividerAfterError === 'none' || r.statusUi.dividerAfterError === 'normal', String(r.statusUi.dividerAfterError)],
] });
scenarios.push({ name: 'errors: Copy details falls back to the older copy command when the clipboard API refuses', s: { ...base, clicks: [], backgroundUpdates: true, check: { build: 'B2', files: 3, bytes: 3000000 }, updateError: 'disk is full', end: 7500, actions: [{ at: 3000, kind: 'click', id: 'status-copy' }], clipboard: 'fallback' }, expect: r => [
  ['the diagnostics text was copied the other way', r.copied === 'diagnostics text', String(r.copied)],
  ['the line confirms the copy', /Copied\. Paste it in Discord/.test(r.statusUi.text), JSON.stringify(r.statusUi.text)],
] });
scenarios.push({ name: 'errors: when nothing can copy, the line says where to find the log instead', s: { ...base, clicks: [], backgroundUpdates: true, check: { build: 'B2', files: 3, bytes: 3000000 }, updateError: 'disk is full', end: 7500, actions: [{ at: 3000, kind: 'click', id: 'status-copy' }], clipboard: 'fail' }, expect: r => [
  ['the Copy button is still there to press again', r.statusUi.copy],
  ['the status area keeps one height from the error to the result', new Set(r.statusH.filter(h => h[2]).map(h => h[1])).size === 1, JSON.stringify(r.statusH.filter(h => h[2]).map(h => h[1]))],
  ['it says in one short line where to find the log', /Couldn't copy\. Use Settings > Open log folder\.$/.test(r.statusUi.text.replace(/v\d[\d.]*$/, '')) && !/Copied\./.test(r.statusUi.text), JSON.stringify(r.statusUi.text)],
] });
scenarios.push({ name: 'errors: a normal status line has no Copy details button', s: { ...base, clicks: [] }, expect: r => [
  ['no button on a healthy status', !r.statusUi.copy],
] });
scenarios.push({ name: 'download progress: after the last byte, hashing says Checking file', s: { ...base, clicks: [], backgroundUpdates: true, check: { build: 'B2', files: 3, bytes: 3000000 }, delay: { update: 20000 }, end: 8000, actions: [
  { at: 2000, kind: 'event', name: 'sync-progress', payload: { bytesDone: 0, bytesTotal: 3000000, filesDone: 0, filesTotal: 3, file: 'Data/big.bsa' } },
  { at: 3000, kind: 'event', name: 'sync-progress', payload: { bytesDone: 3000000, bytesTotal: 3000000, filesDone: 2, filesTotal: 3, file: 'Data/big.bsa' } },
] }, expect: r => [
  ['it says the file is being checked, not that the server is silent', r.progress.speed === 'Checking file…', JSON.stringify(r.progress.speed)],
] });

scenarios.push({ name: 'Settings: grouped under headings', s: { ...base, clicks: [], end: 3000, actions: [{ at: 1500, kind: 'click', id: 'w-settings' }] }, expect: r => [
  ['three headings in order', JSON.stringify(r.settingsGroups.heads) === JSON.stringify(['Game', 'Launcher', 'Privacy']), JSON.stringify(r.settingsGroups.heads)],
  ['the Skyrim folder is under Game', r.settingsGroups.folder === 'Game', String(r.settingsGroups.folder)],
  ['close and automatic updates are under Launcher', r.settingsGroups.close === 'Launcher' && r.settingsGroups.bg === 'Launcher', JSON.stringify([r.settingsGroups.close, r.settingsGroups.bg])],
  ['health reports are under Privacy', r.settingsGroups.share === 'Privacy', String(r.settingsGroups.share)],
] });
scenarios.push({ name: 'Settings: a changed setting says Saved', s: { ...base, clicks: [], end: 3000, actions: [{ at: 1500, kind: 'click', id: 'w-settings' }, { at: 2000, kind: 'click', id: 'set-bg' }] }, expect: r => [
  ['it says Saved', r.saved === 'Saved', String(r.saved)],
  ['the switch changed to on', r.bgChecked === 'true', String(r.bgChecked)],
] });
scenarios.push({ name: 'Settings: a setting that cannot be saved flips back and says so', s: { ...base, clicks: [], setPrefsError: 'settings are read-only', end: 3000, actions: [{ at: 1500, kind: 'click', id: 'w-settings' }, { at: 2000, kind: 'click', id: 'set-bg' }] }, expect: r => [
  ['it says it could not save', /Couldn't save/.test(r.saved || ''), String(r.saved)],
  ['the switch is back where it was (off)', r.bgChecked === 'false', String(r.bgChecked)],
] });

scenarios.push({ name: 'Settings: the save message stays in view at the top, not below the fold', s: { ...base, scrollSettings: true, clicks: [], end: 3000, actions: [{ at: 1500, kind: 'click', id: 'w-settings' }, { at: 2000, kind: 'click', id: 'set-bg' }] }, expect: r => [
  ['the message is on screen near the top', r.toast.shown && r.toast.top >= 0 && r.toast.bottom <= 120, JSON.stringify(r.toast)],
  ['the settings really scrolled (scrollTop moved)', r.settingsScrolled > 0, String(r.settingsScrolled)],
  ['the live regions are inside the aria-modal dialog', r.toastInModal === true, String(r.toastInModal)],
  ['and outside the blurred scroller', r.toastOutsideBlur === true, String(r.toastOutsideBlur)],
  ['it stays put when the settings scroll', r.toast.fixed === 'fixed', String(r.toast.fixed)],
] });
scenarios.push({ name: 'Settings: a failed save is shown as a warning, not in the success colour', s: { ...base, clicks: [], setPrefsError: 'read-only', end: 3000, actions: [{ at: 1500, kind: 'click', id: 'w-settings' }, { at: 2000, kind: 'click', id: 'set-bg' }] }, expect: r => [
  ['it carries the warning class', /\bbad\b/.test(r.toast.cls), r.toast.cls],
  ['it is not green', r.toast.color !== r.okColor, r.toast.color],
] });
scenarios.push({ name: 'Settings: a saved change uses the success style', s: { ...base, clicks: [], end: 3000, actions: [{ at: 1500, kind: 'click', id: 'w-settings' }, { at: 2000, kind: 'click', id: 'set-bg' }] }, expect: r => [
  ['it carries the ok class', /\bok\b/.test(r.toast.cls), r.toast.cls],
] });
scenarios.push({ name: 'Settings: menu music that cannot be saved flips back and says so', s: { ...base, clicks: [], setMusicError: 'no audio device', end: 3000, actions: [{ at: 1500, kind: 'click', id: 'w-settings' }, { at: 2000, kind: 'click', id: 'set-music' }] }, expect: r => [
  ['it says it could not save', /Couldn't save/.test(r.saved || ''), String(r.saved)],
  ['the switch is back where it was', r.musicChecked === 'false', String(r.musicChecked)],
] });
scenarios.push({ name: 'Settings: menu music that saves says Saved and stays on', s: { ...base, clicks: [], end: 3000, actions: [{ at: 1500, kind: 'click', id: 'w-settings' }, { at: 2000, kind: 'click', id: 'set-music' }] }, expect: r => [
  ['it says Saved', r.saved === 'Saved', String(r.saved)],
  ['the switch is on', r.musicChecked === 'true', String(r.musicChecked)],
] });

scenarios.push({ name: 'First run and sign-in: a step indicator shows where the player is', s: { ...signedOut, end: 3000 }, expect: r => [
  ['first run lists three steps with the first current', JSON.stringify(r.steps.first) === JSON.stringify(['Your game*', 'Sign in', 'Play']), JSON.stringify(r.steps.first)],
  ['the finished step is marked done for screen readers', /done/.test((r.steps.signin || [])[0] || ''), JSON.stringify(r.steps.signin)],
  ['steps still to do are readable (the muted colour, not the faint one)', !!r.stepColors && r.stepColors.other !== r.faintColor, JSON.stringify(r.stepColors)],
  ['the waiting line is a status message', r.slotRole === 'status', String(r.slotRole)],
  ['sign-in lists the same steps with the second current', JSON.stringify(r.steps.signin) === JSON.stringify(['Your game done', 'Sign in*', 'Play']), JSON.stringify(r.steps.signin)],
] });
scenarios.push({ name: 'Sign-in: waiting for the browser keeps the dialog the same size and says so on the button', s: { ...signedOut, signIn: { doneAfter: 60 * 1000 }, end: 5000 }, expect: r => [
  ['the dialog does not change height while waiting', Math.abs(r.siAfter - r.siBefore) < 1, JSON.stringify([r.siBefore, r.siAfter])],
  ['the button says what it is waiting for', /WAITING FOR DISCORD/.test(r.siGoLabel), r.siGoLabel],
  ['the button is disabled while waiting', r.siGoDisabled === true, String(r.siGoDisabled)],
] });
scenarios.push({ name: 'Settings: screen readers hear Saved politely and a failure as an alert', s: { ...base, clicks: [], end: 3000, actions: [{ at: 1500, kind: 'click', id: 'w-settings' }] }, expect: r => [
  ['Saved is a status message', r.toastRoles.ok === 'status', String(r.toastRoles.ok)],
  ['a failure is an alert', r.toastRoles.bad === 'alert', String(r.toastRoles.bad)],
  ['an empty message stays in the page, hidden by opacity, so it can be announced', r.toastRoles.emptyDisplay !== 'none' && r.toastRoles.emptyOpacity === '0', JSON.stringify(r.toastRoles)],
] });

const chrome = findChrome();
let failed = 0;
scenarios.push({ name: 'Home: optional rows do not move the Play button', s: { ...base, clicks: [], jank: true, end: 3000 }, expect: r => [...Object.entries(r.jank).filter(([k]) => k !== 'base').map(([k, d]) => [`showing ${k} moves Play by ${d}px`, Math.abs(d) < 1, String(d)]),
  ...Object.entries(r.jankStatus).map(([k, d]) => [`${k} moves Play by ${d}px`, Math.abs(d) < 1, String(d)])] });
scenarios.push({ name: 'Status line: no separator at the start of a line', s: { ...base, clicks: [], seps: true, end: 4600 }, expect: r => r.seps.flatMap((row, n) => [
  [`state ${n + 1}: the line wraps (so this checks something)`, n === 0 || row.some(k => k.rowStart && k !== row[0]), JSON.stringify(row)],
  [`state ${n + 1}: no separator on the first item of a line`, row.every(k => !(k.rowStart && k.sep)), JSON.stringify(row)],
  [`state ${n + 1}: the other items keep theirs`, row.every(k => k.rowStart || k.error || k.sep), JSON.stringify(row)]
]) });
scenarios.push({ name: 'Feed refused (unsigned or tampered): the launcher update still installs and the words are plain', s: { ...base, update: '9.9.10', clicks: [], checkError: "The launcher couldn't confirm the server's files are genuine, so it did not use them. It tries again the next time you press Play.", end: 5000 }, expect: r => [
  ['the launcher update installs', askedAt(r, 'updater_install') !== null],
  ['the sentence gives no second set of advice; the Copy details button is the way to staff', !/keeps happening|diagnostics/.test(r.statusUi.text) && r.statusUi.copy, r.statusUi.text],
  ['the player reads the plain words, no file names', /couldn't confirm the server's files/.test(r.statusUi.text) && !/signature|mods\.json|manifest/i.test(r.statusUi.text), r.statusUi.text],
] });
const holdOf = (versions, until) => { const l = { hold: versions }; if (until) l.holdUntil = until; return { launcher: l }; };
scenarios.push({ name: 'Release hold: a held launcher version is not installed while the hold lasts', s: { ...base, update: '9.9.10', clicks: [], statusNow: holdOf(['9.9.10'], new Date(Date.now() + 36e5).toISOString()), end: 4000 }, expect: r => [
  ['nothing is downloaded or installed', askedAt(r, 'updater_download') === null && askedAt(r, 'updater_install') === null],
] });
scenarios.push({ name: 'Release hold: a hold that has run out does not stop the update', s: { ...base, update: '9.9.10', clicks: [], statusNow: holdOf(['9.9.10'], new Date(Date.now() - 36e5).toISOString()), end: 4000 }, expect: r => [
  ['the update installs', askedAt(r, 'updater_install') !== null],
] });
scenarios.push({ name: 'Release hold: without an end time it is ignored, so it can never hold updates for good', s: { ...base, update: '9.9.10', clicks: [], statusNow: holdOf(['9.9.10']), end: 4000 }, expect: r => [
  ['the update installs', askedAt(r, 'updater_install') !== null],
] });
scenarios.push({ name: 'Release hold: another version is not held', s: { ...base, update: '9.9.10', clicks: [], statusNow: holdOf(['9.9.11'], new Date(Date.now() + 36e5).toISOString()), end: 4000 }, expect: r => [
  ['the update installs', askedAt(r, 'updater_install') !== null],
] });
scenarios.push({ name: 'Release hold: if the status file cannot be read the last hold stays', s: { ...base, update: '9.9.10', clicks: [], statusNow: holdOf(['9.9.10'], new Date(Date.now() + 36e5).toISOString()), updaterDelay: 33000, actions: [{ at: 3000, kind: 'statusFile', value: true }], end: 37000 }, expect: r => [
  ['still nothing is installed', askedAt(r, 'updater_install') === null && askedAt(r, 'updater_download') === null],
] });
scenarios.push({ name: 'Release hold: staff removing the hold lets the update through', s: { ...base, update: '9.9.10', clicks: [], statusNow: holdOf(['9.9.10'], new Date(Date.now() + 36e5).toISOString()), updaterDelay: 33000, actions: [{ at: 3000, kind: 'status', value: {} }], end: 37000 }, expect: r => [
  ['the update installs', askedAt(r, 'updater_install') !== null],
] });
scenarios.push({ name: 'Minimum launcher version: a long message is cut at a word with an ellipsis', s: { ...base, clicks: [], statusNow: { launcher: { minVersion: '99.0.0', message: 'Server changes need a newer launcher. '.repeat(10).trim() } }, end: 4000 }, expect: r => [
  ['it ends with an ellipsis', /…$/.test(r.minNotice || ''), r.minNotice],
  ['it is no longer than 201 characters', (r.minNotice || '').length <= 201, String((r.minNotice || '').length)],
  ['it does not stop in the middle of a word', /(launcher|Server|changes|need|a|newer)…$/.test(r.minNotice || ''), (r.minNotice || '').slice(-20)],
] });
scenarios.push({ name: 'Minimum launcher version: an older launcher shows staff\'s message and Play is not blocked', s: { ...base, clicks: [], statusNow: { launcher: { minVersion: '99.0.0', message: 'Server changes need a newer launcher.' } }, end: 4000 }, expect: r => [
  ['the notice shows with the message', /Server changes need a newer launcher/.test(r.minNotice || ''), r.minNotice],
  ['Play is still available', r.playState === 'ready', r.playState],
] });
scenarios.push({ name: 'Minimum launcher version: a current launcher shows no notice', s: { ...base, clicks: [], statusNow: { launcher: { minVersion: '0.0.1', message: 'Old.' } }, end: 4000 }, expect: r => [
  ['no notice', !r.minNotice, r.minNotice],
] });
scenarios.push({ name: 'Tokens: the launcher colours are the design system values (Dawnglass)', s: { ...base, clicks: [], end: 2000 }, expect: r => Object.entries({ text: '#ece6d6', muted: '#b3b8c0', faint: '#8a93a0', ok: '#93cf9e', warn: '#e6b866', gold: '#d8b56a', frost: '#b6dcf7', glow: '#5ea4d8', danger: '#f0998a' }).map(([k, v]) => [`--${k} is ${v}`, r.tokens[k] === v, String(r.tokens[k])]) });
const hueOf = c => { const [r, g, b] = (c.match(/[\d.]+/g) || []).slice(0, 3).map(Number).map(v => v / 255), mx = Math.max(r, g, b), mn = Math.min(r, g, b), d = mx - mn; if (!d) return 0; const h = mx === r ? ((g - b) / d) % 6 : mx === g ? (b - r) / d + 2 : (r - g) / d + 4; return (h * 60 + 360) % 360; };
scenarios.push({ name: 'Tokens: errors and failures are clearly not gold', s: { ...base, clicks: [], end: 2000 }, expect: r => [
  ['an error line is far from gold in hue (25 degrees or more)', Math.abs(hueOf(r.errorColor) - hueOf(r.goldColor)) >= 25, `${r.errorColor} vs ${r.goldColor}`],
  ['a failed-save message is far from gold too', Math.abs(hueOf(r.badToastColor) - hueOf(r.goldColor)) >= 25, `${r.badToastColor} vs ${r.goldColor}`],
] });
for (const sc of scenarios) {
  if (process.env.AD_UI_SCENARIO && !sc.name.includes(process.env.AD_UI_SCENARIO)) continue;
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ad-ui-'));
  fs.cpSync(UI, dir, { recursive: true });
  const html = fs.readFileSync(path.join(dir, 'index.html'), 'utf8')
    .replace('<head>', `<head><script>window.__S=${JSON.stringify(sc.s)};(${fakeBackEnd})();</script>`);
  fs.writeFileSync(path.join(dir, 'index.html'), html);
  const url = 'file:///' + path.join(dir, 'index.html').replace(/\\/g, '/').replace(/^\//, '');
  const run = () => {
    try {
      // Chrome refuses to run as root on Linux without this.
      const root = process.getuid && process.getuid() === 0 ? ['--no-sandbox'] : [];
      return execFileSync(chrome, [...root, '--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check',
        `--user-data-dir=${path.join(dir, 'profile')}`, '--allow-file-access-from-files', '--window-size=1360,880',
        `--virtual-time-budget=${Math.max(8000, (sc.s.end || 6000) + 2000)}`, '--dump-dom', url], { encoding: 'utf8', timeout: 60000, stdio: ['ignore', 'pipe', 'ignore'] });
    } catch (e) { return String(e.stdout || ''); }
  };
  const result = /<pre id="ui-test-result">([\s\S]*?)<\/pre>/;
  let out = run();
  // A cold Chrome on a fresh CI machine can take longer than the 60 s limit
  // to open its first page, before the scenario runs at all: that one
  // scenario gets a second, fresh start. A page that runs and fails a check
  // is never retried.
  let retried = false;
  if (!result.test(out)) {
    retried = true;
    fs.rmSync(path.join(dir, 'profile'), { recursive: true, force: true });
    out = run();
  }
  const m = out.match(result);
  console.log(`== ${sc.name}${retried ? ' (Chrome gave no page on the first start; started again)' : ''}`);
  if (!m) { console.log('FAIL  the page gave no result'); failed++; continue; }
  const r = JSON.parse(m[1].replace(/&quot;/g, '"').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&amp;/g, '&'));
  for (const [what, pass, shown] of sc.expect(r)) {
    console.log(`${pass ? 'PASS' : 'FAIL'}  ${what}${shown ? `  (${shown})` : ''}`);
    if (!pass) failed++;
  }
  if (sc.expect(r).some(x => !x[1])) console.log('      labels: ' + JSON.stringify(r.labels) + '\n      invokes: ' + JSON.stringify(r.invokes.filter(i => i[1] !== 'answer' || ['auth_status', 'check', 'update'].includes(i[2]))));
  fs.rmSync(dir, { recursive: true, force: true });
}
if (failed) { console.log(`${failed} check(s) failed`); process.exit(1); }
console.log('fast-Play checks passed: every check');
