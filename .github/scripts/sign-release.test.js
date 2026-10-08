'use strict';
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const yml = fs.readFileSync(path.join(__dirname, '..', 'workflows', 'sign-release.yml'), 'utf8');

// The body of a step's `run: |` block, de-indented, with the one expression this step uses filled in.
function stepScript(name) {
  const lines = yml.split('\n');
  const at = lines.findIndex((l) => l.trim() === `- name: ${name}`);
  assert.ok(at >= 0, `step "${name}" exists`);
  const run = lines.findIndex((l, i) => i > at && l.trim() === 'run: |');
  const indent = lines[run + 1].match(/^ */)[0].length;
  const body = [];
  for (let i = run + 1; i < lines.length && (lines[i].trim() === '' || lines[i].match(/^ */)[0].length >= indent); i++) body.push(lines[i].slice(indent));
  return body.join('\n').replace(/\$\{\{ steps\.rel\.outputs\.id \}\}/g, '1');
}

test('a runner that kept dist/ from an earlier run still downloads, and the old files are not signed or published', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'signrel-'));
  const bin = path.join(dir, 'bin');
  fs.mkdirSync(bin);
  // stand-in gh: the asset lookup answers with an id, the download writes the installer bytes
  fs.writeFileSync(path.join(bin, 'gh'), '#!/bin/sh\ncase "$*" in *octet-stream*) printf NEWINSTALLER;; *) echo 42;; esac\n', { mode: 0o755 });
  const work = path.join(dir, 'work');
  fs.mkdirSync(path.join(work, 'dist'), { recursive: true });
  fs.writeFileSync(path.join(work, 'dist', 'latest.json'), 'STALE');
  const r = spawnSync('bash', ['-c', stepScript('Download the unsigned installer')], {
    cwd: work, encoding: 'utf8',
    env: { ...process.env, PATH: bin + ':' + process.env.PATH, VERSION: '0.1.107', GITHUB_REPOSITORY: 'o/r' },
  });
  assert.strictEqual(r.status, 0, r.stderr);
  assert.strictEqual(fs.readFileSync(path.join(work, 'dist', 'AetherialDawn-Launcher-0.1.107-setup.exe'), 'utf8'), 'NEWINSTALLER');
  assert.ok(!fs.existsSync(path.join(work, 'dist', 'latest.json')), 'a stale latest.json from an earlier run must not survive');
});

test('dist/ is removed when the job ends, so the next run on this runner starts clean', () => {
  assert.match(yml, /if: always\(\)\s*\n\s*run: rm -rf dist/);
});
