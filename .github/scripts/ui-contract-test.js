// Checks that the launcher page (ui/app.js) and the Rust side (src-tauri/src)
// still agree on the names they share, without running either:
//   - every command the page invokes is registered in generate_handler! and
//     is a real #[tauri::command] function;
//   - every event the page listens for is emitted by Rust;
//   - every "PREFIX:" error code the page tests for is produced by Rust.
// Registered commands the page never calls, and events Rust emits that the
// page never hears, are listed as notes but do not fail the check.
//
//   node .github/scripts/ui-contract-test.js
'use strict';
const fs = require('fs'), path = require('path');

const root = path.join(__dirname, '..', '..');
const read = p => fs.readFileSync(path.join(root, p), 'utf8');
const rustFiles = fs.readdirSync(path.join(root, 'src-tauri', 'src')).filter(f => f.endsWith('.rs'));
const rust = rustFiles.map(f => read(path.join('src-tauri', 'src', f))).join('\n');
const ui = read('ui/app.js');
const problems = [], notes = [];
const uniq = a => [...new Set(a)].sort();

const handler = /generate_handler!\s*\[([^\]]*)\]/.exec(rust);
if (!handler) { console.error('generate_handler![...] not found in src-tauri/src'); process.exit(1); }
const registered = uniq(handler[1].split(',').map(s => s.trim().replace(/^.*::/, '')).filter(Boolean));
const commandFns = new Set([...rust.matchAll(/#\[tauri::command[^\]]*\]\s*(?:(?:#\[[^\]]*\]|\/\/[^\n]*)\s*)*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)/g)].map(m => m[1]));
const invoked = uniq([...ui.matchAll(/\binvoke\(\s*'([^']+)'/g)].map(m => m[1]));
const listened = uniq([...ui.matchAll(/\blisten\(\s*'([^']+)'/g)].map(m => m[1]));
const emitted = uniq([...rust.matchAll(/\.emit\(\s*"([^"]+)"/g)].map(m => m[1]));
const uiPrefixes = uniq([...ui.matchAll(/'([A-Z][A-Z_]+:)/g)].map(m => m[1]));

for (const c of invoked) if (!registered.includes(c)) problems.push(`ui/app.js invokes '${c}' but it is not in generate_handler!`);
for (const c of registered) if (!commandFns.has(c)) problems.push(`'${c}' is in generate_handler! but no #[tauri::command] fn has that name`);
for (const e of listened) if (!emitted.includes(e)) problems.push(`ui/app.js listens for '${e}' but Rust never emits it`);
for (const p of uiPrefixes) if (!rust.includes(`"${p}`)) problems.push(`ui/app.js tests for '${p}' errors but Rust never produces one`);

const unusedCmds = registered.filter(c => !invoked.includes(c));
const unheard = emitted.filter(e => !listened.includes(e));
if (unusedCmds.length) notes.push(`registered but never invoked by the page: ${unusedCmds.join(', ')}`);
if (unheard.length) notes.push(`emitted but never heard by the page: ${unheard.join(', ')}`);

console.log(`commands: ${registered.length} registered, ${invoked.length} invoked by the page`);
console.log(`events: ${emitted.length} emitted, ${listened.length} listened for`);
console.log(`error prefixes checked: ${uiPrefixes.join(' ')}`);
for (const n of notes) console.log(`note: ${n}`);
if (invoked.length < 10 || !listened.length || !uiPrefixes.length) problems.push('the page scan found suspiciously little; update this script if ui/app.js changed shape');
if (problems.length) { for (const p of problems) console.error(`FAIL: ${p}`); process.exit(1); }
console.log('ok');
