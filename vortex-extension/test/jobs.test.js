// Fixture tests for the extension's job rules, with a stand-in for Vortex's
// api (no Vortex here). They prove the rules, not Vortex: the real proof is
// on a disposable profile on Timothy's PC (Package C evidence gate).
'use strict';

const test = require('node:test');
const assert = require('node:assert');
const crypto = require('crypto');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { Jobs, sign, GAME, PROFILE_NAME } = require('../jobs');

const TOKEN = 'a'.repeat(64);

function fakeVortex() {
  const st = {
    persistent: {
      profiles: {
        p1: { id: 'p1', gameId: GAME, name: PROFILE_NAME, modState: {} },
        p2: { id: 'p2', gameId: GAME, name: 'Default', modState: {} },
      },
      mods: { [GAME]: {
        // Timothy's Unofficial Patch 4.3.9c, from Vortex.
        ussep439c: { id: 'ussep439c', state: 'installed', attributes: { modId: 266, fileId: 999999, version: '4.3.9c' } },
      } },
    },
    settings: { profiles: { activeProfileId: 'p1' } },
  };
  st.persistent.profiles.p1.modState.ussep439c = { enabled: true };
  const calls = [];
  let n = 0;
  return {
    st,
    calls,
    state: () => st,
    actions: { setModEnabled: (profileId, modId, enabled) => ({ type: 'SET_MOD_ENABLED', profileId, modId, enabled }) },
    dispatch: a => {
      calls.push(a);
      if (a.type === 'SET_MOD_ENABLED') st.persistent.profiles[a.profileId].modState[a.modId] = { enabled: a.enabled };
    },
    install: async archive => {
      calls.push({ type: 'install', archive });
      const id = `mod${++n}`;
      st.persistent.mods[GAME][id] = { id, state: 'installed', attributes: {} };
      return id;
    },
    deploy: async () => calls.push({ type: 'deploy' }),
    setAttribute: (id, k, v) => { st.persistent.mods[GAME][id].attributes[k] = v; },
  };
}

function setup() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ad-vx-'));
  const downloads = path.join(dir, 'downloads');
  fs.mkdirSync(downloads);
  const archive = path.join(downloads, 'ussep.7z');
  fs.writeFileSync(archive, 'stand-in archive 4.3.8a');
  const sha = crypto.createHash('sha256').update('stand-in archive 4.3.8a').digest('hex');
  const v = fakeVortex();
  let now = 1_000_000;
  const jobs = new Jobs(v, { token: TOKEN, downloads, journalPath: path.join(dir, 'journal.json'), now: () => now });
  let k = 0;
  const call = (verb, args, extra = {}) => {
    const body = JSON.stringify({ verb, args, revision: 'r1', ts: now, nonce: `nonce-${++k}-0123456789`, ...extra });
    return jobs.handle(body, extra.sig || sign(TOKEN, body));
  };
  const manifest = [{ id: 'ussep', modId: 266, fileId: 733846, sha256: sha }];
  return { dir, downloads, archive, sha, v, jobs, call, manifest, tick: ms => { now += ms; } };
}

test('an unsigned, wrongly signed, stale or replayed request is refused', async () => {
  const t = setup();
  const body = JSON.stringify({ verb: 'status', ts: 1_000_000, nonce: 'n-0123456789abcdef' });
  assert.equal((await t.jobs.handle(body, undefined)).code, 'unsigned');
  assert.equal((await t.jobs.handle(body, sign('b'.repeat(64), body))).code, 'unsigned');
  assert.equal((await t.jobs.handle(body, sign(TOKEN, body))).ok, true);
  assert.equal((await t.jobs.handle(body, sign(TOKEN, body))).code, 'replay');
  const old = JSON.stringify({ verb: 'status', ts: 1_000_000 - 120_000, nonce: 'n-fedcba9876543210' });
  assert.equal((await t.jobs.handle(old, sign(TOKEN, old))).code, 'stale');
});

test('nothing is installed before the manifest, or off it, or from outside the downloads folder, or with other bytes', async () => {
  const t = setup();
  assert.equal((await t.call('install', { modId: 266, fileId: 733846, archive: t.archive })).code, 'revision');
  assert.equal((await t.call('manifest', { revision: 'r1', mods: t.manifest })).ok, true);
  assert.equal((await t.call('install', { modId: 266, fileId: 1, archive: t.archive })).code, 'not-listed');
  const outside = path.join(t.dir, 'elsewhere.7z');
  fs.writeFileSync(outside, 'stand-in archive 4.3.8a');
  assert.equal((await t.call('install', { modId: 266, fileId: 733846, archive: outside })).code, 'path');
  assert.equal((await t.call('install', { modId: 266, fileId: 733846, archive: path.join(t.downloads, '..', 'elsewhere.7z') })).code, 'path');
  fs.writeFileSync(t.archive, 'other bytes');
  assert.equal((await t.call('install', { modId: 266, fileId: 733846, archive: t.archive })).code, 'hash');
  assert.equal(t.v.calls.filter(c => c.type === 'install').length, 0);
});

test('only the Aetherial Dawn profile, never switched: another active profile, none, or two are refused', async () => {
  const t = setup();
  await t.call('manifest', { revision: 'r1', mods: t.manifest });
  t.v.st.settings.profiles.activeProfileId = 'p2';
  assert.equal((await t.call('install', { modId: 266, fileId: 733846, archive: t.archive })).code, 'other-profile');
  assert.equal(t.v.st.settings.profiles.activeProfileId, 'p2');
  t.v.st.persistent.profiles.p3 = { id: 'p3', gameId: GAME, name: PROFILE_NAME, modState: {} };
  assert.equal((await t.call('deploy', {})).code, 'two-profiles');
  delete t.v.st.persistent.profiles.p3;
  delete t.v.st.persistent.profiles.p1;
  assert.equal((await t.call('deploy', {})).code, 'no-profile');
});

test('USSEP: 4.3.8a installed, tagged and switched on, 4.3.9c switched off (never removed); a rerun is a no-op; rollback undoes only its own changes', async () => {
  const t = setup();
  await t.call('manifest', { revision: 'r1', mods: t.manifest });
  const got = await t.call('install', { modId: 266, fileId: 733846, archive: t.archive });
  assert.equal(got.ok, true);
  const mod = t.v.st.persistent.mods[GAME][got.vortexId];
  assert.deepEqual([mod.attributes.modId, mod.attributes.fileId, mod.attributes.aetherialDawn], [266, 733846, 'r1']);
  assert.equal((await t.call('enable', { vortexId: got.vortexId })).ok, true);
  // The listed file can't be switched off; the other version can.
  assert.equal((await t.call('disable', { vortexId: got.vortexId })).code, 'listed-file');
  assert.equal((await t.call('disable', { vortexId: 'ussep439c' })).ok, true);
  assert.equal((await t.call('deploy', {})).ok, true);
  const st = (await t.call('status', {}));
  const on = Object.fromEntries(st.mods.map(m => [m.id, m.enabled]));
  assert.deepEqual(on, { ussep439c: false, [got.vortexId]: true });
  assert.ok(t.v.st.persistent.mods[GAME].ussep439c, '4.3.9c is still installed');
  // A second run installs nothing new.
  const again = await t.call('install', { modId: 266, fileId: 733846, archive: t.archive });
  assert.equal(again.already, true);
  assert.equal(t.v.calls.filter(c => c.type === 'install').length, 1);
  // Rollback: 4.3.9c back on, 4.3.8a off, one deploy; nothing else touched.
  const r = await t.call('rollback', {});
  assert.deepEqual(r.undone, { added: 1, enabled: 1, disabled: 1 });
  assert.equal(t.v.st.persistent.profiles.p1.modState.ussep439c.enabled, true);
  assert.equal(t.v.st.persistent.profiles.p1.modState[got.vortexId].enabled, false);
  assert.equal(t.v.st.persistent.profiles.p2.modState[got.vortexId], undefined, 'the other profile is untouched');
  assert.equal(t.v.calls.filter(c => c.type === 'deploy').length, 2);
});

test('the journal survives a restart, so a rollback after a crash still knows what it did', async () => {
  const t = setup();
  await t.call('manifest', { revision: 'r1', mods: t.manifest });
  const got = await t.call('install', { modId: 266, fileId: 733846, archive: t.archive });
  const again = new Jobs(t.v, { token: TOKEN, downloads: t.downloads, journalPath: path.join(t.dir, 'journal.json'), now: () => 1_000_000 });
  assert.deepEqual(again.journal.added.map(a => a.vortexId), [got.vortexId]);
});

test('a mod not on the list is never switched on or off', async () => {
  const t = setup();
  await t.call('manifest', { revision: 'r1', mods: t.manifest });
  t.v.st.persistent.mods[GAME].personal = { id: 'personal', state: 'installed', attributes: { modId: 12345, fileId: 1 } };
  assert.equal((await t.call('enable', { vortexId: 'personal' })).code, 'not-listed');
  assert.equal((await t.call('disable', { vortexId: 'personal' })).code, 'not-listed');
});
