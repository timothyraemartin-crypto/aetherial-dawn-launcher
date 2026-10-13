#!/usr/bin/env python3
"""List progress entries (newest first) or validate them.

  python3 .claude/progress/now.py [N]    print the newest N entries (default 8)
  python3 .claude/progress/now.py --check  fail on a bad file name or heading
"""
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
NAME = re.compile(r"^\d{4}-\d{2}-\d{2}-[a-z0-9][a-z0-9-]*\.md$")


def entries():
    files = [p for p in HERE.glob("*.md") if p.name != "README.md"]
    return sorted(files, key=lambda p: p.name, reverse=True)


def check():
    bad = []
    for p in entries():
        if not NAME.match(p.name):
            bad.append(f"{p.name}: name must be YYYY-MM-DD-slug.md (lowercase, digits, hyphens)")
            continue
        first = p.read_text(encoding="utf-8").split("\n", 1)[0]
        if not first.startswith("# " + p.name[:10]):
            bad.append(f"{p.name}: first line must start with '# {p.name[:10]} '")
    for line in bad:
        print(line)
    return 1 if bad else 0


def main(argv):
    if "--check" in argv:
        return check()
    limit = int(argv[0]) if argv and argv[0].isdigit() else 8
    for p in entries()[:limit]:
        print(p.read_text(encoding="utf-8").rstrip() + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
