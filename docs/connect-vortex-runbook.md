# Connect Vortex test on Timothy's PC (disposable profile)

**Status: HELD. Do not start yet.** Start only when all three of these are true:

1. The Mods chat has a fresh manual Vortex backup and a current inventory receipt of the real "Aetherial Dawn" profile.
2. Codex has reviewed launcher PR #27 and agreed to this test on PR #7.
3. The test launcher build from PR #27 is green in Windows CI.

This test only checks one thing: that the launcher can put its read-only helper into Vortex and read Vortex's state back. It installs, enables, deploys and removes no mods. It does not change the real "Aetherial Dawn" profile. The helper answers only "what is installed", and nothing else.

What it proves, which fixture tests can't:
- Vortex 2.7.1 keeps user extensions in `%APPDATA%\Vortex\plugins`.
- Vortex 2.7.1 loads helper 0.2.1.
- The helper reports the active profile without switching it.
- The launcher shows the right line for the profile that is active.

---

## Part A: before you start

1. **Skyrim:** close Skyrim if it is running.
2. **Aetherial Dawn Launcher window:** close the launcher.
3. **Vortex window:** wait until nothing is downloading or deploying. The bar at the bottom should be empty.
4. **Vortex window:** if you do not already have the profile `AD collection receipt disposable`, make it now with steps 5 to 16 of ChatGPT's disposable-profile guide (PR #7 comment 5876255083). Make it as a new, empty profile, not a copy.
5. **Vortex window:** open **Profiles**. Take a screenshot that shows which profile is active. This is screenshot 1.

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

## Part D: let Vortex load it, with the disposable profile active

16. **Vortex window:** open Vortex.
17. **Vortex window:** open **Extensions**. Find **Aetherial Dawn**. Write down its version, which should be 0.2.1, and whether it is switched on. Take a screenshot. This is screenshot 4.
18. **Vortex window:** open **Profiles** and click **Enable** on `AD collection receipt disposable`. Wait until any deployment Vortex starts has finished.
19. **Launcher window:** close **Required mods**, then click **Required mods** again.
20. **Launcher window:** read the Vortex line. It should say that another profile is active, not the Aetherial Dawn profile. It must not say Ready. Take a screenshot. This is screenshot 5.

## Part E: back to your real profile

21. **Vortex window:** open **Profiles** and click **Enable** on **Aetherial Dawn**. Wait until the deployment has finished completely.
22. **Vortex window:** confirm that the active marker is beside **Aetherial Dawn**.
23. **Launcher window:** close **Required mods**, then click **Required mods** again. Take a screenshot of the Vortex line. This is screenshot 6. The line should count your real profile's mods. With the current list it should be incomplete, not Ready.
24. **File Explorer window:** click the address bar, type `%LOCALAPPDATA%\gg.aetherialdawn.launcher\logs`, and press Enter. Open `launcher.log` in Notepad, copy every line that contains `vortex:`, and save them in a new text file.

Send all six screenshots, the notes from steps 8 and 17, and the `vortex:` lines to the Vortex extension chat.

## Stop right away if

- Any mod in Vortex changes state from installed, enabled or deployed, other than the deployment Vortex does itself when you switch profiles.
- Vortex shows an error about the Aetherial Dawn extension.
- The launcher says **Ready** at step 20 or at step 23.
- There is a `.staging` or `.previous` folder left at step 15.

Report what you saw and where you stopped. Don't try again or experiment.

## Undo (only if asked)

Close Vortex. In File Explorer, delete the folder `%APPDATA%\Vortex\plugins\aetherial-dawn`, then open Vortex again. This removes only the helper. No mod is touched.
