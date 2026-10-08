'use strict';
// Open (or update) one GitHub issue per error, so the same error never makes a second open issue.
//   GH_TOKEN=... REPO=owner/name KEY=<stable id> TITLE=... BODY_FILE=path node .github/auto-issue.js
// The issue carries the `auto-error` label and a hidden marker with KEY. An open issue with that marker gets
// a comment (at most one every 6 hours); none open makes a new one, so a fix that regressed is reported again.
const fs = require('fs');
const { GH_TOKEN, REPO, KEY, TITLE, BODY_FILE } = process.env;
if (!GH_TOKEN || !REPO || !KEY || !TITLE) { console.error('GH_TOKEN, REPO, KEY and TITLE are required'); process.exit(1); }
const marker = `<!-- auto-error:${KEY} -->`;
const body = (BODY_FILE ? fs.readFileSync(BODY_FILE, 'utf8') : '').slice(0, 60000);
const api = async (method, path, data) => {
  const res = await fetch(`https://api.github.com/repos/${REPO}${path}`, {
    method, headers: { Authorization: `Bearer ${GH_TOKEN}`, Accept: 'application/vnd.github+json', 'Content-Type': 'application/json', 'User-Agent': 'ad-auto-issue' },
    body: data ? JSON.stringify(data) : undefined,
  });
  const text = await res.text();
  if (!res.ok && res.status !== 422) throw new Error(`${method} ${path}: ${res.status} ${text.slice(0, 200)}`);
  return text ? JSON.parse(text) : {};
};
(async () => {
  await api('POST', '/labels', { name: 'auto-error', color: 'b60205', description: 'Captured automatically; a fix task picks it up' }); // 422 when it exists
  const open = await api('GET', '/issues?labels=auto-error&state=open&per_page=100');
  const hit = (Array.isArray(open) ? open : []).find((i) => !i.pull_request && (i.body || '').includes(marker));
  if (hit) {
    if (Date.now() - Date.parse(hit.updated_at) < 6 * 3600 * 1000) { console.log(`#${hit.number} already open and recent; nothing added`); return; }
    await api('POST', `/issues/${hit.number}/comments`, { body: `Still happening.\n\n${body}`.slice(0, 60000) });
    console.log(`commented on #${hit.number}`);
    return;
  }
  const made = await api('POST', '/issues', { title: TITLE.slice(0, 200), body: `${marker}\n${body}\n\n_Captured automatically. A fix task finds the cause and opens a draft PR._`, labels: ['auto-error'] });
  console.log(`opened #${made.number}`);
})().catch((e) => { console.error(e.message); process.exit(1); });
