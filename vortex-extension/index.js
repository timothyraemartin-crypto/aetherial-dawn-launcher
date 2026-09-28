// Aetherial Dawn for Vortex: tells the Aetherial Dawn launcher, read-only,
// what Vortex holds (profiles, mods, the Aetherial Dawn collection), so the
// launcher can show exact counts and decide when Play is ready. Vortex
// installs the collection itself (Package C re-scope, PR #7 5876088533).
// Listens on 127.0.0.1 only; every request must be signed with the token the
// launcher keeps in this Windows user's local app data. See jobs.js.
//
// Vortex 2.7.1 API used: api.getState() only.

'use strict';

const http = require('http');
const fs = require('fs');
const path = require('path');
const { Jobs } = require('./jobs');

const HOME = path.join(process.env.LOCALAPPDATA || '', 'gg.aetherialdawn.launcher', 'vortex');

function start(api) {
  let token;
  try {
    token = fs.readFileSync(path.join(HOME, 'token'), 'utf8').trim();
  } catch {
    // The launcher hasn't paired yet: nothing listens.
    return;
  }
  if (!/^[0-9a-f]{64}$/.test(token)) return;
  const jobs = new Jobs({ state: () => api.getState() }, { token });
  const server = http.createServer((req, res) => {
    if (req.method !== 'POST' || req.url !== '/job') {
      res.writeHead(404).end();
      return;
    }
    let raw = '';
    req.setEncoding('utf8');
    req.on('data', c => {
      raw += c;
      if (raw.length > 1 << 20) req.destroy();
    });
    req.on('end', async () => {
      const out = await jobs.handle(raw, req.headers['x-ad-sig']);
      res.writeHead(out.ok ? 200 : 400, { 'Content-Type': 'application/json' }).end(JSON.stringify(out));
    });
  });
  server.listen(0, '127.0.0.1', () => {
    const tmp = path.join(HOME, 'port.part');
    fs.writeFileSync(tmp, JSON.stringify({ port: server.address().port, pid: process.pid }));
    fs.renameSync(tmp, path.join(HOME, 'port'));
  });
}

function init(context) {
  context.once(() => start(context.api));
  return true;
}

module.exports = { default: init };
