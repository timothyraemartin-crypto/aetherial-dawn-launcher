'use strict';
const test = require('node:test');
const assert = require('node:assert');
const { collect, render, staffMessage } = require('./patch-notes.js');

const c = (subject, files, body = '') => ({ sha: 'abc1234', subject, body, files: files.map((file) => ({ status: 'M', file })) });

// Real launcher history (merge commits carry the PR title in the body; 0.1.x commits are titled by version).
test('real launcher history: merges and version commits become player wording, internal ones are left out', () => {
  const r = collect([
    c('Merge pull request #54 from timothyraemartin-crypto/claude/fix-plugins-txt-data-loss-24p3y7', ['core/src/loadorder.rs', 'core/tests/x.rs'], 'Launcher: plugins.txt is never emptied when a write fails'),
    c('Merge pull request #55 from timothyraemartin-crypto/claude/fix-play-stall-q87nj5', ['ui/app.js', 'core/src/game.rs'], 'Play no longer stalls on the status line'),
    c('0.1.87: the Nexus copy-key sign-in stops watching the clipboard as soon as the mods sheet is left', ['ui/app.js']),
    c('Add skymp-server Claude Code skill', ['.claude/skills/skymp-server/SKILL.md']),
    c('ci: cache cargo (#70)', ['.github/workflows/build.yml']),
    c('Vortex extension: mods the server does not list are never touched (#57)', ['vortex-extension/src/index.js']),
  ]);
  assert.deepStrictEqual(r.groups.map((g) => g.name), ['Launcher', 'Mods and Vortex']);
  assert.deepStrictEqual(r.groups[0].items, [
    'Plugins.txt is never emptied when a write fails',
    'Play no longer stalls on the status line',
    'The Nexus copy-key sign-in stops watching the clipboard as soon as the mods sheet is left',
  ]);
  assert.strictEqual(r.skipped, 2);
});

test('a title with commas stays one bullet', () => {
  const r = collect([c('Launcher: bzip2, for archive entries, and more (#5)', ['core/src/bsa.rs'])]);
  assert.deepStrictEqual(r.groups[0].items, ['Bzip2, for archive entries, and more']);
});

test('a sentence before the colon is not mistaken for an area prefix', () => {
  const r = collect([c('Harden the sign-in check: request timeouts, no overlapping ticks (#10)', ['core/src/auth.rs'])]);
  assert.deepStrictEqual(r.groups[0].items, ['Harden the sign-in check: request timeouts, no overlapping ticks']);
});

test('PR description "Patch note:" line wins; none hides the PR', () => {
  const prs = { 58: { title: 'x (#58)', body: 'Patch note: Mods with a download link in the list are no longer installed from it.' }, 59: { title: 'y (#59)', body: 'Patch note: none' } };
  const r = collect([c('Merge pull request #58 from a/b', ['core/src/modlist.rs']), c('Merge pull request #59 from a/c', ['core/src/x.rs'])], prs);
  assert.deepStrictEqual(r.groups[0].items, ['Mods with a download link in the list are no longer installed from it']);
  assert.strictEqual(r.count, 1);
});

test('release post is a titled embed that cannot ping anyone; empty range posts nothing', () => {
  const p = render(collect([c('Launcher: Play is faster (#1)', ['core/src/game.rs'])]), { title: 'Launcher 0.1.104' });
  assert.strictEqual(p.embeds[0].title, 'Launcher 0.1.104');
  assert.deepStrictEqual(p.allowed_mentions, { parse: [] });
  assert.strictEqual(render(collect([c('docs: readme (#2)', ['README.md'])])), null);
  assert.match(staffMessage('released', { repo: 'o/r', to: 'v0.1.104' }).content, /Release published/);
});
