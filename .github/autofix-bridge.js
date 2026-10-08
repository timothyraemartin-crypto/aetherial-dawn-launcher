'use strict';
// Bridge between auto-fix pull requests and the Discord bot, run by autofix-bridge.yml (never PR code).
//   1. A pull request labelled `auto-fix-ready` (set by the fix task once CI is green and it is checked) is
//      told to the bot once (label `fix-asked`), which pings the owner with Apply / Skip.
//   2. The owner's presses are read back. Apply: re-check the pull request, merge it with this workflow's own
//      token, start the repo's tests and deploy (a merge by the built-in token does not start them itself).
//      Skip: comment, label `fix-skipped`, drop `auto-fix-ready`.
// The bot is reached over SSH on 127.0.0.1 with the bot's INTERNAL_TOKEN read on the server, so no new secret exists.
//   env: GH_TOKEN, REPO, SSH_TARGET, DRY_RUN (optional: "true" prints and changes nothing)
const { spawnSync } = require('child_process');
const { GH_TOKEN, REPO, SSH_TARGET } = process.env;
const DRY = process.env.DRY_RUN === 'true';
const BOT = 'http://127.0.0.1:3090';
const READY = 'auto-fix-ready', ASKED = 'fix-asked', SKIPPED = 'fix-skipped';
const PROTECTED = ['.github/', 'deploy/', '.claude/', 'package.json', 'package-lock.json', 'npm-shrinkwrap.json', 'yarn.lock',
  'pnpm-lock.yaml', 'Cargo.toml', 'Cargo.lock', 'tauri.conf.json', '.npmrc', 'CLAUDE.md'];
const DEPLOY = { 'aetherial-dawn-ingame-ui': { test: 'test.yml', deploy: 'deploy-server.yml', inputs: { apply: 'true' } },
  'aetherial-dawn-discord': { test: 'test.yml', deploy: 'deploy-server.yml', inputs: { ref: 'main' } } };
const MIN_AGE_MS = 3 * 60_000;

if (!GH_TOKEN || !REPO || !SSH_TARGET) { console.error('GH_TOKEN, REPO and SSH_TARGET are required'); process.exit(1); }

const touched = (names) => names.filter((n) => PROTECTED.some((p) => (p.endsWith('/') ? n.startsWith(p) : n === p || n.endsWith(`/${p}`))));

async function gh(method, path, data) {
  const res = await fetch(`https://api.github.com/repos/${REPO}${path}`, {
    method, headers: { Authorization: `Bearer ${GH_TOKEN}`, Accept: 'application/vnd.github+json', 'Content-Type': 'application/json', 'User-Agent': 'ad-autofix-bridge' },
    body: data ? JSON.stringify(data) : undefined,
  });
  const text = await res.text();
  let j = {}; try { j = text ? JSON.parse(text) : {}; } catch { /* not json */ }
  if (!res.ok) { const e = new Error(`${method} ${path.split('?')[0]}: ${res.status} ${String(j.message || text).slice(0, 200)}`); e.status = res.status; throw e; }
  return j;
}

// One request to the bot, through SSH. The token is read and used on the server and never printed.
function bot(method, path, data) {
  const remote = `T=$(sudo -n sed -n 's/^INTERNAL_TOKEN=//p' /etc/aetherial-dawn/secrets.env | tail -1 | tr -d '"\\r'); `
    + `[ -n "$T" ] || { echo '{"error":"no_token"}'; exit 0; }; `
    + `curl -sS -m 20 -X ${method} -H "Authorization: Bearer $T" -H 'Content-Type: application/json' --data-binary @- '${BOT}${path}'`;
  const r = spawnSync('ssh', ['-o', 'BatchMode=yes', '-o', 'ConnectTimeout=15', SSH_TARGET, remote], { input: data ? JSON.stringify(data) : '{}', encoding: 'utf8', timeout: 60_000 });
  if (r.status !== 0) throw new Error(`ssh to the bot failed: ${String(r.stderr).split('\n').slice(-3).join(' ').slice(0, 200)}`);
  let j; try { j = JSON.parse(r.stdout); } catch { throw new Error(`the bot answered something unreadable on ${path}`); }
  if (j.error) throw new Error(`the bot refused ${path}: ${j.error} ${j.message || ''}`.trim());
  return j;
}

const labelsOf = (o) => (o.labels || []).map((l) => l.name);
const addLabel = (n, l) => (DRY ? null : gh('POST', `/issues/${n}/labels`, { labels: [l] }));
const removeLabel = (n, l) => (DRY ? null : gh('DELETE', `/issues/${n}/labels/${encodeURIComponent(l)}`).catch((e) => { if (e.status !== 404) throw e; }));

async function propose() {
  const list = await gh('GET', `/issues?labels=${READY}&state=open&per_page=30`);
  for (const it of list.filter((x) => x.pull_request && !labelsOf(x).includes(ASKED) && !labelsOf(x).includes(SKIPPED))) {
    const pull = await gh('GET', `/pulls/${it.number}`);
    if (pull.merged || pull.state !== 'open') continue;
    const files = (await gh('GET', `/pulls/${it.number}/files?per_page=100`)).map((f) => f.filename);
    const m = /\b(?:fixes|closes)\s+#(\d+)/i.exec(pull.body || '');
    const issue = m ? await gh('GET', `/issues/${m[1]}`).catch(() => null) : null;
    const summary = String(pull.body || '').replace(/<!--[\s\S]*?-->/g, '').replace(/^_Requested by.*$/m, '').replace(/🤖.*$/gm, '').replace(/^https:\/\/claude\.ai\/code\/\S+$/gm, '').trim();
    const payload = { repo: REPO, number: pull.number, sha: pull.head.sha, title: pull.title, summary, issueTitle: issue?.title, issueUrl: issue?.html_url, prUrl: pull.html_url, files, protected: touched(files) };
    console.log(`propose ${REPO}#${pull.number} at ${pull.head.sha.slice(0, 7)}`);
    if (DRY) continue;
    bot('POST', '/internal/autofix/propose', payload);
    await addLabel(it.number, ASKED);
  }
}

async function gate(pull, files) {
  if (pull.merged) return 'It is already merged.';
  if (pull.state !== 'open') return 'The pull request is no longer open.';
  if (pull.head.repo?.full_name !== pull.base.repo?.full_name) return 'The fix comes from another repository.';
  const bad = touched(files.map((f) => f.filename).concat(files.map((f) => f.previous_filename).filter(Boolean)));
  if (bad.length) return `It changes ${bad.join(', ')}, which is not merged automatically. Read it on GitHub.`;
  if (files.length >= 100) return 'It changes 100 or more files.';
  const runs = (await gh('GET', `/commits/${pull.head.sha}/check-runs?per_page=100`)).check_runs || [];
  if (!runs.length) return 'There are no CI results for the fix (no CI results means no merge).';
  if (runs.some((c) => c.status !== 'completed')) return 'CI is still running.';
  if (runs.some((c) => !['success', 'neutral', 'skipped'].includes(c.conclusion))) return 'CI failed on the fix.';
  const st = await gh('GET', `/commits/${pull.head.sha}/status`);
  if (st.total_count > 0 && st.state !== 'success') return 'A commit status is not green.';
  const c = await gh('GET', `/commits/${pull.head.sha}`);
  if (Date.now() - Date.parse(c.commit?.committer?.date) < MIN_AGE_MS) return 'The fix was pushed minutes ago and CI may not have started everything.';
  return null;
}

async function decide() {
  const { decisions } = bot('GET', '/internal/autofix/decisions');
  for (const d of decisions.filter((x) => x.repo === REPO)) {
    const report = (result, note) => (DRY ? console.log(`${d.key}: ${result} ${note || ''}`) : bot('POST', '/internal/autofix/done', { key: d.key, result, note }));
    try {
      if (d.decision === 'skip') {
        if (!DRY) {
          await gh('POST', `/issues/${d.number}/comments`, { body: 'Skipped from Discord. Left open; nothing was merged.\n\n---\n_Generated by [Claude Code](https://claude.ai/code)_' });
          await addLabel(d.number, SKIPPED);
          await removeLabel(d.number, READY);
        }
        report('skipped');
        continue;
      }
      const pull = await gh('GET', `/pulls/${d.number}`);
      if (!pull.head.sha.toLowerCase().startsWith(d.sha.toLowerCase())) { report('refused', 'The pull request changed after you were asked. A new question will follow.'); if (!DRY) { await removeLabel(d.number, ASKED); } continue; }
      const files = await gh('GET', `/pulls/${d.number}/files?per_page=100`);
      const why = await gate(pull, files);
      if (why) { report('refused', why); continue; }
      if (DRY) { console.log(`${d.key}: would merge ${pull.head.sha.slice(0, 7)}`); continue; }
      if (pull.draft) {
        await fetch('https://api.github.com/graphql', { method: 'POST', headers: { Authorization: `Bearer ${GH_TOKEN}`, 'Content-Type': 'application/json', 'User-Agent': 'ad-autofix-bridge' },
          body: JSON.stringify({ query: 'mutation($id:ID!){markPullRequestReadyForReview(input:{pullRequestId:$id}){pullRequest{isDraft}}}', variables: { id: pull.node_id } }) });
      }
      await gh('PUT', `/pulls/${d.number}/merge`, { sha: pull.head.sha, merge_method: 'merge' });
      await removeLabel(d.number, READY);
      const flow = DEPLOY[REPO.split('/')[1]];
      let started = 'It ships with the next launcher release.';
      if (flow) {
        // A merge made with the built-in token starts no workflows, so start the tests and the deploy by hand.
        await gh('POST', `/actions/workflows/${flow.test}/dispatches`, { ref: 'main' }).catch((e) => console.error(`tests: ${e.message}`));
        await gh('POST', `/actions/workflows/${flow.deploy}/dispatches`, { ref: 'main', inputs: flow.inputs }).then(() => { started = 'The deploy was started.'; }).catch((e) => { started = `The deploy could not be started (${e.message}).`; });
      }
      report('merged', started);
    } catch (e) {
      console.error(`${d.key}: ${e.message}`);
      try { report('refused', `I could not merge it: ${e.message}`.slice(0, 280)); } catch { /* the bot is the problem; the next run tries again */ }
    }
  }
}

(async () => { await propose(); await decide(); })().catch((e) => { console.error(e.message); process.exit(1); });
