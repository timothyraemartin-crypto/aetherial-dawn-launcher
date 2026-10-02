// Proves the launcher Play path: a returning player waits for fresh game,
// Discord, and Vortex checks before PLAY is enabled. The launcher's page runs
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
  if (S.authIntervalMs) {
    const every = window.setInterval.bind(window);
    window.setInterval = (fn, ms, ...args) => every(fn, ms === 10 * 60 * 1000 ? S.authIntervalMs : ms, ...args);
  }
  try { localStorage.clear(); if (S.seed) localStorage.setItem('ad.lastReady', JSON.stringify(S.seed)); } catch (_) {}
  const game = Object.assign({ needed: false, installed: '1.6.1170.0', target: '1.6.1170.0', skseOk: true, canDowngrade: true }, S.game || {});
  const answers = {
    get_state: () => ({ launcherVersion: S.version, config: { gameDir: S.dir, closeOnLaunch: false, backgroundUpdates: !!S.backgroundUpdates, shareHealth: true, music: false, onlyServerMods: true }, game: S.gameGone ? null : { dir: S.dir, hasSkse: S.hasSkse !== false }, gameError: S.gameGone || null }),
    auth_status: () => (S.authSequence && S.authSequence.shift()) || ((S.authAfter && (S.authCalls = (S.authCalls || 0) + 1) > 1) ? S.authAfter : S.auth),
    check: () => Object.assign({ build: 'B2', server: { name: 'Aetherial Dawn', ip: '127.0.0.1', port: 7777 }, files: 0, remove: 0, bytes: 0, strays: [], game }, (S.checkSequence && S.checkSequence.shift()) || S.check || {}),
    update: () => null,
    server_status: () => ({ online: true, players: 1, maxPlayers: 50, discordInvite: S.invite, news: [{ title: 'News', date: '28 Sep', body: 'Body.' }, { title: 'More', date: '27 Sep', body: 'Body.' }] }),
    files: () => [], game_check: () => game, play: () => S.playWarnings || null,
    patch_game: () => S.patchResult || { ...game, needed: false },
    game_running: () => !!S.outsideGame,
    self_update_begin: () => !S.updateReservationFails,
    self_update_end: () => null,
    mods_state: () => Object.assign({ mods: [], nexus: null, vortex: true, vortex_ready: true, vortex_paired: true, running: false, sso: false }, S.modsState || {}),
    download_all_mods: () => ({ installed: [], failed: [], cancelled: false }),
    plain_error: a => a.text,
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
        if (cmd === 'auth_status' && S.authFails) return rej('network down');
        if (cmd === 'game_running' && (S.gameCheckFails || (S.gameCheckFailsFrom && (S.gameChecks = (S.gameChecks || 0) + 1) >= S.gameCheckFailsFrom))) return rej('process list unavailable');
        if (cmd === 'set_game_dir' && S.setDirError) return rej(S.setDirError);
        if (cmd === 'set_game_dir' && S.picks && S.picks.find(p => p.dir === args.dir && p.error)) return rej(S.picks.find(p => p.dir === args.dir).error);
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
    updater: { check: () => new Promise(r => setTimeout(() => r(S.update ? {
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
    const noteStatus = () => log.statuses.push([at(), status.textContent]);
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
    // Keyboard use of a sheet: open Settings, check focus, press Escape.
    if (S.dialogTest) {
      const id = () => document.activeElement && document.activeElement.id;
      const dock = () => document.querySelector('.dock').inert;
      const press = b => { const el = document.getElementById(b); el.focus(); el.click(); };
      setTimeout(() => press('nav-settings'), 3000);
      setTimeout(() => {
        const sheet = document.getElementById('settings');
        log.dialog = { role: sheet.getAttribute('role'), modal: sheet.getAttribute('aria-modal'), label: sheet.getAttribute('aria-labelledby'), focusInside: sheet.contains(document.activeElement), dockInert: dock(), navLive: !document.querySelector('nav').inert };
        press('set-health');
      }, 3300);
      setTimeout(() => { log.dialog.healthFocus = document.getElementById('health').contains(document.activeElement); press('hl-close'); }, 3600);
      setTimeout(() => { log.dialog.backTo = id(); document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); }, 3900);
      setTimeout(() => { log.dialog.closedByEscape = document.getElementById('settings').hidden; log.dialog.focusAfter = id(); log.dialog.dockAfter = dock(); }, 4200);
    }
    if (S.signIn) setTimeout(() => { S.signInAt = at(); document.getElementById('si-go').click(); }, 1500);
    // Cancel while the first answer is on its way, then sign in again.
    if (S.signIn && S.signIn.cancelAt) setTimeout(() => document.getElementById('si-cancel').click(), S.signIn.cancelAt);
    if (S.signIn && S.signIn.restartAt) setTimeout(() => { document.getElementById('si-cancel').click(); document.getElementById('si-go').click(); }, S.signIn.restartAt);
    for (const t of S.clicks || []) setTimeout(() => { log.invokes.push([at(), 'click', label.textContent]); btn.click(); }, t);
    for (const action of S.actions || []) setTimeout(() => {
      if (action.kind === 'click') document.getElementById(action.id).click();
      if (action.kind === 'event') for (const callback of listeners[action.name] || []) callback({ payload: action.payload });
      if (action.kind === 'focus') document.getElementById(action.id).focus();
      if (action.kind === 'key') document.dispatchEvent(new KeyboardEvent('keydown', { key: action.key, shiftKey: !!action.shiftKey, bubbles: true, cancelable: true }));
      log.actions.push([at(), action.kind, action.id || action.name || action.key, document.activeElement && document.activeElement.id]);
    }, action.at);
    setTimeout(() => {
      try { log.lastReady = JSON.parse(localStorage.getItem('ad.lastReady')); } catch (_) {}
      log.signinShown = !document.getElementById('signin').hidden;
      log.status = document.getElementById('status').textContent;
      const live = document.getElementById('status-live');
      log.announced = live ? live.textContent : null;
      const playBtn = document.getElementById('play');
      playBtn.focus({ focusVisible: true });
      log.playRing = playBtn.matches(':focus-visible') ? getComputedStyle(document.getElementById('play-wrap')).outlineStyle : 'not focus-visible';
      log.gameRow = document.querySelector('#c-game small').textContent;
      const join = document.getElementById('si-join');
      log.join = join.hidden ? null : join.textContent;
      const errLink = document.querySelector('#si-error button');
      log.errorLink = errLink ? errLink.textContent : null;
      log.skse = ['g-skse', 'c-skse'].map(id => { const el = document.getElementById(id); return el.querySelector('b').textContent + el.querySelector('small').textContent; });
      log.skseRecheck = !document.getElementById('c-skse-recheck').hidden;
      log.modsShown = !document.getElementById('reqs').hidden;
      log.modNames = [...document.querySelectorAll('#rq-list b')].map(el => el.textContent);
      log.modSummary = document.getElementById('rq-summary').textContent;
      const vx = document.getElementById('rq-vortex-step');
      log.vortexLine = vx.hidden ? null : vx.textContent;
      const own = document.getElementById('rq-ownership');
      log.ownership = own.hidden ? null : own.textContent;
      log.vortexConnect = !document.getElementById('rq-vortex-connect').hidden;
      log.installerControl = !!document.querySelector('#rq-all, #rq-stop, #rq-sso-go, #rq-signin');
      log.playDisabled = btn.disabled;
      log.focus = document.activeElement && document.activeElement.id;
      log.modal = [...document.querySelectorAll('.sheet')].find(el => !el.hidden)?.id || null;
      log.navInert = document.getElementById('nav-home').closest('.nav').inert;
      log.titlebarInert = document.querySelector('.titlebar').inert;
      log.modRow = document.getElementById('rq-list').textContent;
      log.modLead = document.querySelector('#reqs .lead').textContent;
      log.firstError = document.getElementById('first-error').hidden ? null : document.getElementById('first-error').textContent;
      log.retryShown = !document.getElementById('f-retry').hidden;
      log.statusRole = document.getElementById('status').getAttribute('role');
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
  { name: 'returning player: Play waits for fresh game, Discord and Vortex checks', s: base, expect: r => [
    ['PLAY is not enabled before fresh checks answer', firstLabel(r, 'PLAY') !== null && firstLabel(r, 'PLAY') >= Math.max(answeredAt(r, 'auth_status'), answeredAt(r, 'check'), answeredAt(r, 'mods_state'))],
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
  { name: 'missing mods show the exact backend list for manual Vortex setup', s: { ...base,
    playError: 'NEEDS_NEXUS_MODS:[{"id":"racemenu","name":"RaceMenu","in_vortex":true,"looks_for":"Data/RaceMenu.esp, Data/SKSE/Plugins/skee64.dll","page":"https://www.nexusmods.com/skyrimspecialedition/mods/19080"},{"id":"ussep","name":"USSEP","looks_for":"Data/Unofficial Skyrim Special Edition Patch.esp"}]',
    modsState: { mods: [{ id: 'other', name: 'Other mod', installed: true }], nexus: { name: 'Player', is_premium: false }, vortex: false, running: false, sso: true },
  }, expect: r => [
    ['the required mods dialog opens', r.modsShown],
    ['only the two missing mods appear in backend order', JSON.stringify(r.modNames) === JSON.stringify(['RaceMenu', 'USSEP']), JSON.stringify(r.modNames)],
    ['required file paths appear', /Data\/RaceMenu\.esp/.test(r.modRow) && /Data\/Unofficial Skyrim Special Edition Patch\.esp/.test(r.modRow), r.modRow],
    ['Play missing rows never claim exact Vortex confirmation from the legacy inventory field', !/profile, deployment, and game files confirmed/i.test(r.modRow)],
    ['the dialog explains Vortex setup', /manual|Vortex/i.test(r.modLead) && /Vortex/.test(r.status), r.modLead],
    ['the launcher keeps the exact missing list when Play stops', JSON.stringify(r.modNames) === JSON.stringify(['RaceMenu', 'USSEP'])],
    ['the launcher does not start an installer or Nexus sign-in', askedAt(r, 'download_all_mods') === null && askedAt(r, 'nexus_sso') === null && !r.installerControl],
  ] },
  { name: 'Premium Nexus account does not auto-install missing mods', s: { ...base, playError: 'NEEDS_NEXUS_MODS:[{"id":"a","name":"A"}]', modsState: { mods: [{ id: 'a', name: 'A', installed: false }], nexus: { name: 'Player', is_premium: true }, vortex: false, running: false, sso: true } }, expect: r => [
    ['the missing mod is listed', r.modsShown && JSON.stringify(r.modNames) === JSON.stringify(['A']), JSON.stringify(r.modNames)],
    ['there is no automatic mod install', askedAt(r, 'download_all_mods') === null],
    ['there are no installer controls', !r.installerControl],
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
  { name: 'Play resumes after version repair and Vortex verification', s: { ...base,
    game: { needed: true, canDowngrade: true, reason: 'Skyrim needs changing.' },
    checkSequence: [
      { game: { needed: true, canDowngrade: true, reason: 'Skyrim needs changing.' } },
      { game: { needed: false, installed: '1.6.1170.0', target: '1.6.1170.0', skseOk: true } },
    ],
    patchResult: { needed: false, installed: '1.6.1170.0', target: '1.6.1170.0', skseOk: true },
    delay: { mods_state: 1000 },
  }, expect: r => [
    ['the game version patch ran once', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'patch_game').length === 1],
    ['Play resumed after Vortex answered', askedAt(r, 'play') !== null && askedAt(r, 'play') >= answeredAt(r, 'mods_state')],
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
  { name: 'launcher update found while Play is starting the game: it waits', s: { ...base, update: '9.9.10', updaterDelay: 900, delay: { play: 3000 } }, expect: r => [
    ['the game starts', played(r)],
    ['the launcher never installs its update while Play runs or the game is up', askedAt(r, 'updater_install') === null, JSON.stringify(r.invokes.filter(i => /updater|play/.test(i[2])))],
  ] },
  { name: 'launcher update found while Skyrim runs from Steam or Vortex: it waits', s: { ...base, update: '9.9.10', outsideGame: true, clicks: [] }, expect: r => [
    ['the launcher asks Windows whether Skyrim runs', askedAt(r, 'game_running') !== null],
    ['the update never installs while that Skyrim runs', askedAt(r, 'updater_install') === null, JSON.stringify(r.invokes.filter(i => /updater|game_running/.test(i[2])))],
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
  { name: 'Skyrim running: PLAY does not come back, and a second press starts nothing', s: { ...base, clicks: [60, 9500], end: 10500 }, expect: r => [
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
  { name: 'Settings works as a dialog from the keyboard', s: { ...base, clicks: [], dialogTest: true }, expect: r => [
    ['it is a labelled modal dialog', !!r.dialog && r.dialog.role === 'dialog' && r.dialog.modal === 'true' && !!r.dialog.label, JSON.stringify(r.dialog)],
    ['focus moves into it', !!r.dialog && r.dialog.focusInside],
    ['Escape closes it', !!r.dialog && r.dialog.closedByEscape],
    ['the PLAY dock behind it is inert, the side menu is not', !!r.dialog && r.dialog.dockInert === true && r.dialog.navLive],
    ['Health opened from it takes focus', !!r.dialog && r.dialog.healthFocus],
    ['closing Health returns focus to the Health button', !!r.dialog && r.dialog.backTo === 'set-health'],
    ['closing Settings returns focus to the menu item and frees the dock', !!r.dialog && r.dialog.focusAfter === 'nav-settings' && r.dialog.dockAfter === false],
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
  { name: 'required mods remain viewable during the game without installer controls', s: { ...base, modsState: {
    mods: [{ id: 'a', name: 'A', installed: false }], nexus: { name: 'Player', is_premium: true }, vortex: false, running: false, sso: true,
  }, actions: [{ at: 2800, kind: 'click', id: 'files-mods' }] }, expect: r => [
    ['the game starts', played(r)],
    ['the required mods dialog opens', r.modsShown],
    ['no mod installer is exposed or started', !r.installerControl && askedAt(r, 'download_all_mods') === null],
  ] },
  { name: 'a found mod file is not called deployed in Vortex', s: { ...base, clicks: [], modsState: {
    mods: [{ id: 'a', name: 'A', installed: true, from: 'nexus' }], nexus: null, vortex: true, running: false, sso: true,
  }, actions: [{ at: 1500, kind: 'click', id: 'files-mods' }] }, expect: r => [
    ['the required mods dialog opens', r.modsShown],
    ['no ownership line without a report', r.ownership === null, r.ownership],
    ['the row waits for Vortex profile evidence', /Game files found.*waiting for Vortex profile check/.test(r.modRow), r.modRow],
    ['the dialog distinguishes files from deployment', /does not confirm deployment/.test(r.modLead), r.modLead],
    ['there is no direct installer control', !r.installerControl],
  ] },
  { name: 'Requirements shows exact Vortex deployment failure', s: { ...base, clicks: [], modsState: {
    mods: [{ id: 'a', name: 'Test Mod', installed: true, in_vortex: false, from: 'nexus', looks_for: 'Data/Test.esp', vortex_installed: true, vortex_enabled: false, vortex_deployed: false }],
    counts_text: 'Vortex: 0 of 1 required Nexus mods confirmed · Game files: 1 of 1 present',
    vortex_line: 'Vortex: 1 required mod needs deployment or game files in Skyrim: Test Mod. Deploy in Vortex, then Check again.',
    vortex_ready: false, vortex_paired: true,
    ownership_text: 'Installed by the launcher: 2 mods · 1 only in the launcher\'s files · 1 also deployed by Vortex (two owners): Test Mod',
  }, actions: [{ at: 1500, kind: 'click', id: 'files-mods' }] }, expect: r => [
    ['mods held by both the launcher and Vortex are named', /1 also deployed by Vortex \(two owners\): Test Mod$/.test(r.ownership || ''), r.ownership],
    ['the missing count and name are visible', /1 required mod.*Test Mod/.test(r.vortexLine || ''), r.vortexLine],
    ['the summary does not report Vortex ready', /0 of 1 required Nexus mods confirmed/.test(r.modSummary), r.modSummary],
    ['the mod row identifies deployment as missing', /Vortex deployment or game files need attention/.test(r.modRow), r.modRow],
    ['the row says which Vortex step is missing', /In Vortex: installed, not switched on, not deployed$/.test(r.modRow || ''), r.modRow],
    ['the player is not offered a direct installer', !r.installerControl],
  ] },
  { name: 'Requirements refresh cannot enable Play ahead of hero Vortex check', s: { ...base, clicks: [],
    delay: { mods_state: 2000 }, actions: [{ at: 200, kind: 'click', id: 'files-mods' }],
  }, expect: r => [
    ['both Requirements and hero Vortex requests completed', r.invokes.filter(i => i[1] === 'answer' && i[2] === 'mods_state').length >= 2],
    ['Play waits for the hero Vortex answer', firstLabel(r, 'PLAY') >= Math.max(...r.invokes.filter(i => i[1] === 'answer' && i[2] === 'mods_state').map(i => i[0]))],
  ] },
  { name: 'incomplete Vortex setup changes the hero action to Requirements', s: { ...base, clicks: [], modsState: {
    mods: [{ id: 'a', name: 'Test Mod', installed: false, in_vortex: false, from: 'nexus' }],
    vortex_ready: false, vortex_paired: true,
    vortex_line: 'Vortex: 1 required mod needs deployment or game files in Skyrim: Test Mod.',
  }, actions: [{ at: 1800, kind: 'click', id: 'play' }] }, expect: r => [
    ['PLAY never appears before the Vortex failure', firstLabel(r, 'PLAY') === null],
    ['the button becomes MODS NEEDED', lastLabel(r) === 'MODS NEEDED', lastLabel(r)],
    ['the hero action opens Requirements', r.modsShown && r.modal === 'reqs'],
    ['the status names the missing mod', /Test Mod/.test(r.status), r.status],
    ['the game never starts', !played(r)],
  ] },
  { name: 'hero summarizes a long missing Vortex list', s: { ...base, clicks: [], modsState: {
    mods: ['One', 'Two', 'Three', 'Four', 'Five', 'Six', 'Seven'].map(name => ({ name, from: 'nexus', in_vortex: false, installed: false })),
    vortex_ready: false, vortex_paired: true,
    vortex_line: 'Aetherial Dawn profile: 0 of 7 installed · 0 of 7 switched on · waiting: One, Two, Three, Four, Five, Six, Seven',
  } }, expect: r => [
    ['the hero shows count, three names and the remainder', /7 required mods.*One, Two, Three, \+4 more/.test(r.status), r.status],
    ['the hero does not repeat the full seven-name list', !/Four, Five, Six, Seven/.test(r.status)],
    ['the button points to full Requirements', lastLabel(r) === 'MODS NEEDED'],
  ] },
  { name: 'a missing direct-source mod keeps the hero from claiming overall readiness', s: { ...base, clicks: [], modsState: {
    mods: [{ id: 'a', name: 'Direct Helper', installed: false, from: 'direct' }],
    vortex_ready: true, vortex_paired: true,
    vortex_line: 'Aetherial Dawn profile: every required mod installed and switched on',
  } }, expect: r => [
    ['the button points to Requirements', lastLabel(r) === 'MODS NEEDED', lastLabel(r)],
    ['the missing direct mod is named', /Direct Helper/.test(r.status), r.status],
    ['the game never starts', !played(r)],
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
    ['titlebar controls are inert behind the dialog', r.titlebarInert],
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

const chrome = findChrome();
let failed = 0;
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
