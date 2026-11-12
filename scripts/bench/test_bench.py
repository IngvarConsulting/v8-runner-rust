"""Checks of scripts/bench that need no platform: `python scripts/bench/test_bench.py`."""
import json
import os
import re
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import bench  # noqa: E402
import describe  # noqa: E402
import executors as ex  # noqa: E402
import flag  # noqa: E402
import report  # noqa: E402

FOREIGN = "unica worker-1 measuring unica#2839"
SIBLING = "v8-runner 89ada31d s1 #574 round"
OURS = "v8-runner 89ada31d s3 #604"


class FlagTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.path = os.path.join(self.dir.name, ".measuring")

    def put(self, text):
        with open(self.path, "w", encoding="utf-8") as f:
            f.write(text + "\n")

    def test_free_flag_is_taken_and_released(self):
        flag.acquire(self.path, OURS, say=lambda _: None)
        self.assertEqual(flag.holder(self.path), OURS)
        self.assertTrue(flag.release(self.path, OURS, say=lambda _: None))
        self.assertFalse(os.path.exists(self.path))

    def test_an_empty_or_blank_flag_is_free(self):
        for left in ("", "  \r\n"):
            with open(self.path, "w", encoding="utf-8") as f:
                f.write(left)
            looks = []
            flag.acquire(self.path, OURS, poll=0, say=looks.append, sleep=lambda _: None)
            self.assertEqual(flag.holder(self.path), OURS)
            self.assertEqual(looks, [], "no waiting on a free flag")
            self.assertTrue(flag.release(self.path, OURS, say=lambda _: None))

    def test_foreign_flag_is_not_overwritten_without_waiting(self):
        for other in (FOREIGN, SIBLING):
            self.put(other)
            with self.assertRaises(flag.FlagBusy):
                flag.acquire(self.path, OURS, wait=False, say=lambda _: None)
            self.assertEqual(flag.holder(self.path), other)

    def test_waits_for_foreign_and_sibling_flags(self):
        for other in (FOREIGN, SIBLING):
            self.put(other)
            looks = []

            def sleep(_):
                # The flag is still the other's while we wait; then its owner removes it.
                looks.append(flag.holder(self.path))
                if len(looks) == 2:
                    os.remove(self.path)

            flag.acquire(self.path, OURS, poll=0, say=lambda _: None, sleep=sleep)
            self.assertEqual(looks, [other, other])
            self.assertEqual(flag.holder(self.path), OURS)
            flag.release(self.path, OURS, say=lambda _: None)

    def test_timeout_leaves_the_foreign_flag(self):
        self.put(FOREIGN)
        with self.assertRaises(flag.FlagBusy):
            flag.acquire(self.path, OURS, poll=0, timeout=0, say=lambda _: None, sleep=lambda _: None)
        self.assertEqual(flag.holder(self.path), FOREIGN)

    def test_a_flag_with_our_text_is_refused_not_taken_over(self):
        self.put(OURS)
        with self.assertRaises(flag.FlagBusy):
            flag.acquire(self.path, OURS, say=lambda _: None, sleep=lambda _: self.fail("must not wait"))
        self.assertEqual(flag.holder(self.path), OURS)

    def test_release_keeps_a_flag_that_is_not_ours(self):
        self.put(FOREIGN)
        self.assertFalse(flag.release(self.path, OURS, say=lambda _: None))
        self.assertEqual(flag.holder(self.path), FOREIGN)

    def test_empty_text_is_refused(self):
        with self.assertRaises(ValueError):
            flag.acquire(self.path, "  ", say=lambda _: None)
        self.assertFalse(os.path.exists(self.path))

    def test_held_releases_on_error_and_keeps_a_replaced_flag(self):
        with self.assertRaises(RuntimeError):
            with flag.Held(self.path, OURS, say=lambda _: None):
                raise RuntimeError("scenario failed")
        self.assertFalse(os.path.exists(self.path))
        with flag.Held(self.path, OURS, say=lambda _: None):
            self.put(FOREIGN)  # somebody replaced it by hand while we measured
        self.assertEqual(flag.holder(self.path), FOREIGN)

    def test_cli_run_waits_for_a_foreign_flag_and_removes_only_its_own(self):
        """`bench.py run` under a foreign flag: waits, never writes over it, then measures under its own."""
        self.put(FOREIGN)
        seen_during_wait = []
        seen_during_run = []

        def owner_finishes():
            time.sleep(0.3)
            seen_during_wait.append(flag.holder(self.path))
            os.remove(self.path)

        threading.Thread(target=owner_finishes).start()
        with mock.patch.object(bench, "run", lambda *a, **k: seen_during_run.append(flag.holder(self.path))):
            with mock.patch("builtins.print"):
                bench.main(["run", "--stand", "ka", "--flag", self.path, "--flag-text", OURS, "--flag-poll", "0.05"])
        self.assertEqual(seen_during_wait, [FOREIGN])
        self.assertEqual(seen_during_run, [OURS])
        self.assertFalse(os.path.exists(self.path))


class DescriptionTest(unittest.TestCase):
    def test_every_stand_serves_every_scenario(self):
        scenarios = describe.load_scenarios()
        self.assertTrue({"push", "pull", "status"} <= {s.kind for s in scenarios.values()})
        for name in describe.stand_names():
            stand = describe.load_stand(name, stands_root=r"X:\stands")
            self.assertEqual(stand.work, os.path.join(r"X:\stands", f"bench-{name}"))
            if stand.created:
                self.assertTrue(stand.project.startswith(stand.work))
                self.assertTrue(stand.infobase.startswith(stand.work))
            for scenario in scenarios.values():
                describe.check_pair(stand, scenario)

    def test_first_set_and_executors(self):
        scenarios = describe.load_scenarios()
        used = {e for s in scenarios.values() for e in s.executors}
        self.assertEqual(used, set(describe.EXECUTORS))
        kinds = {s.modules: s.kind for s in scenarios.values() if s.modules and s.kind != "pair"}
        self.assertEqual(kinds.get("common2"), "push")
        self.assertEqual(kinds.get("form"), "push")
        self.assertEqual(kinds.get("object"), "push")
        self.assertIn("pull", kinds.values())
        self.assertEqual(scenarios["pair-common2"].executors, ["runner-live"])

    def write(self, text):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        path = os.path.join(d.name, "s.toml")
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)
        return path

    def test_bad_scenarios_are_refused(self):
        for text in ('[a]\nkind = "build"\nmodules = "x"\n',
                     '[a]\nkind = "status"\nmodules = "x"\n',
                     '[a]\nkind = "push"\n',
                     '[a]\nkind = "pull"\nmodules = "x"\nexecutors = ["ibcmd"]\n',
                     '[a]\nkind = "push"\nmodules = "x"\nexecutors = ["gate"]\n'):
            with self.assertRaises(describe.DescriptionError, msg=text):
                describe.load_scenarios(self.write(text))


class SetupTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.stand = describe.load_stand("ssl321", stands_root=self.dir.name)
        os.makedirs(self.stand.work)

    def test_a_directory_the_script_did_not_create_is_not_touched(self):
        keep = os.path.join(self.stand.work, "somebody.txt")
        with open(keep, "w") as f:
            f.write("x")
        for recreate in (False, True):
            with self.assertRaises(SystemExit):
                bench.setup(self.stand, "runner.exe", OURS, recreate)
            self.assertTrue(os.path.exists(keep))

    def test_an_unfinished_setup_asks_for_recreate(self):
        bench.write_owner(self.stand.work, OURS, "test", created=True)
        with self.assertRaises(SystemExit) as caught:
            bench.setup(self.stand, "runner.exe", OURS, False)
        self.assertIn("--recreate", str(caught.exception))

    def test_removal_of_an_own_stand_drops_the_owner_file_last(self):
        bench.write_owner(self.stand.work, OURS, "test", created=True)
        os.makedirs(os.path.join(self.stand.work, "ib"))
        with mock.patch.object(bench.shutil, "rmtree", side_effect=PermissionError("base is open")):
            with self.assertRaises(PermissionError):
                bench.remove_own_stand(self.stand.work)
        self.assertTrue(bench.created_by_bench(self.stand.work))

    def test_unknown_stand_name_is_refused(self):
        with self.assertRaises(describe.DescriptionError):
            describe.load_stand(r"..\stands\ka")


class RestoreTest(unittest.TestCase):
    """The stand goes back to the original modules whatever state a failure left."""

    def bench(self):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        stand = describe.load_stand("ssl321", stands_root=d.name)
        b = bench.Bench(stand, "runner.exe", os.path.join(d.name, "r.jsonl"), trace=False, verify=True)
        calls = []
        b.runner = mock.Mock()
        b.runner.must.side_effect = lambda label, *args: (calls.append(args[0]) or ({"seconds": 1.0}, {}))
        self.originals = {"CommonModules/М/Ext/Module.bsl": b"original"}
        return b, calls

    def run_restore(self, b):
        with mock.patch.object(bench, "commit"), mock.patch("builtins.print"):
            b.restore(list(self.originals), self.originals, "s")
        with open(b.project_file(list(self.originals)[0]), "rb") as f:
            self.assertEqual(f.read(), b"original")
        self.assertFalse(b.base_marked or b.needs_resync)

    def test_nothing_reached_the_base(self):
        b, calls = self.bench()
        self.run_restore(b)
        self.assertEqual(calls, [])

    def test_marker_in_the_base_is_pushed_back_even_when_git_is_clean(self):
        b, calls = self.bench()
        b.base_marked = True
        self.run_restore(b)
        self.assertEqual(calls, ["push"])

    def test_a_base_changed_behind_the_memory_is_pulled_first(self):
        b, calls = self.bench()
        b.base_marked = b.needs_resync = True
        self.run_restore(b)
        self.assertEqual(calls, ["pull", "push"])


class CommandTest(unittest.TestCase):
    def test_no_executor_allows_the_dynamic_update(self):
        stand = describe.load_stand("ka", stands_root=r"X:\s")
        push = ex.batch_push_args("stage", "stage.txt")
        self.assertEqual(push[push.index("/UpdateDBCfg") + 1], "-Dynamic-")
        self.assertIn("--dynamic=disable", ex.ibcmd_apply_args(stand, "data"))
        update = dict(ex.agent_push_commands("s", "s.txt"))["update"]
        self.assertTrue(update.endswith("--dynamic-disable"), update)

    def test_reintroduction_guard_every_update_in_the_sources_forbids_dynamic(self):
        for name in ("bench.py", "executors.py"):
            with open(os.path.join(HERE, name), encoding="utf-8") as f:
                code = f.read()
            for m in re.finditer(r'"/UpdateDBCfg"', code):
                self.assertTrue(code[m.end():m.end() + 16].startswith(', "-Dynamic-"'), f"{name}: {code[m.start():m.end() + 30]}")
            for m in re.finditer(r"update-db-cfg", code):
                self.assertIn("--dynamic-disable", code[m.end():m.end() + 25], f"{name}: {code[m.start():m.end() + 30]}")
            for m in re.finditer(r'"config", "apply"', code):
                self.assertIn('"--dynamic=disable"', code[m.end():m.end() + 80], name)

    def test_marker_keeps_the_bom_and_is_found(self):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        for original in (b"\xef\xbb\xbfProcedure A()\r\n", b"Procedure A()\r\n"):
            data = ex.marked(original, "r1 push 1")
            self.assertEqual(data.startswith(b"\xef\xbb\xbf"), original.startswith(b"\xef\xbb\xbf"))
            ex.write_files(d.name, {"CommonModules/М/Ext/Module.bsl": data})
            path = os.path.join(d.name, "CommonModules", "М", "Ext", "Module.bsl")
            self.assertTrue(ex.has_marker(path, "r1 push 1"))
            self.assertFalse(ex.has_marker(path, "r1 push 2"))

    def test_a_description_is_marked_in_its_own_comment(self):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        original = (b"\xef\xbb\xbf<?xml version=\"1.0\"?>\r\n<MetaDataObject><Catalog><Properties>"
                    b"<Comment/></Properties><ChildObjects><Attribute><Comment/></Attribute>"
                    b"</ChildObjects></Catalog></MetaDataObject>\r\n")
        data = ex.marked(original, "r1 pair 1")
        self.assertTrue(data.startswith(b"\xef\xbb\xbf<?xml"))
        self.assertEqual(data.count(b"<Comment/>"), 1, "only the object's own comment is marked")
        ex.write_files(d.name, {"Catalogs/К.xml": data})
        path = os.path.join(d.name, "Catalogs", "К.xml")
        self.assertTrue(ex.has_marker(path, "r1 pair 1"))
        self.assertFalse(ex.has_marker(path, "r1 pair 2"))
        with self.assertRaises(ValueError):
            ex.marked(b"<?xml version=\"1.0\"?><Form/>", "r1 pair 1")

    def test_stage_writes_a_list_file_with_bom(self):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        listing = ex.stage(os.path.join(d.name, "s"), {"A/B.bsl": b"x", "C/D.bsl": b"y"})
        with open(listing, "rb") as f:
            self.assertEqual(f.read(), b"\xef\xbb\xbfA/B.bsl\r\nC/D.bsl\r\n")


class ReportTest(unittest.TestCase):
    def test_medians_skip_warmups_failures_and_unverified(self):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        path = os.path.join(d.name, "r.jsonl")
        base = {"event": "measure", "stand": "ka", "scenario": "push-common2", "ok": True, "verified": True}
        rows = [dict(base, executor="runner", rep=1, seconds=50.0), dict(base, executor="runner", rep=2, seconds=60.0),
                dict(base, executor="runner", rep=3, seconds=55.0),
                dict(base, executor="runner-live", rep=0, warmup=True, seconds=99.0),
                dict(base, executor="runner-live", rep=1, seconds=40.0),
                dict(base, executor="runner-live", rep=2, seconds=41.0, verified=False),
                dict(base, executor="batch", rep=1, ok=False, error="designer rc=1", seconds=None),
                dict(base, executor="agent", rep=1, seconds=30.0, steps={"load": 1.5, "update": 25.0}),
                {"event": "start", "stand": "ka"}]
        with open(path, "w", encoding="utf-8") as f:
            f.write("\n".join(json.dumps(r, ensure_ascii=False) for r in rows) + "\n")
        text = report.table([path], steps=True)
        self.assertIn("| ka | push-common2 | 55,0 (3) | 40,0 (1) | 30,0 (1) |", text)
        self.assertIn("load 1,5, update 25,0", text)
        self.assertIn("runner-live #2: not verified", text)
        self.assertIn("batch #1: designer rc=1", text)

    def test_phases_offsets(self):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        path = os.path.join(d.name, "t.log")
        with open(path, "w", encoding="utf-8") as f:
            f.write("12:24:03.500 DEBUG starting command\n  continuation\n12:24:05.000 DEBUG agent replied\n")
        lines = report.phases(path).splitlines()
        self.assertEqual(len(lines), 2)
        self.assertIn("1.50 +  1.50  DEBUG agent replied", lines[1])


if __name__ == "__main__":
    unittest.main()
