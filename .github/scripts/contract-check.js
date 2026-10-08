// Fails if the launcher's calls to the login service and contract/launcher-server.json disagree.
// Scans the Rust sources for server paths written after a base URL ("{}/api/...", "{AUTH_URL}/health"),
// then checks, both ways: every call is in the contract with the same method and query names, and every
// contract endpoint is still called. Also checks the server key. No dependencies.
//   node .github/scripts/contract-check.js [repo root]
const fs = require('node:fs');
const path = require('node:path');

const root = path.resolve(process.argv[2] || path.join(__dirname, '..', '..'));
const contract = JSON.parse(fs.readFileSync(path.join(root, 'contract', 'launcher-server.json'), 'utf8'));

// faces.rs builds its URLs from "{api}", which is "<base>/faces".
const ALIASES = { 'src-tauri/src/faces.rs': { '{api}': '/faces' } };
// Bases that are only a prefix for other calls, not a call themselves.
const BASES = new Set(['/faces']);
const VERB = /\.(get|post|put|delete|patch)\(|\b(get_json)\(/;
const ANNOTATION = /contract-method:\s*(GET|POST|PUT|DELETE|PATCH)/;

const norm = (p) => p.split('?')[0].replace(/\{[^}]*\}/g, '{}');

function* rustFiles(dir) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, e.name);
    if (e.isDirectory()) { if (e.name !== 'target') yield* rustFiles(full); } else if (e.name.endsWith('.rs')) yield full;
  }
}

function methodNear(lines, i) {
  const window = lines.slice(Math.max(0, i - 1), i + 8).join('\n');
  const note = window.match(ANNOTATION);
  if (note) return note[1];
  const m = window.match(VERB);
  if (!m) return null;
  return m[2] ? 'GET' : m[1].toUpperCase();
}

const found = []; // { file, line, path, query, method }
for (const dir of ['core/src', 'src-tauri/src']) {
  for (const file of rustFiles(path.join(root, dir))) {
    const rel = path.relative(root, file).split(path.sep).join('/');
    const lines = fs.readFileSync(file, 'utf8').split('\n');
    lines.forEach((line, i) => {
      if (/^\s*\/\//.test(line)) return;
      for (const lit of line.matchAll(/"((?:[^"\\]|\\.)*)"/g)) {
        let text = lit[1];
        for (const [from, to] of Object.entries(ALIASES[rel] || {})) text = text.replace(from, `}${to}`);
        const m = text.match(/\}(\/(?:api|health|faces)[^\s"]*)/);
        if (!m) continue;
        const [pathPart, query = ''] = m[1].split('?');
        if (BASES.has(norm(pathPart))) continue;
        found.push({ rel, line: i + 1, path: norm(pathPart), query, method: methodNear(lines, i) });
      }
    });
  }
}

const problems = [];
const byPath = new Map(contract.endpoints.map((e) => [norm(e.path), e]));
const seen = new Set();
for (const f of found) {
  const where = `${f.rel}:${f.line}`;
  const e = byPath.get(f.path);
  if (!e) { problems.push(`${where}: the launcher calls ${f.path}, which is not in contract/launcher-server.json`); continue; }
  seen.add(e.id);
  if (!e.browser) {
    if (!f.method) problems.push(`${where}: can't tell which HTTP method ${f.path} is called with; put \`// contract-method: ${e.method}\` on a line next to it`);
    else if (f.method !== e.method) problems.push(`${where}: ${f.path} is called with ${f.method}, the contract says ${e.method}`);
  }
  for (const q of e.query || []) {
    if (!new RegExp(`(^|&)${q}=`).test(f.query)) problems.push(`${where}: the call to ${f.path} has no "${q}" parameter, the contract lists it`);
  }
}
for (const e of contract.endpoints) {
  if (!seen.has(e.id)) problems.push(`contract endpoint ${e.id} (${e.method} ${e.path}) is not called anywhere in the launcher; remove it from the contract in both repos`);
}

const key = fs.readFileSync(path.join(root, 'core/src/auth.rs'), 'utf8').match(/SERVER_KEY:\s*&str\s*=\s*"([^"]*)"/);
if (!key) problems.push('core/src/auth.rs: SERVER_KEY not found');
else if (key[1] !== contract.defaultServerKey) problems.push(`SERVER_KEY is "${key[1]}", the contract says "${contract.defaultServerKey}"`);

if (problems.length) {
  console.error(`Launcher and contract disagree (${problems.length}):\n- ${problems.join('\n- ')}`);
  process.exit(1);
}
console.log(`Launcher calls match the contract: ${contract.endpoints.length} endpoints, ${found.length} call sites.`);
