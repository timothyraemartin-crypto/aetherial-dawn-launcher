// Aetherial Dawn one-click bug report, client half. Drop in Data/Platform/Plugins/ad-report.js.
// Press F10 in game: writes one "[ad-report]" line (position, cell, nearby players) to the SkyrimPlatform log
// and shows a message. Every 20 s it also writes a quiet "[ad-report] crumb" line, so after a crash the
// launcher can still say where the player was and who was around. The launcher does the sending.
// Plain JS on purpose: no build step.
var sp = require('skyrimPlatform');
var HOTKEY = 0x44;        // F10 (DirectInput scan code). F3 is the F3 menu.
var CRUMB_MS = 20000;
var NEAR = 4096;          // game units, ~ 58 m
var lastCrumb = 0;
var lastPress = 0;

function nearbyPlayers(me) {
  var out = [];
  try {
    var cell = me.getParentCell();
    if (!cell) return out;
    var n = cell.getNumRefs(43); // 43 = kNPC
    for (var i = 0; i < n && i < 400; i++) {
      var ref = cell.getNthRef(i, 43);
      if (!ref) continue;
      var id = ref.getFormID();
      // SkyMP remote players are dynamic forms (0xff000000+); the local player is 0x14.
      if (id < 0xff000000 || id === 0x14) continue;
      var d = me.getDistance(ref);
      if (d > NEAR) continue;
      var a = sp.Actor.from(ref);
      out.push({ id: id.toString(16), name: a ? a.getDisplayName() : '', dist: Math.round(d) });
    }
    out.sort(function (a, b) { return a.dist - b.dist; });
  } catch (e) { /* a report is never worth an error */ }
  return out.slice(0, 20);
}

function snapshot(kind) {
  var me = sp.Game.getPlayer();
  var s = { kind: kind, at: new Date().toISOString() };
  try {
    s.pos = [Math.round(me.getPositionX()), Math.round(me.getPositionY()), Math.round(me.getPositionZ())];
    var cell = me.getParentCell();
    s.cell = cell ? cell.getFormID().toString(16) : null;
    s.cellName = cell ? cell.getName() : null;
    var ws = me.getWorldSpace();
    s.world = ws ? ws.getName() : null;
    s.name = me.getDisplayName();
    s.near = nearbyPlayers(me);
  } catch (e) { s.error = String(e); }
  return s;
}

function emit(s) {
  var line = '[ad-report] ' + JSON.stringify(s);
  try { sp.writeLogs('ad-report', line); } catch (e) { /* ignore */ }
  try { console.log(line); } catch (e) { /* ignore */ }
}

sp.on('buttonEvent', function (e) {
  if (e.code !== HOTKEY || !e.isDown) return;
  var now = Date.now();
  if (now - lastPress < 3000) return; // ignore key repeat / double taps
  lastPress = now;
  emit(snapshot('bug'));
  sp.printConsole('Report saved. Open the Aetherial Dawn launcher to send it.');
  try { sp.Debug.notification('Bug report saved. The launcher will offer to send it.'); } catch (e2) { /* ignore */ }
});

sp.on('update', function () {
  var now = Date.now();
  if (now - lastCrumb < CRUMB_MS) return;
  lastCrumb = now;
  emit(snapshot('crumb'));
});
