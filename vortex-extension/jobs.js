// The Aetherial Dawn Vortex extension's job handling, kept apart from
// Vortex so it can be tested without it (test/jobs.test.js drives it with a
// stand-in for Vortex's api). Package C, PR #7 (Codex, 2026-09-28).
//
// Rules it enforces:
// - Only the launcher can ask: every request is HMAC-SHA256 signed with a
//   per-install token only this Windows user can read, carries a fresh
//   timestamp and a nonce never seen before.
// - Only listed mods: the launcher first sends the manifest revision (the
//   exact mod list, each entry's Nexus mod id, file id and archive sha256);
//   an install must match one entry, and the archive must hash to it and sit
//   in the launcher's downloads folder. No other path or URL is taken.
// - Only the Aetherial Dawn profile: jobs are refused unless Vortex's active
//   profile is the one named "Aetherial Dawn" for Skyrim SE. Nothing
//   switches profiles.
// - Only Vortex's own API: install (start-install), enable
//   (actions.setModEnabled), deploy (deploy-mods) and state reads. Vortex's
//   state files are never read or written by this code.
// - Rollback touches only what the journal says this extension did.

'use strict';

const crypto = require('crypto');
const fs = require('fs');
const path = require('path');

const GAME = 'skyrimse';
const PROFILE_NAME = 'Aetherial Dawn';
const MAX_SKEW_MS = 60 * 1000;
const VERBS = ['manifest', 'status', 'install', 'enable', 'disable', 'deploy', 'rollback'];

function sign(token, body) {
  return crypto.createHmac('sha256', token).update(body).digest('hex');
}

function sha256File(p) {
  const h = crypto.createHash('sha256');
  const fd = fs.openSync(p, 'r');
  try {
    const buf = Buffer.alloc(1 << 20);
    let n;
    while ((n = fs.readSync(fd, buf, 0, buf.length, null)) > 0) h.update(buf.subarray(0, n));
  } finally {
    fs.closeSync(fd);
  }
  return h.digest('hex');
}

function inside(dir, p) {
  const rel = path.relative(path.resolve(dir), path.resolve(p));
  return rel !== '' && !rel.startsWith('..') && !path.isAbsolute(rel);
}

class Jobs {
  // vortex: { state(), dispatch(action), actions, install(archive) -> Promise<modId>,
  //           deploy() -> Promise, setAttribute(modId, key, value) }
  // opts: { token, downloads, journalPath, now() }
  constructor(vortex, opts) {
    this.v = vortex;
    this.token = opts.token;
    this.downloads = opts.downloads;
    this.journalPath = opts.journalPath;
    this.now = opts.now || (() => Date.now());
    this.seen = new Map();
    this.list = null;
    this.journal = this.loadJournal();
  }

  loadJournal() {
    try { return JSON.parse(fs.readFileSync(this.journalPath, 'utf8')); } catch { return { added: [], disabled: [], enabled: [] }; }
  }

  saveJournal() {
    const tmp = this.journalPath + '.part';
    fs.writeFileSync(tmp, JSON.stringify(this.journal, null, 2));
    fs.renameSync(tmp, this.journalPath);
  }

  // The request as the launcher sent it: raw body and its signature.
  async handle(raw, sig) {
    const want = sign(this.token, raw);
    if (typeof sig !== 'string' || sig.length !== want.length || !crypto.timingSafeEqual(Buffer.from(sig), Buffer.from(want))) {
      return { ok: false, code: 'unsigned', error: 'The request is not signed by the launcher.' };
    }
    let req;
    try { req = JSON.parse(raw); } catch { return { ok: false, code: 'bad', error: 'Not JSON.' }; }
    const t = Number(req.ts);
    if (!Number.isFinite(t) || Math.abs(this.now() - t) > MAX_SKEW_MS) return { ok: false, code: 'stale', error: 'The request is too old.' };
    if (typeof req.nonce !== 'string' || req.nonce.length < 16 || this.seen.has(req.nonce)) return { ok: false, code: 'replay', error: 'The request was already used.' };
    for (const [n, at] of this.seen) if (this.now() - at > 2 * MAX_SKEW_MS) this.seen.delete(n);
    this.seen.set(req.nonce, this.now());
    if (!VERBS.includes(req.verb)) return { ok: false, code: 'verb', error: `Unknown request ${req.verb}.` };
    if (req.verb !== 'manifest' && req.verb !== 'status') {
      const guard = this.profileGuard();
      if (guard) return guard;
      if (!this.list || req.revision !== this.list.revision) return { ok: false, code: 'revision', error: 'The mod list revision does not match the one the launcher sent.' };
    }
    try {
      return await this[req.verb](req.args || {});
    } catch (e) {
      return { ok: false, code: 'failed', error: String(e && e.message || e) };
    }
  }

  profile() {
    const s = this.v.state();
    const profiles = Object.values(((s.persistent || {}).profiles) || {});
    const mine = profiles.filter(p => p.gameId === GAME && p.name === PROFILE_NAME);
    const active = ((s.settings || {}).profiles || {}).activeProfileId;
    return { mine, active };
  }

  profileGuard() {
    const { mine, active } = this.profile();
    if (mine.length === 0) return { ok: false, code: 'no-profile', error: `Vortex has no profile named "${PROFILE_NAME}" for Skyrim Special Edition.` };
    if (mine.length > 1) return { ok: false, code: 'two-profiles', error: `Vortex has ${mine.length} profiles named "${PROFILE_NAME}"; keep one.` };
    if (active !== mine[0].id) return { ok: false, code: 'other-profile', error: `Switch Vortex to the "${PROFILE_NAME}" profile, then press Retry.` };
    return null;
  }

  async manifest(args) {
    if (typeof args.revision !== 'string' || !Array.isArray(args.mods)) return { ok: false, code: 'bad', error: 'A manifest needs a revision and mods.' };
    for (const m of args.mods) {
      if (!Number.isInteger(m.modId) || !Number.isInteger(m.fileId) || !/^[0-9a-f]{64}$/.test(m.sha256 || '')) return { ok: false, code: 'bad', error: `Manifest entry ${m.id} needs a Nexus mod id, file id and sha256.` };
    }
    this.list = { revision: args.revision, mods: args.mods };
    return { ok: true, revision: args.revision, entries: args.mods.length };
  }

  // What Vortex says now, for the launcher's Ready check: the profile, and
  // each mod of the game with its Nexus ids, enabled state and install path.
  async status() {
    const s = this.v.state();
    const { mine, active } = this.profile();
    const prof = mine[0];
    const mods = ((s.persistent || {}).mods || {})[GAME] || {};
    const modState = (prof && prof.modState) || {};
    return {
      ok: true,
      profile: prof ? { id: prof.id, name: prof.name, active: active === prof.id } : null,
      mods: Object.values(mods).map(m => ({
        id: m.id,
        state: m.state,
        nexusModId: (m.attributes || {}).modId,
        nexusFileId: (m.attributes || {}).fileId,
        version: (m.attributes || {}).version,
        enabled: !!(modState[m.id] && modState[m.id].enabled),
      })),
    };
  }

  entry(modId, fileId) {
    return (this.list.mods || []).find(m => m.modId === modId && m.fileId === fileId);
  }

  async install(args) {
    const e = this.entry(args.modId, args.fileId);
    if (!e) return { ok: false, code: 'not-listed', error: `Mod ${args.modId} file ${args.fileId} is not on the mod list.` };
    if (typeof args.archive !== 'string' || !inside(this.downloads, args.archive) || !fs.existsSync(args.archive)) return { ok: false, code: 'path', error: 'The archive is not in the launcher\'s downloads folder.' };
    const got = sha256File(args.archive);
    if (got !== e.sha256) return { ok: false, code: 'hash', error: `The archive for ${e.id} does not match the mod list (sha256 ${got.slice(0, 12)}…).` };
    // Already installed from this exact file: nothing to do.
    const mods = ((this.v.state().persistent || {}).mods || {})[GAME] || {};
    const have = Object.values(mods).find(m => (m.attributes || {}).modId === e.modId && (m.attributes || {}).fileId === e.fileId && m.state === 'installed');
    if (have) return { ok: true, vortexId: have.id, already: true };
    const vortexId = await this.v.install(args.archive);
    // The Nexus ids go on the mod, so the launcher's allowlist and its Ready
    // check find it whatever Vortex named its folder.
    this.v.setAttribute(vortexId, 'modId', e.modId);
    this.v.setAttribute(vortexId, 'fileId', e.fileId);
    this.v.setAttribute(vortexId, 'source', 'nexus');
    this.v.setAttribute(vortexId, 'aetherialDawn', this.list.revision);
    this.journal.added.push({ vortexId, id: e.id, revision: this.list.revision });
    this.saveJournal();
    return { ok: true, vortexId };
  }

  setEnabled(vortexId, on) {
    const prof = this.profile().mine[0];
    this.v.dispatch(this.v.actions.setModEnabled(prof.id, vortexId, on));
  }

  async enable(args) {
    const mods = ((this.v.state().persistent || {}).mods || {})[GAME] || {};
    const m = mods[args.vortexId];
    if (!m) return { ok: false, code: 'unknown', error: 'Vortex has no such mod.' };
    const a = m.attributes || {};
    if (!this.entry(a.modId, a.fileId)) return { ok: false, code: 'not-listed', error: 'Only mods on the list are switched on.' };
    this.setEnabled(args.vortexId, true);
    this.journal.enabled.push({ vortexId: args.vortexId, revision: this.list.revision });
    this.saveJournal();
    return { ok: true };
  }

  // Switches a mod off (never removes it): the launcher uses it for a
  // version the server can't run, such as the Unofficial Patch 4.3.9c.
  async disable(args) {
    const mods = ((this.v.state().persistent || {}).mods || {})[GAME] || {};
    const m = mods[args.vortexId];
    if (!m) return { ok: false, code: 'unknown', error: 'Vortex has no such mod.' };
    const a = m.attributes || {};
    const listedMod = (this.list.mods || []).some(e => e.modId === a.modId);
    if (!listedMod) return { ok: false, code: 'not-listed', error: 'Only a version of a listed mod is switched off.' };
    if (this.entry(a.modId, a.fileId)) return { ok: false, code: 'listed-file', error: 'That is the listed file; it stays on.' };
    this.setEnabled(args.vortexId, false);
    this.journal.disabled.push({ vortexId: args.vortexId, revision: this.list.revision });
    this.saveJournal();
    return { ok: true };
  }

  async deploy() {
    await this.v.deploy();
    return { ok: true };
  }

  // Undoes this extension's own changes, newest first, through the API:
  // added mods switched off (they stay installed for a later try), and mods
  // it switched off switched back on. Then a deploy.
  async rollback() {
    for (const d of [...this.journal.disabled].reverse()) this.setEnabled(d.vortexId, true);
    for (const a of [...this.journal.added, ...this.journal.enabled].reverse()) this.setEnabled(a.vortexId, false);
    await this.v.deploy();
    const undone = { added: this.journal.added.length, enabled: this.journal.enabled.length, disabled: this.journal.disabled.length };
    this.journal = { added: [], disabled: [], enabled: [] };
    this.saveJournal();
    return { ok: true, undone };
  }
}

module.exports = { Jobs, sign, GAME, PROFILE_NAME };
