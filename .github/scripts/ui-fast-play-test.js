// Proves the fast-Play path (0.1.96): a returning player sees PLAY at once
// while the game and Discord checks run behind it, and those checks still
// decide. The launcher's page runs in headless Chrome with a fake launcher
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
  const log = window.__T = { invokes: [], labels: [], lastReady: null, newsHeights: [] };
  if (S.pageSize) Object.assign(document.documentElement.style, { width: S.pageSize[0] + 'px', height: S.pageSize[1] + 'px' });
  const at = () => Math.round(performance.now() - t0);
  try { localStorage.clear(); if (S.seed) localStorage.setItem('ad.lastReady', JSON.stringify(S.seed)); } catch (_) {}
  const game = Object.assign({ needed: false, installed: '1.6.1170.0', target: '1.6.1170.0', skseOk: true, canDowngrade: true }, S.game || {});
  const answers = {
    get_state: () => ({ launcherVersion: S.version, config: { gameDir: S.dir, closeOnLaunch: false, backgroundUpdates: !!S.backgroundUpdates, shareHealth: true, music: false, onlyServerMods: true }, game: { dir: S.dir, hasSkse: S.hasSkse !== false } }),
    auth_status: () => (S.authAfter && (S.authCalls = (S.authCalls || 0) + 1) > 1) ? S.authAfter : S.auth,
    check: () => Object.assign({ build: 'B2', server: { name: 'Aetherial Dawn', ip: '127.0.0.1', port: 7777 }, files: 0, remove: 0, bytes: 0, strays: [], game }, S.check || {}),
    update: () => null,
    server_status: () => ({ online: true, players: 1, maxPlayers: 50, discordInvite: S.invite, news: [{ title: 'News', date: '28 Sep', body: 'Body.' }, { title: 'More', date: '27 Sep', body: 'Body.' }] }),
    files: () => [], game_check: () => game, play: () => S.playWarnings || null,
    game_running: () => !!S.outsideGame,
    mods_state: () => S.modsState || ({ mods: [], nexus: null, vortex: false, running: false, sso: false }),
    download_all_mods: () => ({ installed: [], failed: [], cancelled: false }),
    plain_error: a => a.text,
  };
  const delay = Object.assign({ get_state: 20, auth_status: 300, check: 900, update: 600, server_status: 300, play: 50, default: 10 }, S.delay || {});
  window.__TAURI__ = {
    core: { invoke: (cmd, args) => new Promise((res, rej) => {
      log.invokes.push([at(), 'ask', cmd]);
      const pickDelay = cmd === 'set_game_dir' && S.picks && (S.picks.find(p => p.dir === (args || {}).dir) || {}).delay;
      setTimeout(() => {
        log.invokes.push([at(), 'answer', cmd]);
        if (cmd === 'auth_status' && S.authFails) return rej('network down');
        if (cmd === 'game_running' && (S.gameCheckFails || (S.gameCheckFailsFrom && (S.gameChecks = (S.gameChecks || 0) + 1) >= S.gameCheckFailsFrom))) return rej('process list unavailable');
        if (cmd === 'set_game_dir' && S.setDirError) return rej(S.setDirError);
        if (cmd === 'set_game_dir' && S.picks && S.picks.find(p => p.dir === args.dir && p.error)) return rej(S.picks.find(p => p.dir === args.dir).error);
        if (cmd === 'play' && S.playError && !S.played) { S.played = true; return rej(S.playError); }
        res(answers[cmd] ? answers[cmd](args || {}) : null);
      }, pickDelay || (delay[cmd] ?? delay.default));
    }) },
    event: { listen: async () => () => {} },
    window: { getCurrentWindow: () => ({ minimize: async () => {}, close: async () => {}, unminimize: async () => {}, setFocus: async () => {}, show: async () => {} }) },
    // S.picks: what each press of "Choose folder" returns in turn (dir null = cancelled).
    dialog: { open: async () => S.picks ? (S.picks[(S.picked = (S.picked || 0) + 1) - 1] || {}).dir || null : S.pickDir || null },
    // The fake latest.json: this launcher is the newest, unless S.update names a newer one.
    updater: { check: () => new Promise(r => setTimeout(() => r(S.update ? {
      version: S.update,
      download: () => { log.invokes.push([at(), 'ask', 'updater_download']); return new Promise(res => setTimeout(res, 200)); },
      install: async () => { log.invokes.push([at(), 'ask', 'updater_install']); },
      downloadAndInstall: async () => { log.invokes.push([at(), 'ask', 'updater_install']); },
    } : null), 100)) },
    process: { relaunch: async () => {} },
  };
  document.addEventListener('DOMContentLoaded', () => {
    const label = document.getElementById('play-label'), btn = document.getElementById('play');
    const note = () => log.labels.push([at(), label.textContent + (btn.disabled ? ' [off]' : '')]);
    note();
    new MutationObserver(note).observe(label, { childList: true, characterData: true, subtree: true });
    new MutationObserver(note).observe(btn, { attributes: true, attributeFilter: ['disabled'] });
    const box = () => log.newsHeights.push([at(), document.getElementById('news-box').offsetHeight]);
    box();
    setInterval(box, 100);
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
    for (const t of S.clicks || []) setTimeout(() => { log.invokes.push([at(), 'click', label.textContent]); btn.click(); }, t);
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
      const free = document.getElementById('rq-nx-free');
      log.freeNote = free.hidden ? null : free.textContent;
      log.modsButton = document.getElementById('rq-all').textContent;
      log.modsShown = !document.getElementById('reqs').hidden;
      const rect = sel => { const b = document.querySelector(sel).getBoundingClientRect(); return [Math.round(b.top), Math.round(b.bottom)]; };
      log.layout = { height: document.querySelector('.app').offsetHeight, width: document.querySelector('.app').offsetWidth, titlebar: rect('.titlebar'), dock: rect('.dock'), play: rect('#play') };
      const pre = document.createElement('pre');
      pre.id = 'ui-test-result';
      pre.textContent = JSON.stringify(log);
      document.body.appendChild(pre);
    }, S.end || 6000);
  });
}

const seed = { dir: DIR, build: 'B1', version: VERSION, play: true, status: { online: true, players: 1, maxPlayers: 50 } };
const ok = { signedIn: true, account: { discordUsername: 'Player' }, offline: false, locked: false };
const base = { version: VERSION, dir: DIR, auth: ok, seed, clicks: [60] };

const played = r => r.invokes.some(i => i[1] === 'ask' && i[2] === 'play');
const askedAt = (r, cmd) => (r.invokes.find(i => i[1] === 'ask' && i[2] === cmd) || [null])[0];
const answeredAt = (r, cmd) => (r.invokes.find(i => i[1] === 'answer' && i[2] === cmd) || [null])[0];
const firstLabel = (r, text) => (r.labels.find(l => l[1] === text) || [null])[0];
const lastLabel = r => r.labels[r.labels.length - 1][1];

const scenarios = [
  { name: 'returning player: PLAY shows at once, and Play waits for both checks', s: base, expect: r => [
    // Timed from the page's own start, which a cold CI machine delays.
    ['PLAY is enabled before either check answers', firstLabel(r, 'PLAY') !== null && firstLabel(r, 'PLAY') < Math.min(answeredAt(r, 'auth_status'), answeredAt(r, 'check')),
      `${firstLabel(r, 'PLAY') - askedAt(r, 'get_state')} ms after the page asked for its settings`],
    ['Play pressed early shows STARTING', firstLabel(r, 'STARTING [off]') !== null],
    ['the game starts once', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'play').length === 1],
    ['the game starts only after the Discord answer', askedAt(r, 'play') >= answeredAt(r, 'auth_status')],
    ['the game starts only after the game-files answer', askedAt(r, 'play') >= answeredAt(r, 'check')],
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
  { name: 'not a member, and the refusal carries the invite: one clickable link', s: { ...base, auth: { signedIn: false, message: 'Join the Aetherial Dawn Discord first: https://discord.gg/aetherial. Then press Sign in with Discord again.' }, invite: 'https://discord.gg/aetherial' }, expect: r => [
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
  { name: 'free Nexus account: told first, and the list waits for Start', s: { ...base, playError: 'NEEDS_NEXUS_MODS:[{"id":"a","name":"A"},{"id":"b","name":"B"}]', modsState: { mods: [{ id: 'a', name: 'A', installed: false }, { id: 'b', name: 'B', installed: false }], nexus: { name: 'Player', is_premium: false }, vortex: false, running: false, sso: true } }, expect: r => [
    ['the mods window is open', r.modsShown],
    ['it says a free account needs one press per mod, and Premium is one button', /one press per mod/.test(r.freeNote || '') && /Premium it's one button/.test(r.freeNote || ''), r.freeNote],
    ['it says how many', /2 mods to get/.test(r.freeNote || '')],
    ['the button reads START (2)', r.modsButton === 'START (2)', r.modsButton],
    ['nothing starts before Start is pressed', askedAt(r, 'download_all_mods') === null],
  ] },
  { name: 'Premium Nexus account: Play installs the mods by itself', s: { ...base, playError: 'NEEDS_NEXUS_MODS:[{"id":"a","name":"A"}]', modsState: { mods: [{ id: 'a', name: 'A', installed: false }], nexus: { name: 'Player', is_premium: true }, vortex: false, running: false, sso: true } }, expect: r => [
    ['no free-account note', r.freeNote === null],
    ['the mods download by themselves', askedAt(r, 'download_all_mods') !== null],
  ] },
  { name: 'Discord cannot be reached: SIGN IN, the game never starts', s: { ...base, authFails: true }, expect: r => [
    ['the game never starts', !played(r)],
    ['the button ends on SIGN IN', lastLabel(r) === 'SIGN IN'],
  ] },
  { name: 'account locked: RETRY, the game never starts', s: { ...base, auth: { ...ok, locked: true, message: 'Your account is locked.' } }, expect: r => [
    ['the game never starts', !played(r)],
    ['the button ends on RETRY, which can be pressed', lastLabel(r) === 'RETRY', lastLabel(r)],
    ['next start does not open on PLAY', r.lastReady && r.lastReady.play === false],
  ] },
  { name: 'account locked, then the login service answers: Retry brings PLAY back', s: { ...base, auth: { ...ok, locked: true, message: 'Your account is locked.' }, authAfter: ok, clicks: [2500] }, expect: r => [
    ['Retry asked the login service again', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'auth_status').length >= 2],
    ['the button ends on PLAY', lastLabel(r) === 'PLAY', lastLabel(r)],
    ['the game never starts by itself', !played(r)],
  ] },
  { name: 'account locked, then the login service answers: the minute retry brings PLAY back once', s: { ...base, auth: { ...ok, locked: true, message: 'Your account is locked.' }, authAfter: ok, clicks: [], end: 130000 }, expect: r => [
    ['the login service was asked twice, not more', r.invokes.filter(i => i[1] === 'ask' && i[2] === 'auth_status').length === 2, String(r.invokes.filter(i => i[1] === 'ask' && i[2] === 'auth_status').length)],
    ['the button ends on PLAY', lastLabel(r) === 'PLAY', lastLabel(r)],
  ] },
  { name: 'an update found behind PLAY takes over the button', s: { ...base, check: { build: 'B2', files: 3, bytes: 3000000 } }, expect: r => [
    ['the game never starts', !played(r)],
    ['the button ends on UPDATE', lastLabel(r) === 'UPDATE'],
    ['next start does not open on PLAY', r.lastReady && r.lastReady.play === false],
  ] },
  { name: 'an update found behind PLAY with background updates: updates, then plays', s: { ...base, backgroundUpdates: true, check: { build: 'B2', files: 3, bytes: 3000000 } }, expect: r => [
    ['the button shows UPDATING', firstLabel(r, 'UPDATING [off]') !== null],
    ['the game starts only after the update', askedAt(r, 'play') !== null && askedAt(r, 'play') >= answeredAt(r, 'update')],
  ] },
  { name: 'wrong game version (no fix possible): Play stops', s: { ...base, game: { needed: true, canDowngrade: false, reason: 'Skyrim is not the version the server needs.' } }, expect: r => [
    ['the game never starts', !played(r)],
    ['no version fix starts by itself', askedAt(r, 'patch_game') === null],
    ['the button ends on WRONG VERSION', lastLabel(r) === 'WRONG VERSION [off]'],
    ['next start does not open on PLAY', r.lastReady && r.lastReady.play === false],
  ] },
  { name: 'wrong game version (fixable): the early press is not taken as a yes to the fix', s: { ...base, game: { needed: true, canDowngrade: true, reason: 'Skyrim needs changing.' } }, expect: r => [
    ['the game never starts', !played(r)],
    ['the fix waits for a press made after the reason shows', askedAt(r, 'patch_game') === null],
    ['the status line gives the reason', /Skyrim needs changing/.test(r.status)],
  ] },
  { name: 'a helper mod could not be installed: the game still starts and the reason stays shown', s: { ...base, playWarnings: ["Crash Logger 1.25.0 isn't installed: the download didn't get through. Skyrim starts without it; Play tries again next time."] }, expect: r => [
    ['the game starts', played(r)],
    ['the status line keeps the warning', /Crash Logger 1\.25\.0 isn't installed/.test(r.status), r.status],
  ] },
  { name: 'login service unreachable, still signed in: told before Play', s: { ...base, auth: { ...ok, offline: true, message: "Couldn't reach the login service. You're still signed in, but Play needs it to start the game, so try again when it's back." }, clicks: [] }, expect: r => [
    ['the status line says Play needs the login service', /Play needs it/.test(r.status), r.status],
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
  { name: 'launcher update found while Play is starting the game: it waits', s: { ...base, update: '9.9.10', delay: { play: 3000 } }, expect: r => [
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

const chrome = findChrome();
let failed = 0;
for (const sc of scenarios) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ad-ui-'));
  fs.cpSync(UI, dir, { recursive: true });
  const html = fs.readFileSync(path.join(dir, 'index.html'), 'utf8')
    .replace('<head>', `<head><script>window.__S=${JSON.stringify(sc.s)};(${fakeBackEnd})();</script>`);
  fs.writeFileSync(path.join(dir, 'index.html'), html);
  const url = 'file:///' + path.join(dir, 'index.html').replace(/\\/g, '/').replace(/^\//, '');
  let out = '';
  try {
    // Chrome refuses to run as root on Linux without this.
    const root = process.getuid && process.getuid() === 0 ? ['--no-sandbox'] : [];
    out = execFileSync(chrome, [...root, '--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check',
      `--user-data-dir=${path.join(dir, 'profile')}`, '--allow-file-access-from-files', '--window-size=1360,880',
      `--virtual-time-budget=${Math.max(8000, (sc.s.end || 6000) + 2000)}`, '--dump-dom', url], { encoding: 'utf8', timeout: 60000, stdio: ['ignore', 'pipe', 'ignore'] });
  } catch (e) { out = String(e.stdout || ''); }
  const m = out.match(/<pre id="ui-test-result">([\s\S]*?)<\/pre>/);
  console.log(`== ${sc.name}`);
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
