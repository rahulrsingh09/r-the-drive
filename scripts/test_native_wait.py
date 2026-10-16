"""Exercise asynchronous guard completion without production data or credentials."""

import base64
from pathlib import Path
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parent.parent
SHA = "a" * 40
IMAGE = "postgres:18.4-bookworm@sha256:1961f96e6029a02c3812d7cb329a3b03a3ac2bb067058dec17b0f5596aca9296"
SUCCESS = 'printf "PANDA_NATIVE_backup_OK %s %s\\n" ' + SHA + ' "$(date -u +%s)"'


class NativeWaitTests(unittest.TestCase):
    @classmethod
    def docker(cls, *args, **kwargs):
        return subprocess.run(["docker", *args], capture_output=True, text=True,
                              timeout=45, **kwargs)

    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.name = "panda-native-wait-" + Path(cls.tmp.name).name
        cls.docker("run", "-d", "--rm", "--network", "none", "--name", cls.name,
                   "--entrypoint", "sleep", IMAGE, "infinity", check=True)
        cls.docker("exec", cls.name, "mkdir", "/fixture", check=True)

    @classmethod
    def tearDownClass(cls):
        cls.docker("rm", "-f", cls.name)
        cls.tmp.cleanup()

    def setUp(self):
        self.docker("exec", self.name, "rm", "-rf", "/backups/native-release", check=True)
        self.install(SUCCESS)

    def install(self, guard):
        source = "GUARD_B64='" + base64.b64encode(guard.encode()).decode() + "'\n"
        source += (ROOT / "scripts/native-release/wait.sh").read_text()
        self.encoded = base64.b64encode(source.encode()).decode()
        path = Path(self.tmp.name) / "control.sh"
        path.write_text(source)
        self.docker("cp", str(path), self.name + ":/fixture/control.sh", check=True)

    def args(self, mode, op="", sha=SHA, run="101"):
        return ["exec", "-e", "CONTROL_B64=" + self.encoded, self.name,
                "sh", "/fixture/control.sh", mode, "backup",
                base64.b64encode(sha.encode()).decode(),
                base64.b64encode(run.encode()).decode(),
                base64.b64encode(op.encode()).decode()]

    def call(self, mode, op="", **kwargs):
        return self.docker(*self.args(mode, op, **kwargs))

    def prepare(self):
        result = self.call("prepare")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertRegex(result.stdout, r"^PANDA_ASYNC_OP op\.[A-Za-z0-9]{16}\n$")
        return result.stdout.strip().split(" ")[1]

    def shell(self, command):
        return self.docker("exec", self.name, "sh", "-c", command, check=True).stdout

    def test_complete_preserves_original_attestation_and_single_execution(self):
        op = self.prepare()
        self.assertEqual(self.call("worker", op).returncode, 0)
        first = self.call("collect", op)
        self.assertEqual(first.returncode, 0, first.stderr)
        self.assertRegex(first.stdout, r"^PANDA_NATIVE_backup_OK " + SHA + r" [0-9]{10}\n$")
        self.assertEqual(self.call("poll", op).stdout, "")
        self.assertNotEqual(self.call("worker", op).returncode, 0)
        self.assertEqual(self.call("collect", op).stdout, first.stdout)

    def test_pending_then_ready_and_no_reuse_of_an_earlier_operation(self):
        self.install("sleep 6\n" + SUCCESS)
        op = self.prepare()
        worker = subprocess.Popen(["docker", *self.args("worker", op)],
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            self.assertEqual(self.call("poll", op).stdout, "PANDA_ASYNC_PENDING")
            worker.communicate(timeout=15)
            self.assertEqual(worker.returncode, 0)
            self.assertEqual(self.call("poll", op).stdout, "")
            later = self.prepare()
            self.assertNotEqual(later, op)
            self.assertNotEqual(self.call("collect", later).returncode, 0)
        finally:
            if worker.poll() is None:
                worker.kill()
                worker.wait()

    def test_failure_truncated_extra_and_wrong_markers_never_collect(self):
        for guard in [SUCCESS + "\nexit 1", "printf partial", SUCCESS + "\necho extra",
                      SUCCESS.replace(SHA, "b" * 40), SUCCESS.replace("backup", "verify")]:
            with self.subTest(guard=guard):
                self.install(guard)
                op = self.prepare()
                self.assertNotEqual(self.call("worker", op).returncode, 0)
                self.assertNotEqual(self.call("poll", op).returncode, 0)
                self.assertNotEqual(self.call("collect", op).returncode, 0)

    def test_identity_and_symlink_rejections(self):
        op = self.prepare()
        for changes in [{"sha": "b" * 40}, {"run": "102"}, {"run": "0"},
                        {"sha": "$(touch /tmp/unsafe)"}]:
            with self.subTest(changes=changes):
                self.assertNotEqual(self.call("worker", op, **changes).returncode, 0)
        self.assertNotEqual(self.call("worker", "../anything").returncode, 0)
        self.shell(f"mv /backups/native-release/async/{op}/created /fixture/created; "
                   f"ln -s /fixture/created /backups/native-release/async/{op}/created")
        self.assertNotEqual(self.call("worker", op).returncode, 0)

    def test_private_setgid_volume_supported_but_symlink_directory_rejected(self):
        self.shell("mkdir -p /backups/native-release; chmod 2700 /backups/native-release")
        op = self.prepare()
        self.assertEqual(self.call("worker", op).returncode, 0)
        self.shell(f"mv /backups/native-release/async/{op} /fixture/old-operation; "
                   f"ln -s /fixture/old-operation /backups/native-release/async/{op}")
        self.assertNotEqual(self.call("collect", op).returncode, 0)

    def test_expired_request_and_tampered_result_fail(self):
        op = self.prepare()
        self.assertEqual(self.call("worker", op).returncode, 0)
        self.shell(f"echo unexpected >> /backups/native-release/async/{op}/result")
        self.assertNotEqual(self.call("collect", op).returncode, 0)
        old = self.prepare()
        self.shell(f"date -u -d '10 minutes ago' +%s > /backups/native-release/async/{old}/created")
        for mode in ["worker", "poll", "collect"]:
            self.assertNotEqual(self.call(mode, old).returncode, 0)

    def test_stored_guard_cannot_replace_the_installed_policy(self):
        op = self.prepare()
        self.shell(f"echo 'touch /fixture/unapproved-policy' >> /backups/native-release/async/{op}/guard.sh")
        self.assertNotEqual(self.call("worker", op).returncode, 0)
        self.assertNotEqual(self.call("collect", op).returncode, 0)
        self.assertEqual(self.docker("exec", self.name, "test", "-e", "/fixture/unapproved-policy").returncode, 1)

    def test_deadline_kills_child_process_group_and_never_reports_success(self):
        self.install("(sleep 7; touch /fixture/descendant-survived) &\nwait\n" + SUCCESS)
        op = self.prepare()
        self.shell(f"date -u -d '292 seconds ago' +%s > /backups/native-release/async/{op}/created")
        started = time.monotonic()
        self.assertNotEqual(self.call("worker", op).returncode, 0)
        self.assertLess(time.monotonic() - started, 6)
        time.sleep(5)
        self.assertNotEqual(self.call("collect", op).returncode, 0)
        result = self.docker("exec", self.name, "test", "-e", "/fixture/descendant-survived")
        self.assertEqual(result.returncode, 1)


if __name__ == "__main__":
    unittest.main()
