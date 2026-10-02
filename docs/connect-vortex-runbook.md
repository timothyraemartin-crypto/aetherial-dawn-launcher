# Connect Vortex test on Timothy's PC (disposable profile only)

**Status: HELD. Do not start yet.** This runs with Timothy's end-of-work PC checks, and only when all of these are true:

1. Quality checks has approved launcher PR #27. It approved `47e05a3` on 2026-09-30.
2. The test launcher build from PR #27 is green in Windows CI.
3. The Mods chat's disposable profile `AD test` exists, made with steps 1 to 3 of section 13 of `world-mods/target/INSTALL-PLAN.md`. Those steps include the fresh Vortex backup.

This test checks one thing: that the launcher can put its read-only helper into Vortex and read Vortex's state back. It installs, enables, deploys and removes no mods. **The whole test runs with `AD test` active.** It never switches to, or changes, the real "Aetherial Dawn" profile. Switching back is not part of this test: it is section 13 steps 5 and 6 of the install plan, which include the inventory check. The helper answers only "what is installed", and nothing else.

What it proves, which fixture tests can't:
- Vortex 2.7.1 keeps user extensions in `%APPDATA%\Vortex\plugins`.
- Vortex 2.7.1 loads helper 0.2.1.
- The helper reports the active profile without switching it.
- The launcher shows the right line when a profile other than "Aetherial Dawn" is active, and never says Ready.

---

## Part A: before you start

1. **Skyrim:** close Skyrim if it is running.
2. **Aetherial Dawn Launcher window:** close the launcher.
3. **Vortex window:** open **Profiles**. Check that the active marker is beside **AD test**. If it isn't, stop here: `AD test` is made and switched on only by the Mods chat's section 13 steps, not by this test.
4. **Vortex window:** wait until nothing is downloading or deploying. The bar at the bottom should be empty.
5. **Vortex window:** take a screenshot of **Profiles** that shows **AD test** active. This is screenshot 1.

## Part B: check where Vortex keeps extensions

6. **Vortex window:** close Vortex completely. Also check the tray icons near the clock, and quit Vortex there if it is still shown.
7. **File Explorer window:** click the address bar, type `%APPDATA%\Vortex`, and press Enter.
8. **File Explorer window:** write down whether there is a folder named `plugins`. If there is, open it and write down the names of the folders inside it.
9. **File Explorer window:** if there is no `Vortex` folder at all, stop here and report it.

## Part C: put the helper in

10. **Aetherial Dawn Launcher window (test build from PR #27):** open it and sign in as you normally do.
11. **Launcher window:** go to **Mods**, then click **Required mods**.
12. **Launcher window:** click **Connect Vortex**. Vortex must still be closed.
    - If the launcher says **"Close Vortex first…"**, Vortex is still running. Quit it from the tray icon and click **Connect Vortex** again.
    - The expected answer is **"The Aetherial Dawn helper is now in Vortex. Close Vortex and open it again once, then press Check again."**
13. **Launcher window:** take a screenshot of that answer. This is screenshot 2.
14. **File Explorer window:** open `%APPDATA%\Vortex\plugins\aetherial-dawn`. You should see `index.js`, `jobs.js` and `info.json`. Turn on **View > Show > Hidden items** and you should also see `.aetherial-dawn-complete`. Take a screenshot. This is screenshot 3.
15. **File Explorer window:** check `%APPDATA%\Vortex` again. There should be no folder ending in `.staging` or `.previous`.

## Part D: let Vortex load it, with AD test still active

16. **Vortex window:** open Vortex. Don't click **Enable** on any profile.
17. **Vortex window:** open **Extensions**. Find **Aetherial Dawn**. Write down its version, which should be 0.2.1, and whether it is switched on. Take a screenshot. This is screenshot 4.
18. **Vortex window:** open **Profiles** and check that **AD test** is still the active one. Take a screenshot. This is screenshot 5.
19. **Launcher window:** close **Required mods**, then click **Required mods** again.
20. **Launcher window:** read the Vortex line. It should say another profile (AD test) is active, not the Aetherial Dawn profile. It must not say Ready. Take a screenshot. This is screenshot 6.

## Part E: collect the log and stop

21. **File Explorer window:** click the address bar, type `%LOCALAPPDATA%\gg.aetherialdawn.launcher\logs`, and press Enter. Open `launcher.log` in Notepad, copy every line that contains `vortex:`, and save them in a new text file.
22. **Vortex window:** leave **AD test** active. Don't switch profiles here. The Mods chat takes you back to Aetherial Dawn with section 13 steps 5 and 6 of the install plan.

Send all six screenshots, the notes from steps 8 and 17, and the `vortex:` lines to the Vortex extension chat.

## Stop right away if

- Any mod in Vortex changes state from installed, enabled or deployed.
- Vortex switches to, or deploys, any profile other than **AD test**.
- Vortex shows an error about the Aetherial Dawn extension.
- The launcher says **Ready** at step 20.
- There is a `.staging` or `.previous` folder left at step 15.

Report what you saw and where you stopped. Don't try again or experiment.

## Undo (only if asked)

Close Vortex. In File Explorer, delete the folder `%APPDATA%\Vortex\plugins\aetherial-dawn`, then open Vortex again. This removes only the helper. No mod is touched.
