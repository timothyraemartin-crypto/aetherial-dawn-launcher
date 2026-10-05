#!/usr/bin/env python3
"""
scan_plugins.py - inspect Skyrim SE plugin headers for SkyMP server safety.

For every .esp/.esm/.esl in a folder (default: ./data) this reads the TES4
record header and reports:
  * file extension .esl                           -> UNSUPPORTED on SkyMP server
  * ESL (light master) flag 0x200 in TES4 flags   -> UNSUPPORTED on SkyMP server
  * ESM flag 0x1
  * HEDR header version (1.71 means the "extended ESL range" format)
  * list of masters (MAST subrecords)
Optionally, with --settings server-settings.json, it also checks that:
  * every loadOrder entry exists in the folder (case-exact)
  * every plugin's masters appear earlier in loadOrder
  * no loadOrder entry is ESL-flagged or .esl
  * the five vanilla masters come first in the canonical order

No third-party dependencies. Exit code 1 if any problem is found.

Format notes (TES4 header, Skyrim SE):
  record: type[4] dataSize[u32] flags[u32] formID[u32] vc[u32] version[u16] unknown[u16]
  subrecords: type[4] size[u16] data[size]
  HEDR: version[f32] numRecords[u32] nextObjectID[u32]
  MAST: zero-terminated master filename
"""
import argparse
import json
import os
import struct
import sys

VANILLA = ["Skyrim.esm", "Update.esm", "Dawnguard.esm", "HearthFires.esm", "Dragonborn.esm"]
FLAG_ESM = 0x1
FLAG_ESL = 0x200
PLUGIN_EXT = (".esp", ".esm", ".esl")


def read_header(path):
    """Return dict with flags, version, masters, or raise ValueError."""
    with open(path, "rb") as f:
        rec = f.read(24)
        if len(rec) < 24 or rec[:4] != b"TES4":
            raise ValueError("not a TES4 plugin (bad record header)")
        _type, data_size, flags, _form, _vc, _ver, _unk = struct.unpack("<4sIIIIHH", rec)
        data = f.read(data_size)
    masters = []
    hedr_version = None
    pos = 0
    while pos + 6 <= len(data):
        stype = data[pos:pos + 4]
        (ssize,) = struct.unpack("<H", data[pos + 4:pos + 6])
        sdata = data[pos + 6:pos + 6 + ssize]
        if stype == b"HEDR" and ssize >= 4:
            hedr_version = struct.unpack("<f", sdata[:4])[0]
        elif stype == b"MAST":
            masters.append(sdata.rstrip(b"\x00").decode("utf-8", errors="replace"))
        pos += 6 + ssize
    return {"flags": flags, "version": hedr_version, "masters": masters}


def scan_folder(folder):
    results = {}
    for name in sorted(os.listdir(folder)):
        if not name.lower().endswith(PLUGIN_EXT):
            continue
        path = os.path.join(folder, name)
        try:
            info = read_header(path)
        except Exception as e:  # noqa: BLE001
            results[name] = {"error": str(e)}
            continue
        info["size"] = os.path.getsize(path)
        info["is_esl_ext"] = name.lower().endswith(".esl")
        info["is_esl_flag"] = bool(info["flags"] & FLAG_ESL)
        info["is_esm_flag"] = bool(info["flags"] & FLAG_ESM)
        results[name] = info
    return results


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("folder", nargs="?", default="data", help="server dataDir (default: data)")
    ap.add_argument("--settings", help="server-settings.json to check loadOrder against")
    ap.add_argument("--json", action="store_true", help="machine-readable output")
    args = ap.parse_args()

    if not os.path.isdir(args.folder):
        print(f"ERROR: folder not found: {args.folder}")
        return 2

    results = scan_folder(args.folder)
    problems = []

    if args.json:
        print(json.dumps(results, indent=2))
    else:
        print(f"Scanned {len(results)} plugin(s) in {args.folder}\n")
        for name, info in results.items():
            if "error" in info:
                print(f"  !! {name}: {info['error']}")
                problems.append(f"{name}: unreadable header")
                continue
            tags = []
            if info["is_esl_ext"]:
                tags.append("ESL-EXTENSION")
            if info["is_esl_flag"]:
                tags.append("ESL-FLAG(0x200)")
            if info["is_esm_flag"]:
                tags.append("esm-flag")
            if info["version"] is not None and info["version"] >= 1.705:
                tags.append(f"header {info['version']:.2f} (1.71 extended-ESL format)")
            ver = f"{info['version']:.2f}" if info["version"] is not None else "?"
            print(f"  {name}  v{ver}  masters={info['masters']}  {' '.join(tags)}")
            if info["is_esl_ext"] or info["is_esl_flag"]:
                problems.append(f"{name}: light plugin (.esl or ESL flag) - unsupported on SkyMP server (issue #530)")
            if info["version"] is not None and info["version"] >= 1.705:
                problems.append(f"{name}: header version {info['version']:.2f} - parser support unverified; test from vanilla baseline")

    if args.settings:
        try:
            with open(args.settings, "r", encoding="utf-8") as f:
                settings = json.load(f)
        except json.JSONDecodeError as e:
            problems.append(f"{args.settings}: not strict JSON ({e}); SkyMP does not accept comments or trailing commas")
            settings = {}
        except OSError as e:
            problems.append(f"{args.settings}: {e}")
            settings = {}
        lo = settings.get("loadOrder", [])
        if not isinstance(lo, list):
            problems.append("loadOrder is not a list")
            lo = []
        print(f"\nloadOrder has {len(lo)} entries")
        if lo[: len(VANILLA)] != VANILLA:
            problems.append(f"loadOrder should start with {VANILLA} in this exact order")
        seen = []
        for entry in lo:
            if entry not in results:
                # case-insensitive hint
                ci = [n for n in results if n.lower() == entry.lower()]
                hint = f" (found {ci[0]} - case mismatch, Linux is case-sensitive)" if ci else ""
                problems.append(f"loadOrder entry not in {args.folder}: {entry}{hint}")
                seen.append(entry)
                continue
            info = results[entry]
            if "error" not in info:
                if info["is_esl_ext"] or info["is_esl_flag"]:
                    problems.append(f"loadOrder contains light plugin: {entry}")
                for m in info["masters"]:
                    if m not in seen:
                        problems.append(f"{entry}: master {m} is missing or listed after it")
            seen.append(entry)
        extra = [n for n in results if n not in lo]
        if extra:
            print(f"Plugins in folder but not in loadOrder (ignored by server): {extra}")

    print()
    if problems:
        print("PROBLEMS:")
        for p in problems:
            print(f"  - {p}")
        return 1
    print("No problems found.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
