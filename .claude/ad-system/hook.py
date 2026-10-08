#!/usr/bin/env python3
"""Aetherial Dawn v2: local evidence and bounded handoff hooks; no network/SDK.

This is a workflow aid, not a tamper-proof or unbypassable deployment boundary.
All runtime records are local. Read OPERATING_PROTOCOL.md and CONFIGURATION.md.
"""
import argparse
import contextlib
import fnmatch
import hashlib
import json
import os
from pathlib import Path
import signal
import stat
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
import uuid

VERSION = "2.0.0"
CONFIG = ".claude/ad-hook.json"
RUNTIME = ".claude/runtime/ad-hook"
STATUSES = {"COMPLETE_WITHIN_SCOPE", "NEEDS_GAME_CHECK", "BLOCKED", "IN_PROGRESS"}


class HookError(Exception):
    pass


def now():
    return datetime.now(timezone.utc).isoformat(timespec="seconds")


def digest(data):
    return hashlib.sha256(data).hexdigest()


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True).encode()


def read_json(path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as exc:
        raise HookError("Cannot read valid JSON at %s: %s" % (path, exc)) from exc


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix=".ad-write-", dir=str(path.parent))
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            json.dump(value, handle, indent=2, ensure_ascii=True)
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def inside(root, relative):
    if not isinstance(relative, str) or not relative or Path(relative).is_absolute():
        raise HookError("Expected a nonempty repo-relative path: %r" % relative)
    candidate = (root / relative).resolve()
    try:
        candidate.relative_to(root)
    except ValueError as exc:
        raise HookError("Path escapes repository: %s" % relative) from exc
    return candidate


def bounded_number(config, name, low, high):
    value = config.get(name)
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not low <= value <= high:
        raise HookError("%s must be between %s and %s" % (name, low, high))


def load_config(root):
    config = read_json(inside(root, CONFIG))
    expected = {"schema_version", "source_roots", "exclude_globs", "startup_files",
                "startup_budget_bytes", "snapshot_timeout_seconds", "max_files", "max_file_bytes",
                "max_total_bytes", "require_progress_update", "require_check_for_source_change", "checks"}
    if not isinstance(config, dict) or set(config) != expected or type(config["schema_version"]) is not int or config["schema_version"] != 1:
        raise HookError("Config must use schema_version 1 and exactly the documented fields")
    for key in ("source_roots", "startup_files", "exclude_globs"):
        if not isinstance(config[key], list) or not all(isinstance(x, str) and x for x in config[key]):
            raise HookError("%s must be a list of nonempty strings" % key)
    if not config["source_roots"] or not config["startup_files"]:
        raise HookError("source_roots and startup_files must not be empty")
    for key in ("source_roots", "startup_files"):
        for value in config[key]:
            inside(root, value)
            if ".." in Path(value).parts or Path(value).as_posix() != value:
                raise HookError("Use canonical forward-slash relative paths without '..': " + value)
    for key in ("require_progress_update", "require_check_for_source_change"):
        if not isinstance(config[key], bool):
            raise HookError("%s must be true or false" % key)
    bounded_number(config, "startup_budget_bytes", 1024, 100000)
    bounded_number(config, "snapshot_timeout_seconds", 0.01, 30)
    bounded_number(config, "max_files", 1, 200000)
    bounded_number(config, "max_file_bytes", 1, 1073741824)
    bounded_number(config, "max_total_bytes", 1, 10737418240)
    for name in ("startup_budget_bytes", "max_files", "max_file_bytes", "max_total_bytes"):
        if type(config[name]) is not int:
            raise HookError(name + " must be an integer")
    if not isinstance(config["checks"], list):
        raise HookError("checks must be an array")
    ids = set()
    for check in config["checks"]:
        if not isinstance(check, dict) or set(check) != {"id", "argv", "cwd", "timeout_seconds", "required"}:
            raise HookError("Each check needs id, argv, cwd, timeout_seconds, required")
        if (not isinstance(check["id"], str) or not check["id"] or
                any(c not in "abcdefghijklmnopqrstuvwxyz0123456789-_" for c in check["id"]) or check["id"] in ids):
            raise HookError("Check IDs must be unique lowercase letters/numbers/hyphens/underscores")
        ids.add(check["id"])
        if not isinstance(check["argv"], list) or not check["argv"] or not all(isinstance(x, str) and x for x in check["argv"]):
            raise HookError("Check argv must be a nonempty argument array")
        inside(root, check["cwd"])
        bounded_number(check, "timeout_seconds", 0.01, 3600)
        if not isinstance(check["required"], bool):
            raise HookError("Check required must be true or false")
    return config


def excluded(relative, config):
    # Operational outputs are always outside code fingerprints; no self-invalidation.
    if relative == ".git" or relative.startswith(".git/") or relative == ".claude/runtime" or relative.startswith(".claude/runtime/"):
        return True
    return any(fnmatch.fnmatchcase(relative, pattern) for pattern in config["exclude_globs"])


def covered(relative, config):
    for prefix in config["source_roots"]:
        prefix = Path(prefix).as_posix().rstrip("/")
        if prefix == "." or relative == prefix or relative.startswith(prefix + "/"):
            return not excluded(relative, config)
    return False


def git_run(root, args, timeout):
    try:
        result = subprocess.run(["git", "-C", str(root)] + args, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, timeout=max(0.01, timeout), check=False)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise HookError("Git inventory unavailable: %s" % exc) from exc
    if result.returncode:
        raise HookError("Git inventory command failed: %s" % result.stderr.decode(errors="replace")[:300])
    return result.stdout


def snapshot(root, config):
    started = time.monotonic()
    limit = config["snapshot_timeout_seconds"]
    for source in config["source_roots"]:
        if not inside(root, source).exists():
            raise HookError("Configured source root is missing: " + source)

    def budget():
        remaining = limit - (time.monotonic() - started)
        if remaining <= 0:
            raise HookError("Source snapshot timed out; narrow source_roots, do not accept partial coverage")
        return remaining

    paths, stage_entries = set(), []
    # .git may be a directory or a worktree gitfile. Root must be the installed repo root.
    mode = "git" if (root / ".git").exists() else "local"
    head = None
    if mode == "git":
        staged = git_run(root, ["ls-files", "--stage", "-z"], budget())
        for item in staged.split(b"\0"):
            if item:
                metadata, name = item.split(b"\t", 1)
                relative = os.fsdecode(name)
                if covered(relative, config):
                    paths.add(relative)
                    stage_entries.append([relative, metadata.decode("ascii")])
        extra = git_run(root, ["ls-files", "--others", "--exclude-standard", "-z"], budget())
        paths.update(os.fsdecode(name) for name in extra.split(b"\0") if name and covered(os.fsdecode(name), config))
        try:
            head = git_run(root, ["rev-parse", "HEAD"], budget()).decode().strip()
        except HookError:
            head = None  # An unborn repo is supported; inventory errors above still fail.
    else:
        def walk_error(exc):
            raise HookError("Cannot inventory source: %s" % exc)
        for source in config["source_roots"]:
            top = inside(root, source)
            if not top.exists():
                paths.add(Path(source).as_posix())
                continue
            if top.is_file():
                relative = top.relative_to(root).as_posix()
                if covered(relative, config):
                    paths.add(relative)
                continue
            for directory, subdirs, files in os.walk(top, followlinks=False, onerror=walk_error):
                budget()
                kept = []
                for name in subdirs:
                    child = Path(directory) / name
                    rel = child.relative_to(root).as_posix()
                    if excluded(rel, config):
                        continue
                    if child.is_symlink():
                        paths.add(rel)
                    else:
                        kept.append(name)
                subdirs[:] = kept
                for name in files:
                    relative = (Path(directory) / name).relative_to(root).as_posix()
                    if covered(relative, config):
                        paths.add(relative)
                if len(paths) > config["max_files"]:
                    raise HookError("Source file limit exceeded; narrow source_roots")
    budget()
    if len(paths) > config["max_files"]:
        raise HookError("Source file limit exceeded; narrow source_roots")
    records = {}
    total = 0
    for relative in sorted(paths):
        budget()
        # Validate lexical ancestry; hash a symlink itself, never its target's bytes.
        lexical = root / relative
        inside(root, str(Path(relative).parent))
        try:
            info = lexical.lstat()
        except FileNotFoundError:
            records[relative] = {"kind": "missing"}
            continue
        if stat.S_ISLNK(info.st_mode):
            records[relative] = {"kind": "symlink", "sha256": digest(os.fsencode(os.readlink(lexical)))}
        elif stat.S_ISREG(info.st_mode):
            if info.st_size > config["max_file_bytes"]:
                raise HookError("Source file too large for configured coverage: %s" % relative)
            total += info.st_size
            if total > config["max_total_bytes"]:
                raise HookError("Source byte limit exceeded; narrow source_roots")
            with lexical.open("rb") as handle:
                data = handle.read(config["max_file_bytes"] + 1)
            after = lexical.stat()
            if len(data) > config["max_file_bytes"] or (info.st_size, info.st_mtime_ns) != (after.st_size, after.st_mtime_ns):
                raise HookError("Source changed during snapshot: %s" % relative)
            records[relative] = {"kind": "file", "sha256": digest(data), "executable": bool(info.st_mode & 0o111)}
        else:
            raise HookError("Unsupported source entry (including submodules/directories): %s" % relative)
    budget()
    # Configuration always participates, even if source_roots is only ["src"].
    value = {"files": records, "index": sorted(stage_entries), "config": config,
             "hook_sha256": digest(Path(__file__).read_bytes())}
    return {"fingerprint": digest(canonical(value)), "files": records, "mode": mode,
            "head": head, "time": now(), "file_count": len(records), "byte_count": total,
            "source_roots": config["source_roots"], "exclude_globs": config["exclude_globs"]}


def file_hash(path):
    if not path.is_file():
        return None
    return digest(path.read_bytes())


def session_path(root, session):
    if not isinstance(session, str) or not session.strip() or len(session) > 1024:
        raise HookError("A real nonempty session_id is required")
    # Never interpolate session IDs or user input into a filesystem path.
    relative = RUNTIME + "/" + digest(session.encode())[:24]
    target = inside(root, relative)
    target.mkdir(parents=True, exist_ok=True)
    return target


@contextlib.contextmanager
def lock(folder):
    path = folder / ".lock"
    started = time.monotonic()
    while True:
        try:
            descriptor = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
            os.write(descriptor, (str(os.getpid()) + "\n").encode())
            os.close(descriptor)
            break
        except FileExistsError:
            if time.monotonic() - started > 2:
                raise HookError("Session record busy or stale lock; inspect owning process before removing %s" % path)
            time.sleep(0.05)
    try:
        yield
    finally:
        path.unlink(missing_ok=True)


def baseline(folder):
    path = folder / "baseline.json"
    if not path.exists():
        raise HookError("No session baseline; run SessionStart in the actual session before work")
    return read_json(path)


def context_bundle(root, config):
    chunks, receipts, issues = [], [], []
    for required in (".claude/ad-system/BOOTSTRAP.md", ".claude/ad-system/OPERATING_PROTOCOL.md"):
        if not inside(root, required).is_file():
            issues.append("Missing operating instruction: " + required)
    used = 0
    for relative in config["startup_files"]:
        path = inside(root, relative)
        if not path.is_file():
            issues.append("Missing startup file: " + relative)
            continue
        if path.stat().st_size + used > config["startup_budget_bytes"]:
            issues.append("Startup budget exceeded: read the complete file explicitly: " + relative)
            continue
        with path.open("rb") as handle:
            data = handle.read(config["startup_budget_bytes"] - used + 1)
        if used + len(data) > config["startup_budget_bytes"]:
            issues.append("Startup file grew beyond budget: read explicitly: " + relative)
            continue
        receipts.append({"path": relative, "sha256": digest(data), "bytes": len(data)})
        used += len(data)
        chunks.append("--- " + relative + " ---\n" + data.decode("utf-8", errors="replace"))
    return chunks, receipts, issues


def emit_context(event, text):
    print(json.dumps({"hookSpecificOutput": {"hookEventName": event, "additionalContext": text}}))


def start(root, config, payload):
    session = payload.get("session_id")
    folder = session_path(root, session)
    chunks, receipts, issues = context_bundle(root, config)
    with lock(folder):
        path = folder / "baseline.json"
        if not path.exists():
            snap = snapshot(root, config)
            write_json(path, {"version": VERSION, "session_id": session, "repo": str(root),
                              "source": snap, "progress_hash": file_hash(root / "PROGRESS.md"), "time": now()})
        original = baseline(folder)
        previous = read_json(folder / "outcome.json") if (folder / "outcome.json").exists() else None
        write_json(folder / "context.json", {"time": now(), "source": payload.get("source", "unknown"),
                                            "prepared_files": receipts, "issues": issues,
                                            "delivery": "prepared for hook output; verify actual runtime injection separately"})
    text = "Aetherial Dawn hook v%s\nRepo: %s\nSession ID: %s\nBaseline preserved: %s\n" % (
        VERSION, root, session, original["source"]["fingerprint"])
    text += "Read .claude/ad-system/BOOTSTRAP.md and follow the operating protocol. This payload records prepared context; it does not prove comprehension.\n"
    if previous:
        text += "Prior outcome: " + previous["status"] + "\n"
    if issues:
        text += "CONTEXT INCOMPLETE: " + "; ".join(issues) + "\n"
    emit_context("SessionStart", text + "\n" + "\n".join(chunks))
    return 0


def latest_checks(folder):
    results = {}
    for path in sorted((folder / "checks").glob("*.json")):
        record = read_json(path)
        key = record["check_id"]
        if key not in results or record["finished_ns"] > results[key]["finished_ns"]:
            results[key] = record
    return results


def check_issues(config, snap, folder, force=False):
    required = [c for c in config["checks"] if c["required"]]
    if (force or config["require_check_for_source_change"]) and not required:
        return ["No required checks are configured; discover actual commands before claiming completion"]
    results, issues = latest_checks(folder), []
    for check in required:
        record = results.get(check["id"])
        if not record:
            issues.append("Missing required check: " + check["id"])
        elif record["status"] != "PASS":
            issues.append("Required check %s: %s" % (check["id"], record["status"]))
        elif record["source_before"] != snap["fingerprint"] or record["source_after"] != snap["fingerprint"]:
            issues.append("Stale check for current source: " + check["id"])
        else:
            log_path = folder / "checks" / record["log_file"]
            if file_hash(log_path) != record.get("log_sha256"):
                issues.append("Missing or changed check log: " + check["id"])
    return issues


def validate_handoff(value):
    keys = {"task", "summary", "scope", "delivery_status", "changed_paths", "verification",
            "limitations", "next_action", "runtime_verification"}
    if not isinstance(value, dict) or set(value) != keys:
        raise HookError("Handoff must contain exactly the template fields")
    for key in ("task", "summary", "scope", "next_action"):
        if not isinstance(value[key], str) or not value[key].strip():
            raise HookError("Handoff needs nonempty " + key)
    if value["delivery_status"] not in STATUSES:
        raise HookError("Unrecognized handoff delivery_status")
    for key in ("changed_paths", "verification", "limitations"):
        if not isinstance(value[key], list) or not all(isinstance(x, str) and x.strip() for x in value[key]):
            raise HookError("Handoff %s must be an array of nonempty strings" % key)
    if value["delivery_status"] != "COMPLETE_WITHIN_SCOPE" and not value["limitations"]:
        raise HookError("An incomplete handoff must identify its limitations")
    runtime = value["runtime_verification"]
    if not isinstance(runtime, dict) or set(runtime) != {"status", "evidence", "reason"}:
        raise HookError("runtime_verification needs status, evidence, reason")
    if runtime["status"] not in {"not_applicable", "pending", "verified"}:
        raise HookError("Invalid runtime verification status")
    if not isinstance(runtime["reason"], str) or not runtime["reason"].strip():
        raise HookError("Runtime verification needs an explicit reason/scope")
    if not isinstance(runtime["evidence"], list) or not all(isinstance(x, str) and x for x in runtime["evidence"]):
        raise HookError("Runtime evidence must be an array of references")
    if runtime["status"] == "verified" and not runtime["evidence"]:
        raise HookError("Verified runtime behavior needs evidence references; references still require review")
    if value["delivery_status"] == "COMPLETE_WITHIN_SCOPE" and runtime["status"] == "pending":
        raise HookError("Pending runtime verification cannot be COMPLETE_WITHIN_SCOPE")
    if value["delivery_status"] == "NEEDS_GAME_CHECK" and runtime["status"] != "pending":
        raise HookError("NEEDS_GAME_CHECK must mark runtime verification pending")


def handoff_template():
    return {"task": "Describe the requested task", "summary": "Describe the actual result",
            "scope": "Describe exactly what completion covers", "delivery_status": "IN_PROGRESS",
            "changed_paths": [], "verification": [], "limitations": ["Replace with actual remaining work"],
            "next_action": "Describe the next concrete action",
            "runtime_verification": {"status": "pending", "evidence": [],
                                     "reason": "Replace with why a runtime check is needed or out of scope"}}


def record_handoff(root, config, session, input_file):
    value = read_json(Path(input_file))
    validate_handoff(value)
    folder = session_path(root, session)
    snap = snapshot(root, config)
    with lock(folder):
        original = baseline(folder)
        previous = read_json(folder / "handoff.json") if (folder / "handoff.json").exists() else None
        progress = file_hash(root / "PROGRESS.md")
        reference = previous if previous else {"source_fingerprint": original["source"]["fingerprint"],
                                               "progress_hash": original["progress_hash"]}
        if snap["fingerprint"] != reference["source_fingerprint"] and config["require_progress_update"]:
            if progress is None or progress == reference["progress_hash"]:
                raise HookError("Changed source needs a new accurate PROGRESS.md update before the handoff")
        if value["delivery_status"] == "COMPLETE_WITHIN_SCOPE" and snap["fingerprint"] != original["source"]["fingerprint"]:
            issues = check_issues(config, snap, folder)
            if issues:
                raise HookError("Cannot record complete handoff: " + "; ".join(issues))
        record = {"time": now(), "source_fingerprint": snap["fingerprint"], "progress_hash": progress,
                  "handoff": value, "manual_evidence": "author supplied; references need review"}
        write_json(folder / "handoff.json", record)
    print(json.dumps({"status": value["delivery_status"], "record": str(folder / "handoff.json"),
                      "source_fingerprint": snap["fingerprint"]}))
    return 0


def evaluate(root, config, session, gate=False):
    folder = session_path(root, session)
    original = baseline(folder)
    snap = snapshot(root, config)
    changed = snap["fingerprint"] != original["source"]["fingerprint"]
    context_issues = context_bundle(root, config)[2]
    issues = list(context_issues)
    record = read_json(folder / "handoff.json") if (folder / "handoff.json").exists() else None
    handoff = None
    if changed or gate:
        if not record:
            issues.append("Missing structured handoff")
        elif record["source_fingerprint"] != snap["fingerprint"]:
            issues.append("Handoff is stale for current source")
        elif record["progress_hash"] != file_hash(root / "PROGRESS.md"):
            issues.append("PROGRESS.md changed after the recorded handoff; reconcile and record again")
        else:
            validate_handoff(record["handoff"])
            handoff = record["handoff"]
        issues += check_issues(config, snap, folder, force=gate)
    status = "NO_SOURCE_CHANGE" if not changed and not gate else "INCOMPLETE"
    if handoff:
        if handoff["delivery_status"] == "COMPLETE_WITHIN_SCOPE" and not issues:
            status = "COMPLETE_WITHIN_SCOPE"
        elif handoff["delivery_status"] != "COMPLETE_WITHIN_SCOPE":
            status = handoff["delivery_status"]
    return folder, {"time": now(), "status": status, "source_fingerprint": snap["fingerprint"],
                    "source_changed_since_start": changed, "issues": issues,
                    "scope": handoff["scope"] if handoff else None,
                    "prior_handoff_status": record["handoff"]["delivery_status"] if record else None,
                    "manual_evidence": "not independently validated by this script"}


def stop(root, config, payload):
    folder, result = evaluate(root, config, payload.get("session_id"))
    can_finish = result["status"] in {"COMPLETE_WITHIN_SCOPE", "BLOCKED", "NEEDS_GAME_CHECK", "IN_PROGRESS", "NO_SOURCE_CHANGE"}
    if result["status"] == "NO_SOURCE_CHANGE" and result["issues"]:
        can_finish = False
        result["status"] = "INCOMPLETE"
    with lock(folder):
        write_json(folder / "outcome.json", result)
    if not can_finish and payload.get("stop_hook_active") is not True:
        print(json.dumps({"decision": "block", "reason":
            "Aetherial Dawn handoff incomplete: " + "; ".join(result["issues"]) +
            ". Complete appropriate checks and record a handoff, or record BLOCKED/NEEDS_GAME_CHECK/IN_PROGRESS honestly. "
            "Do not invent passing evidence. This hook permits only one corrective continuation."}))
    else:
        print(json.dumps({"systemMessage": "Aetherial Dawn: " + result["status"] +
                         ("; " + "; ".join(result["issues"]) if result["issues"] else "") +
                         ("; prior handoff: " + result["prior_handoff_status"] if result["prior_handoff_status"] else "")}))
    return 0


def run_check(root, config, folder, check):
    before = snapshot(root, config)
    checks_dir = folder / "checks"
    checks_dir.mkdir(parents=True, exist_ok=True)
    token = uuid.uuid4().hex
    log_path = checks_dir / (token + ".log")
    status, code, error = "ERROR", None, None
    start_time = now()
    process = None
    # Invalidate an earlier pass BEFORE launching. Cancellation/crash must not resurrect it.
    write_json(checks_dir / (token + ".json"), {
        "check_id": check["id"], "status": "RUNNING", "started": start_time,
        "finished": None, "finished_ns": time.time_ns(), "source_before": before["fingerprint"],
        "source_after": None, "argv": check["argv"], "cwd": check["cwd"],
        "log_file": log_path.name, "log_sha256": None, "exit_code": None,
        "error": "No completed result yet; interrupted attempts do not count as passing"})
    try:
        with log_path.open("wb") as output:
            process = subprocess.Popen(check["argv"], cwd=str(inside(root, check["cwd"])),
                                       stdin=subprocess.DEVNULL, stdout=output, stderr=subprocess.STDOUT,
                                       start_new_session=(os.name != "nt"), shell=False)
            try:
                code = process.wait(timeout=check["timeout_seconds"])
                status = "PASS" if code == 0 else "FAIL"
            except subprocess.TimeoutExpired:
                status = "TIMEOUT"
                if os.name == "nt":
                    # taskkill terminates the process tree; direct kill is a fallback.
                    try:
                        subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
                    except (OSError, subprocess.TimeoutExpired):
                        process.kill()
                else:
                    try:
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                code = process.wait(timeout=5)
    except (OSError, subprocess.TimeoutExpired) as exc:
        status, error = "ERROR", str(exc)
    finally:
        # A normal interrupt of this wrapper should not leave the check running.
        # SIGKILL/power loss cannot run finally; the RUNNING receipt still invalidates old passes.
        if process is not None and process.poll() is None:
            if os.name == "nt":
                try:
                    subprocess.run(["taskkill", "/PID", str(process.pid), "/T", "/F"],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
                except (OSError, subprocess.TimeoutExpired):
                    process.kill()
            else:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            process.wait(timeout=5)
    after = None
    try:
        after = snapshot(root, config)["fingerprint"]
        if status == "PASS" and before["fingerprint"] != after:
            status = "SOURCE_CHANGED"
    except HookError as exc:
        status, error = "ERROR", str(exc)
    record = {"check_id": check["id"], "argv": check["argv"], "cwd": check["cwd"],
              "started": start_time, "finished": now(), "finished_ns": time.time_ns(),
              "status": status, "exit_code": code, "error": error,
              "source_before": before["fingerprint"], "source_after": after,
              "log_file": log_path.name, "log_sha256": file_hash(log_path),
              "environment": {"platform": sys.platform, "python": sys.version.split()[0]},
              "coverage": {"mode": before["mode"], "source_roots": before["source_roots"],
                           "exclude_globs": before["exclude_globs"]}}
    write_json(checks_dir / (token + ".json"), record)
    return {"check_id": check["id"], "status": status, "record": str(checks_dir / (token + ".json")),
            "log": str(log_path), "error": error}


def verify(root, config, session, requested):
    folder = session_path(root, session)
    baseline(folder)
    selected = [c for c in config["checks"] if not requested or c["id"] in requested]
    if not selected or (requested and set(requested) != {c["id"] for c in selected}):
        raise HookError("No matching checks; configure actual commands and use their IDs")
    # Serialize checks for this session. Separate sessions still need separate worktrees.
    with lock(folder):
        results = [run_check(root, config, folder, check) for check in selected]
    print(json.dumps({"results": results}))
    return 0 if all(r["status"] == "PASS" for r in results) else 2


def doctor(root, config):
    snap = snapshot(root, config)
    _, _, issues = context_bundle(root, config)
    if config["require_check_for_source_change"] and not any(c["required"] for c in config["checks"]):
        issues.append("No required verification checks configured")
    print(json.dumps({"version": VERSION, "repo": str(root), "status": "READY_FOR_RUNTIME_TEST" if not issues else "NOT_READY",
                      "issues": issues, "snapshot": snap, "runtime_activation": "not tested by doctor"}))
    return 2 if issues else 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", help="Explicit installed project root; defaults to the script's repo")
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("session-start", "stop", "doctor", "handoff-template"):
        sub.add_parser(name)
    for name in ("verify", "handoff", "gate"):
        part = sub.add_parser(name)
        part.add_argument("--session", required=True, help="Exact ID supplied by SessionStart")
        if name == "verify":
            part.add_argument("--check", action="append", help="Check ID; repeat, or omit for all configured checks")
        if name == "handoff":
            part.add_argument("--file", required=True, help="Completed JSON template, preferably within ignored runtime")
    args = parser.parse_args(argv)
    payload = {}
    try:
        root = Path(args.repo).resolve() if args.repo else Path(__file__).resolve().parents[2]
        if args.command == "handoff-template":
            print(json.dumps(handoff_template(), indent=2))
            return 0
        if args.command in {"session-start", "stop"}:
            raw = sys.stdin.read(2 * 1024 * 1024)
            payload = json.loads(raw)
            if not isinstance(payload, dict):
                raise HookError("Hook input must be a JSON object")
        config = load_config(root)
        if args.command == "session-start":
            return start(root, config, payload)
        if args.command == "stop":
            return stop(root, config, payload)
        if args.command == "doctor":
            return doctor(root, config)
        if args.command == "verify":
            return verify(root, config, args.session, args.check)
        if args.command == "handoff":
            return record_handoff(root, config, args.session, args.file)
        if args.command == "gate":
            folder, result = evaluate(root, config, args.session, gate=True)
            write_json(folder / "gate.json", result)
            print(json.dumps(result))
            return 0 if result["status"] == "COMPLETE_WITHIN_SCOPE" and not result["issues"] else 2
    except (HookError, OSError, ValueError, KeyError, TypeError) as exc:
        message = "Aetherial Dawn INCOMPLETE: " + str(exc)
        if args.command == "session-start":
            emit_context("SessionStart", message + ". Startup is not verified; inspect configuration and missing context.")
            return 0
        if args.command == "stop":
            if isinstance(payload, dict) and payload.get("stop_hook_active") is not True:
                print(json.dumps({"decision": "block", "reason": message + ". Correct once or report the blocker honestly."}))
            else:
                print(json.dumps({"systemMessage": message + ". Allowing an incomplete stop; no success is certified."}))
            return 0
        print(json.dumps({"status": "ERROR", "error": message}))
        return 2
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
