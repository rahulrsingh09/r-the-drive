import json
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
BINARIES = ["panda-api", "panda-restore-agent", "rebaseline_db", "panda-keys"]
CROSS = {
    "CC_aarch64_unknown_linux_gnu": "aarch64-linux-gnu-gcc",
    "CXX_aarch64_unknown_linux_gnu": "aarch64-linux-gnu-g++",
    "AR_aarch64_unknown_linux_gnu": "aarch64-linux-gnu-ar",
    "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER": "aarch64-linux-gnu-gcc",
    "PKG_CONFIG_ALLOW_CROSS_aarch64_unknown_linux_gnu": "1",
    "PKG_CONFIG_PATH_aarch64_unknown_linux_gnu": "/usr/lib/aarch64-linux-gnu/pkgconfig",
    "AARCH64_UNKNOWN_LINUX_GNU_OPENSSL_LIB_DIR": "/usr/lib/aarch64-linux-gnu",
    "AARCH64_UNKNOWN_LINUX_GNU_OPENSSL_INCLUDE_DIR": "/usr/include",
}


class ContainerBuildContract(unittest.TestCase):
    def run_build(self, directory, phase, target, host, jobs=None, fail=False):
        fake_bin = directory / "fake-bin"
        fake_bin.mkdir()
        cargo = fake_bin / "cargo"
        cargo.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, pathlib, sys\n"
            "args = sys.argv[1:]\n"
            "keys = " + repr(list(CROSS)) + "\n"
            "pathlib.Path('record.json').write_text(json.dumps({'args': args, "
            "'env': {key: os.environ[key] for key in keys if key in os.environ}}))\n"
            "if os.environ.get('FAIL_BUILD'): sys.exit(42)\n"
            "destination = pathlib.Path('target')\n"
            "if '--target' in args: destination /= args[args.index('--target') + 1]\n"
            "destination /= args[args.index('--profile') + 1]\n"
            "destination.mkdir(parents=True, exist_ok=True)\n"
            "for index, value in enumerate(args):\n"
            "    if value == '--bin': (destination / args[index+1]).write_text('artifact:' + args[index+1])\n"
        )
        cargo.chmod(0o755)
        nproc = fake_bin / "nproc"
        nproc.write_text("#!/bin/sh\nprintf '2\\n'\n")
        nproc.chmod(0o755)
        if phase == "build":
            (directory / "src/bin").mkdir(parents=True)
            (directory / "migrations").mkdir()
            (directory / "src/main.rs").write_text("original source")
        env = {
            "PATH": str(fake_bin) + ":/usr/bin:/bin",
            "TARGETARCH": target,
            "BUILDARCH": host,
        }
        if jobs is not None:
            env["CARGO_BUILD_JOBS"] = jobs
        if fail:
            env["FAIL_BUILD"] = "1"
        return subprocess.run(
            ["/bin/sh", str(ROOT / "compose/build-api.sh"), phase],
            cwd=str(directory), env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            universal_newlines=True,
        )

    def test_platform_phase_and_jobs_matrix(self):
        for target, host in [
            ("amd64", "amd64"), ("arm64", "amd64"),
            ("arm64", "arm64"), ("amd64", "arm64"),
        ]:
            for phase in ["seed", "build"]:
                for jobs in [None, "", "3"]:
                    with self.subTest(target=target, host=host, phase=phase, jobs=jobs):
                        with tempfile.TemporaryDirectory(prefix="container-build-contract-") as temporary:
                            directory = Path(temporary)
                            result = self.run_build(directory, phase, target, host, jobs=jobs)
                            self.assertEqual(result.returncode, 0, result.stderr)
                            record = json.loads((directory / "record.json").read_text())
                            expected = ["build", "--locked", "--profile", "container", "--jobs", jobs or "2"]
                            cross = target == "arm64" and host != "arm64"
                            if cross:
                                expected += ["--target", "aarch64-unknown-linux-gnu"]
                            for binary in BINARIES:
                                expected += ["--bin", binary]
                            self.assertEqual(record["args"], expected)
                            self.assertEqual(record["env"], CROSS if cross else {})
                            if phase == "seed":
                                self.assertFalse((directory / "src").exists())
                                for binary in BINARIES:
                                    self.assertFalse((directory / binary).exists())
                            else:
                                self.assertEqual((directory / "src/main.rs").read_text(), "original source")
                                for binary in BINARIES:
                                    self.assertEqual((directory / binary).read_text(), "artifact:" + binary)

    def test_compile_failure_stops_before_copy_or_seed_cleanup(self):
        for phase in ["seed", "build"]:
            with self.subTest(phase=phase):
                with tempfile.TemporaryDirectory(prefix="container-build-failure-") as temporary:
                    directory = Path(temporary)
                    result = self.run_build(directory, phase, "arm64", "amd64", fail=True)
                    self.assertEqual(result.returncode, 42)
                    self.assertTrue((directory / "src").is_dir())
                    for binary in BINARIES:
                        self.assertFalse((directory / binary).exists())

    def test_invalid_phase_does_not_start_a_build(self):
        with tempfile.TemporaryDirectory(prefix="container-build-phase-") as temporary:
            directory = Path(temporary)
            result = self.run_build(directory, "unknown", "amd64", "amd64")
            self.assertEqual(result.returncode, 2)
            self.assertIn("Usage: build-api.sh seed|build", result.stderr)
            self.assertFalse((directory / "record.json").exists())
            self.assertFalse((directory / "src").exists())


if __name__ == "__main__":
    unittest.main()
