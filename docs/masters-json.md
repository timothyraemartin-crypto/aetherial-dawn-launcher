# Making and serving masters.json from an export

`masters.json` tells every player's launcher which plugins the server loads, in which order, and what each PC's copy must hash to. This page is for Server deploys and whoever serves the file. Nothing here happens without Timothy's release decision.

## When a generated file may be served
- **Only once the players' minimum launcher version includes this change.** That's PR #41: canonical fields and name checks.
  - Older launchers compare the extra entries by name and position, then by size, crc32 and sha256.
  - `masters-json` writes the plain `size`, `crc32` and `sha256` with the same canonical values, so an older launcher still checks the converted copy and never the original file.
  - Older launchers don't refuse a bad name, though. Don't serve a generated file to them.
- **Go-live order:** launcher code first, and not before line 1. The generated masters.json comes at the export step, after Quality checks and Timothy's word.

## Make it
```
masters-json <current masters.json> <export.json> > masters.json
```
- The five base masters are kept exactly as the current file has them.
- Every master past them (`export.json` `masters[]`) and every lane plugin (`files{}`) follows in `index` order, under its `run_name`. Each has `canonical_sha256`, `canonical_size` (from `canonical_bytes`) and `canonical_crc32`, plus the same values as `sha256`, `size` and `crc32`.
- The export's own `sha256` is the exporting PC's original file and is never written.
- It refuses the file if:
  - an index is missing or repeated;
  - a canonical value is missing;
  - a name isn't a plain plugin file name (for example it has a path separator, a control character or a leading `*` or `#`);
  - a name is a base master's, or is repeated.

## After it is served: the bot's health check
The Discord bot's launcher check only alerts on a changed masters.json if `HEALTH_LAUNCHER_EXPECT` pins it. That value is a comma list. Its masters.json entry holds both the **entry count** and the **SHA-1**.
1. Take the SHA-1 of the bytes **as served**: download the file back from the server and hash that, not the local copy.
2. In the comma list, change the masters.json entry's count from 5 to 5 + N (N = entries past the base five) and its SHA-1 to the value from step 1. Edit that one entry in place and keep the rest of the list unchanged. The entry reads exactly:
   ```
   masters.json=<5+N>:<sha1>
   ```
   For example, with 58 lane plugins: `masters.json=63:<40 hex characters>`.
   - Pin the new value **before or at the moment you publish** the file. Otherwise the bot sees a changed masters.json and posts a one-off "changed" warning within 5 minutes.
   - Read the whole line when checking it. A grep for a single key misses a line that holds several keys.
3. Expect the alert, if anything is wrong, about **10 minutes** after the change, not 5.

Note: the launcher's "server order" health row reads Ok even on an unusable list (a bad or repeated name). Play still refuses such a list, so the row is not the check to trust here; the steps above are.
