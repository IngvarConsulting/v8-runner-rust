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
                # The pair series (#646) is measured on `ka` and `ssl321` only; release stands
                # (#585) serve the rest.
                if scenario.kind == "pair" and name not in ("ka", "ssl321"):
                    continue
                describe.check_pair(stand, scenario)

    def test_first_set_and_executors(self):
        scenarios = describe.load_scenarios()
        used = {e for s in scenarios.values() for e in s.executors}
        self.assertEqual(used, set(describe.EXECUTORS))
        self.assertEqual({s.kind for s in scenarios.values()}, set(describe.KINDS))
        pairs = {(s.modules, s.kind) for s in scenarios.values() if s.modules}
        self.assertTrue({("common2", "push"), ("form", "push"), ("object", "push"), ("common1", "pull")} <= pairs)
        self.assertEqual(scenarios["pair-common2"].executors, ["runner-live"])

    def test_release_set_of_585(self):
        """#585: status, pull without changes, pull 1, push 1, push after pull, full pull; bare
        Designer, bare agent, runner 0.16, runner 0.17 first command and with its live agent."""
        scenarios = describe.load_scenarios()
        want = {"status": ("status", ""), "pull-none": ("pull-none", ""), "pull-common1": ("pull", "common1"),
                "push-common1": ("push", "common1"), "push-after-pull": ("push-after-pull", "common1"),
                "pull-full": ("pull-full", "")}
        for name, (kind, modules) in want.items():
            self.assertEqual((scenarios[name].kind, scenarios[name].modules), (kind, modules), name)
            self.assertIn("runner", scenarios[name].executors, name)
            self.assertIn("runner-016", scenarios[name].executors, name)
        for name in ("pull-none", "pull-common1", "push-common1", "push-after-pull"):
            self.assertIn("runner-live", scenarios[name].executors, name)
        for name in ("status", "pull-none", "pull-common1", "push-common1", "pull-full"):
            self.assertTrue({"batch", "agent"} <= set(scenarios[name].executors), name)

    def test_release_stands_on_both_platforms(self):
        by = {}
        for name in describe.stand_names():
            stand = describe.load_stand(name, stands_root=r"X:\stands")
            source = stand.template_cf or stand.template_dt or stand.project
            by.setdefault(os.path.dirname(source), set()).add(stand.platform)
        self.assertEqual(by[r"E:\1c\tmplts\1c\accountingkz\3_0_75_1"], {"8.3.27.2342", "8.5.1.1529"})
        # УНФ 3.0.14.187 требует платформу не ниже 8.5.1: стенда на 8.3 у неё нет.
        self.assertEqual(by[r"E:\1c\tmplts\1c\smallbusiness\3_0_14_187"], {"8.5.1.1529"})
        self.assertEqual(by[r"E:\1c\tmplts\1c\ARAutomation20\2_6_2_72"], {"8.5.1.1529"})
        self.assertEqual(describe.load_stand("ka").platform, "8.3.27.2342")

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
                     '[a]\nkind = "pull-none"\nmodules = "x"\n',
                     '[a]\nkind = "pull-full"\nmodules = "x"\n',
                     '[a]\nkind = "push"\n',
                     '[a]\nkind = "push-after-pull"\n',
                     '[a]\nkind = "pull"\nmodules = "x"\nexecutors = ["ibcmd"]\n',
                     '[a]\nkind = "push-after-pull"\nmodules = "x"\nexecutors = ["batch"]\n',
                     '[a]\nkind = "pull-none"\nexecutors = ["ibcmd"]\n',
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


class Runner016Test(unittest.TestCase):
    """The runner-016 executor: another runner binary on its own project over the same base."""

    EXE = r"X:\bin-0.16\v8-runner.exe"

    def bench(self):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        stand = describe.load_stand("ssl321", stands_root=d.name)
        b = bench.Bench(stand, "runner.exe", os.path.join(d.name, "r.jsonl"), trace=False, verify=False,
                        runner_016=self.EXE)
        return stand, b

    def test_runner_016_works_in_its_own_project(self):
        stand, b = self.bench()
        r = b.runner_for("runner-016")
        self.assertEqual(r.exe, self.EXE)
        self.assertEqual(r.stand.project, os.path.join(stand.work, "project-016"))
        self.assertEqual(r.stand.infobase, stand.infobase)
        self.assertEqual(r.base_cmd()[:3], [self.EXE, "--config", os.path.join(stand.project_016, "v8project.yaml")])
        self.assertIs(b.runner_for("runner"), b.runner)
        self.assertIs(b.runner_for("runner-live"), b.runner)

    def test_without_the_binary_runner_016_is_left_out_unless_named(self):
        stand = describe.load_stand("ssl321", stands_root=r"X:\s")
        scenarios = [describe.load_scenarios()["pull-common1"]]
        plan = bench.plan_of(stand, scenarios, None, None)
        self.assertNotIn("runner-016", plan[0][1])
        self.assertIn("runner-016", bench.plan_of(stand, scenarios, None, self.EXE)[0][1])
        with self.assertRaises(SystemExit):
            bench.plan_of(stand, scenarios, ["runner-016"], None)

    def test_cli_passes_the_binary_to_run_and_setup(self):
        seen = {}
        with mock.patch.object(bench, "run", lambda *a, **k: seen.setdefault("run", a)), \
                mock.patch.object(bench, "setup", lambda *a, **k: seen.setdefault("setup", a)), \
                mock.patch.object(bench.flag, "Held", mock.MagicMock()):
            bench.main(["run", "--stand", "ka", "--flag-text", OURS, "--runner-016", self.EXE])
            bench.main(["setup", "--stand", "ka", "--flag-text", OURS, "--runner-016", self.EXE])
        self.assertEqual(seen["run"][-1], self.EXE)
        self.assertEqual(seen["setup"][-1], self.EXE)

    def test_an_existing_stand_gets_only_project_016(self):
        stand = describe.load_stand("ka", stands_root=r"X:\s")
        with self.assertRaises(SystemExit):
            bench.setup(stand, "runner.exe", OURS, False)
        with mock.patch.object(bench, "setup_016") as setup_016:
            bench.setup(stand, "runner.exe", OURS, False, self.EXE)
        setup_016.assert_called_once_with(stand, self.EXE, OURS)

    def test_push_by_runner_016_lands_in_its_project_and_the_stand_runner_catches_up(self):
        stand, b = self.bench()
        b.runner, b.runner_016 = mock.Mock(), mock.Mock(stand=b.runner_016.stand)
        b.runner_016.run.return_value = ({"seconds": 1.0, "rc": 0}, {})
        b.runner.must.return_value = ({"seconds": 1.0}, {})
        rel = "CommonModules/М/Ext/Module.bsl"
        scn = describe.load_scenarios()["push-common1"]
        with mock.patch.object(bench, "commit"), mock.patch("builtins.print"):
            b.push_runner(scn, "runner-016", [rel], {rel: b"original"}, "m1")
        self.assertTrue(ex.has_marker(os.path.join(b.runner_016.stand.src, *rel.split("/")), "m1"))
        self.assertFalse(os.path.exists(b.project_file(rel)))
        self.assertEqual(b.runner_016.run.call_args.args[1], "push")
        self.assertEqual([c.args[1] for c in b.runner.must.call_args_list], ["pull"])
        self.assertTrue(b.base_marked)
        self.assertFalse(b.needs_resync)

    def test_restore_after_runner_016_syncs_its_project(self):
        stand, b = self.bench()
        b.runner, b.runner_016 = mock.Mock(), mock.Mock(stand=b.runner_016.stand)
        b.runner.must.return_value = ({"seconds": 1.0}, {})
        b.runner_016.must.return_value = ({"seconds": 1.0}, {})
        b.base_marked = b.used_016 = True
        originals = {"CommonModules/М/Ext/Module.bsl": b"original"}
        with mock.patch.object(bench, "commit"), mock.patch("builtins.print"):
            b.restore(list(originals), originals, "s")
        self.assertEqual([c.args[1] for c in b.runner.must.call_args_list], ["push"])
        self.assertEqual([c.args[1] for c in b.runner_016.must.call_args_list], ["pull"])
        self.assertFalse(b.used_016)

    def test_runner_016_is_measured_after_its_sync_with_no_agent_left(self):
        """The untimed sync pull must not leave a warm agent for the timed command."""
        stand, b = self.bench()
        order = []
        b.runner = mock.Mock()
        b.runner.stop_agents.side_effect = lambda: order.append("stop")
        b.sync_016 = lambda: order.append("sync")
        b.restore = lambda *a: None
        b.status_runner = lambda *a: order.append("measure") or {"seconds": 1.0, "verified": True}
        scn = describe.load_scenarios()["status"]
        with mock.patch("builtins.print"):
            b.run_scenario(scn, ["runner-016"], 1)
        self.assertEqual(order[:4], ["stop", "sync", "stop", "measure"])

    def test_every_kind_has_its_measure(self):
        for kind in describe.KINDS:
            for executor in describe.EXECUTORS:
                if kind in describe.CAN[executor]:
                    family = "runner" if executor in describe.RUNNERS else "other"
                    self.assertTrue(callable(getattr(bench.Bench, f"{kind.replace('-', '_')}_{family}", None)),
                                    f"{kind} {executor}")

    def test_dumped_files_ignore_the_version_file(self):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        self.assertEqual(ex.dumped_files(os.path.join(d.name, "absent")), [])
        ex.write_files(d.name, {"ConfigDumpInfo.xml": b"x"})
        self.assertEqual(ex.dumped_files(d.name), [])
        ex.write_files(d.name, {"CommonModules/М/Ext/Module.bsl": b"x"})
        self.assertEqual(ex.dumped_files(d.name), ["CommonModules/М/Ext/Module.bsl"])


class FreeBaseTest(unittest.TestCase):
    """Reintroduction guard (#585, #644): the live agent of the runner under test stays connected
    to the base between its commands. Every step of another tool on the same base — batch
    Designer, ibcmd, the bare agent, runner 0.16 — must come after that agent was released or
    stopped, never right after a command of the runner under test."""

    EXE = r"X:\bin-0.16\v8-runner.exe"

    def bench(self):
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        stand = describe.load_stand("ssl321", stands_root=d.name)
        b = bench.Bench(stand, "runner.exe", os.path.join(d.name, "r.jsonl"), trace=False, verify=True,
                        runner_016=self.EXE)
        for files in stand.modules.values():
            ex.write_files(stand.src, {rel: b"original" for rel in files})
        events, rows = [], []
        b.record = lambda **row: rows.append(row)

        def runner(name, payload=None):
            def call(label, *args):
                events.append((name, " ".join(args[:1])))
                return {"seconds": 1.0, "rc": 0, "live": []}, payload or {"changed_files": 0}
            return call

        b.runner = mock.Mock(stand=stand)
        b.runner.run.side_effect = b.runner.must.side_effect = runner("runner")
        b.runner.release_agents.side_effect = lambda: events.append(("free", "release"))
        b.runner.stop_agents.side_effect = lambda: events.append(("free", "stop"))
        b.runner_016 = mock.Mock(stand=b.runner_016.stand)
        b.runner_016.run.side_effect = b.runner_016.must.side_effect = runner("other", None)

        def designer(stand_, args, out):
            events.append(("other", f"designer {args[0]}"))
            if "-configDumpInfoOnly" in args:
                ex.write_files(args[1], {"ConfigDumpInfo.xml": b"x"})
            if "-getChanges" in args:
                ex.write_files(os.path.dirname(args[args.index("-getChanges") + 1]), {"changes.txt": b""})
            return 1.0

        def ibcmd(stand_, args):
            events.append(("other", f"ibcmd {' '.join(args[:4])}"))
            for arg in args:
                if arg.startswith("--out="):
                    ex.write_files(os.path.dirname(arg[6:]), {os.path.basename(arg[6:]): b""})
            return 1.0

        def bare_agent_session(stand_, base_dir, prepare, commands):
            events.append(("other", "bare agent"))
            user = os.path.join(base_dir, "0")
            prepare(user)
            ex.write_files(user, {"ch.txt": b""})
            return 1.0, {}, user

        for name, fake in (("designer", designer), ("ibcmd", ibcmd), ("bare_agent_session", bare_agent_session)):
            patcher = mock.patch.object(ex, name, side_effect=fake)
            patcher.start()
            self.addCleanup(patcher.stop)
        for patcher in (mock.patch.object(bench, "commit"), mock.patch("builtins.print"),
                        mock.patch.object(bench, "git", return_value=mock.Mock(stdout="", returncode=0))):
            patcher.start()
            self.addCleanup(patcher.stop)
        return b, events, rows

    def assert_free_before_others(self, events):
        held = None  # the last command of the runner under test, while its agent may hold the base
        for who, what in events:
            if who == "runner":
                held = what
            elif who == "free":
                held = None
            else:
                self.assertIsNone(held, f"{what} runs while the agent of `runner {held}` may hold the base")

    def test_every_other_tool_waits_for_the_live_agent_to_let_the_base_go(self):
        b, events, rows = self.bench()
        for scn in describe.load_scenarios().values():
            b.run_scenario(scn, scn.executors, 2)
        failed = [(r.get("scenario"), r.get("executor"), r.get("error")) for r in rows if r.get("ok") is False]
        self.assertEqual(failed, [])
        others = {what.split(" ")[0] for who, what in events if who == "other"}
        self.assertEqual(others, {"designer", "ibcmd", "bare", "pull", "push", "status"})
        self.assert_free_before_others(events)

    def test_a_live_pull_is_measured_with_the_agent_reconnected(self):
        """The owner's call (#585): the foreign edit releases the live agent; the timed pull of
        runner-live must not pay the reconnect a live agent sitting on the base would not pay."""
        for name, measured in (("pull-common1", "pull"), ("push-common1", "push")):
            b, events, rows = self.bench()
            b.run_scenario(describe.load_scenarios()[name], ["runner-live"], 2)
            self.assertEqual([r.get("error") for r in rows if r.get("ok") is False], [], name)
            # Every timed command of runner-live comes right after a reconnect, never right after a release.
            for i, (who, what) in enumerate(events):
                if (who, what) == ("runner", measured):
                    before = [e for e in events[:i] if e[0] in ("runner", "free")]
                    # (after a stop comes the untimed restore, not a measure)
                    if before and before[-1] == ("free", "release"):
                        self.fail(f"{name}: the timed {measured} follows a release: {events}")

    def test_the_restore_lets_the_base_go_before_runner_016_syncs(self):
        """The failure on kz83: restore-push left the live agent connected and the sync by 0.16
        that followed got `agent ... rejected the credentials of user ''`."""
        b, events, _ = self.bench()
        b.base_marked = b.used_016 = True
        originals = {"CommonModules/М/Ext/Module.bsl": b"original"}
        b.restore(list(originals), originals, "s")
        self.assertIn(("runner", "push"), events)
        self.assertIn(("other", "pull"), events)
        self.assert_free_before_others(events)


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
