// Aetherial Dawn: writes the names of the plugins the game really loaded, once per session, and nothing else.
// Output: "[ad-modlist] <index> <name>" lines in the Skyrim Platform log files (writeLogs 'ad-modlist') and the console.
// Plain JS on purpose; uses the global skyrimPlatform (require() fails here). Every game call is guarded.
var sp = (typeof skyrimPlatform !== 'undefined') ? skyrimPlatform : null;
var done = false;
try {
  if (sp) {
    sp.on('update', function () {
      if (done) return;
      done = true;
      try {
        var n = sp.Game.getModCount();
        var out = ['[ad-modlist] ' + new Date().toISOString() + ' count=' + n];
        for (var i = 0; i < n; i++) out.push('[ad-modlist] ' + i + ' ' + sp.Game.getModName(i));
        for (var j = 0; j < out.length; j++) {
          try { sp.writeLogs('ad-modlist', out[j]); } catch (e1) { /* ignore */ }
          try { console.log(out[j]); } catch (e2) { /* ignore */ }
        }
      } catch (e) { /* never break the other plugins */ }
    });
  }
} catch (e) { /* never break the other plugins */ }
