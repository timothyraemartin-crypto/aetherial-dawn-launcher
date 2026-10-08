import json, os, shutil, subprocess, sys, tempfile, unittest

HOOK = os.path.join(os.path.dirname(__file__), "..", "ad_hook.py")
CFG = {"source_roots": ["src", "pkg.json"], "exclude_dirs": ["node_modules", "runtime"], "exclude_files": [],
       "inject": ["PROGRESS.md", "INDEX.md"],
       "limits": {"max_files": 50, "max_total_bytes": 100000, "max_file_bytes": 50000, "max_seconds": 10, "inject_budget_bytes": 4000},
       "checks": [{"id": "ok", "argv": [sys.executable, "-c", "pass"], "required": True},
                  {"id": "bad", "argv": [sys.executable, "-c", "raise SystemExit(3)"]},
                  {"id": "slow", "argv": [sys.executable, "-c", "import time; time.sleep(5)"], "timeout": 1},
                  {"id": "nolaunch", "argv": ["/nonexistent/binary"]},
                  {"id": "mutates", "argv": [sys.executable, "-c", "open('src/a.txt','a').write('x')"]}]}


class T(unittest.TestCase):
    def setUp(self):
        self.d = tempfile.mkdtemp()
        os.makedirs(self.d + "/.claude/ad-hook"); os.makedirs(self.d + "/src")
        self.cfg(CFG)
        for f, c in (("src/a.txt", "a"), ("pkg.json", "{}"), ("PROGRESS.md", "p"), ("INDEX.md", "i")):
            self.w(f, c)
        self.git("init", "-q"); self.git("add", "."); self.git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "i")
        self.env = dict(os.environ, CLAUDE_PROJECT_DIR=self.d); self.env.pop("AD_HOOK_SESSION", None)

    def tearDown(self): shutil.rmtree(self.d)
    def git(self, *a): subprocess.run(["git", "-C", self.d] + list(a), check=True, capture_output=True)
    def w(self, f, c):
        with open(os.path.join(self.d, f), "w") as h: h.write(c)
    def cfg(self, c): self.w(".claude/ad-hook/config.json", json.dumps(c))

    def hook(self, *a, inp=None):
        r = subprocess.run([sys.executable, HOOK] + list(a), input=json.dumps(inp) if inp is not None else "",
                           capture_output=True, text=True, env=self.env, cwd=self.d)
        out = r.stdout.strip().splitlines()
        try: j = json.loads(out[-1]) if out else {}
        except ValueError: j = {}
        return r.returncode, j, r.stdout

    def start(self, sid="s1", source="startup"): return self.hook("start", inp={"session_id": sid, "source": source})
    def stop(self, sid="s1", active=False): return self.hook("stop", inp={"session_id": sid, "stop_hook_active": active})[1]
    def handoff(self, sid="s1", state="COMPLETE_WITHIN_SCOPE", **kw):
        a = ["handoff", "--session", sid, "--state", state, "--objective", "o", "--summary", "s", "--evidence", "e", "--next", "n", "--shared-sync", "n/a"]
        if state != "COMPLETE_WITHIN_SCOPE": a += ["--unresolved", "u"]
        return self.hook(*a)

    def test_start_injects_and_baselines(self):
        _, j, _ = self.start()
        ctx = j["hookSpecificOutput"]["additionalContext"]
        self.assertIn("Baseline recorded", ctx); self.assertIn("PROGRESS.md sha256:", ctx)

    def test_read_only_not_forced(self):
        self.start(); self.assertEqual(self.stop(), {})

    def test_change_requires_progress_and_handoff_then_bounded(self):
        self.start(); self.w("src/a.txt", "b")
        self.assertEqual(self.stop()["decision"], "block")
        self.assertNotIn("decision", self.stop(active=True))  # bounded, INCOMPLETE
        self.assertIn("INCOMPLETE", open(self.d + "/.claude/runtime/ad-hook/sessions/s1/outcome.json").read())

    def test_detects_added_deleted_renamed_staged_committed(self):
        self.start()
        self.w("src/new.txt", "n"); os.remove(self.d + "/src/a.txt"); self.git("add", "-A")
        self.git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "c")  # committed: still detected
        _, _, out = self.hook("status", "--session", "s1")
        self.assertIn('"added": ["src/new.txt"]', out); self.assertIn('"deleted": ["src/a.txt"]', out)

    def test_dirty_start_is_baseline(self):
        self.w("src/a.txt", "dirty"); self.start(); self.assertEqual(self.stop(), {})

    def test_complete_path_and_stale_check(self):
        self.start(); self.w("src/a.txt", "b"); self.w("PROGRESS.md", "p2")
        self.assertEqual(self.hook("run", "ok", "--session", "s1")[0], 0)
        self.assertEqual(self.handoff()[0], 0); self.assertEqual(self.stop(), {})
        self.assertEqual(self.hook("gate", "--session", "s1")[0], 0)
        self.w("src/a.txt", "c")  # covered change invalidates PASS and handoff
        self.assertEqual(self.hook("gate", "--session", "s1")[0], 1)

    def test_complete_without_required_check_blocks(self):
        self.start(); self.w("src/a.txt", "b"); self.w("PROGRESS.md", "p2"); self.handoff()
        self.assertIn("required check", self.stop()["reason"])

    def test_failing_checks_not_pass(self):
        self.start()
        for cid, st in (("bad", "FAIL"), ("slow", "TIMEOUT"), ("nolaunch", "ERROR"), ("mutates", "SOURCE_CHANGED")):
            self.assertEqual(self.hook("run", cid, "--session", "s1")[0], 1)
            r = json.load(open(self.d + "/.claude/runtime/ad-hook/sessions/s1/checks/%s.json" % cid))
            self.assertEqual(r["status"], st, cid)

    def test_manual_check_states_stop_honestly_but_fail_gate(self):
        self.start(); self.w("src/a.txt", "b"); self.w("PROGRESS.md", "p2")
        self.assertEqual(self.handoff(state="NEEDS_GAME_CHECK")[0], 0)
        self.assertIn("NEEDS_GAME_CHECK", self.stop()["systemMessage"])
        self.assertEqual(self.hook("gate", "--session", "s1")[0], 1)

    def test_invalid_config_and_missing_baseline_visible(self):
        self.w(".claude/ad-hook/config.json", "{nope")
        _, j, _ = self.start(); self.assertIn("INCOMPLETE", j["systemMessage"])
        self.assertIn("INCOMPLETE", self.stop()["systemMessage"])
        self.cfg(CFG); self.assertIn("no baseline", self.stop("never-started")["systemMessage"])

    def test_resume_keeps_baseline(self):
        self.start(); self.w("src/a.txt", "b")
        _, j, _ = self.start(source="resume")
        ctx = j["hookSpecificOutput"]["additionalContext"]
        self.assertIn("original baseline kept", ctx); self.assertIn("~1", ctx)

    def test_sessions_separate(self):
        self.start("s1"); self.w("src/a.txt", "b"); self.start("s2")
        self.assertEqual(self.stop("s2"), {}); self.assertEqual(self.stop("s1")["decision"], "block")

    def test_limits_fail_snapshot_not_partial(self):
        c = json.loads(json.dumps(CFG)); c["limits"]["max_files"] = 1; self.cfg(c)
        _, j, _ = self.start(); self.assertIn("exceeded", j["systemMessage"])

    def test_path_prefix_exclude(self):
        c = json.loads(json.dumps(CFG)); c["source_roots"].append("docs"); c["exclude_dirs"].append("docs/operations"); self.cfg(c)
        os.makedirs(self.d + "/docs/operations"); self.w("docs/operations/h.md", "1"); self.w("docs/x.md", "1")
        self.start(); self.w("docs/operations/h.md", "2"); self.assertEqual(self.stop(), {})
        self.w("docs/x.md", "2"); self.assertEqual(self.stop()["decision"], "block")

    def test_oversize_inject_not_silently_truncated(self):
        self.w("PROGRESS.md", "x" * 5000)
        _, j, _ = self.start(); self.assertIn("NOT injected", j["systemMessage"])


if __name__ == "__main__":
    unittest.main()
