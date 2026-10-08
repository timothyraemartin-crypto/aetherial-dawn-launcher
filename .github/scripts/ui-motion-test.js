// Proves the launcher's enter motion: a sheet, its panel and a page animate in
// when they stop being hidden, and none of it runs when the player asks their
// system for reduced motion. Style-only, so no back end is faked: the real
// style.css is loaded in headless Chrome and computed styles are read.
//
//   node .github/scripts/ui-motion-test.js [path to chrome]
'use strict';
const fs = require('fs'), os = require('os'), path = require('path');
const { execFileSync } = require('child_process');

const UI = path.join(__dirname, '..', '..', 'ui');
const chrome = [process.argv[2], process.env.CHROME,
  'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
  'C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe',
  '/usr/bin/google-chrome', '/usr/bin/chromium', '/usr/bin/chromium-browser']
  .filter(Boolean).find(c => fs.existsSync(c));
if (!chrome) { console.error('no Chrome found; pass its path'); process.exit(1); }

// The stylesheet is inlined: a file:// link breaks when the temp folder and the repo are on different drives.
const css = fs.readFileSync(path.join(UI, 'style.css'), 'utf8');
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ad-motion-'));
const page = path.join(dir, 'page.html');
fs.writeFileSync(page, `<!doctype html><html><head><style>${css}</style></head><body>
<div class="app"><section class="page" id="pg"></section><div class="sheet" id="sh"><div class="panel" id="pn"></div></div>
<div class="sheet" id="hid" hidden><div class="panel" id="hpn"></div></div></div>
<script>
  window.addEventListener('load', () => {
    const name = id => getComputedStyle(document.getElementById(id)).animationName;
    const out = { page: name('pg'), sheet: name('sh'), panel: name('pn'), rules: document.styleSheets[0] ? document.styleSheets[0].cssRules.length : 0, hiddenSheet: getComputedStyle(document.getElementById('hid')).display };
    const pre = document.createElement('pre'); pre.id = 'r'; pre.textContent = JSON.stringify(out); document.body.appendChild(pre);
  });
</script></body></html>`);

function run(extra) {
  const out = execFileSync(chrome, ['--headless', '--no-sandbox', '--disable-gpu', '--allow-file-access-from-files',
    '--virtual-time-budget=2000', '--dump-dom', ...extra, 'file:///' + page.replace(/\\/g, '/').replace(/^\//, '')], { encoding: 'utf8' });
  return JSON.parse(/<pre id="r">(.*?)<\/pre>/s.exec(out)[1].replace(/&quot;/g, '"'));
}
const normal = run(['--force-prefers-no-reduced-motion']);
const reduced = run(['--force-prefers-reduced-motion']);

const checks = [
  ['the stylesheet loaded', normal.rules > 50],
  ['a page animates in', normal.page !== 'none'],
  ['a sheet fades in', normal.sheet !== 'none'],
  ['a sheet panel rises in', normal.panel !== 'none'],
  ['a hidden sheet stays hidden', normal.hiddenSheet === 'none'],
  ['reduced motion: page does not animate', reduced.page === 'none'],
  ['reduced motion: sheet does not animate', reduced.sheet === 'none'],
  ['reduced motion: panel does not animate', reduced.panel === 'none'],
];
let failed = 0;
// Decorative glows and strokes use the design system's values, not the older blues and gold.
for (const [old, now] of [['74,163,220', 'glow #5EA4D8'], ['169,214,242', 'frost #B6DCF7'], ['110,190,240', 'aether #8FC3EC'], ['120,180,220', 'aether #8FC3EC'], ['217,194,154', 'gold #D8B56A']]) {
  const left = (css.match(new RegExp(old.replace(/,/g, ',\\s*'), 'g')) || []).length;
  console.log(`${left === 0 ? 'ok  ' : 'FAIL'} no leftover ${old} (use ${now}): ${left}`);
  if (left) failed++;
}
for (const [name, ok] of checks) { console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}`); if (!ok) failed++; }
fs.rmSync(dir, { recursive: true, force: true });
if (failed) { console.error(`${failed} check(s) failed`, JSON.stringify({ normal, reduced })); process.exit(1); }
