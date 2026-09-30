# Run sheet: Steam repair and read-only game file (D13, D14)

For Timothy's PC. The script only **reads**. It changes no files, no Steam
settings and nothing in Vortex, and it never opens your sign-in, keys or
`skymp5-client-settings.txt`. Each run saves one text file on your Desktop
named `AD-evidence-<label>-<date>.txt`. Send those files back in the chat.

**Never press Steam's "Verify integrity of game files" yourself for this.**
Steam would put back the newest Skyrim and undo the launcher's version fix.
Only the launcher itself may ask Steam to check files.

## Get the script (once)
1. Open this page in your browser:
   https://github.com/timothyraemartin-crypto/aetherial-dawn-launcher/blob/qa-windows-evidence/docs/qa/windows-evidence.ps1
2. Click the **Download raw file** button (the arrow pointing down, top right of the code).
3. It saves to your **Downloads** folder as `windows-evidence.ps1`.

## How to run it (every time)
1. Click **Start**, type `PowerShell`, and click **Windows PowerShell**. A blue window opens.
2. Type this, then press **Enter**:
   `cd $HOME\Downloads`
3. Type the line from the step below, then press **Enter**. For example:
   `powershell -ExecutionPolicy Bypass -File .\windows-evidence.ps1 -Label held`
4. It prints `Saved:` and the file's name. The file is on your Desktop.

## Step 1: now, any time (D14)
Do this with the launcher closed and Skyrim closed.
- Run it with `-Label held`.
- This shows whether Steam's file for Skyrim (`appmanifest_489830.acf`) is
  still read-only and set to "only update when I launch it" from the last
  version fix.

## Step 2: only if the launcher asks Steam to check files (D13 and D14)
You'll only see this if Steam updated Skyrim and the launcher is fixing the
version. The launcher then says: *"Steam is checking Skyrim's files. The
launcher carries on by itself when Steam is done."*
1. As soon as that message shows, run it with `-Label before`.
2. While Steam's Downloads page shows Skyrim being checked, run it with
   `-Label during`. Once is enough; twice is better.
3. Note the time on your clock when **Steam** shows the check is finished.
4. Note the time when the **launcher** moves on (the message changes).
5. Then run it with `-Label after`.
6. Send the files and the two times from steps 3 and 4.

If Steam shows an error about files it can't write, take a screenshot of it
too. That is exactly what D14 is about.

## What the results answer
- **D14:** is the manifest read-only (`read-only: True`) when the launcher
  asks Steam to repair, and did Steam's repair fail or stall because of it
  (`UpdateResult`, `StateFlags`, content log lines).
- **D13:** how long after Steam really finished did the launcher notice
  (content log "Fully Installed" time versus the launcher log's next
  `patch:` line), and whether the launcher retried while Steam was still
  working.
