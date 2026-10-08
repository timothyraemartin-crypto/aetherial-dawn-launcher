#!/usr/bin/env python3
"""Aetherial Dawn operating protocol v2.0.0 hook (stdlib only, Python 3.9+).

Subcommands:
  start     SessionStart hook: inject context, record a baseline (stdin = hook JSON)
  stop      Stop hook: compare source to baseline, require handoff/evidence
  run ID    run a configured check (argv array, no shell) and write a receipt
  handoff   write the structured handoff for this session
  gate      local release preflight: nonzero on unresolved checks/handoff
  status    print what the hook currently knows
A workflow aid, not a security boundary. See claude-setup/operating-protocol-v2.md.
"""
import argparse, datetime, hashlib, json, os, re, subprocess, sys, time

VERSION = "2.0.0"
RUNTIME = ".claude/runtime/ad-hook"
CONFIG = ".claude/ad-hook/config.json"
STATES = ("COMPLETE_WITHIN_SCOPE", "NEEDS_GAME_CHECK", "BLOCKED", "IN_PROGRESS")
SHARED = ("synced", "pending", "n/a")


class HookError(Exception):
    pass


def utc():
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def sha(data):
    return hashlib.sha256(data).hexdigest()


def git(root, *args):
    try:
        r = subprocess.run(["git", "-C", root] + list(args), capture_output=True, text=True, timeout=30)
        return r.stdout.strip() if r.returncode == 0 else ""
    except (OSError, subprocess.SubprocessError):
        return ""


def find_root():
    env = os.environ.get("CLAUDE_PROJECT_DIR")
    if env and os.path.isdir(env):
        return os.path.abspath(env)
    top = git(os.getcwd(), "rev-parse", "--show-toplevel")
    return top or os.getcwd()


def load_config(root):
    path = os.path.join(root, CONFIG)
    try:
        raw = open(path, "rb").read()
        cfg = json.loads(raw)
    except (OSError, ValueError) as e:
        raise HookError("config %s unreadable/invalid: %s" % (CONFIG, e))
    for key, typ in (("source_roots", list), ("exclude_dirs", list), ("exclude_files", list),
                     ("inject", list), ("checks", list), ("limits", dict)):
        if not isinstance(cfg.get(key), typ):
            raise HookError("config key %r missing or wrong type" % key)
    if not cfg["source_roots"]:
        raise HookError("config source_roots is empty (coverage must be chosen deliberately)")
    lim = cfg["limits"]
    for k in ("max_files", "max_total_bytes", "max_file_bytes", "max_seconds", "inject_budget_bytes"):
        if not isinstance(lim.get(k), int) or lim[k] <= 0:
            raise HookError("config limits.%s must be a positive integer" % k)
    ids = set()
    for c in cfg["checks"]:
        if not (isinstance(c.get("id"), str) and re.fullmatch(r"[A-Za-z0-9_.-]+", c["id"])
                and isinstance(c.get("argv"), list) and c["argv"]
                and all(isinstance(a, str) for a in c["argv"])):
            raise HookError("config check needs id and non-empty argv string array: %r" % c)
        if c["id"] in ids:
            raise HookError("duplicate check id %s" % c["id"])
        ids.add(c["id"])
    cfg["_hash"] = sha(raw)
    return cfg


def excluded(rel, cfg):
    parts = rel.split("/")
    if any(p in cfg["exclude_dirs"] for p in parts[:-1]) or rel in cfg["exclude_files"]:
        return True
    if any("/" in d and rel.startswith(d.rstrip("/") + "/") for d in cfg["exclude_dirs"]):
        return True  # path-prefix entry such as docs/operations
    return rel.startswith(RUNTIME + "/") or rel.startswith(".git/")


def snapshot(root, cfg):
    """Content-hash every file under source_roots. Raises HookError if any limit is hit."""
    lim, start = cfg["limits"], time.time()
    files, total = {}, 0

    def add(rel, full):
        nonlocal total
        if excluded(rel, cfg):
            return
        if time.time() - start > lim["max_seconds"]:
            raise HookError("snapshot exceeded %ss; narrow source_roots" % lim["max_seconds"])
        if len(files) >= lim["max_files"]:
            raise HookError("snapshot exceeded %d files; narrow source_roots" % lim["max_files"])
        if os.path.islink(full):
            files[rel] = sha(("link:" + os.readlink(full)).encode())
            return
        if not os.path.isfile(full):
            return
        size = os.path.getsize(full)
        if size > lim["max_file_bytes"]:
            raise HookError("%s is %d bytes (> max_file_bytes); exclude or raise limit" % (rel, size))
        total += size
        if total > lim["max_total_bytes"]:
            raise HookError("snapshot exceeded %d bytes; narrow source_roots" % lim["max_total_bytes"])
        with open(full, "rb") as f:
            files[rel] = sha(f.read())

    for entry in cfg["source_roots"]:
        base = os.path.join(root, entry)
        if os.path.isdir(base) and not os.path.islink(base):
            for d, dirs, names in os.walk(base):
                rd = os.path.relpath(d, root).replace(os.sep, "/")
                dirs[:] = sorted(x for x in dirs if x not in cfg["exclude_dirs"]
                                 and not excluded(rd + "/" + x + "/_", cfg))
                for n in sorted(names):
                    add(rd + "/" + n, os.path.join(d, n))
        elif os.path.lexists(base):
            add(entry.replace(os.sep, "/"), base)
    staged = []
    for line in git(root, "ls-files", "-s", "--", *cfg["source_roots"]).splitlines():
        m = line.split("\t", 1)
        if len(m) == 2 and not excluded(m[1], cfg):
            staged.append(line)
    cov = {"source_roots": cfg["source_roots"], "exclude_dirs": cfg["exclude_dirs"],
           "exclude_files": cfg["exclude_files"], "files": len(files), "bytes": total}
    fp = sha(json.dumps({"config": cfg["_hash"], "files": files, "staged": staged}, sort_keys=True).encode())
    return {"fingerprint": fp, "files": files, "staged": len(staged), "coverage": cov, "taken": utc()}


def diff(base, cur):
    b, c = base["files"], cur["files"]
    return {"added": sorted(set(c) - set(b)), "deleted": sorted(set(b) - set(c)),
            "modified": sorted(p for p in set(b) & set(c) if b[p] != c[p])}


def aux_hashes(root, cfg):
    out = {}
    for rel in cfg["inject"]:
        p = os.path.join(root, rel)
        out[rel] = sha(open(p, "rb").read()) if os.path.isfile(p) else None
    return out


# ---- session records -------------------------------------------------------

def safe_sid(sid):
    return re.sub(r"[^A-Za-z0-9_.-]", "_", sid or "unknown")[:80]


def sess_dir(root, sid):
    return os.path.join(root, RUNTIME, "sessions", safe_sid(sid))


def wjson(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(obj, f, indent=1, sort_keys=True)
    os.replace(tmp, path)


def rjson(path):
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def event(root, sid, kind, **kw):
    d = sess_dir(root, sid)
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, "events.jsonl"), "a") as f:
        f.write(json.dumps(dict(kw, time=utc(), event=kind)) + "\n")


def resolve_sid(root, arg):
    sid = arg or os.environ.get("AD_HOOK_SESSION")
    if sid:
        return safe_sid(sid)
    base = os.path.join(root, RUNTIME, "sessions")
    try:
        dirs = [os.path.join(base, x) for x in os.listdir(base)]
    except OSError:
        raise HookError("no session record; pass --session or run from a session with AD_HOOK_SESSION")
    if not dirs:
        raise HookError("no session record found")
    return os.path.basename(max(dirs, key=os.path.getmtime))


def read_stdin():
    try:
        d = json.loads(sys.stdin.read() or "{}")
        return d if isinstance(d, dict) else {}
    except ValueError:
        return {}


def emit(**obj):
    print(json.dumps(obj))


# ---- checks ----------------------------------------------------------------

def check_receipt(root, sid, cid):
    return rjson(os.path.join(sess_dir(root, sid), "checks", cid + ".json"))


def required_problems(root, sid, cfg, fp):
    out = []
    for c in cfg["checks"]:
        if not c.get("required"):
            continue
        r = check_receipt(root, sid, c["id"])
        if not r:
            out.append("required check %r has no receipt (run: python3 .claude/ad-hook/ad_hook.py run %s)" % (c["id"], c["id"]))
        elif r.get("status") != "PASS":
            out.append("required check %r is %s" % (c["id"], r.get("status")))
        elif r.get("fingerprint_after") != fp:
            out.append("required check %r passed on older source (fingerprint differs); re-run" % c["id"])
    return out


def cmd_run(root, args):
    cfg = load_config(root)
    sid = resolve_sid(root, args.session)
    chk = next((c for c in cfg["checks"] if c["id"] == args.id), None)
    if not chk:
        raise HookError("unknown check %r; configured: %s" % (args.id, ", ".join(c["id"] for c in cfg["checks"])))
    d = os.path.join(sess_dir(root, sid), "checks")
    rp, lp = os.path.join(d, chk["id"] + ".json"), os.path.join(d, chk["id"] + ".log")
    rec = {"id": chk["id"], "argv": chk["argv"], "cwd": chk.get("cwd", "."), "started": utc(), "status": "RUNNING"}
    try:
        before = snapshot(root, cfg)
        rec.update(fingerprint_before=before["fingerprint"], coverage=before["coverage"])
    except HookError as e:
        rec.update(status="ERROR", error=str(e), ended=utc())
        wjson(rp, rec)
        print("ERROR:", e)
        return 1
    wjson(rp, rec)  # RUNNING supersedes any earlier PASS before launch
    status, code, out = "ERROR", None, b""
    try:
        r = subprocess.run(chk["argv"], cwd=os.path.join(root, rec["cwd"]), capture_output=True,
                           timeout=chk.get("timeout", 600))
        code, out = r.returncode, r.stdout + r.stderr
        status = "PASS" if code == 0 else "FAIL"
    except subprocess.TimeoutExpired as e:
        status, out = "TIMEOUT", (e.stdout or b"") + (e.stderr or b"")
    except OSError as e:
        rec["error"] = "launch failed: %s" % e
    os.makedirs(d, exist_ok=True)
    with open(lp, "wb") as f:
        f.write(out)
    try:
        after = snapshot(root, cfg)
        rec["fingerprint_after"] = after["fingerprint"]
        if status == "PASS" and after["fingerprint"] != before["fingerprint"]:
            status = "SOURCE_CHANGED"
    except HookError as e:
        rec["error"] = str(e)
        status = "ERROR"
    rec.update(status=status, exit_code=code, ended=utc(), log=os.path.relpath(lp, root), log_sha256=sha(out))
    wjson(rp, rec)
    event(root, sid, "check", id=chk["id"], status=status)
    sys.stdout.write(out.decode("utf-8", "replace")[-2000:])
    print("\n[ad-hook] check %s: %s" % (chk["id"], status))
    return 0 if status == "PASS" else 1


# ---- start / stop ----------------------------------------------------------

def cmd_start(root):
    inp = read_stdin()
    sid, source = safe_sid(inp.get("session_id")), inp.get("source", "startup")
    d = sess_dir(root, sid)
    warns, lines = [], []
    cfg = None
    try:
        cfg = load_config(root)
    except HookError as e:
        warns.append("AD-HOOK INCOMPLETE: %s. Fix %s; no baseline or change detection this session." % (e, CONFIG))
    if cfg:
        base = rjson(os.path.join(d, "baseline.json"))
        if base and source in ("resume", "compact"):
            event(root, sid, "resume", source=source)
            lines.append("Resumed (%s): original baseline kept (fingerprint %s, taken %s)." % (source, base["fingerprint"][:12], base["taken"]))
            hf = rjson(os.path.join(d, "handoff.json"))
            if hf:
                lines.append("Prior handoff: state=%s next=%s" % (hf.get("state"), hf.get("next_action")))
            cur_sn = None
            try:
                cur_sn = snapshot(root, cfg)
                ch = diff(base, cur_sn)
                lines.append("Source changed since baseline: +%d -%d ~%d" % (len(ch["added"]), len(ch["deleted"]), len(ch["modified"])))
            except HookError as e:
                warns.append("AD-HOOK: snapshot failed on resume: %s" % e)
        else:
            try:
                sn = snapshot(root, cfg)
                sn.update(aux=aux_hashes(root, cfg), head=git(root, "rev-parse", "HEAD"),
                          branch=git(root, "rev-parse", "--abbrev-ref", "HEAD"), session=sid, source=source,
                          dirty_at_start=git(root, "status", "--porcelain").splitlines()[:50])
                wjson(os.path.join(d, "baseline.json"), sn)
                event(root, sid, "start", source=source, fingerprint=sn["fingerprint"])
                lines.append("Baseline recorded: %s @ %s, fingerprint %s, covers %d files / %d bytes of %s. "
                             "Pre-existing uncommitted changes (%d) are baseline, not this session's work."
                             % (sn["branch"], sn["head"][:10], sn["fingerprint"][:12], sn["coverage"]["files"],
                                sn["coverage"]["bytes"], ",".join(cfg["source_roots"]), len(sn["dirty_at_start"])))
            except HookError as e:
                warns.append("AD-HOOK INCOMPLETE: baseline failed: %s" % e)
        budget, used, ctx = cfg["limits"]["inject_budget_bytes"], 0, []
        for rel in cfg["inject"]:
            p = os.path.join(root, rel)
            if not os.path.isfile(p):
                warns.append("AD-HOOK: missing %s (create it; not invented)" % rel)
                continue
            data = open(p, "rb").read()
            note = "%s sha256:%s %dB" % (rel, sha(data)[:12], len(data))
            if used + len(data) > budget:
                warns.append("AD-HOOK: %s (%dB) exceeds remaining injection budget; NOT injected, read it yourself" % (rel, len(data)))
                ctx.append("[not injected] " + note)
            else:
                used += len(data)
                ctx.append("----- %s -----\n%s" % (note, data.decode("utf-8", "replace")))
        lines.extend(ctx)
    head = ["Aetherial Dawn operating protocol %s hook (prepared output; delivery is confirmed only by you seeing this). "
            "Full protocol: /mnt/project-files/aetherial-dawn/claude-setup/operating-protocol-v2.md; bootstrap: .claude/ad-hook/BOOTSTRAP.md. "
            "Session id for the wrapper: %s" % (VERSION, sid)]
    context = "\n".join(head + warns + lines)
    wjson(os.path.join(d, "start.json"), {"time": utc(), "source": source, "warnings": warns,
                                          "context_sha256": sha(context.encode()), "context_bytes": len(context)})
    envf = os.environ.get("CLAUDE_ENV_FILE")
    if envf:
        try:
            with open(envf, "a") as f:
                f.write("export AD_HOOK_SESSION=%s\n" % sid)
        except OSError:
            pass
    out = {"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": context}}
    if warns:
        out["systemMessage"] = "\n".join(warns)
    emit(**out)
    return 0


def validate_handoff(hf, fp):
    if not hf:
        return ["no handoff for this session (run: python3 .claude/ad-hook/ad_hook.py handoff --state ... )"]
    p = []
    if hf.get("state") not in STATES:
        p.append("handoff state must be one of %s" % ", ".join(STATES))
    for k in ("objective", "summary", "evidence", "next_action"):
        if not str(hf.get(k, "")).strip():
            p.append("handoff field %r is empty" % k)
    if hf.get("state") in ("BLOCKED", "NEEDS_GAME_CHECK", "IN_PROGRESS") and not str(hf.get("unresolved", "")).strip():
        p.append("handoff state %s needs 'unresolved' (what blocks / what to observe / what is unfinished)" % hf.get("state"))
    if hf.get("shared_sync") not in SHARED:
        p.append("handoff shared_sync must be one of %s" % ", ".join(SHARED))
    if hf.get("fingerprint") != fp:
        p.append("source changed after the handoff was written; rewrite it")
    return p


def cmd_stop(root):
    inp = read_stdin()
    sid, active = safe_sid(inp.get("session_id")), bool(inp.get("stop_hook_active"))
    d = sess_dir(root, sid)

    def finish(outcome, msg=None, block=None):
        wjson(os.path.join(d, "outcome.json"), {"outcome": outcome, "time": utc(), "message": msg or block})
        event(root, sid, "stop", outcome=outcome)
        if block:
            emit(decision="block", reason=block)
        elif msg:
            emit(systemMessage=msg)
        return 0

    try:
        cfg = load_config(root)
    except HookError as e:
        return finish("INCOMPLETE", "AD-HOOK INCOMPLETE: %s" % e)
    base = rjson(os.path.join(d, "baseline.json"))
    if not base:
        return finish("INCOMPLETE", "AD-HOOK INCOMPLETE: no baseline for session %s (SessionStart hook did not run or failed); change detection unavailable." % sid)
    try:
        cur = snapshot(root, cfg)
    except HookError as e:
        return finish("INCOMPLETE", "AD-HOOK INCOMPLETE: snapshot failed: %s" % e)
    ch = diff(base, cur)
    n = len(ch["added"]) + len(ch["deleted"]) + len(ch["modified"])
    if not n:
        return finish("NO_SOURCE_CHANGE")
    problems = []
    now = aux_hashes(root, cfg)
    if "PROGRESS.md" in cfg["inject"] and now.get("PROGRESS.md") == base.get("aux", {}).get("PROGRESS.md"):
        problems.append("source changed (+%d -%d ~%d) but PROGRESS.md was not updated" % (len(ch["added"]), len(ch["deleted"]), len(ch["modified"])))
    hf = rjson(os.path.join(d, "handoff.json"))
    problems += validate_handoff(hf, cur["fingerprint"])
    if hf and hf.get("state") == "COMPLETE_WITHIN_SCOPE":
        problems += required_problems(root, sid, cfg, cur["fingerprint"])
    if problems:
        text = "AD-HOOK: " + "; ".join(problems)
        if active:
            return finish("INCOMPLETE", text + " (stop hook already active: ending turn, recorded INCOMPLETE)")
        return finish("PENDING_CORRECTION", block=text)
    state = hf["state"]
    if state == "COMPLETE_WITHIN_SCOPE":
        return finish(state)
    return finish(state, "AD-HOOK: ended with %s (not a passing release gate). Unresolved: %s" % (state, hf.get("unresolved")))


# ---- handoff / gate / status ----------------------------------------------

def cmd_handoff(root, a):
    cfg = load_config(root)
    sid = resolve_sid(root, a.session)
    d = sess_dir(root, sid)
    base = rjson(os.path.join(d, "baseline.json"))
    cur = snapshot(root, cfg)
    hf = {"state": a.state, "objective": a.objective, "summary": a.summary, "evidence": a.evidence,
          "unresolved": a.unresolved or "", "failed_attempts": a.failed or "", "next_action": a.next,
          "shared_sync": a.shared_sync, "written": utc(), "session": sid, "head": git(root, "rev-parse", "HEAD"),
          "fingerprint": cur["fingerprint"], "changed": diff(base, cur) if base else None,
          "checks": {c["id"]: (check_receipt(root, sid, c["id"]) or {}).get("status", "none") for c in cfg["checks"]}}
    wjson(os.path.join(d, "handoff.json"), hf)
    bad = validate_handoff(hf, cur["fingerprint"])
    out = os.path.join(root, "docs", "operations", "handoffs", "%s-%s.md" % (utc()[:10], sid[:8]))
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w") as f:
        f.write("# Handoff %s (%s)\n\nState: **%s** · shared records: %s · head: %s\n\n" % (sid[:8], utc(), a.state, a.shared_sync, hf["head"][:10]))
        for k, t in (("objective", "Objective"), ("summary", "Changes"), ("evidence", "Evidence"),
                     ("failed_attempts", "Failed attempts"), ("unresolved", "Unresolved"), ("next_action", "Next action")):
            f.write("## %s\n%s\n\n" % (t, hf[k] or "(none)"))
        f.write("## Checks\n%s\n" % "\n".join("- %s: %s" % kv for kv in hf["checks"].items()))
    event(root, sid, "handoff", state=a.state)
    print("handoff written (%s); durable copy %s" % (a.state, os.path.relpath(out, root)))
    for m in bad:
        print("WARN:", m)
    return 1 if bad else 0


def cmd_gate(root, a):
    cfg = load_config(root)
    sid = resolve_sid(root, a.session)
    cur = snapshot(root, cfg)
    hf = rjson(os.path.join(sess_dir(root, sid), "handoff.json"))
    problems = validate_handoff(hf, cur["fingerprint"])
    if hf and hf.get("state") != "COMPLETE_WITHIN_SCOPE":
        problems.append("handoff state is %s (pending/unresolved work is not a passing gate)" % hf.get("state"))
    problems += required_problems(root, sid, cfg, cur["fingerprint"])
    for p in problems:
        print("GATE FAIL:", p)
    if not problems:
        print("GATE PASS (local preflight only; not a deployment gate) fingerprint %s" % cur["fingerprint"][:12])
    return 1 if problems else 0


def cmd_status(root, a):
    cfg = load_config(root)
    sid = resolve_sid(root, a.session)
    d = sess_dir(root, sid)
    base = rjson(os.path.join(d, "baseline.json"))
    cur = snapshot(root, cfg)
    print("session", sid, "| fingerprint", cur["fingerprint"][:12], "| coverage", cur["coverage"]["files"], "files")
    if base:
        print("changes since baseline:", json.dumps(diff(base, cur)))
    print("outcome:", (rjson(os.path.join(d, "outcome.json")) or {}).get("outcome", "none"))
    for c in cfg["checks"]:
        r = check_receipt(root, sid, c["id"])
        print("check %s: %s%s" % (c["id"], r["status"] if r else "none",
                                  " (current)" if r and r.get("fingerprint_after") == cur["fingerprint"] else ""))
    return 0


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("start")
    sub.add_parser("stop")
    r = sub.add_parser("run"); r.add_argument("id"); r.add_argument("--session")
    g = sub.add_parser("gate"); g.add_argument("--session")
    s = sub.add_parser("status"); s.add_argument("--session")
    h = sub.add_parser("handoff"); h.add_argument("--session")
    h.add_argument("--state", required=True, choices=STATES)
    for k in ("objective", "summary", "evidence", "next"):
        h.add_argument("--" + k, required=True)
    h.add_argument("--unresolved"); h.add_argument("--failed")
    h.add_argument("--shared-sync", required=True, choices=SHARED)
    a = ap.parse_args()
    root = find_root()
    try:
        if a.cmd == "start":
            return cmd_start(root)
        if a.cmd == "stop":
            return cmd_stop(root)
        return {"run": lambda: cmd_run(root, a), "gate": lambda: cmd_gate(root, a),
                "status": lambda: cmd_status(root, a), "handoff": lambda: cmd_handoff(root, a)}[a.cmd]()
    except HookError as e:
        print("ad-hook:", e, file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
