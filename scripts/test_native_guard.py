"""Exercise the installed shell guard against an isolated synthetic PostgreSQL."""

import base64
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parent.parent
SHA = "a" * 40
IMAGE = "postgres:18.4-bookworm@sha256:1961f96e6029a02c3812d7cb329a3b03a3ac2bb067058dec17b0f5596aca9296"


class GuardTests(unittest.TestCase):
    @classmethod
    def docker(cls, *args, input=None, check=True):
        return subprocess.run(["docker", *args], input=input, text=True, capture_output=True,
                              check=check, timeout=90)

    @classmethod
    def setUpClass(cls):
        cls.name = "panda-native-guard-" + str(os.getpid())
        cls.tmp = tempfile.TemporaryDirectory()
        files = Path(cls.tmp.name)
        cls.catalog = json.loads((ROOT / "config/native-release-catalog.json").read_text())
        guard = (ROOT / "scripts/native-release/guard.sh").read_text()
        (files / "guard.sh").write_text("EXPECTED_LEDGER='" + json.dumps(cls.catalog) + "'\n" + guard)
        (files / "catalog").write_text(json.dumps(cls.catalog))
        repo = {"id": 0, "full_name": "your-org/r-the-drive"}
        cls.ci_run = {"id": 101, "workflow_id": 0, "path": ".github/workflows/fork-ci.yml",
                   "name": "Fork validation", "event": "push", "head_branch": "mainline",
                   "head_sha": SHA, "status": "completed", "conclusion": "success",
                   "run_number": 20, "run_attempt": 1, "repository": repo, "head_repository": repo}
        names = ["Fork frontend and policy", "Fork backend and security regressions"]
        branch = {"name": "mainline", "protected": True, "commit": {"sha": SHA},
                  "protection": {"required_status_checks": {"enforcement_level": "everyone",
                      "contexts": names, "checks": [{"context": n, "app_id": 15368} for n in names]}}}
        jobs = {"total_count": 2, "jobs": [{"id": i + 201, "name": n, "run_id": 101, "run_attempt": 1,
                 "head_sha": SHA, "head_branch": "mainline", "workflow_name": "Fork validation",
                 "status": "completed", "conclusion": "success"} for i, n in enumerate(names)]}
        for name, data in {"branch": branch, "run": cls.ci_run, "runs": {"total_count": 1, "workflow_runs": [cls.ci_run]},
                           "jobs": jobs}.items():
            (files / name).write_text(json.dumps(data))
        (files / "curl").write_text("""#!/bin/sh
set -eu
output=/dev/null
prev=
for arg in "$@"; do
  case "$prev" in --output|-o) output=$arg ;; esac
  prev=$arg
  url=$arg
done
case "$url" in
  */branches/*) file=branch ;;
  */attempts/*/jobs*) file=jobs ;;
  */runs/101) file=run ;;
  */runs\\?*) file=runs ;;
  */native-release-catalog.json) file=catalog ;;
  */health) printf 200; exit ;;
  */v1/vehicles) printf 401; exit ;;
  */) printf 403; exit ;;
  *) exit 1 ;;
esac
cp "/fixture/$file" "$output"
""")
        (files / "curl").chmod(0o755)
        cls.docker("run", "-d", "--rm", "--network", "none", "--name", cls.name,
                   "-e", "POSTGRES_HOST_AUTH_METHOD=trust", IMAGE)
        cls.docker("cp", str(files) + "/.", cls.name + ":/fixture")
        cls.docker("exec", cls.name, "sh", "-c", "mkdir -p /backups /app; chmod 755 /fixture/curl")
        for _ in range(60):
            if cls.docker("exec", cls.name, "sh", "-c",
                          'test "$(cat /proc/1/comm)" = postgres && pg_isready -U postgres',
                          check=False).returncode == 0:
                break
            time.sleep(1)
        else:
            raise RuntimeError("Synthetic database did not start")
        sql = """
CREATE SCHEMA panda; CREATE SCHEMA timeseries;
CREATE TABLE public._sqlx_migrations(version bigint, checksum bytea, success boolean);
CREATE TABLE timeseries.telemetry(id int);
CREATE TABLE panda.trips(id int);
CREATE TABLE panda.charge_sessions(id int);
CREATE TABLE panda.vehicle_state_periods(id int);
CREATE TABLE panda.vehicles(id int);
CREATE TABLE panda.users(id int);
CREATE TABLE panda.vehicle_credentials(vehicle_id int);
CREATE TABLE panda.vehicle_runtime_state(vehicle_id int, auth_state text, worker_health text,
  collector_heartbeat_at timestamptz, trip_persistence_error text);
CREATE TABLE panda.active_trip_checkpoints(vehicle_id int, snapshot jsonb);
CREATE TABLE panda.pending_trip_completions(id int);
INSERT INTO panda.vehicle_credentials VALUES(1);
INSERT INTO panda.vehicle_runtime_state VALUES(1,'authorized','connected',now(),null);
"""
        sql += "\n".join("INSERT INTO public._sqlx_migrations VALUES(%d,decode('%s','hex'),true);" %
                         (m["version"], m["checksum"]) for m in cls.catalog)
        cls.sql(sql)

    @classmethod
    def tearDownClass(cls):
        cls.docker("rm", "-f", cls.name, check=False)
        cls.tmp.cleanup()

    @classmethod
    def sql(cls, text):
        return cls.docker("exec", "-i", cls.name, "psql", "-X", "-U", "postgres",
                          "-v", "ON_ERROR_STOP=1", input=text)

    def setUp(self):
        self.docker("exec", self.name, "sh", "-c", "rm -rf /backups/native-release")
        self.docker("exec", "-i", self.name, "sh", "-c", "cat > /fixture/run", input=json.dumps(self.ci_run))
        self.docker("exec", "-i", self.name, "sh", "-c", "cat > /app/release-sha", input=SHA)
        self.sql("UPDATE panda.vehicle_runtime_state SET collector_heartbeat_at=now(); "
                 "DELETE FROM panda.active_trip_checkpoints;")

    def guard(self, phase, *, sha=SHA, key="synthetic-key"):
        env = {"DATABASE_URL": "postgresql://postgres@127.0.0.1/postgres",
               "PANDA_REQUIRE_GATEWAY": "true", "PANDA_ENV": "production",
               "JWT_SECRET": key, "JWT_PUBLIC_KEY": "synthetic-public-key",
               "AGE_ENCRYPTION_KEY": "synthetic-age-key", "REDIS_URL": "redis://synthetic",
               "PANDA_GATEWAY_TOKEN": "synthetic-edge", "PATH": "/fixture:/usr/local/bin:/usr/bin:/bin"}
        args = ["exec"]
        for name, value in env.items():
            args.extend(["-e", name + "=" + value])
        args.extend([self.name, "sh", "/fixture/guard.sh", phase,
                     base64.b64encode(sha.encode()).decode(), base64.b64encode(b"101").decode()])
        return self.docker(*args, check=False)

    def assert_guard(self, phase):
        result = self.guard(phase)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("PANDA_NATIVE_" + phase + "_OK", result.stdout)

    def test_full_backup_and_verification(self):
        for phase in ["preflight", "backup", "verify"]:
            self.assert_guard(phase)
        result = self.docker("exec", self.name, "sh", "-c",
                             "test ! -e /backups/native-release/attempt && "
                             "test -s /backups/native-release/" + SHA + "/database.dump")
        self.assertEqual(result.returncode, 0)

    def test_missing_backup_and_shell_input_do_not_advance(self):
        self.assertNotEqual(self.guard("verify").returncode, 0)
        self.assertNotEqual(self.guard("preflight", sha="$(touch /tmp/unsafe)").returncode, 0)

    def test_failed_ci_is_rejected_before_backup(self):
        run = dict(self.ci_run, conclusion="failure")
        self.docker("exec", "-i", self.name, "sh", "-c", "cat > /fixture/run", input=json.dumps(run))
        self.assertNotEqual(self.guard("preflight").returncode, 0)

    def test_explicit_default_ci_dispatch_preserves_backup_and_data_guards(self):
        run = dict(self.ci_run, event="workflow_dispatch")
        self.docker("exec", "-i", self.name, "sh", "-c", "cat > /fixture/run", input=json.dumps(run))
        for phase in ["preflight", "backup", "verify"]:
            self.assert_guard(phase)

    def test_dispatch_on_candidate_branch_and_other_events_are_rejected(self):
        for mutation in [
            {"event": "workflow_dispatch", "head_branch": "review/upstream/main/" + SHA},
            {"head_branch": "main"}, {"head_branch": "hardening/private-telemetry"},
            {"event": "pull_request"}, {"event": "schedule"}, {"event": "workflow_run"},
        ]:
            run = dict(self.ci_run, **mutation)
            self.docker("exec", "-i", self.name, "sh", "-c", "cat > /fixture/run", input=json.dumps(run))
            self.assertNotEqual(self.guard("preflight").returncode, 0)

    def test_active_trip_is_rejected(self):
        self.sql("""INSERT INTO panda.active_trip_checkpoints VALUES(1,'{"detector":{"active_trip_id":"synthetic"}}');""")
        self.assertNotEqual(self.guard("preflight").returncode, 0)

    def test_changed_keys_are_rejected_before_deployment(self):
        self.assert_guard("preflight")
        self.assertNotEqual(self.guard("backup", key="changed-key").returncode, 0)

    def test_changed_image_stamp_is_rejected(self):
        self.assert_guard("preflight")
        self.assert_guard("backup")
        self.docker("exec", "-i", self.name, "sh", "-c", "cat > /app/release-sha", input="b" * 40)
        self.assertNotEqual(self.guard("verify").returncode, 0)


if __name__ == "__main__":
    unittest.main()
