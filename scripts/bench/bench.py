"""Repeatable stand: the runner against batch Designer, the bare agent and ibcmd on the same edit.

    python scripts/bench/bench.py list
    python scripts/bench/bench.py setup  --stand ssl321 --flag-text "<who measures>" [--recreate]
    python scripts/bench/bench.py run    --stand ka --flag-text "<who measures>" [--scenario S ...]
                                         [--executor E ...] [--reps N] [--runner EXE] [--runner-016 EXE]
                                         [--dry-run]
    python scripts/bench/bench.py report <results.jsonl> ... [--steps]
    python scripts/bench/bench.py phases <trace.log>

See README.md next to this file.
"""
import argparse
import dataclasses
import datetime as dt
import json
import os
import platform as host
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

import describe  # noqa: E402
import executors as ex  # noqa: E402
import flag  # noqa: E402
import report  # noqa: E402

REPO = os.path.dirname(os.path.dirname(HERE))
# Kinds that use the runner's agent; runner-live warms it up first. `status` asks no agent, and the
# full pull of a file base goes through ibcmd (#559), so runner-live has nothing to keep there.
LIVE_KINDS = ("push", "pull", "pull-none", "push-after-pull", "pair")


def say(text):
    print(f"{time.strftime('%H:%M:%S')} {text}", flush=True)


def git(project, *args, check=True):
    q = subprocess.run(["git", "-C", project, *args], capture_output=True, text=True, encoding="utf-8",
                       errors="replace", stdin=subprocess.DEVNULL)
    if check and q.returncode:
        raise ex.StepFailed(f"git {' '.join(args)}: {q.stderr.strip()[-300:]}")
    return q


def commit(project, message):
    git(project, "add", "-A", "--", ".")
    if git(project, "diff", "--cached", "--quiet", check=False).returncode:
        git(project, "-c", "user.name=bench", "-c", "user.email=bench@example.invalid", "commit", "-q", "-m", message)


def default_runner():
    target = os.environ.get("CARGO_TARGET_DIR") or os.path.join(REPO, "target")
    return os.path.join(target, "release", "v8-runner.exe" if os.name == "nt" else "v8-runner")


def write_owner(work, text, purpose, created):
    os.makedirs(work, exist_ok=True)
    lines = [f"owner: {text}", f"purpose: {purpose}"]
    if created:
        lines.append(describe.CREATED_MARK)
    lines.append("remove: when the owner says the stand is not needed; "
                 "`bench.py setup --recreate` rebuilds a stand the script created")
    with open(os.path.join(work, "OWNER.md"), "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")


def created_by_bench(work):
    try:
        with open(os.path.join(work, "OWNER.md"), encoding="utf-8") as f:
            return describe.CREATED_MARK in f.read()
    except FileNotFoundError:
        return False


def remove_own_stand(work):
    """Remove a stand the script created; OWNER.md goes last, so a failed removal stays ours."""
    for entry in os.listdir(work):
        path = os.path.join(work, entry)
        if entry == "OWNER.md":
            continue
        if os.path.isdir(path):
            shutil.rmtree(path)
        else:
            os.remove(path)
    os.remove(os.path.join(work, "OWNER.md"))


# ---- setup --------------------------------------------------------------------------------

def stand_016(stand):
    """The stand as the runner-016 executor sees it: the same base, its own project."""
    return dataclasses.replace(stand, project=stand.project_016)


def write_project(stand):
    """v8project.yaml, the local layer with the base and .gitignore of a project the script owns."""
    os.makedirs(stand.project, exist_ok=True)
    with open(stand.config, "w", encoding="utf-8") as f:
        f.write("workPath: .build\nformat: DESIGNER\nsource-set:\n"
                f"  - name: {stand.source_set}\n    type: CONFIGURATION\n    path: {stand.source_path}\n"
                f"tools:\n  platform:\n    version: {stand.platform}\n")
    with open(os.path.join(stand.project, "v8project.local.yaml"), "w", encoding="utf-8") as f:
        f.write(f"infobases:\n  origin:\n    connection: 'File={stand.infobase}'\n")
        if stand.user:
            f.write(f"    user: '{stand.user}'\n    password: '{stand.password}'\n")
    with open(os.path.join(stand.project, ".gitignore"), "w", encoding="utf-8") as f:
        f.write(".build/\n.local/\nConfigDumpInfo.xml\nv8project.local.yaml\n")
    git(stand.project, "init", "-q")


def setup_step(stand, name, fn):
    t0 = time.perf_counter()
    fn()
    row = {"step": name, "seconds": round(time.perf_counter() - t0, 2), "at": dt.datetime.now().isoformat()}
    with open(os.path.join(stand.work, "setup.jsonl"), "a", encoding="utf-8") as f:
        f.write(json.dumps(row, ensure_ascii=False) + "\n")
    say(f"setup {stand.name} {name}: {row['seconds']} s")


def setup_016(stand, runner_016, owner_text):
    """The runner-016 project: git and a full pull by that runner, so it has its own memory of the base."""
    s16 = stand_016(stand)
    if os.path.isdir(os.path.join(s16.project, ".git")) and git(s16.project, "rev-parse", "HEAD", check=False).returncode == 0:
        say(f"stand {stand.name}: project-016 is already set up at {s16.project}")
        return
    if not os.path.exists(stand.work):
        raise SystemExit(f"{stand.work} does not exist: set the stand up first")
    if not stand.created and not os.path.exists(os.path.join(stand.work, "OWNER.md")):
        write_owner(stand.work, owner_text, f"work files of the bench on the existing stand {stand.project}",
                    created=False)
    if os.path.exists(s16.project):
        shutil.rmtree(s16.project)  # an unfinished project-016: ours, under the stand's work directory
    write_project(s16)
    runner = ex.Runner(runner_016, s16, os.path.join(stand.work, "traces"), trace=False)
    setup_step(stand, "runner-016 full pull", lambda: runner.must("setup-pull-016", "pull", "--force", s16.source_set))
    commit(s16.project, "bench: full pull by runner-016")
    say(f"stand {stand.name}: project-016 is ready: {s16.project}")


def setup(stand, runner_exe, owner_text, recreate, runner_016=None):
    if not stand.created:
        if not runner_016:
            raise SystemExit(f"stand {stand.name!r} is an existing stand ({stand.project}); the script does not "
                             "create it, only its project-016 with --runner-016")
        return setup_016(stand, runner_016, owner_text)
    if os.path.exists(stand.work):
        if not created_by_bench(stand.work):
            raise SystemExit(f"{stand.work} exists and was not created by scripts/bench: not touched")
        if not recreate:
            if os.path.exists(os.path.join(stand.project, ".git")):
                say(f"stand {stand.name} is already set up at {stand.work}; --recreate rebuilds it")
                if runner_016:
                    setup_016(stand, runner_016, owner_text)
                return
            raise SystemExit(f"{stand.work} holds a setup that did not finish: run setup with --recreate")
        if os.path.exists(stand.config):
            ex.Runner(runner_exe, stand, "").stop_agents()
        remove_own_stand(stand.work)
    # Before anything long: a missing binary would fail only after the template load.
    for exe in (runner_exe, runner_016):
        if exe and not os.path.exists(exe):
            raise SystemExit(f"not found: {exe}")
    write_owner(stand.work, owner_text, f"bench stand {stand.name}: {stand.title}", created=True)
    out = os.path.join(stand.work, "designer.out")

    def step(name, fn):
        setup_step(stand, name, fn)

    def create_base():
        seconds, q = ex.timed([stand.v8, "CREATEINFOBASE", f"File={stand.infobase}", "/Out", out])
        if q.returncode:
            raise ex.StepFailed(f"CREATEINFOBASE rc={q.returncode}")

    step("create infobase", create_base)
    if stand.template_cf:
        step("load cf", lambda: ex.designer(stand, ["/LoadCfg", stand.template_cf, "/UpdateDBCfg", "-Dynamic-"], out))
    else:
        step("restore dt", lambda: ex.designer(stand, ["/RestoreIB", stand.template_dt], out))
    if stand.disable_support:
        step("disable support", lambda: ex.designer(
            stand, ["/ManageCfgSupport", "-disableSupport", "-force", "/UpdateDBCfg", "-Dynamic-"], out))
    write_project(stand)
    runner = ex.Runner(runner_exe, stand, os.path.join(stand.work, "traces"), trace=False)
    step("runner full pull", lambda: runner.must("setup-pull", "pull", "--force", stand.source_set))
    runner.stop_agents()
    commit(stand.project, "bench: full pull")
    missing = [rel for files in stand.modules.values() for rel in files
               if not os.path.exists(os.path.join(stand.src, *rel.split("/")))]
    say(f"stand {stand.name} is ready: {stand.work}")
    if runner_016:
        setup_016(stand, runner_016, owner_text)
    if missing:
        raise SystemExit(f"module sets name files the configuration does not have: {missing}")


# ---- run ----------------------------------------------------------------------------------

class Bench:
    def __init__(self, stand, runner_exe, out, trace, verify, runner_016=None):
        self.stand, self.out, self.verify = stand, out, verify
        self.work = stand.work
        traces = os.path.join(self.work, "traces")
        self.runner = ex.Runner(runner_exe, stand, traces, trace=trace)
        # runner-016 works in its own project; the stand's runner (above) owns the restore.
        self.runner_016 = ex.Runner(runner_016, stand_016(stand), traces, trace=trace) if runner_016 else None
        self.run_id = time.strftime("%Y%m%d-%H%M%S")
        self.base_marked = False    # the base may hold a marker the originals do not
        self.needs_resync = False   # the base changed behind the runner's memory
        self.used_016 = False       # project-016 may lag behind the base after the restore

    def runner_for(self, executor):
        if executor == "runner-016":
            if not self.runner_016:
                raise ex.StepFailed("runner-016 needs --runner-016 <exe>")
            return self.runner_016
        return self.runner

    def sync_016(self):
        """project-016 takes what the base got from others since its last command (not timed)."""
        r = self.runner_016
        # The stand's live agent stays connected after its last command (#644), the restore's
        # push included; runner 0.16 starts its own agent, which cannot open the held base and
        # answers `rejected the credentials of user ''`.
        self.runner.release_agents()
        commit(r.stand.project, f"bench {self.run_id}: before sync")
        r.must("sync-016", "pull", r.stand.source_set)
        commit(r.stand.project, f"bench {self.run_id}: synced with the base")

    def catch_up(self):
        """The stand's runner takes what runner-016 put into the base (not timed)."""
        if self.needs_resync:
            self.runner.must("resync-pull", "pull", self.stand.source_set)
            self.needs_resync = False
            commit(self.stand.project, f"bench {self.run_id}: pulled what runner-016 did")

    def tree_clean(self, project):
        return not git(project, "status", "--porcelain", "--", ".").stdout.strip()

    # -- bookkeeping
    def record(self, **row):
        row = {"run": self.run_id, "stand": self.stand.name, **row, "at": dt.datetime.now().isoformat(timespec="seconds")}
        os.makedirs(os.path.dirname(self.out), exist_ok=True)
        with open(self.out, "a", encoding="utf-8") as f:
            f.write(json.dumps(row, ensure_ascii=False) + "\n")
        brief = {k: v for k, v in row.items() if k in ("scenario", "executor", "rep", "seconds", "verified", "error", "step")}
        say(json.dumps(brief, ensure_ascii=False))

    def path(self, *parts):
        return os.path.join(self.work, *parts)

    def version_file(self):
        """The base's version file now, by batch Designer (not timed).

        The project's ConfigDumpInfo.xml may lag behind the base after `push`; a stale one makes
        the delta dump bigger or a FullDump, so every comparison gets a fresh one.
        """
        target = self.path("version")
        shutil.rmtree(target, ignore_errors=True)
        self.runner.release_agents()
        ex.designer(self.stand, ["/DumpConfigToFiles", target, "-configDumpInfoOnly"], self.path("designer.out"))
        return os.path.join(target, "ConfigDumpInfo.xml")

    def version_file_copy(self, source, name="cdi.xml", into=None):
        dst = os.path.join(into or self.work, name)
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        shutil.copyfile(source, dst)
        return dst

    def project_file(self, rel):
        return os.path.join(self.stand.src, *rel.split("/"))

    def markers_in(self, root, rels, marker):
        return all(ex.has_marker(os.path.join(root, *rel.split("/")), marker) for rel in rels)

    def resync(self, rels, marker):
        """The runner takes the base's change into the project: the check that the edit reached the base."""
        row, _ = self.runner.must("resync-pull", "pull", self.stand.source_set)
        self.needs_resync = False
        commit(self.stand.project, f"bench {marker}: pulled")
        return row["seconds"], self.markers_in(self.stand.src, rels, marker)

    def base_edit(self, rels, originals, marker):
        """Put a marked module into the base with a separate batch Designer (not timed)."""
        stage_dir = self.path("base-edit")
        listing = ex.stage(stage_dir, {rel: ex.marked(originals[rel], marker) for rel in rels})
        self.base_marked = self.needs_resync = True
        # A live agent stays connected between commands (#644) and holds the base for configuring:
        # the other editor gets in only after it is released, as a real one would have to wait.
        self.runner.release_agents()
        ex.designer(self.stand, ex.batch_push_args(stage_dir, listing), self.path("designer.out"))

    # -- push: edit the module, load it into the base, update the database configuration
    def push_runner(self, scn, executor, rels, originals, marker):
        r = self.runner_for(executor)
        before = self.version_file() if self.verify else None
        ex.write_files(r.stand.src, {rel: ex.marked(originals[rel], marker) for rel in rels})
        self.base_marked = True  # set before the call: a failed push may have loaded part of it
        if r is self.runner_016:
            self.needs_resync = True  # the stand's runner did not see this push
        row, _ = r.run(f"push-{scn.name}-{executor}", "push", r.stand.source_set, *scn.runner_args)
        if row["rc"]:
            self.needs_resync = True  # the load may have reached the base before the failure
            raise ex.StepFailed(f"push rc={row['rc']} {row['error']}: {row['message']}")
        commit(r.stand.project, f"bench {marker}")
        if self.verify:
            # Independent of the runner: what changed in the base against the version file of
            # before the push, dumped by batch Designer into an empty directory.
            dump = self.path("verify-dump")
            shutil.rmtree(dump, ignore_errors=True)
            self.runner.release_agents()
            row["verify_seconds"] = ex.designer(self.stand, ex.batch_pull_args(dump, before), self.path("designer.out"))
            row["verified"] = self.markers_in(dump, rels, marker)
            shutil.rmtree(dump, ignore_errors=True)
        self.catch_up()
        return row

    def push_other(self, scn, executor, rels, originals, marker):
        texts = {rel: ex.marked(originals[rel], marker) for rel in rels}
        row = {}
        self.base_marked = self.needs_resync = True  # before the call: a failure may leave half of it
        if executor == "batch":
            stage_dir = self.path("stage")
            listing = ex.stage(stage_dir, texts)
            row["seconds"] = ex.designer(self.stand, ex.batch_push_args(stage_dir, listing), self.path("designer.out"))
        elif executor == "ibcmd":
            stage_dir = self.path("stage")
            ex.stage(stage_dir, texts)
            data = self.path("ibcmd-data")
            t0 = time.perf_counter()
            load = ex.ibcmd(self.stand, ex.ibcmd_import_args(self.stand, data, stage_dir, rels))
            apply = ex.ibcmd(self.stand, ex.ibcmd_apply_args(self.stand, data))
            row["seconds"] = round(time.perf_counter() - t0, 2)
            row["steps"] = {"load": load, "update": apply}
        else:  # agent
            def prepare(user):
                ex.stage(os.path.join(user, "s"), texts)
            row["seconds"], row["steps"], _ = ex.bare_agent_session(
                self.stand, self.path("agent"), prepare, ex.agent_push_commands("s", "s.txt"))
        seconds, verified = self.resync(rels, marker)
        if self.verify:
            row["verify_seconds"], row["verified"] = seconds, verified
        return row

    # -- pull: the module changed in the base; take the change into files
    def reconnect_live(self, r, label):
        """The foreign edit needed the base free, so the live agent was released from it. A live agent
        normally sits connected between commands: reconnect it before the timed step (not timed), so
        the measure is the step itself and not a reconnect a real live agent would not pay."""
        row, _ = r.run(f"reconnect-{label}", "status", "--deep")
        if row["rc"]:
            raise ex.StepFailed(f"status --deep before the measure rc={row['rc']} {row['error']}: {row['message']}")

    def pull_runner(self, scn, executor, rels, originals, marker):
        r = self.runner_for(executor)
        self.base_edit(rels, originals, marker)
        if executor == "runner-live":
            self.reconnect_live(r, f"{scn.name}-{executor}")
        row, _ = r.run(f"pull-{scn.name}-{executor}", "pull", r.stand.source_set, *scn.runner_args)
        if row["rc"]:
            raise ex.StepFailed(f"pull rc={row['rc']} {row['error']}: {row['message']}")
        if r is self.runner:
            self.needs_resync = False
        row["verified"] = self.markers_in(r.stand.src, rels, marker)
        commit(r.stand.project, f"bench {marker}")
        self.catch_up()
        return row

    # -- pull-none: nothing changed in the base; take "nothing" into files
    def pull_none_runner(self, scn, executor, rels, originals, marker):
        r = self.runner_for(executor)
        row, _ = r.run(f"pull-none-{scn.name}-{executor}", "pull", r.stand.source_set, *scn.runner_args)
        if row["rc"]:
            raise ex.StepFailed(f"pull rc={row['rc']} {row['error']}: {row['message']}")
        row["verified"] = self.tree_clean(r.stand.project)
        commit(r.stand.project, f"bench {marker}")
        return row

    def pull_none_other(self, scn, executor, rels, originals, marker):
        current = self.version_file()
        row = {}
        if executor == "batch":
            version = self.version_file_copy(current)
            dump = self.path("pull-dump")
            shutil.rmtree(dump, ignore_errors=True)
            row["seconds"] = ex.designer(self.stand, ex.batch_pull_args(dump, version), self.path("designer.out"))
        else:  # agent
            def prepare(user):
                self.version_file_copy(current, "cdi.xml", into=user)
            row["seconds"], row["steps"], dump_user = ex.bare_agent_session(
                self.stand, self.path("agent"), prepare, ex.agent_pull_commands("d", "cdi.xml"))
            dump = os.path.join(dump_user, "d")
        dumped = ex.dumped_files(dump)
        row["dumped"] = dumped[:5]
        row["verified"] = not dumped
        shutil.rmtree(dump, ignore_errors=True)
        return row

    # -- push-after-pull: a module changed in the base, the runner pulled it (not timed); push now
    def push_after_pull_runner(self, scn, executor, rels, originals, marker):
        r = self.runner_for(executor)
        self.base_edit(rels, originals, marker)
        before, _ = r.run(f"pull-before-{scn.name}-{executor}", "pull", r.stand.source_set)
        if before["rc"]:
            raise ex.StepFailed(f"pull before push rc={before['rc']} {before['error']}: {before['message']}")
        if r is self.runner:
            self.needs_resync = False
        commit(r.stand.project, f"bench {marker}: pulled")
        row, _ = r.run(f"push-{scn.name}-{executor}", "push", r.stand.source_set, *scn.runner_args)
        if row["rc"]:
            raise ex.StepFailed(f"push rc={row['rc']} {row['error']}: {row['message']}")
        row["pull_seconds"] = before["seconds"]
        row["verified"] = self.markers_in(r.stand.src, rels, marker) and self.tree_clean(r.stand.project)
        commit(r.stand.project, f"bench {marker}")
        self.catch_up()
        return row

    # -- pull-full: the whole configuration into files
    def pull_full_runner(self, scn, executor, rels, originals, marker):
        r = self.runner_for(executor)
        row, _ = r.run(f"pull-full-{scn.name}-{executor}", "pull", "--force", r.stand.source_set, *scn.runner_args)
        if row["rc"]:
            raise ex.StepFailed(f"pull --force rc={row['rc']} {row['error']}: {row['message']}")
        # The same base into the same tree: the full pull rewrites the files with what they were.
        row["verified"] = self.tree_clean(r.stand.project)
        commit(r.stand.project, f"bench {marker}")
        return row

    def pull_full_other(self, scn, executor, rels, originals, marker):
        row = {}
        if executor == "batch":
            dump = self.path("full-dump")
            shutil.rmtree(dump, ignore_errors=True)
            row["seconds"] = ex.designer(self.stand, ex.batch_full_args(dump), self.path("designer.out"))
        elif executor == "ibcmd":
            dump = self.path("full-dump")
            shutil.rmtree(dump, ignore_errors=True)
            row["seconds"] = ex.ibcmd(self.stand, ex.ibcmd_export_args(self.stand, self.path("ibcmd-data"), dump))
        else:  # agent
            row["seconds"], row["steps"], dump_user = ex.bare_agent_session(
                self.stand, self.path("agent"), lambda user: os.makedirs(user, exist_ok=True),
                ex.agent_full_commands("f"))
            dump = os.path.join(dump_user, "f")
        row["verified"] = os.path.exists(os.path.join(dump, "Configuration.xml"))
        shutil.rmtree(dump, ignore_errors=True)
        return row

    def pull_other(self, scn, executor, rels, originals, marker):
        before = self.version_file()
        self.base_edit(rels, originals, marker)
        row = {}
        if executor == "batch":
            version = self.version_file_copy(before)
            dump = self.path("pull-dump")
            shutil.rmtree(dump, ignore_errors=True)
            row["seconds"] = ex.designer(self.stand, ex.batch_pull_args(dump, version), self.path("designer.out"))
        else:  # agent
            def prepare(user):
                self.version_file_copy(before, "cdi.xml", into=user)
            row["seconds"], row["steps"], dump_user = ex.bare_agent_session(
                self.stand, self.path("agent"), prepare, ex.agent_pull_commands("d", "cdi.xml"))
            dump = os.path.join(dump_user, "d")
        row["verified"] = self.markers_in(dump, rels, marker)
        shutil.rmtree(dump, ignore_errors=True)
        self.resync(rels, marker)
        return row

    # -- pair: `push --no-apply` and the next `pull` by changes (#646); the database configuration
    # is not updated, as in the bare-agent measurement the pair is compared with
    def pair_runner(self, scn, executor, rels, originals, marker):
        ex.write_files(self.stand.src, {rel: ex.marked(originals[rel], marker) for rel in rels})
        commit(self.stand.project, f"bench {marker}")
        self.base_marked = True  # set before the call: a failed push may have loaded part of it
        push, _ = self.runner.run(f"pair-push-{scn.name}-{executor}", "push", self.stand.source_set,
                                  "--no-apply", *scn.runner_args)
        if push["rc"]:
            self.needs_resync = True
            raise ex.StepFailed(f"push rc={push['rc']} {push['error']}: {push['message']}")
        pull, _ = self.runner.run(f"pair-pull-{scn.name}-{executor}", "pull", self.stand.source_set)
        if pull["rc"]:
            self.needs_resync = True
            raise ex.StepFailed(f"pull rc={pull['rc']} {pull['error']}: {pull['message']}")
        dirty = git(self.stand.project, "status", "--porcelain", "--", ".").stdout.strip().splitlines()
        row = {"seconds": round(push["seconds"] + pull["seconds"], 2),
               "steps": {"push": push["seconds"], "pull": pull["seconds"]},
               "mode": pull.get("mode"), "live": (push.get("live") or []) + (pull.get("live") or []),
               "dirty": dirty[:10]}
        # After the pair the tree is what was committed: the dump rewrote the loaded objects
        # with the same text and brought nothing else.
        row["verified"] = self.markers_in(self.stand.src, rels, marker) and not dirty
        commit(self.stand.project, f"bench {marker}: pulled")
        return row

    def pair_other(self, scn, executor, rels, originals, marker):
        raise ex.StepFailed("the pair is measured for the runner only")

    # -- status: nothing changed; ask what changed
    def status_runner(self, scn, executor, rels, originals, marker):
        r = self.runner_for(executor)
        row, payload = r.run(f"status-{scn.name}-{executor}", "status", *scn.runner_args)
        if row["rc"]:
            raise ex.StepFailed(f"status rc={row['rc']} {row['error']}: {row['message']}")
        changed = ex.find_all(payload, "changed_files")
        row["changed_files"] = changed
        row["verified"] = bool(changed) and all(c == 0 for c in changed)
        row["detection"] = ex.find_all(payload, "detection")
        return row

    def status_other(self, scn, executor, rels, originals, marker):
        row = {}
        current = self.version_file()
        if executor == "batch":
            version = self.version_file_copy(current)
            changes = self.path("changes.txt")
            dump = self.path("changes-dump")  # must not exist: -getChanges only computes the list
            shutil.rmtree(dump, ignore_errors=True)
            if os.path.exists(changes):
                os.remove(changes)
            row["seconds"] = ex.designer(self.stand, ex.batch_status_args(dump, changes, version), self.path("designer.out"))
        elif executor == "ibcmd":
            version = self.version_file_copy(current)
            changes = self.path("changes.txt")
            if os.path.exists(changes):
                os.remove(changes)
            row["seconds"] = ex.ibcmd(self.stand, ex.ibcmd_status_args(self.stand, self.path("ibcmd-data"), version, changes))
        else:  # agent
            def prepare(user):
                self.version_file_copy(current, "cdi.xml", into=user)
            row["seconds"], row["steps"], user = ex.bare_agent_session(
                self.stand, self.path("agent"), prepare, ex.agent_status_commands("g", "ch.txt", "cdi.xml"))
            changes = os.path.join(user, "ch.txt")
        with open(changes, encoding="utf-8-sig", errors="replace") as f:
            entries = [line.strip() for line in f if line.strip()]
        row["changes"] = entries[:5]
        row["verified"] = not entries
        return row

    # -- the stand back to where it was: the base and the project hold the original modules
    def restore(self, rels, originals, scenario):
        if not (self.base_marked or self.needs_resync):
            # Nothing reached the base since the last restore: the committed text is the original.
            ex.write_files(self.stand.src, originals)
            return
        # Commit whatever the tree holds first: the runner's pull refuses to overwrite uncommitted work.
        commit(self.stand.project, f"bench {self.run_id} {scenario}: before restore")
        if self.needs_resync:
            self.runner.must("restore-pull", "pull", self.stand.source_set)
            self.needs_resync = False
            commit(self.stand.project, f"bench {self.run_id} {scenario}: the base state pulled")
        ex.write_files(self.stand.src, originals)
        # Always push: the runner compares the files with its memory of the base, not with git.
        row, _ = self.runner.must("restore-push", "push", self.stand.source_set)
        commit(self.stand.project, f"bench {self.run_id} {scenario}: originals back")
        self.record(event="restore", scenario=scenario, seconds=row["seconds"])
        self.base_marked = False
        if self.used_016:
            # project-016 holds the markers runner-016 took or pushed; the base has the originals again.
            self.sync_016()
            self.used_016 = False

    def run_scenario(self, scn, executors, reps):
        rels = self.stand.modules.get(scn.modules, [])
        missing = [rel for rel in rels if not os.path.exists(self.project_file(rel))]
        if missing:
            raise ex.StepFailed(f"{scn.name}: the project has no {missing}")
        originals = {}
        for rel in rels:
            with open(self.project_file(rel), "rb") as f:
                originals[rel] = f.read()
            if originals[rel].lstrip(b"\xef\xbb\xbf").startswith(ex.MARK_PREFIX.encode()):
                raise ex.StepFailed(f"{rel} already starts with a bench marker: restore the stand first")
        try:
            for executor in executors:
                family = "runner" if executor in describe.RUNNERS else "other"
                measure = getattr(self, f"{scn.kind.replace('-', '_')}_{family}")
                warm = executor == "runner-live" and scn.kind in LIVE_KINDS
                for rep in range(0 if warm else 1, reps + 1):
                    if executor != "runner-live":
                        self.runner.stop_agents()
                    marker = f"{self.run_id} {scn.name} {executor} {rep}"
                    row = {"event": "measure", "scenario": scn.name, "kind": scn.kind, "executor": executor,
                           "rep": rep, "warmup": rep == 0, "files": len(rels)}
                    try:
                        if executor == "runner-016":
                            self.used_016 = True
                            self.sync_016()  # what the stand's runner and others put into the base
                            self.runner.stop_agents()  # the sync must not leave a warm agent for the measure
                        row.update(measure(scn, executor, rels, originals, marker))
                        row["ok"] = True
                    except Exception as error:  # noqa: BLE001 - a failed step is a result row
                        row.update(ok=False, error=str(error)[:500])
                    if executor == "runner-live" and row.get("ok") and scn.kind in LIVE_KINDS:
                        kept = [live.get("kept") for live in row.get("live") or [] if isinstance(live, dict)]
                        if kept and not all(kept):
                            row["warning"] = "the live agent was not kept: run outside a Job that kills children"
                    self.record(**row)
                    if not row["ok"]:
                        self.runner.stop_agents()
                        self.restore(rels, originals, scn.name)
                        break
        finally:
            self.runner.stop_agents()
            try:
                self.restore(rels, originals, scn.name)
            finally:
                self.runner.stop_agents()  # restore ran the runner, which keeps its agent


def plan_of(stand, scenarios, executors_wanted, runner_016):
    """(scenario, executors) pairs. runner-016 without its binary is left out, unless asked for by name."""
    if executors_wanted and "runner-016" in executors_wanted and not runner_016:
        raise SystemExit("--executor runner-016 needs --runner-016 <exe> (or V8_BENCH_RUNNER_016)")
    plan = []
    for scn in scenarios:
        describe.check_pair(stand, scn)
        chosen = [e for e in scn.executors if (not executors_wanted or e in executors_wanted)
                  and (e != "runner-016" or runner_016)]
        plan.append((scn, chosen))
    return plan


def run(stand, scenarios, executors_wanted, reps, runner_exe, out, trace, verify, dry_run, runner_016=None):
    plan = plan_of(stand, scenarios, executors_wanted, runner_016)
    if not runner_016:
        say("runner-016 is left out: no --runner-016")
    for scn, chosen in plan:
        say(f"plan {scn.name} ({scn.kind}, {scn.modules or '-'}): {', '.join(chosen) or 'no executors'} x {reps}")
    if dry_run:
        return
    for path in (stand.project, stand.config, stand.infobase, stand.v8, runner_exe):
        if not os.path.exists(path):
            raise SystemExit(f"not found: {path}")
    uses_016 = any("runner-016" in chosen for _, chosen in plan)
    if uses_016:
        if not os.path.exists(runner_016):
            raise SystemExit(f"not found: {runner_016}")
        ready = (os.path.isdir(os.path.join(stand.project_016, ".git"))
                 and git(stand.project_016, "rev-parse", "HEAD", check=False).returncode == 0)
        if not ready:
            raise SystemExit(f"no {stand.project_016}: run `bench.py setup --stand {stand.name} --runner-016 <exe>` first")
    if git(stand.project, "status", "--porcelain", "--", ".").stdout.strip():
        raise SystemExit(f"{stand.project} has uncommitted changes: the bench starts from a clean project")
    if not stand.created:
        os.makedirs(stand.work, exist_ok=True)
        if not os.path.exists(os.path.join(stand.work, "OWNER.md")):
            write_owner(stand.work, "scripts/bench", f"work files of the bench on the existing stand {stand.project}",
                        created=False)
    bench = Bench(stand, runner_exe, out, trace, verify, runner_016 if uses_016 else None)
    extra = {}
    if uses_016:
        extra = dict(runner_016=runner_016, runner_016_version=bench.runner_016.version(),
                     runner_016_sha256=ex.file_sha256(runner_016), project_016=stand.project_016)
    bench.record(event="start", runner=runner_exe, runner_version=bench.runner.version(),
                 runner_sha256=ex.file_sha256(runner_exe), **extra, platform=stand.platform, project=stand.project,
                 project_head=git(stand.project, "rev-parse", "HEAD").stdout.strip(),
                 repo_head=git(REPO, "rev-parse", "HEAD", check=False).stdout.strip(), host=host.node(),
                 reps=reps, plan={s.name: c for s, c in plan})
    try:
        for scn, chosen in plan:
            bench.run_scenario(scn, chosen, reps)
    finally:
        bench.record(event="end", project_head=git(stand.project, "rev-parse", "HEAD").stdout.strip(),
                     project_dirty=bool(git(stand.project, "status", "--porcelain", "--", ".").stdout.strip()))
        shutil.rmtree(bench.path("agent"), ignore_errors=True)
    say(f"results: {out}")
    print(report.table([out]))


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="command", required=True)
    sub.add_parser("list", help="stands, scenarios and executors")

    def stand_options(p):
        p.add_argument("--stand", required=True)
        p.add_argument("--stands-root", help=f"default {describe.DEFAULT_STANDS_ROOT} or V8_BENCH_STANDS_ROOT")
        p.add_argument("--platform-root", help=f"default {describe.DEFAULT_PLATFORM_ROOT} or V8_BENCH_PLATFORM_ROOT")
        p.add_argument("--runner", default=os.environ.get("V8_BENCH_RUNNER") or default_runner(),
                       help="v8-runner executable (default: release build under CARGO_TARGET_DIR or target/)")
        p.add_argument("--runner-016", default=os.environ.get("V8_BENCH_RUNNER_016"),
                       help="another runner binary for the runner-016 executor (release 0.16), on project-016")
        p.add_argument("--flag", default=os.environ.get("V8_BENCH_FLAG", flag.DEFAULT_PATH))
        p.add_argument("--flag-text", default=os.environ.get("V8_BENCH_FLAG_TEXT"),
                       help="text of our measuring flag: who measures (required unless --dry-run)")
        p.add_argument("--flag-poll", type=float, default=60.0, help="seconds between looks at a busy flag")

    p = sub.add_parser("setup", help="create a stand described with [create]")
    stand_options(p)
    p.add_argument("--recreate", action="store_true", help="remove the stand the script created and build it again")
    p = sub.add_parser("run", help="run scenarios on a stand")
    stand_options(p)
    p.add_argument("--scenario", action="append", help="scenario name (repeat); default: all")
    p.add_argument("--executor", action="append", choices=describe.EXECUTORS, help="executor (repeat); default: all")
    p.add_argument("--reps", type=lambda v: max(1, int(v)), default=3, help="repetitions, at least 1")
    p.add_argument("--out", help="results JSONL (default: <work>/results/<stand>-<time>.jsonl)")
    p.add_argument("--no-trace", action="store_true", help="no runner debug trace")
    p.add_argument("--no-verify", action="store_true", help="skip the check that the edit reached the base")
    p.add_argument("--dry-run", action="store_true", help="print the plan; touch nothing")
    p = sub.add_parser("report", help="table of medians from result files")
    p.add_argument("files", nargs="+")
    p.add_argument("--steps", action="store_true", help="also medians of executor steps")
    p = sub.add_parser("phases", help="a runner trace with offsets from its first line")
    p.add_argument("trace")
    args = ap.parse_args(argv)

    if args.command == "list":
        for name in describe.stand_names():
            s = describe.load_stand(name)
            print(f"stand {name}: {s.title} ({'created by setup' if s.created else 'existing'}; "
                  f"module sets: {', '.join(s.modules)})")
        for name, scn in describe.load_scenarios().items():
            print(f"scenario {name}: {scn.kind} {scn.modules or ''} — {scn.title}; executors: {', '.join(scn.executors)}")
        return
    if args.command == "report":
        print(report.table(args.files, steps=args.steps))
        return
    if args.command == "phases":
        print(report.phases(args.trace))
        return

    stand = describe.load_stand(args.stand, args.stands_root, args.platform_root)
    if args.command == "run":
        known = describe.load_scenarios()
        names = args.scenario or list(known)
        unknown = [n for n in names if n not in known]
        if unknown:
            raise SystemExit(f"unknown scenarios {unknown}; known: {', '.join(known)}")
        scenarios = [known[n] for n in names]
        out = args.out or os.path.join(stand.work, "results", f"{stand.name}-{time.strftime('%Y%m%d-%H%M%S')}.jsonl")
        if args.dry_run:
            return run(stand, scenarios, args.executor, args.reps, args.runner, out, False, False, True,
                       args.runner_016)
        with flag.Held(args.flag, args.flag_text, poll=args.flag_poll, say=say):
            run(stand, scenarios, args.executor, args.reps, args.runner, out, not args.no_trace, not args.no_verify,
                False, args.runner_016)
    elif args.command == "setup":
        with flag.Held(args.flag, args.flag_text, poll=args.flag_poll, say=say):
            setup(stand, args.runner, args.flag_text, args.recreate, args.runner_016)


if __name__ == "__main__":
    main()
