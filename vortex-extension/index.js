// Aetherial Dawn for Vortex: lets the Aetherial Dawn launcher install,
// switch on and deploy the server's mods in the "Aetherial Dawn" profile
// through Vortex's own extension API (Package C, PR #7). Listens on
// 127.0.0.1 only; every request must be signed with the token the launcher
// keeps in this Windows user's local app data. See jobs.js for the rules.
//
// Vortex 2.7.1 API used (checked at tag v2.7.1, c8ea03d):
//   events 'start-install' (archivePath, cb(err, modId)) and 'deploy-mods'
//   (cb(err)); actions.setModEnabled(profileId, modId, enabled) and
//   actions.setModAttribute(gameId, modId, key, value); api.getState().

'use strict';

const http = require('http');
const fs = require('fs');
const path = require('path');
const { actions } = require('vortex-api');
const { Jobs, GAME } = require('./jobs');

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
  const vortex = {
    state: () => api.getState(),
    dispatch: a => api.store.dispatch(a),
    actions,
    install: archive => new Promise((resolve, reject) => {
      api.events.emit('start-install', archive, (err, modId) => (err ? reject(err) : resolve(modId)));
    }),
    deploy: () => new Promise((resolve, reject) => {
      api.events.emit('deploy-mods', err => (err ? reject(err) : resolve()));
    }),
    setAttribute: (modId, key, value) => api.store.dispatch(actions.setModAttribute(GAME, modId, key, value)),
  };
  const jobs = new Jobs(vortex, {
    token,
    downloads: path.join(HOME, '..', 'vortex-downloads'),
    journalPath: path.join(HOME, 'journal.json'),
  });
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
