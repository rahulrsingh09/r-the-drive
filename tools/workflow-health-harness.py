"""Execute the inline supervisor with inert process, signal and clock boundaries."""
import contextlib
import io
import json
import os
import signal
import subprocess
import sys
import time
import types

fixture = json.load(sys.stdin)
events = []
responses = iter(fixture["probes"])
timeout = subprocess.TimeoutExpired


class Process:
    pid = 12345

    def __init__(self, argv, **options):
        events.append(["spawn", argv, options])
        self.waits = 0

    def wait(self, **options):
        events.append(["wait", options])
        self.waits += 1
        if fixture.get("ignore_term") and self.waits == 1:
            raise timeout("fixture", 5)
        return 0


def run(argv, **options):
    events.append(["probe", argv, options])
    result = next(responses)
    if result == "timeout":
        raise timeout(argv, options["timeout"])
    if result == "error":
        raise OSError("fixture probe failure")
    return types.SimpleNamespace(returncode=result)


def killpg(pid, sig):
    events.append(["signal", pid, int(sig)])
    if fixture.get("missing_process"):
        raise ProcessLookupError()


os.killpg = killpg
time.sleep = lambda seconds: events.append(["sleep", seconds])
sys.modules["subprocess"] = types.SimpleNamespace(
    Popen=Process, run=run, TimeoutExpired=timeout,
)
output = io.StringIO()
status = 0
with contextlib.redirect_stdout(output):
    try:
        exec(compile(fixture["script"], "<workflow-health>", "exec"), {})
    except SystemExit as error:
        status = error.code
    except OSError:
        status = "error"
print(json.dumps({
    "status": status, "events": events, "stdout": output.getvalue(),
    "term": int(signal.SIGTERM), "kill": int(signal.SIGKILL),
}))
