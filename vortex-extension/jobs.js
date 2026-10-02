// The Aetherial Dawn Vortex extension's job handling, kept apart from
// Vortex so it can be tested without it (test/jobs.test.js drives it with a
// stand-in for Vortex's api). Package C, PR #7 (Codex, 2026-09-28), re-scoped
// to the Vortex collection route (5876088533): Vortex installs the Aetherial
// Dawn collection with its own flow, and this extension only reports.
//
// Rules it enforces:
// - Only the launcher can ask: every request is HMAC-SHA256 signed with a
//   per-install token only this Windows user can read, carries a fresh
//   timestamp and a nonce never seen before.
// - Read-only: the one request is `status`, answered from api.getState().
//   Nothing here installs, switches on or off, deploys, or switches
//   profiles, and Vortex's state files are never read or written directly.

'use strict';

const crypto = require('crypto');

const GAME = 'skyrimse';
const PROFILE_NAME = 'Aetherial Dawn';
const MAX_SKEW_MS = 60 * 1000;
const VERBS = ['status'];

function sign(token, body) {
  return crypto.createHmac('sha256', token).update(body).digest('hex');
}

class Jobs {
  // vortex: { state() }
  // opts: { token, now(), version }
  constructor(vortex, opts) {
    this.v = vortex;
    this.token = opts.token;
    // The version in this extension's own info.json, so the launcher can
    // prove which version Vortex actually loaded.
    this.version = opts.version || null;
    this.now = opts.now || (() => Date.now());
    this.seen = new Map();
  }

  // The request as the launcher sent it: raw body and its signature.
  async handle(raw, sig) {
    const want = Buffer.from(sign(this.token, raw));
    // Byte lengths, not string lengths: timingSafeEqual throws on buffers of
    // different sizes, and a multi-byte signature can match in characters.
    const got = typeof sig === 'string' ? Buffer.from(sig) : null;
    if (!got || got.length !== want.length || !crypto.timingSafeEqual(got, want)) {
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
    try {
      return await this[req.verb](req.args || {});
    } catch (e) {
      return { ok: false, code: 'failed', error: String(e && e.message || e) };
    }
  }

  // What Vortex says now, for the launcher's step lines and Ready check:
  // every Skyrim SE profile named Aetherial Dawn and which profile is
  // active; each mod with its Nexus ids, state and whether that profile has
  // it on; and the collections, with slug and revision. Attribute names
  // marked (verify) come from Vortex's source and are proven on the
  // disposable profile before the launcher relies on them.
  async status() {
    const s = this.v.state();
    const profiles = Object.values(((s.persistent || {}).profiles) || {}).filter(p => p.gameId === GAME);
    const mine = profiles.filter(p => p.name === PROFILE_NAME);
    const active = ((s.settings || {}).profiles || {}).activeProfileId;
    const prof = mine.length === 1 ? mine[0] : null;
    const modState = (prof && prof.modState) || {};
    const mods = Object.values(((s.persistent || {}).mods || {})[GAME] || {});
    const attr = m => m.attributes || {};
    return {
      ok: true,
      extensionVersion: this.version,
      activeProfile: profiles.some(p => p.id === active) ? { id: active, name: (profiles.find(p => p.id === active) || {}).name } : null,
      aetherialProfiles: mine.length,
      profile: prof ? { id: prof.id, name: prof.name, active: active === prof.id } : null,
      mods: mods.filter(m => m.type !== 'collection').map(m => ({
        id: m.id,
        // Vortex uses this staging folder name as the deployment record's
        // `source`; the launcher binds it to the exact Nexus file below.
        installationPath: m.installationPath || null,
        state: m.state,
        nexusModId: attr(m).modId,
        nexusFileId: attr(m).fileId,
        version: attr(m).version,
        enabled: !!(modState[m.id] && modState[m.id].enabled),
        // (verify) the FOMOD choices Vortex saved for this install.
        installerChoices: attr(m).installerChoices || null,
      })),
      collections: mods.filter(m => m.type === 'collection').map(m => ({
        id: m.id,
        state: m.state,
        enabled: !!(modState[m.id] && modState[m.id].enabled),
        // (verify) collection attributes set by Vortex's collections extension.
        slug: attr(m).collectionSlug || null,
        revision: attr(m).revisionNumber != null ? attr(m).revisionNumber : null,
      })),
    };
  }
}

module.exports = { Jobs, sign, GAME, PROFILE_NAME };
