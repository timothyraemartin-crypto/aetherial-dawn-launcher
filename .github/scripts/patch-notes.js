#!/usr/bin/env node
'use strict';
// Player patch notes and the staff live feed, posted to Discord with webhooks.
//   node tools/patch-notes.js notes --from <ref> --to <ref>     print the notes (change nothing)
//   node tools/patch-notes.js post  --from <ref> --to <ref>     post them to AD_PATCHNOTES_WEBHOOK
//   node tools/patch-notes.js staff --event started|succeeded|failed --from <ref> --to <ref> [--detail text]
//                                                               post a line to AD_STAFF_WEBHOOK
// Options: --title <text>  --json (print the webhook payload)  --limit <n> (no range start: last n merges)
// One copy lives in each of ingame-ui (tools/) and launcher (.github/scripts/); keep them identical.
// Nothing here may fail a deploy or a release: every posting problem is printed and exits 0.
//
// Where the player wording comes from, best first:
//   1. A line in the PR description:  `Patch note: Guards no longer re-set a bounty right after it is paid.`
//      Several lines make several bullets. `Patch note (Trading): ...` picks the group.
//      `Patch note: none` keeps a PR out of the notes.
//   2. Otherwise the PR title, cleaned up (version prefix, audit codes and "(#12)" removed).
// Left out: tests, CI, docs, tools, memory/hooks, and PRs whose files the deploy would not upload.
const { execFileSync } = require('child_process');

// ---- grouping ----------------------------------------------------------------------------------
const GROUPS = [
  'F3 menu', 'Holds, homes and bounties', 'Anti-cheat', 'Skills and crafting', 'Trading',
  'Staff tools and tickets', 'Looks and faces', 'Game master benches', 'Chat and roleplay', 'World and server',
  'Launcher', 'Mods and Vortex', 'Other fixes',
];
const PREFIX_GROUP = [
  [/^(f3|ad-ui|menu)$/i, 'F3 menu'], [/^(holds?|homes?|bounty|jail|guilds?)$/i, 'Holds, homes and bounties'],
  [/^(anti-?cheat|watch)$/i, 'Anti-cheat'], [/^(skills?|crafting|leather)$/i, 'Skills and crafting'],
  [/^(trad(e|ing))$/i, 'Trading'], [/^(staff|staffops|tickets?)$/i, 'Staff tools and tickets'],
  [/^(faces?|looks?|appearance)$/i, 'Looks and faces'], [/^(gmbench|gm ?benches)$/i, 'Game master benches'],
  [/^(chat|roleplay|role-play|couriers?)$/i, 'Chat and roleplay'], [/^(sync|fx|world|health-?watch|plugin-?scan)$/i, 'World and server'],
  [/^(launcher|\d+\.\d+\.\d+)$/i, 'Launcher'], [/^(vortex|mods?|nexus)$/i, 'Mods and Vortex'],
];
const PATH_GROUP = [
  [/^(ui|gamemode)\/ad-ui\.|^gamemode\/ad-ui/, 'F3 menu'], [/^(holds|bounty)\//, 'Holds, homes and bounties'],
  [/^anticheat\//, 'Anti-cheat'], [/^skills\//, 'Skills and crafting'], [/^(trade\/|ui\/ad-trade)/, 'Trading'],
  [/^(staffops|tickets)\//, 'Staff tools and tickets'], [/^(faces|looks)\//, 'Looks and faces'],
  [/^gmbench\//, 'Game master benches'], [/^chat\//, 'Chat and roleplay'], [/^(sync|fx|health)\//, 'World and server'],
  [/^vortex-extension\//, 'Mods and Vortex'], [/^(core|src-tauri)\/|^ui\/(app\.js|index\.html|style\.css|ambient\.js|art|fonts)/, 'Launcher'],
];
// Prefixes that mark a change players never see.
const INTERNAL_PREFIX = /^(tests?|ci|docs?|chore|refactor|build|tools?|hooks?|memory|progress|readme|deploy|workflow|lint|merge)$/i;
// Files that never reach a player.
const NON_PLAYER = [/^(test|tools|harness|docs|deploy|\.github|\.claude)\//, /(^|\/)(tools|integration|test|tests)\//,
  /\.(md|patch|test\.js)$/, /^(package(-lock)?\.json|\.gitignore|PROGRESS\.md|CLAUDE\.md|Cargo\.lock)$/];

// ---- wording -----------------------------------------------------------------------------------
const CODE = /\((?:[^()]*\b(?:audit|QA|red team|finding|fix-list|triple-check)\b[^()]*|[A-Z]{1,3}\d+(?:[-, ]+[A-Z]{0,3}\d+)*|#\d+|draft)\)/gi;

function splitTitle(title) {
  let t = String(title || '').replace(/\s*\(#\d+\)\s*$/, '').trim();
  let prefix = '';
  const m = t.match(/^([A-Za-z][A-Za-z0-9 ./_-]{0,22}?|\d+\.\d+\.\d+)(?:\s+v?\d+(?:\.\d+){1,3})?:\s+(.+)$/);
  if (m) { prefix = m[1].trim(); t = m[2]; }
  return { prefix, text: t };
}

function sentence(s) {
  s = s.replace(CODE, '').replace(/\s{2,}/g, ' ').replace(/\s+([,.;])/g, '$1').trim().replace(/[.;,]+$/, '');
  if (!s) return '';
  return s[0].toUpperCase() + s.slice(1);
}

function groupFor(prefix, files) {
  for (const [re, g] of PREFIX_GROUP) if (prefix && re.test(prefix)) return g;
  const tally = new Map();
  for (const f of files) for (const [re, g] of PATH_GROUP) if (re.test(f)) { tally.set(g, (tally.get(g) || 0) + 1); break; }
  let best = 'Other fixes', n = 0;
  for (const [g, c] of tally) if (c > n) { best = g; n = c; }
  return best;
}

function groupByName(name) {
  const hit = GROUPS.find((g) => g.toLowerCase() === String(name).trim().toLowerCase());
  if (hit) return hit;
  for (const [re, g] of PREFIX_GROUP) if (re.test(String(name).trim())) return g;
  return null;
}

// "Patch note (Trading): text" lines in a PR description. null = none written; [] = "none" (skip the PR).
function noteLines(body) {
  const out = []; let any = false;
  for (const line of String(body || '').split(/\r?\n/)) {
    const m = line.match(/^\s*(?:[-*]\s*)?patch notes?\s*(?:\(([^)]+)\))?\s*:\s*(.+?)\s*$/i);
    if (!m) continue;
    any = true;
    if (/^(none|skip|internal|n\/a)\.?$/i.test(m[2])) continue;
    out.push({ group: m[1] ? groupByName(m[1]) : null, text: sentence(m[2]) });
  }
  return any ? out : null;
}

// "a; b" is two items; "a, b, c, d" (3+ commas) is a list of items, but a sentence with one comma stays whole.
function titleItems(text) {
  const out = [];
  for (const part of text.split(/;\s+/)) {
    const bits = part.split(/,\s+/);
    const list = bits.length >= 3 && !bits.some((b) => /^(and|or|so|but|then|because)\b/i.test(b));
    for (const b of list ? bits : [part]) out.push({ group: null, text: sentence(b) });
  }
  return out;
}

function prNumber(subject) {
  const m = subject.match(/\(#(\d+)\)\s*$/) || subject.match(/^Merge pull request #(\d+)\b/);
  return m ? Number(m[1]) : null;
}

// commits: [{sha, subject, body, files:[{status, file}]}]; prs: {number: {title, body}}
// opts.live(files) -> bool says whether a change reaches players (default: any file players could see).
function collect(commits, prs = {}, opts = {}) {
  const live = opts.live || ((files) => files.some((f) => !NON_PLAYER.some((r) => r.test(f.file))));
  const groups = new Map(); const seen = new Set(); let skipped = 0;
  for (const c of commits) {
    const n = prNumber(c.subject);
    const pr = (n && prs[n]) || {};
    const isMerge = /^Merge pull request/.test(c.subject);
    const title = pr.title || (isMerge ? String(c.body || '').split(/\r?\n/).find((l) => l.trim()) || '' : c.subject);
    const body = pr.body != null ? pr.body : c.body;
    const { prefix, text } = splitTitle(title);
    const filesNames = c.files.map((f) => f.file);
    const written = noteLines(body);
    if (!title || (written && !written.length) || (!written && INTERNAL_PREFIX.test(prefix)) || !live(c.files)) { skipped++; continue; }
    const items = written || titleItems(text);
    for (const it of items) {
      if (!it.text) continue;
      const g = it.group || groupFor(prefix, filesNames);
      const key = g + '|' + it.text.toLowerCase();
      if (seen.has(key)) continue;
      seen.add(key);
      if (!groups.has(g)) groups.set(g, []);
      groups.get(g).push(it.text);
    }
  }
  const ordered = GROUPS.filter((g) => groups.has(g)).map((g) => ({ name: g, items: groups.get(g) }));
  return { groups: ordered, count: ordered.reduce((a, g) => a + g.items.length, 0), skipped };
}

// ---- Discord payloads --------------------------------------------------------------------------
const MAX_DESC = 3900;
function dateLabel(d = new Date()) {
  return d.getUTCDate() + ' ' + ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'][d.getUTCMonth()] + ' ' + d.getUTCFullYear();
}

function render(result, { title, now = new Date() } = {}) {
  if (!result.count) return null;
  const lines = []; let used = 0, shown = 0;
  outer: for (const g of result.groups) {
    const head = '**' + g.name + '**';
    const block = [head];
    for (const it of g.items) {
      const line = '• ' + it;
      if (used + head.length + line.length + 40 > MAX_DESC) { if (block.length > 1) lines.push(block.join('\n')); break outer; }
      block.push(line); used += line.length + 1; shown++;
    }
    lines.push(block.join('\n')); used += head.length + 2;
  }
  let description = lines.join('\n\n');
  if (shown < result.count) description += '\n\n…and ' + (result.count - shown) + ' more fixes.';
  return {
    username: 'Aetherial Dawn',
    allowed_mentions: { parse: [] }, // PR text can never ping anyone
    embeds: [{ title: title || 'Patch notes · ' + dateLabel(now), description, color: 0xc9a227, timestamp: now.toISOString() }],
  };
}

const EVENTS = {
  started: ['🟡', 'Deploy started'], succeeded: ['🟢', 'Deploy succeeded'], failed: ['🔴', 'Deploy FAILED'],
  released: ['🟢', 'Release published'], 'release-failed': ['🔴', 'Release FAILED'],
};
// ctx: {repo, from, to, runUrl, copy, held, manual, detail}
function staffMessage(event, ctx = {}) {
  const [icon, label] = EVENTS[event] || ['ℹ️', event];
  const lines = ['**' + icon + ' ' + label + '** · ' + (ctx.repo || 'aetherial-dawn')];
  if (ctx.to) lines.push('Range: `' + String(ctx.from || 'start').slice(0, 9) + '..' + String(ctx.to).slice(0, 9) + '`');
  if (typeof ctx.copy === 'number') lines.push(ctx.copy + ' file' + (ctx.copy === 1 ? '' : 's') + ' to upload');
  if (ctx.held && ctx.held.length) lines.push('Held back (not uploaded): ' + ctx.held.map((f) => '`' + f + '`').join(', '));
  if (ctx.manual && ctx.manual.length) lines.push('Needs a hand merge: ' + ctx.manual.map((f) => '`' + f + '`').join(', '));
  if (ctx.detail) lines.push(String(ctx.detail).slice(0, 500));
  if (ctx.runUrl) lines.push(ctx.runUrl);
  return { username: 'Aetherial Dawn deploys', allowed_mentions: { parse: [] }, content: lines.join('\n').slice(0, 1900) };
}

// ---- posting -----------------------------------------------------------------------------------
const HOOK = /^https:\/\/(?:ptb\.|canary\.)?discord(?:app)?\.com\/api\/webhooks\/\d+\/[\w-]+$/;
async function postWebhook(url, payload, fetchFn = fetch, sleep = (ms) => new Promise((r) => setTimeout(r, ms))) {
  if (!url) return { posted: false, why: 'no webhook set' };
  if (!HOOK.test(url.trim())) return { posted: false, why: 'the webhook is not a Discord webhook URL' };
  for (let attempt = 1; attempt <= 4; attempt++) {
    let res;
    try { res = await fetchFn(url.trim(), { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(payload) }); }
    catch (e) { if (attempt === 4) return { posted: false, why: 'network error: ' + e.message }; await sleep(2000 * attempt); continue; }
    if (res.ok) return { posted: true };
    if (res.status === 429 || res.status >= 500) {
      let wait = 2000 * attempt;
      try { const j = await res.json(); if (j && j.retry_after) wait = Math.min(15000, Math.ceil(j.retry_after * 1000)); } catch { /* keep default */ }
      if (attempt === 4) return { posted: false, why: 'Discord answered ' + res.status };
      await sleep(wait); continue;
    }
    return { posted: false, why: 'Discord answered ' + res.status };
  }
  return { posted: false, why: 'gave up' };
}

// ---- git and GitHub ----------------------------------------------------------------------------
const git = (args) => execFileSync('git', args, { encoding: 'utf8', maxBuffer: 1 << 28 });
function commitsBetween(from, to, limit = 30) {
  const args = ['log', '--first-parent', '--format=%H%x1f%s%x1f%b%x1e'];
  if (from) args.push(from + '..' + to); else args.push('-n', String(limit), to);
  return git(args).split('\x1e').map((r) => r.replace(/^\n/, '')).filter((r) => r.trim()).map((r) => {
    const [sha, subject, body] = r.split('\x1f');
    let out = '';
    try { out = git(['diff', '--name-status', sha + '^1', sha]); } catch { out = ''; }
    const files = out.split('\n').filter(Boolean).map((l) => { const p = l.split('\t'); return { status: p[0][0], file: p[p.length - 1] }; });
    return { sha, subject, body: (body || '').trim(), files };
  }).reverse();
}

async function fetchPr(n, env = process.env, fetchFn = fetch) {
  if (!env.GH_TOKEN || !env.GITHUB_REPOSITORY) return null;
  try {
    const res = await fetchFn('https://api.github.com/repos/' + env.GITHUB_REPOSITORY + '/pulls/' + n,
      { headers: { authorization: 'Bearer ' + env.GH_TOKEN, accept: 'application/vnd.github+json', 'user-agent': 'aetherial-dawn-patch-notes' } });
    if (!res.ok) return null;
    const j = await res.json();
    return { title: j.title, body: j.body || '' };
  } catch { return null; }
}

function deployFilter() {
  try {
    const { plan, env } = require('../deploy/deploy-fix.js');
    const cfg = env();
    return (files) => plan(files, cfg).copy.length > 0;
  } catch { return null; }
}

function deployPlan(from, to) {
  try {
    const { plan, env } = require('../deploy/deploy-fix.js');
    const files = [];
    for (const c of commitsBetween(from, to, 500)) for (const f of c.files) files.push(f);
    const last = new Map(); for (const f of files) last.set(f.file, f);
    const p = plan([...last.values()], env());
    return { copy: p.copy.length, held: p.manual.filter((m) => /^HELD/.test(m.why)).map((m) => m.file), manual: p.manual.filter((m) => !/^HELD/.test(m.why)).map((m) => m.file) };
  } catch { return {}; }
}

// ---- CLI ---------------------------------------------------------------------------------------
function args(argv) {
  const a = { _: [] };
  for (let i = 0; i < argv.length; i++) {
    const k = argv[i];
    if (k === '--json') a.json = true;
    else if (['--from', '--to', '--title', '--event', '--detail', '--limit'].includes(k)) a[k.slice(2)] = argv[++i];
    else a._.push(k);
  }
  return a;
}

async function main(argv, env = process.env) {
  const a = args(argv); const cmd = a._[0];
  const to = a.to || 'HEAD';
  const runUrl = env.GITHUB_RUN_ID && env.GITHUB_REPOSITORY ? (env.GITHUB_SERVER_URL || 'https://github.com') + '/' + env.GITHUB_REPOSITORY + '/actions/runs/' + env.GITHUB_RUN_ID : '';
  try {
    if (cmd === 'notes' || cmd === 'post') {
      const commits = commitsBetween(a.from, to, Number(a.limit) || 30);
      const prs = {};
      for (const c of commits) { const n = prNumber(c.subject); if (n) { const p = await fetchPr(n, env); if (p) prs[n] = p; } }
      const result = collect(commits, prs, { live: deployFilter() || undefined });
      const payload = render(result, { title: a.title });
      if (!payload) { console.log('No player-facing changes in this range (' + result.skipped + ' left out). Nothing to post.'); return 0; }
      if (cmd === 'notes') { console.log(a.json ? JSON.stringify(payload, null, 2) : payload.embeds[0].title + '\n\n' + payload.embeds[0].description); return 0; }
      const r = await postWebhook(env.AD_PATCHNOTES_WEBHOOK, payload);
      console.log(r.posted ? 'Patch notes posted (' + result.count + ' items).' : 'Patch notes not posted: ' + r.why + '.');
      return 0;
    }
    if (cmd === 'staff') {
      const ctx = { repo: env.GITHUB_REPOSITORY, from: a.from, to: a.to, runUrl, detail: a.detail, ...(a.to ? deployPlan(a.from, a.to) : {}) };
      const r = await postWebhook(env.AD_STAFF_WEBHOOK, staffMessage(a.event, ctx));
      console.log(r.posted ? 'Staff feed: ' + a.event + ' posted.' : 'Staff feed not posted: ' + r.why + '.');
      return 0;
    }
    console.error('usage: patch-notes.js notes|post|staff --from <ref> --to <ref> [--event e] [--title t]');
    return 2;
  } catch (e) {
    console.log('Discord notice skipped: ' + e.message); // never fail a deploy or a release over a notice
    return 0;
  }
}

module.exports = { collect, render, staffMessage, postWebhook, noteLines, splitTitle, sentence, prNumber, fetchPr, groupFor, GROUPS, HOOK };
if (require.main === module) main(process.argv.slice(2)).then((c) => process.exit(c));
