# 2026-10-17 Version warning severity and server-address layout

User screenshot from installed0.1.157 showed a soft manually confirmed game-data
warning presented as a red failure, repeated above Play, and the server port
wrapped onto a second line. User is testing Skyrim; no game files, Steam metadata,
manual version marker, running launcher or services have been modified.

Live read-only evidence: executable resource1.6.1170.0 and SHA
c434208894f07f604b852f29b8edc3a58c4de63de783373733e72b2b73f33be9.
Steam InstalledDepots differs from target. A differing opaque manifest ID is not
proof the data is newer; executable version alone is also not proof all data is
compatible. Existing manual marker/fingerprint is not a trusted full-file hash
verification. Preserve the compatibility/repair gates, do not fake a verified mark.

Changed core warning text to state this uncertainty accurately, without asserting
newer files or linking it to crashes. Soft warning gets amber informational icon
and shorter nonblocking dock message; genuine failures retain red/blocked Play.
Address has enough desktop space, no wrapping, hover title and responsive fact
tiles for small windows. Long addresses can ellipsize rather than split the port.

Three headless Edge scenarios pass13checks at1360/1024px: warning notfailure,
Play available, no automatic patch, full address onone line, hardfailure wins over
simultaneous warning. Existing two wrongversion scenarios pass7checks. Initial
11coreversion tests pass with workspace TMP (sandboxTemp atomicwrite failures
resolved without gamechanges). Main-based core rerun pending current build lock.

Isolated currentmain bf904d4 in A:/Aetherial Dawn/launcher-warning-work,
branchcodex/launcher-version-warning. Avoids unrelated openPR199 in auditcheckout.
Candidate version0.1.158 (workspace+two lockentries). Windows build underway with
two jobs, cachedtarget; first sandboxbuild failed Tauri registryAPI-script access,
retrywith requiredbuildaccess. No release/install claim until actual build receipt.
Do not publish an auto-update while usertests: current0.1.157 may forceclosegame
after countdown unless serverlauncher.forceClose:false. Keep candidate staged and
coordinate installation after testing; never forceclose their game.
