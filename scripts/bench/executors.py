"""How each executor does each kind of work: command lines and the timed calls.

Command lines are pure functions, so tests can check them without a platform. No executor uses
the dynamic update of the database configuration: batch Designer gets `-Dynamic-`, the agent
`--dynamic-disable`, ibcmd `--dynamic=disable`, and the runner forbids it itself.
"""
import hashlib
import json
import os
import shutil
import subprocess
import time

from bare_agent import BareAgent, user_dir

MARK_PREFIX = "// bench "


# ---- module texts and markers -------------------------------------------------------------

EMPTY_COMMENT = b"<Comment/>"


def xml_mark(marker):
    """The marker of a metadata description: its own comment, which a dump writes back as is."""
    return f"<Comment>bench {marker}</Comment>"


def marked(original, marker):
    """The original module bytes with one marker line on top (after the BOM); a description
    (`.xml`) gets the marker as its own comment, in place of the first empty `<Comment/>`."""
    bom = original.startswith(b"\xef\xbb\xbf")
    body = original[3:] if bom else original
    prefix = b"\xef\xbb\xbf" if bom else b""
    if body.lstrip().startswith(b"<?xml"):
        if EMPTY_COMMENT not in body:
            raise ValueError("a marked description needs an empty <Comment/>")
        return prefix + body.replace(EMPTY_COMMENT, xml_mark(marker).encode("utf-8"), 1)
    return prefix + f"{MARK_PREFIX}{marker}\r\n".encode("utf-8") + body


def has_marker(path, marker):
    try:
        with open(path, encoding="utf-8-sig", errors="replace") as f:
            if path.lower().endswith(".xml"):
                return xml_mark(marker) in f.read()
            return f"{MARK_PREFIX}{marker}" in f.read(400)
    except FileNotFoundError:
        return False


def dumped_files(root):
    """Files a dump of changes wrote, apart from its version file; [] when the directory is absent."""
    found = []
    for folder, _, files in os.walk(root):
        for name in files:
            rel = os.path.relpath(os.path.join(folder, name), root).replace(os.sep, "/")
            if rel != "ConfigDumpInfo.xml":
                found.append(rel)
    return sorted(found)


def write_files(root, texts):
    for rel, data in texts.items():
        path = os.path.join(root, *rel.split("/"))
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(data)


def stage(root, texts):
    """A fresh directory with the given files and a list file next to it (UTF-8 with BOM)."""
    shutil.rmtree(root, ignore_errors=True)
    write_files(root, texts)
    listing = root + ".txt"
    with open(listing, "w", encoding="utf-8-sig", newline="") as f:
        f.write("\r\n".join(texts) + "\r\n")
    return listing


# ---- command lines ------------------------------------------------------------------------

def designer_cmd(stand, args, out):
    auth = ["/N", stand.user, "/P", stand.password] if stand.user else []
    return [stand.v8, "DESIGNER", "/F", stand.infobase, *auth, "/DisableStartupDialogs", "/DisableStartupMessages",
            *args, "/Out", out]


def batch_push_args(stage_dir, listing):
    return ["/LoadConfigFromFiles", stage_dir, "-partial", "-listFile", listing, "/UpdateDBCfg", "-Dynamic-"]


def batch_pull_args(dump_dir, version_file):
    # Into an empty directory: only what changed against the version file (references/1c, #173).
    return ["/DumpConfigToFiles", dump_dir, "-update", "-configDumpInfoForChanges", version_file]


def batch_full_args(dump_dir):
    return ["/DumpConfigToFiles", dump_dir]


def batch_status_args(dump_dir, changes, version_file):
    return ["/DumpConfigToFiles", dump_dir, "-getChanges", changes, "-configDumpInfoForChanges", version_file]


def ibcmd_base(stand, data_dir):
    auth = [f"--user={stand.user}", f"--password={stand.password}"] if stand.user else []
    return [f"--data={data_dir}", f"--db-path={stand.infobase}", *auth]


def ibcmd_import_args(stand, data_dir, stage_dir, files):
    return ["infobase", "config", "import", "files", *ibcmd_base(stand, data_dir), f"--base-dir={stage_dir}",
            "--partial", *files]


def ibcmd_apply_args(stand, data_dir):
    return ["infobase", "config", "apply", *ibcmd_base(stand, data_dir), "--force", "--dynamic=disable"]


def ibcmd_export_args(stand, data_dir, out_dir):
    return ["infobase", "config", "export", *ibcmd_base(stand, data_dir), out_dir]


def ibcmd_status_args(stand, data_dir, version_file, out):
    return ["infobase", "config", "export", "status", *ibcmd_base(stand, data_dir), f"--base={version_file}",
            f"--out={out}"]


def agent_push_commands(stage_rel, listing_rel):
    return [("load", f"config load-config-from-files --dir={stage_rel} --partial --list-file={listing_rel}"),
            ("update", "config update-db-cfg --dynamic-disable")]


def agent_pull_commands(dump_rel, version_rel):
    return [("dump", f"config dump-config-to-files --dir={dump_rel} --update --config-dump-info-for-changes={version_rel}")]


def agent_full_commands(dump_rel):
    return [("dump", f"config dump-config-to-files --dir={dump_rel}")]


def agent_status_commands(dump_rel, changes_rel, version_rel):
    return [("changes", f"config dump-config-to-files --dir={dump_rel} --get-changes={changes_rel} "
                        f"--config-dump-info-for-changes={version_rel}")]


# ---- processes ----------------------------------------------------------------------------

def timed(cmd, **kw):
    t0 = time.perf_counter()
    q = subprocess.run(cmd, capture_output=True, stdin=subprocess.DEVNULL, **kw)
    return round(time.perf_counter() - t0, 2), q


def tail(data, n=300):
    if isinstance(data, bytes):
        data = data.decode("utf-8", "replace")
    return (data or "").strip()[-n:]


class StepFailed(RuntimeError):
    pass


def designer(stand, args, out):
    """One batch Designer run; raises StepFailed with the /Out tail on a non-zero exit."""
    if os.path.exists(out):
        os.remove(out)
    seconds, q = timed(designer_cmd(stand, args, out), timeout=4 * 3600)
    if q.returncode:
        log = ""
        if os.path.exists(out):
            with open(out, encoding="utf-8-sig", errors="replace") as f:
                log = f.read()
        raise StepFailed(f"designer {args[0]} rc={q.returncode}: {tail(log)}")
    return seconds


def ibcmd(stand, args):
    seconds, q = timed([stand.ibcmd, *args], timeout=4 * 3600)
    if q.returncode:
        raise StepFailed(f"ibcmd {' '.join(args[:4])} rc={q.returncode}: {tail(q.stdout + q.stderr)}")
    return seconds


def find_all(value, key):
    found, stack = [], [value]
    while stack:
        v = stack.pop()
        if isinstance(v, dict):
            for k, inner in v.items():
                if k == key:
                    found.append(inner)
                else:
                    stack.append(inner)
        elif isinstance(v, list):
            stack.extend(v)
    return found


def file_sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


class Runner:
    def __init__(self, exe, stand, traces, trace=True):
        self.exe, self.stand, self.traces, self.trace = exe, stand, traces, trace

    def base_cmd(self):
        return [self.exe, "--config", self.stand.config, "--json-message"]

    def run(self, label, *args):
        """Timed runner command. Returns a dict for the result row (and the payload)."""
        env = dict(os.environ)
        trace = None
        extra = []
        if self.trace:
            os.makedirs(self.traces, exist_ok=True)
            trace = os.path.join(self.traces, f"{label}-{time.strftime('%Y%m%d-%H%M%S')}.log")
            env["V8TR_ACTION_LOG_FILE"] = trace
            extra = ["--log-level", "debug"]
        seconds, q = timed([*self.base_cmd(), *extra, *args], env=env, cwd=self.stand.project, timeout=4 * 3600)
        out = q.stdout.decode("utf-8", "replace")
        try:
            payload = json.loads(out)
        except ValueError:
            payload = {"raw": tail(out), "stderr": tail(q.stderr)}
        error = payload.get("error") or {}
        data = payload.get("data") if isinstance(payload.get("data"), dict) else {}
        return {
            "seconds": seconds, "rc": q.returncode, "error": error.get("code"),
            "message": (error.get("message") or "")[:300] or None,
            "mode": data.get("mode"), "live": find_all(payload, "live"),
            "trace": os.path.basename(trace) if trace else None,
        }, payload

    def must(self, label, *args):
        row, payload = self.run(label, *args)
        if row["rc"]:
            raise StepFailed(f"runner {' '.join(args)} rc={row['rc']} {row['error']}: {row['message']}")
        return row, payload

    def stop_agents(self):
        subprocess.run([self.exe, "--config", self.stand.config, "agent", "stop", "--all"], capture_output=True,
                       stdin=subprocess.DEVNULL, cwd=self.stand.project)

    def release_agents(self):
        """A live agent stays connected between commands (#644) and holds the base for
        configuring: batch Designer steps of the bench release it first (not timed)."""
        subprocess.run([self.exe, "--config", self.stand.config, "agent", "release"], capture_output=True,
                       stdin=subprocess.DEVNULL, cwd=self.stand.project)

    def version(self):
        q = subprocess.run([self.exe, "--version"], capture_output=True, text=True)
        return q.stdout.strip()


def bare_agent_session(stand, base_dir, prepare, commands):
    """Start the agent, run commands, release it. `prepare(user_dir)` stages files before start.

    Returns (total seconds, step seconds, user dir). The total counts the whole operation as a
    person scripting the agent would pay for it: start, connect, commands, release.
    """
    shutil.rmtree(base_dir, ignore_errors=True)
    user = os.path.join(base_dir, "0")  # the empty user; checked against agentbasedir.json below
    prepare(user)
    agent = BareAgent(stand.v8, stand.infobase, base_dir, stand.user, stand.password)
    t0 = time.perf_counter()
    try:
        agent.start()
        mapped = user_dir(base_dir, stand.user)
        if os.path.normcase(os.path.abspath(mapped)) != os.path.normcase(os.path.abspath(user)):
            raise StepFailed(f"the agent maps the user to {mapped}, files were staged in {user}")
        for step, command in commands:
            agent.timed(step, command)
    finally:
        agent.finish()
    total = round(time.perf_counter() - t0, 2)
    return total, {k: round(v, 2) for k, v in agent.steps.items()}, user
