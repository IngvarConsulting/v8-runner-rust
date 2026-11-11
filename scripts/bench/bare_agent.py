"""The Designer agent without the runner: start, connect, run commands, release, remove.

Needs `paramiko` (`python -m pip install paramiko`). Paths in commands are relative to the
agent's user directory under `AgentBaseDir` (`<base_dir>/0` for the empty user): the agent
refuses absolute paths outside it.
"""
import json
import os
import socket
import subprocess
import time


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


class BareAgent:
    def __init__(self, v8, infobase, base_dir, user="", password=""):
        self.v8, self.infobase, self.base_dir = v8, infobase, base_dir
        self.user, self.password = user or "", password or ""
        self.proc = self.client = self.chan = None
        self.buf = b""
        self.steps = {}

    def start(self, start_timeout=180):
        import paramiko  # noqa: PLC0415 - only this executor needs it

        os.makedirs(self.base_dir, exist_ok=True)
        port = free_port()
        t0 = time.perf_counter()
        self.proc = subprocess.Popen(
            [self.v8, "DESIGNER", "/F", self.infobase, *self._auth(), "/AgentMode", "/AgentPort", str(port),
             "/AgentListenAddress", "127.0.0.1", "/AgentSSHHostKeyAuto", "/AgentBaseDir", self.base_dir,
             "/DisableStartupDialogs"],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        deadline = time.time() + start_timeout
        try:
            while True:
                try:
                    client = paramiko.SSHClient()
                    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
                    client.connect("127.0.0.1", port, username=self.user, password=self.password,
                                   look_for_keys=False, allow_agent=False, timeout=5, banner_timeout=5, auth_timeout=5)
                    break
                except Exception:
                    if time.time() > deadline or self.proc.poll() is not None:
                        raise
                    time.sleep(0.2)
            self.client = client
            # A shell without a pseudo-terminal, as the platform recommends for the agent.
            self.chan = client.get_transport().open_session()
            self.chan.invoke_shell()
            self.steps["start"] = time.perf_counter() - t0
            self.run("options set --show-prompt=no --output-format=json")
            t1 = time.perf_counter()
            try:
                self.run("common connect-ib")
            except RuntimeError as error:
                # A base without users: the first session is already connected.
                if "DesignerAlreadyConnectedToInfoBase" not in str(error):
                    raise
            self.steps["connect"] = time.perf_counter() - t1
        except BaseException:
            self.finish()
            raise
        return self

    def _auth(self):
        return ["/N", self.user, "/P", self.password] if self.user else []

    def run(self, command, timeout=7200):
        self.chan.sendall((command + "\n").encode("utf8"))
        deadline = time.time() + timeout
        messages = []
        decoder = json.JSONDecoder()
        while time.time() < deadline:
            # Replies are JSON arrays, pretty-printed over several lines; a banner and a prompt
            # may come before the first one.
            while True:
                text = self.buf.decode("utf8", "replace")
                start = text.find("[")
                if start < 0:
                    break
                try:
                    batch, end = decoder.raw_decode(text[start:])
                except ValueError:
                    break
                self.buf = text[start + end:].encode("utf8")
                if not isinstance(batch, list):
                    continue
                messages += batch
                kinds = {m.get("type") for m in batch if isinstance(m, dict)}
                if kinds & {"success", "error", "canceled", "question"}:
                    if "success" not in kinds:
                        raise RuntimeError(f"agent: {command}: {batch}")
                    return messages
            if self.chan.recv_ready():
                self.buf += self.chan.recv(65536)
            elif self.chan.exit_status_ready() or self.chan.closed:
                raise RuntimeError(f"agent closed during {command}")
            else:
                time.sleep(0.01)
        raise TimeoutError(command)

    def timed(self, step, command):
        t0 = time.perf_counter()
        try:
            return self.run(command)
        finally:
            self.steps[step] = time.perf_counter() - t0

    def finish(self):
        t0 = time.perf_counter()
        if self.chan is not None:
            try:
                self.run("common disconnect-ib", timeout=10)
            except Exception:
                pass
        for closer in (self.chan, self.client):
            try:
                if closer is not None:
                    closer.close()
            except Exception:
                pass
        if self.proc and self.proc.poll() is None:
            subprocess.run(["taskkill", "/T", "/F", "/PID", str(self.proc.pid)], capture_output=True)
            self.proc.wait(timeout=60)
        self.steps["finish"] = time.perf_counter() - t0


def user_dir(base_dir, user=""):
    """The directory the agent maps a user to, as the platform writes it in agentbasedir.json."""
    with open(os.path.join(base_dir, "agentbasedir.json"), encoding="utf-8-sig") as f:
        info = json.load(f)
    for entry in info.get("usersInfo", []):
        if entry.get("name", "") == (user or ""):
            return os.path.join(base_dir, entry["dir"])
    raise KeyError(user)
