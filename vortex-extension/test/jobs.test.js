// Fixture tests for the extension's rules, with a stand-in for Vortex's
// state (no Vortex here). They prove the rules, not Vortex: the attribute
// names marked (verify) in jobs.js are proven on a disposable profile on a
// real PC (Package C evidence gate).
'use strict';

const test = require('node:test');
const assert = require('node:assert');
const { Jobs, sign, GAME, PROFILE_NAME } = require('../jobs');

const TOKEN = 'a'.repeat(64);

function state() {
  return {
    persistent: {
      profiles: {
        p1: { id: 'p1', gameId: GAME, name: PROFILE_NAME, modState: { ussep439c: { enabled: true }, coll: { enabled: true }, skyui: { enabled: true } } },
        p2: { id: 'p2', gameId: GAME, name: 'Default', modState: {} },
        p3: { id: 'p3', gameId: 'fallout4', name: PROFILE_NAME, modState: {} },
      },
      mods: { [GAME]: {
        ussep439c: { id: 'ussep439c', installationPath: 'Unofficial Skyrim Special Edition Patch 266 4.3.9c 2026-09-05T22-22Z lNkz9LChN', state: 'installed', attributes: { modId: 266, fileId: 999999, version: '4.3.9c' } },
        skyui: { id: 'skyui', installationPath: 'SkyUI-12604-6-11-1778020881', state: 'installed', attributes: { modId: 12604, fileId: 35407, installerChoices: { type: 'fomod', options: [] } } },
        coll: { id: 'coll', type: 'collection', state: 'installed', attributes: { collectionSlug: 'abc123', revisionNumber: 3 } },
      } },
    },
    settings: { profiles: { activeProfileId: 'p1' } },
  };
}

function setup() {
  const st = state();
  const before = JSON.stringify(st);
  let now = 1_000_000;
  const jobs = new Jobs({ state: () => st }, { token: TOKEN, now: () => now });
  let k = 0;
  const call = (verb, extra = {}) => {
    const body = JSON.stringify({ verb, args: {}, revision: 'r1', ts: now, nonce: `nonce-${++k}-0123456789`, ...extra });
    return jobs.handle(body, extra.sig || sign(TOKEN, body));
  };
  return { st, before, jobs, call, tick: ms => { now += ms; } };
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

test('only status is answered: the old install, enable, disable, deploy and rollback requests are refused', async () => {
  const t = setup();
  for (const verb of ['manifest', 'install', 'enable', 'disable', 'deploy', 'rollback']) {
    assert.equal((await t.call(verb)).code, 'verb', verb);
  }
});

test('status reports profiles, mods with Nexus ids and choices, and the collection, and changes nothing', async () => {
  const t = setup();
  const s = await t.call('status');
  assert.equal(s.ok, true);
  assert.deepEqual(s.profile, { id: 'p1', name: PROFILE_NAME, active: true });
  assert.equal(s.aetherialProfiles, 1, 'another game\'s profile of the same name does not count');
  assert.deepEqual(s.activeProfile, { id: 'p1', name: PROFILE_NAME });
  const byId = Object.fromEntries(s.mods.map(m => [m.id, m]));
  assert.deepEqual([byId.ussep439c.nexusModId, byId.ussep439c.nexusFileId, byId.ussep439c.enabled], [266, 999999, true]);
  assert.equal(byId.ussep439c.installationPath, 'Unofficial Skyrim Special Edition Patch 266 4.3.9c 2026-09-05T22-22Z lNkz9LChN');
  assert.equal(byId.skyui.installationPath, 'SkyUI-12604-6-11-1778020881');
  assert.deepEqual(byId.skyui.installerChoices, { type: 'fomod', options: [] });
  assert.equal(byId.coll, undefined, 'the collection is listed apart');
  assert.deepEqual(s.collections, [{ id: 'coll', state: 'installed', enabled: true, slug: 'abc123', revision: 3 }]);
  assert.equal(JSON.stringify(t.st), t.before, 'Vortex state is untouched');
});

test('status with another profile active, or two named Aetherial Dawn, says so without switching', async () => {
  const t = setup();
  t.st.settings.profiles.activeProfileId = 'p2';
  let s = await t.call('status');
  assert.deepEqual(s.activeProfile, { id: 'p2', name: 'Default' });
  assert.equal(s.profile.active, false);
  t.st.persistent.profiles.p4 = { id: 'p4', gameId: GAME, name: PROFILE_NAME, modState: {} };
  s = await t.call('status');
  assert.equal(s.aetherialProfiles, 2);
  assert.equal(s.profile, null);
  assert.equal(t.st.settings.profiles.activeProfileId, 'p2');
});

test('the signature matches the launcher\'s (the same vector as core/src/vortex.rs)', () => {
  const body = '{"verb":"status","args":{},"revision":"r1","ts":1000000,"nonce":"0123456789abcdef0123456789abcdef"}';
  assert.equal(sign('a'.repeat(64), body), '3122895dea18179f37ba45ba8220fd633975ccaf40b2fb1a200c5422edc3ab00');
});

test('status says which extension version Vortex loaded, from its own info.json', async () => {
  const { version } = require('../info.json');
  const body = JSON.stringify({ verb: 'status', args: {}, revision: 'r1', ts: 5, nonce: 'nonce-version-0123456789' });
  const jobs = new Jobs({ state: () => state() }, { token: TOKEN, now: () => 5, version });
  const out = await jobs.handle(body, sign(TOKEN, body));
  assert.strictEqual(out.extensionVersion, version);
  const old = new Jobs({ state: () => state() }, { token: TOKEN, now: () => 5 });
  assert.strictEqual((await old.handle(body, sign(TOKEN, body))).extensionVersion, null);
});
