"""Synthetic adapters only: never load the operator config or contact Northflank."""

import copy
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch
from urllib.error import HTTPError

import northflank_deploy as deploy


OLD = "a" * 40
NEW = "b" * 40
IMAGE = "registry.northflank.com/northflank/service/123:456"
NEW_IMAGE = "registry.northflank.com/northflank/service/123:789"


def ledger(*versions):
    return [{"version": version, "checksum": f"{version:096x}", "success": True}
            for version in versions]


def audit(versions=(1, 28)):
    return {"ledger": ledger(*versions), "schemaReady": True,
            "counts": dict.fromkeys(deploy.HISTORY, 2),
            "collector": {"healthy": 2, "credentials": 2}}


def fixture():
    return {
        "app": {"id": "telemetry-app",
                "billing": {"deploymentPlan": "nf-compute-20", "buildPlan": "nf-compute-400-16"},
                "deployment": {"instances": 1, "imageUrl": IMAGE,
                               "internal": {"deployedSHA": OLD, "buildSHA": "latest",
                                            "branch": deploy.BRANCH, "nfObjectId": "telemetry-app"}},
                "disabledCI": True,
                "vcsData": {"projectUrl": "https://github.com/" + deploy.REPOSITORY,
                            "projectBranch": deploy.BRANCH},
                "status": {"deployment": {"status": "COMPLETED"}},
                "ports": [{"name": "http", "internalPort": 8080, "protocol": "HTTP",
                           "public": True, "vpcAccessible": False, "domains": []}]},
        "redis": {"id": "telemetry-redis", "billing": {"deploymentPlan": "nf-compute-20"},
                  "deployment": {"instances": 1},
                  "ports": [{"public": False, "vpcAccessible": False}]},
        "db": {"id": "telemetry-db", "spec": {"type": "postgresql", "pendingActions": [],
               "config": {"deployment": {"planId": "nf-compute-20", "replicas": 1,
                                         "storageSize": 6144},
                          "networking": {"externalAccessEnabled": False}}}},
        "services": {"services": [{"id": "telemetry-app"}, {"id": "telemetry-redis"}]},
        "addons": {"addons": [{"id": "telemetry-db"}]},
        "jobs": {"jobs": []},
        "volumes": [{"id": "app-data", "spec": {"storageSize": 6144},
                     "owningObject": {"id": "telemetry-app", "type": "service"},
                     "attachedObjects": [{"id": "telemetry-app", "type": "service"}]}],
        "buildArguments": {"buildArguments": {}, "buildFiles": {}, "dockerSecretMounts": []},
        "billing": {"usage": [{"currency": "usd", "total": 0}]},
    }


class Fake:
    def __init__(self):
        self.cloud = fixture()
        self.calls = []
        self.fail = None
        self.readiness_reads = 0
        self.stale_read = None
        self.runtimes = 0
        self.rotate_runtime_at = None
        self.runtime_data = {key: "synthetic-" + key for key in deploy.KEYS}
        self.runtime_data.update(PANDA_REQUIRE_GATEWAY="true", PANDA_ENV="production")
        self.after = audit((1, 28, 1000028, 1000029, 1000030))

    def event(self, name):
        self.calls.append(name)
        if self.fail == name:
            raise deploy.Halt("synthetic-failure")

    def readiness(self, sha):
        self.event("readiness")
        self.readiness_reads += 1
        if self.stale_read == self.readiness_reads:
            raise deploy.Halt("stale-or-foreign-sha")
        return {"sha": sha, "runId": 123, "runAttempt": 1}

    def snapshot(self):
        self.event("snapshot")
        deploy.free_policy(self.cloud)
        return copy.deepcopy(self.cloud)

    def request(self, method, path):
        self.event("list-builds")
        assert method == "GET" and path == deploy.APP + "/build"
        return {"builds": []}

    def runtime(self):
        self.event("runtime")
        self.runtimes += 1
        result = dict(self.runtime_data)
        if self.runtimes == self.rotate_runtime_at:
            result["JWT_SECRET"] = "synthetic-rotated"
        return result

    def gates(self, runtime):
        self.event("gates")

    def audit(self):
        self.event("audit")
        current = self.cloud["app"]["deployment"]["internal"]["deployedSHA"]
        return copy.deepcopy(self.after if current == NEW else audit())

    def pin(self, sha):
        self.event("pin-old" if sha == OLD else "pin-new")
        internal = self.cloud["app"]["deployment"]["internal"]
        internal.update(buildSHA=sha, deployedSHA=sha)
        self.cloud["app"]["deployment"]["imageUrl"] = IMAGE if sha == OLD else NEW_IMAGE

    def wait_deployment(self, sha):
        self.event("wait-old" if sha == OLD else "wait-new")
        return copy.deepcopy(self.cloud["app"])

    def start_build(self, sha):
        assert sha == NEW
        self.event("start-build")
        return "synthetic-build-123"

    def wait_build(self, build_id, sha):
        assert (build_id, sha) == ("synthetic-build-123", NEW)
        self.event("wait-build")

    def backup(self, directory):
        self.event("backup")
        return {"sha256": "c" * 64, "bytes": 99, "local": str(directory / "database.dump")}


class ControllerTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.state_dir = deploy.private_directory(self.root / "state")
        self.reviews = deploy.private_directory(self.root / "reviews")
        self.config = SimpleNamespace(state_dir=self.state_dir, reviews_dir=self.reviews,
                                      state=self.state_dir / "deployed.json")
        deploy.atomic_json(self.config.state, {"schemaVersion": 1, "deployedShas": [OLD],
                                             "attempt": None})
        deploy.atomic_json(self.reviews / (NEW + ".json"),
                           {"schemaVersion": 1, "sha": NEW, "baseSha": OLD, "approved": True,
                            "migrationLedger": ledger(1, 28, 1000028, 1000029, 1000030)})
        self.adapter = Fake()

    def state(self):
        return deploy.read_private(self.config.state)

    def run_controller(self):
        return deploy.run(self.config, self.adapter, NEW)

    def test_success_order_recovery_files_permissions_and_idempotence(self):
        self.assertEqual(self.run_controller(), {"status": "deployed", "sha": NEW})
        calls = self.adapter.calls
        self.assertLess(calls.index("pin-old"), calls.index("start-build"))
        self.assertLess(calls.index("wait-build"), calls.index("backup"))
        self.assertLess(calls.index("backup"), calls.index("pin-new"))
        self.assertEqual(calls[calls.index("start-build") - 1], "readiness")
        self.assertEqual(calls[calls.index("pin-new") - 1], "readiness")
        self.assertEqual(self.state()["deployedShas"], [OLD, NEW])
        self.assertIsNone(self.state()["attempt"])
        self.assertEqual(deploy.read_private(self.state_dir / NEW / "runtime-before.json"),
                         self.adapter.runtime_data)
        for path in (self.state_dir / NEW).iterdir():
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
        self.assertEqual((self.state_dir / NEW).stat().st_mode & 0o777, 0o700)
        count = len(calls)
        self.assertEqual(self.run_controller()["status"], "already-deployed")
        self.assertEqual(len(calls), count)

    def test_stale_sha_initial_before_build_and_before_deploy(self):
        for read in (1, 2, 3):
            with self.subTest(read=read):
                self.adapter = Fake()
                self.adapter.stale_read = read
                deploy.atomic_json(self.config.state, {"schemaVersion": 1, "deployedShas": [OLD],
                                                       "attempt": None})
                candidate_dir = self.state_dir / NEW
                if candidate_dir.exists():
                    import shutil
                    shutil.rmtree(candidate_dir)
                with self.assertRaisesRegex(deploy.Halt, "stale-or-foreign-sha"):
                    self.run_controller()
                self.assertNotIn("pin-new", self.adapter.calls)
                if read < 3:
                    self.assertNotIn("start-build", self.adapter.calls)
                self.assertEqual(self.state()["deployedShas"], [OLD])

    def test_failed_or_uncertain_mutations_never_succeed_or_retry(self):
        for step in ("pin-old", "start-build", "wait-build", "backup", "pin-new", "wait-new"):
            with self.subTest(step=step):
                self.setUp()
                self.adapter.fail = step
                with self.assertRaises(deploy.Halt):
                    self.run_controller()
                self.assertEqual(self.state()["deployedShas"], [OLD])
                self.assertIsNotNone(self.state()["attempt"])
                calls = list(self.adapter.calls)
                with self.assertRaisesRegex(deploy.Halt, "attempt-needs-reconciliation"):
                    self.run_controller()
                self.assertEqual(self.adapter.calls, calls)
                if step in ("pin-old", "start-build", "wait-build", "backup"):
                    self.assertNotIn("pin-new", calls)

    def test_uncertain_attempt_blocks_other_shas_too(self):
        state = self.state()
        state["attempt"] = {"sha": "c" * 40, "phase": "deploy-started"}
        deploy.atomic_json(self.config.state, state)
        with self.assertRaisesRegex(deploy.Halt, "attempt-needs-reconciliation"):
            self.run_controller()
        self.assertEqual(self.adapter.calls, [])

    def test_lock_refuses_concurrent_controller(self):
        with deploy.deployment_lock(self.state_dir / "deployment.lock"):
            with self.assertRaisesRegex(deploy.Halt, "deployment-locked"):
                self.run_controller()
        self.assertEqual(self.adapter.calls, [])
        self.assertEqual(self.run_controller()["status"], "deployed")

    def test_runtime_rotation_blocks_deploy_or_success(self):
        for read in (2, 3, 4):
            with self.subTest(read=read):
                self.setUp()
                self.adapter.rotate_runtime_at = read
                with self.assertRaisesRegex(deploy.Halt, "runtime-changed"):
                    self.run_controller()
                self.assertNotIn(NEW, self.state()["deployedShas"])

    def test_failed_postdeploy_data_verification_never_marks_success(self):
        self.adapter.after["counts"]["trips"] = 1
        with self.assertRaisesRegex(deploy.Halt, "history-count-decreased"):
            self.run_controller()
        self.assertEqual(self.state()["attempt"]["phase"], "verifying")
        self.assertNotIn(NEW, self.state()["deployedShas"])

    def test_missing_review_or_different_base_never_mutates(self):
        review_path = self.reviews / (NEW + ".json")
        for change in ({"approved": False}, {"baseSha": "d" * 40},
                       {"migrationLedger": ledger(1, 27)}):
            with self.subTest(change=change):
                review = deploy.read_private(review_path)
                review.update(approved=True, baseSha=OLD,
                              migrationLedger=ledger(1, 28, 1000028))
                review.update(change)
                deploy.atomic_json(review_path, review)
                with self.assertRaises(deploy.Halt):
                    self.run_controller()
                self.assertNotIn("pin-old", self.adapter.calls)

    def test_atomic_write_failure_preserves_original(self):
        original = self.config.state.read_bytes()
        with patch.object(deploy.os, "replace", side_effect=OSError("synthetic-disk-error")):
            with self.assertRaises(OSError):
                deploy.atomic_json(self.config.state, {"invalid": True})
        self.assertEqual(self.config.state.read_bytes(), original)
        self.assertEqual(list(self.state_dir.glob(".write-*")), [])


class PolicyTests(unittest.TestCase):
    def test_no_paid_guard_and_no_exposure(self):
        mutations = [
            lambda s: s["services"]["services"].append({"id": "extra"}),
            lambda s: s["addons"]["addons"].append({"id": "extra"}),
            lambda s: s["jobs"]["jobs"].append({"id": "restore"}),
            lambda s: s["app"]["billing"].update(deploymentPlan="nf-compute-50"),
            lambda s: s["redis"]["billing"].update(deploymentPlan="nf-compute-50"),
            lambda s: s["app"]["billing"].update(buildPlan="nf-compute-800-32"),
            lambda s: s["app"]["deployment"].update(instances=2),
            lambda s: s["redis"]["deployment"].update(instances=0),
            lambda s: s["app"]["deployment"].update(autoscaling={"enabled": True}),
            lambda s: s["app"].update(disabledCI=False),
            lambda s: s["app"].pop("disabledCI"),
            lambda s: s["app"]["vcsData"].update(projectBranch="review/untrusted"),
            lambda s: s["db"]["spec"]["config"]["deployment"].update(storageSize=6145),
            lambda s: s["db"]["spec"]["config"]["deployment"].update(replicas=2),
            lambda s: s["db"]["spec"]["config"]["deployment"].update(planId="nf-compute-50"),
            lambda s: s["db"]["spec"]["config"]["networking"].update(externalAccessEnabled=True),
            lambda s: s["db"]["spec"].update(pendingActions=[{"action": "resize"}]),
            lambda s: s["volumes"][0]["spec"].update(storageSize=6145),
            lambda s: s["volumes"].append(copy.deepcopy(s["volumes"][0])),
            lambda s: s["volumes"][0].update(attachedObjects=[{"id": "other", "type": "job"}]),
            lambda s: s["redis"]["ports"][0].update(public=True),
            lambda s: s["billing"]["usage"][0].update(total=0.01),
            lambda s: s["billing"]["usage"][0].update(total=-1),
            lambda s: s["billing"]["usage"][0].update(total=float("nan")),
            lambda s: s["billing"]["usage"][0].update(total=False),
            lambda s: s["billing"].update(usage=[]),
            lambda s: s["billing"]["usage"][0].pop("total"),
            lambda s: s["buildArguments"]["buildArguments"].update(TOKEN="synthetic"),
            lambda s: s["buildArguments"]["buildFiles"].update(secret="synthetic"),
            lambda s: s["buildArguments"]["dockerSecretMounts"].append({"id": "secret"}),
            lambda s: s["services"].update(total=3),
        ]
        deploy.free_policy(fixture())
        for index, mutation in enumerate(mutations):
            with self.subTest(index=index):
                cloud = fixture()
                mutation(cloud)
                with self.assertRaises((deploy.Halt, KeyError)):
                    deploy.free_policy(cloud)

    def test_ledger_prefix_checksum_success_and_counts(self):
        expected = ledger(1, 28, 1000028)
        good = audit((1, 28, 1000028))
        deploy.compare_audits(audit(), good, expected)
        mutations = [
            lambda a: a["ledger"].pop(0),
            lambda a: a["ledger"][0].update(checksum="f" * 96),
            lambda a: a["ledger"][1].update(success=False),
            lambda a: a.update(ledger=list(reversed(a["ledger"]))),
            lambda a: a["ledger"].append(ledger(1000029)[0]),
            lambda a: a["counts"].update(trips=1),
            lambda a: a["counts"].pop("telemetry"),
            lambda a: a["counts"].update(credentials=3),
            lambda a: a["collector"].update(healthy=1),
            lambda a: a.update(schemaReady=False),
        ]
        for index, mutation in enumerate(mutations):
            with self.subTest(index=index):
                after = copy.deepcopy(good)
                mutation(after)
                with self.assertRaises(deploy.Halt):
                    deploy.compare_audits(audit(), after, expected)

    def test_remote_script_is_read_only_and_uses_correct_ledger_schema(self):
        self.assertIn("READ ONLY;", deploy.AUDIT_SCRIPT)
        self.assertIn("FROM public._sqlx_migrations", deploy.AUDIT_SCRIPT)
        self.assertNotIn("panda._sqlx_migrations", deploy.AUDIT_SCRIPT)
        self.assertNotIn("encrypted_tokens", deploy.AUDIT_SCRIPT)
        self.assertIn("options=-c%20", deploy.AUDIT_SCRIPT)
        for forbidden in ("DELETE ", "UPDATE ", "INSERT ", "pg_restore --clean"):
            self.assertNotIn(forbidden, deploy.AUDIT_SCRIPT)


class AdapterTests(unittest.TestCase):
    def adapter(self):
        adapter = object.__new__(deploy.Northflank)
        adapter.config = SimpleNamespace(node=Path("/fixed/node"), nf_cli=Path("/fixed/cli.js"),
                                         nf_config_dir=Path("/private/nf"), root=Path("/fixed"),
                                         origin="https://origin.example", access="https://edge.example")
        adapter.env = {"PATH": "/fixed:/usr/bin:/bin", "HOME": "/fixed", "LANG": "C.UTF-8"}
        return adapter

    def test_cli_pin_is_only_exact_sha_and_no_runtime_or_trigger_write(self):
        adapter = self.adapter()
        with patch.object(adapter, "cli") as cli:
            adapter.pin(NEW)
        args = cli.call_args[0]
        self.assertEqual(args[:3], ("update", "service", "deployment"))
        self.assertEqual(json.loads(args[-1]), {"internal": {
            "id": "telemetry-app", "branch": deploy.BRANCH, "buildSHA": NEW}})
        with self.assertRaises(deploy.Halt):
            adapter.pin("latest")

    def test_command_environment_drops_inherited_credentials_and_node_injection(self):
        adapter = self.adapter()
        poison = {"NF_TOKEN": "secret-nf", "GH_TOKEN": "secret-gh", "DATABASE_URL": "secret-db",
                  "NODE_OPTIONS": "--require=/malicious", "HTTPS_PROXY": "https://malicious"}
        result = SimpleNamespace(returncode=0, stdout="ok", stderr="")
        with patch.dict(os.environ, poison), patch.object(deploy.subprocess, "run",
                                                         return_value=result) as call:
            adapter.command(["/fixed/node", "/fixed/readiness.mjs"])
            self.assertEqual(call.call_args[1]["env"], adapter.env)
            adapter.cli("get", "service")
            env = call.call_args[1]["env"]
            self.assertEqual(env, {**adapter.env, "NF_CONFIG_DIR": "/private/nf"})
            self.assertNotIn("secret", repr(env))
            self.assertEqual(call.call_args[0][0][-1], "--quiet")

    def test_command_errors_do_not_include_remote_output(self):
        adapter = self.adapter()
        with patch.object(deploy.subprocess, "run", return_value=SimpleNamespace(
                returncode=1, stdout="secret-db", stderr="secret-token")):
            with self.assertRaisesRegex(deploy.Halt, "^local-command-failed$"):
                adapter.command(["/fixed/node"])
        with patch.object(deploy.subprocess, "run",
                          side_effect=subprocess.TimeoutExpired(["secret-arg"], 1)):
            with self.assertRaisesRegex(deploy.Halt, "^local-command-unavailable-or-timeout$"):
                adapter.command(["/fixed/node"])

    def test_readiness_forwards_only_the_configured_cli_path(self):
        adapter = self.adapter()
        adapter.config.readiness = Path("/private/readiness.mjs")
        adapter.config.state = Path("/private/deployed.json")
        item = {"sha": NEW, "repository": deploy.REPOSITORY, "branch": deploy.BRANCH,
                "id": f"{deploy.REPOSITORY}:{deploy.BRANCH}:{NEW}"}
        output = json.dumps({"schemaVersion": 1, "ready": True, "items": [item]})
        for binary in (None, Path("/private/gh")):
            adapter.config.github_cli = binary
            with patch.object(deploy, "digest", return_value=deploy.READINESS_HASH), \
                 patch.object(adapter, "command", return_value=output) as command:
                self.assertEqual(adapter.readiness(NEW), item)
            expected = ["/fixed/node", "/private/readiness.mjs",
                        "--state", "/private/deployed.json"]
            if binary:
                expected.extend(["--github-cli", "/private/gh"])
            self.assertEqual(command.call_args[0][0], expected)

    def test_build_failure_wrong_sha_and_timeout(self):
        adapter = self.adapter()
        cases = [
            {"id": "build", "sha": NEW, "status": "FAILURE", "concluded": True, "success": False},
            {"id": "build", "sha": OLD, "status": "SUCCESS", "concluded": True, "success": True},
            {"id": "other", "sha": NEW, "status": "SUCCESS", "concluded": True, "success": True},
            {"id": "build", "sha": NEW, "status": "UNKNOWN", "concluded": False},
            *({"id": "build", "sha": NEW, "status": status, "concluded": False}
              for status in ("UNSCHEDULABLE", "ABORTED", "FAILURE",
                             "SUBMISSION_FAILURE", "CRASHED", "SUCCESS")),
        ]
        for result in cases:
            with patch.object(adapter, "request", return_value=result):
                with self.assertRaises(deploy.Halt):
                    adapter.wait_build("build", NEW)
        with patch.object(deploy.time, "monotonic", side_effect=[0, 1801]):
            with self.assertRaisesRegex(deploy.Halt, "build-timeout"):
                adapter.wait_build("build", NEW)

    def test_documented_build_progress_requires_explicit_concluded_success(self):
        adapter = self.adapter()
        progress = ("QUEUED", "PENDING", "STARTING", "CLONING",
                    "BUILDING", "UPLOADING", "IN_PROGRESS")
        responses = [{"id": "build", "sha": NEW, "status": status, "concluded": False}
                     for status in progress]
        responses.append({"id": "build", "sha": NEW, "status": "SUCCESS",
                          "concluded": True, "success": True})
        with patch.object(adapter, "request", side_effect=responses) as request, \
             patch.object(deploy.time, "sleep") as sleep:
            adapter.wait_build("build", NEW)
        self.assertEqual(request.call_count, len(responses))
        self.assertEqual(sleep.call_count, len(progress))
        self.assertTrue(all(call[0] == ("GET", deploy.APP + "/build/build")
                            for call in request.call_args_list))

    def test_provider_build_route_and_in_progress_rollout(self):
        adapter = self.adapter()
        build = {"id": "build", "sha": NEW, "status": "SUCCESS",
                 "concluded": True, "success": True}
        with patch.object(adapter, "request", return_value=build) as request:
            adapter.wait_build("build", NEW)
            request.assert_called_once_with("GET", deploy.APP + "/build/build")
        pending = fixture()["app"]
        pending["status"]["deployment"]["status"] = "IN_PROGRESS"
        complete = copy.deepcopy(pending)
        complete["status"]["deployment"]["status"] = "COMPLETED"
        complete["deployment"]["internal"].update(deployedSHA=NEW, buildSHA=NEW)
        with patch.object(adapter, "request", side_effect=[pending, complete]), \
             patch.object(deploy.time, "sleep"):
            self.assertEqual(adapter.wait_deployment(NEW), complete)

    def test_failed_deployment_and_completed_wrong_sha_cannot_succeed(self):
        adapter = self.adapter()
        cloud = fixture()["app"]
        cloud["status"]["deployment"]["status"] = "FAILED"
        with patch.object(adapter, "request", return_value=cloud):
            with self.assertRaisesRegex(deploy.Halt, "deployment-failed"):
                adapter.wait_deployment(NEW)
        cloud["status"]["deployment"]["status"] = "COMPLETED"
        with patch.object(adapter, "request", return_value=cloud), \
             patch.object(deploy.time, "sleep"), \
             patch.object(deploy.time, "monotonic", side_effect=[0, 1, 601]):
            with self.assertRaisesRegex(deploy.Halt, "deployment-timeout"):
                adapter.wait_deployment(NEW)

    def test_backup_integrity_and_fixed_command(self):
        adapter = self.adapter()
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            target = directory / "database.dump"
            target.write_bytes(b"synthetic-dump")
            checksum = deploy.digest(target)
            with patch.object(adapter, "remote",
                              return_value=f"PANDA_BACKUP={checksum} 14\n") as remote, \
                 patch.object(adapter, "cli") as cli:
                result = adapter.backup(directory)
                self.assertEqual(result["sha256"], checksum)
                self.assertEqual(target.stat().st_mode & 0o777, 0o600)
                script = remote.call_args[0][0]
                self.assertIn('pg_restore --list "$backup" >/dev/null', script)
                self.assertIn("pg_dump --format=custom --no-owner --no-privileges", script)
                self.assertIn('test ! -e "$backup"', script)
                self.assertEqual(cli.call_args[0][:3], ("download", "service", "file"))
                target.write_bytes(b"corrupted")
                with self.assertRaisesRegex(deploy.Halt, "backup-checksum-mismatch"):
                    adapter.backup(directory)
            with patch.object(adapter, "remote", return_value="PANDA_BACKUP=bad 0"):
                with self.assertRaisesRegex(deploy.Halt, "invalid-backup-evidence"):
                    adapter.backup(directory)

    def test_access_gates_never_send_app_auth_or_edge_key_to_workers(self):
        adapter = self.adapter()
        runtime = {"PANDA_GATEWAY_TOKEN": "synthetic-edge"}
        good = [(200, ""), (401, ""), (403, ""),
                (302, "https://team.cloudflareaccess.com/cdn-cgi/access/login/example")]
        with patch.object(adapter, "probe", side_effect=good) as probe:
            adapter.gates(runtime)
            calls = probe.call_args_list
            self.assertEqual(calls[0][0][1], {"X-Panda-Edge": "synthetic-edge"})
            self.assertEqual(calls[2][0][0], "https://origin.example/")
            self.assertEqual(len(calls[2][0]), 1)
            self.assertEqual(len(calls[3][0]), 1)
        for index, bad in ((0, (500, "")), (1, (200, "")), (2, (200, "")),
                           (3, (200, "")), (3, (302, "https://evil.example/login"))):
            responses = list(good)
            responses[index] = bad
            with patch.object(adapter, "probe", side_effect=responses):
                with self.assertRaises(deploy.Halt):
                    adapter.gates(runtime)

    def test_http_errors_return_status_without_following_or_echoing(self):
        adapter = self.adapter()
        adapter.opener = SimpleNamespace(open=lambda *a, **k: None)
        with patch.object(adapter.opener, "open", side_effect=HTTPError(
                "https://origin.example", 403, "secret-body", {}, io.BytesIO(b"secret"))):
            self.assertEqual(adapter.probe("https://origin.example"), (403, ""))
        self.assertIsNone(deploy.NoRedirect().redirect_request(None, None, 302, "", {},
                                                             "https://evil.example"))

    def test_api_mutation_allowlist_and_pagination_fail_closed(self):
        adapter = self.adapter()
        for method, path, body in (("DELETE", deploy.APP, None),
                                   ("PATCH", deploy.APP + "/runtime-environment", {}),
                                   ("POST", deploy.APP + "/build", {"sha": "latest"})):
            with self.assertRaisesRegex(deploy.Halt, "unapproved-api-write"):
                adapter.request(method, path, body)


class LocalTrustTests(unittest.TestCase):
    def test_symlinks_public_secrets_and_git_controls_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            secret = root / "config.json"
            secret.write_text("{}")
            secret.chmod(0o644)
            with self.assertRaisesRegex(deploy.Halt, "path-permissions"):
                deploy.read_private(secret)
            secret.chmod(0o600)
            link = root / "link"
            link.symlink_to(secret)
            with self.assertRaisesRegex(deploy.Halt, "noncanonical-path"):
                deploy.read_private(link)
            (root / ".git").write_text("gitdir: somewhere")
            with self.assertRaisesRegex(deploy.Halt, "control-path-in-git"):
                deploy.outside_git(root / "child")


if __name__ == "__main__":
    unittest.main()
