"""The measuring flag: one file says who measures on this machine now.

Whoever holds the flag measures; everybody else waits. The script never overwrites or removes
a flag it did not write: it waits for any other text, including the flags of parallel threads of
the same session, and removes the flag only when the file still holds its own text.
"""
import os
import time

DEFAULT_PATH = r"E:\stands\.measuring"


class FlagBusy(RuntimeError):
    """The flag is held by somebody else (or by a run with our text) and we may not wait."""


def holder(path):
    """Text of the flag, or None when there is no flag."""
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read().strip()
    except FileNotFoundError:
        return None


def acquire(path, text, *, wait=True, poll=60.0, timeout=None, say=print, sleep=time.sleep):
    """Create the flag with `text`; wait while another flag stands.

    A flag that already holds our own text is refused rather than taken over: it is either a
    parallel run with the same text or a leftover of a crashed run, and only a person can tell.
    """
    text = (text or "").strip()
    if not text:
        raise ValueError("the flag text is empty: say who measures, e.g. --flag-text 'v8-runner 89ada31d s3 #604'")
    started = time.monotonic()
    while True:
        try:
            with open(path, "x", encoding="utf-8") as f:
                f.write(text + "\n")
            return
        except FileExistsError:
            pass
        current = holder(path)
        if current is None:
            continue  # removed between our two looks: try again at once
        if current == text:
            raise FlagBusy(f"the flag {path} already holds our text {text!r}: another run with this text is going, "
                           "or a run crashed; remove the file by hand only if no such run is going")
        if not wait:
            raise FlagBusy(f"the flag {path} is held: {current!r}")
        if timeout is not None and time.monotonic() - started > timeout:
            raise FlagBusy(f"the flag {path} is still held after {timeout:.0f} s: {current!r}")
        say(f"flag busy: {current!r}; waiting")
        sleep(poll)


def release(path, text, *, say=print):
    """Remove the flag only if it still holds our text. Returns True when removed."""
    current = holder(path)
    if current is None:
        say(f"flag {path} is already gone")
        return False
    if current != (text or "").strip():
        say(f"flag {path} now holds {current!r}, not ours: left as is")
        return False
    os.remove(path)
    return True


class Held:
    """`with Held(path, text):` — hold the flag for the block, release only our own."""

    def __init__(self, path, text, **acquire_options):
        self.path, self.text, self.options = path, text, acquire_options

    def __enter__(self):
        acquire(self.path, self.text, **self.options)
        return self

    def __exit__(self, *exc):
        release(self.path, self.text, say=self.options.get("say", print))
        return False
