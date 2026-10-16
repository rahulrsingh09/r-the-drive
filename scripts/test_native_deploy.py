import importlib.util
import io
import json
from pathlib import Path
import sys
import unittest
from urllib.error import HTTPError

sys.path.insert(0, str(Path(__file__).parent))
from northflank_deploy import Halt
from northflank_deploy import free_policy
from test_northflank_deploy import fixture

spec = importlib.util.spec_from_file_location("native_deploy", Path(__file__).parent / "native-release/deploy.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
SHA = "a" * 40
NOW = 1791331200


class Fake:
    def __init__(self):
        self.writes = []
        self.fail_ci = False
        self.fail_budget = False
        self.build_sha = SHA
        self.build_branch = module.BRANCH
        self.build_success = True
        self.current = {"status": {"deployment": {"status": "COMPLETED"}},
                        "serviceType": "combined",
                        "vcsData": {"projectBranch": module.BRANCH,
                                    "projectUrl": "https://github.com/" + module.REPOSITORY},
                        "deployment": {"imageUrl": "old-image", "internal": {
                            "deployedSHA": "b" * 40, "branch": "hardening/private-telemetry"}}}

    def snapshot(self):
        if self.fail_budget:
            raise Halt("paid-usage")
        return {"app": self.current}

    def ci(self, sha, run_id):
        if self.fail_ci:
            raise Halt("ci-not-successful")

    def request(self, path, body=None):
        if body:
            if set(body["internal"]) != {"buildId"}:
                raise Halt("combined-service-source-change")
            self.writes.append((path, body))
            self.current = {"status": {"deployment": {"status": "COMPLETED"}},
                            "deployment": {"imageUrl": "new-image", "internal": {"deployedSHA": SHA}}}
            return {}
        if "/build/" in path:
            return {"id": "valid-build-1234", "sha": self.build_sha, "status": "SUCCESS",
                    "branch": self.build_branch,
                    "concluded": True, "success": self.build_success}
        return self.current


class NativeDeploymentTests(unittest.TestCase):
    def billing_adapter(self, pages):
        class Pages:
            def __init__(self):
                self.requests = []

            def open(self, request, timeout):
                self.requests.append(request)
                page = pages[len(self.requests) - 1]
                if isinstance(page, Exception):
                    raise page
                return io.BytesIO(json.dumps(page).encode())

        adapter = module.Adapter("synthetic-token")
        adapter.opener = Pages()
        return adapter

    def billing_page(self, total=0, cursor=None):
        return {"data": {"usage": [{"currency": "usd", "total": total}]},
                "pagination": {"hasNextPage": cursor is not None, "count": 1,
                               "cursor": cursor}}

    def test_billing_reads_every_page_and_rejects_later_paid_usage(self):
        for total in [0, 0.01]:
            adapter = self.billing_adapter([self.billing_page(cursor="123"),
                                            self.billing_page(total)])
            snapshot = fixture()
            snapshot["billing"] = adapter.request("/v1/billing/usage")
            self.assertEqual(len(snapshot["billing"]["usage"]), 2)
            self.assertEqual([r.full_url for r in adapter.opener.requests],
                             ["https://api.northflank.com/v1/billing/usage",
                              "https://api.northflank.com/v1/billing/usage?cursor=123"])
            self.assertTrue(all(r.get_method() == "GET" and
                                r.get_header("Authorization") == "Bearer synthetic-token"
                                for r in adapter.opener.requests))
            if total:
                with self.assertRaisesRegex(Halt, "paid-usage"):
                    free_policy(snapshot)
            else:
                free_policy(snapshot)

    def test_incomplete_or_malformed_billing_fails_closed(self):
        missing_flag = self.billing_page()
        del missing_flag["pagination"]["hasNextPage"]
        wrong_count = self.billing_page()
        wrong_count["pagination"]["count"] = 2
        for pages in [
            [missing_flag], [wrong_count],
            [self.billing_page(cursor="https://other.example/")],
            [self.billing_page(cursor="")],
            [self.billing_page(cursor="123"), self.billing_page(cursor="123")],
        ]:
            with self.subTest(pages=pages):
                adapter = self.billing_adapter(pages)
                with self.assertRaises(Halt):
                    adapter.request("/v1/billing/usage")
                self.assertEqual(len(adapter.opener.requests), len(pages))

    def test_billing_read_error_is_not_retried_or_accepted_as_complete(self):
        adapter = self.billing_adapter([self.billing_page(cursor="123"),
            HTTPError("https://api.northflank.com/v1/billing/usage", 401,
                      "private-response-secret", {}, None)])
        with self.assertRaisesRegex(Halt, "^northflank-read-http-401$"):
            adapter.request("/v1/billing/usage")
        self.assertEqual(len(adapter.opener.requests), 2)

    def test_billing_page_limit_and_other_pagination_remain_closed(self):
        adapter = self.billing_adapter([self.billing_page(cursor=str(i)) for i in range(100)])
        with self.assertRaisesRegex(Halt, "billing-page-limit"):
            adapter.request("/v1/billing/usage")
        self.assertEqual(len(adapter.opener.requests), 100)
        adapter = self.billing_adapter([self.billing_page(cursor="123")])
        with self.assertRaisesRegex(Halt, "incomplete-response"):
            adapter.request(module.APP)
        self.assertEqual(len(adapter.opener.requests), 1)

    def test_ci_accepts_only_genuine_protected_push_or_dispatch(self):
        class Metadata(module.Adapter):
            def __init__(self, event, branch=module.BRANCH):
                self.event, self.branch = event, branch

            def request(self, path, body=None, github=False):
                if "/branches/" in path:
                    if path != "/repos/your-org/r-the-drive/branches/mainline":
                        raise Halt("wrong-branch-endpoint")
                    return {"commit": {"sha": SHA}, "protected": True}
                return {"id": 101, "head_sha": SHA, "head_branch": self.branch,
                        "event": self.event, "workflow_id": 0,
                        "path": ".github/workflows/fork-ci.yml",
                        "repository": {"id": 0}, "head_repository": {"id": 0},
                        "status": "completed", "conclusion": "success"}

        for event in ["push", "workflow_dispatch"]:
            Metadata(event).ci(SHA, "101")
        for event in ["pull_request", "schedule", "workflow_run"]:
            with self.assertRaises(Halt):
                Metadata(event).ci(SHA, "101")
        for branch in ["main", "hardening/private-telemetry", "review/candidate"]:
            with self.assertRaises(Halt):
                Metadata("workflow_dispatch", branch).ci(SHA, "101")

    def run_deploy(self, adapter, **overrides):
        args = {"sha": SHA, "run_id": "101", "build_id": "valid-build-1234",
                "attestation": "provider warning\nPANDA_NATIVE_backup_OK " + SHA + " " + str(NOW)}
        args.update(overrides)
        return module.deploy(adapter, **args, now=lambda: NOW, sleep=lambda _: None)

    def test_exact_build_is_only_write(self):
        adapter = Fake()
        result = self.run_deploy(adapter)
        self.assertEqual(result["status"], "deployed-awaiting-verification")
        self.assertEqual(adapter.writes, [(module.APP + "/deployment", {"internal": {
            "buildId": "valid-build-1234"}})])

    def test_invalid_inputs_and_missing_or_stale_backup_never_write(self):
        cases = [
            {"sha": "$(touch /tmp/forbidden)"}, {"run_id": "1/anything"},
            {"build_id": "../other-service"}, {"attestation": ""},
            {"attestation": "PANDA_NATIVE_backup_OK " + "b" * 40 + " " + str(NOW)},
            {"attestation": "PANDA_NATIVE_backup_OK " + SHA + " " + str(NOW - 121)},
            {"attestation": "PANDA_NATIVE_backup_OK " + SHA + " " + str(NOW + 1)},
        ]
        for case in cases:
            with self.subTest(case=case):
                adapter = Fake()
                with self.assertRaises(Halt):
                    self.run_deploy(adapter, **case)
                self.assertFalse(adapter.writes)

    def test_ci_budget_and_wrong_build_fail_before_deploy(self):
        for field, value in [("fail_ci", True), ("fail_budget", True),
                             ("build_sha", "b" * 40), ("build_success", False),
                             ("build_branch", "main")]:
            with self.subTest(field=field):
                adapter = Fake()
                setattr(adapter, field, value)
                with self.assertRaises(Halt):
                    self.run_deploy(adapter)
                self.assertFalse(adapter.writes)

    def test_adapter_rejects_other_projects_and_config_writes(self):
        adapter = module.Adapter("synthetic-token")
        for path, body in [("/v1/projects/other/services/app", None),
                           (module.APP, {"runtimeEnvironment": {"unsafe": "value"}}),
                           (module.APP + "/deployment", {"external": {"image": "bad"}}),
                           (module.APP + "/deployment", {"internal": {
                               "buildId": "valid-build-1234", "branch": module.BRANCH}}),
                           (module.APP + "/deployment", {"internal": {"buildId": "../other"}})]:
            with self.subTest(path=path):
                with self.assertRaises(Halt):
                    adapter.request(path, body)

    def test_unexpected_combined_service_source_never_deploys(self):
        for key, value in [("projectBranch", "main"), ("projectUrl", "https://github.com/other/repo")]:
            adapter = Fake()
            adapter.current["vcsData"][key] = value
            with self.assertRaises(Halt):
                self.run_deploy(adapter)
            self.assertFalse(adapter.writes)

    def test_http_failures_report_only_service_operation_and_status_without_retry(self):
        class Reject:
            calls = 0

            def open(self, request, timeout):
                self.calls += 1
                raise HTTPError(request.full_url, 403, "private-response-secret", {}, None)

        cases = [
            (module.APP, None, False, "northflank-read-http-403"),
            ("/repos/your-org/r-the-drive/branches/mainline", None, True,
             "github-read-http-403"),
            (module.APP + "/deployment", {"internal": {
                "buildId": "valid-build-1234"}},
             False, "northflank-deployment-http-403"),
        ]
        for path, body, github, expected in cases:
            with self.subTest(expected=expected):
                adapter = module.Adapter("private-token-secret")
                adapter.opener = Reject()
                with self.assertRaises(Halt) as failure:
                    adapter.request(path, body, github=github)
                self.assertEqual(str(failure.exception), expected)
                self.assertEqual(adapter.opener.calls, 1)

    def test_only_one_included_job_is_allowed(self):
        snapshot = fixture()
        snapshot["jobs"]["jobs"] = [{"id": "verified-release-controller", "jobType": "manual"}]
        snapshot["jobDetails"] = {"verified-release-controller": {
            "billing": {"deploymentPlan": "nf-compute-20"}, "jobType": "manual",
            "settings": {"backoffLimit": 0, "activeDeadlineSeconds": 720, "runOnSourceChange": "never"}}}
        free_policy(snapshot, allowed_jobs=("verified-release-controller",))
        with self.assertRaises(Halt):
            free_policy(snapshot)
        snapshot["jobDetails"]["verified-release-controller"]["billing"]["deploymentPlan"] = "nf-compute-200"
        with self.assertRaises(Halt):
            free_policy(snapshot, allowed_jobs=("verified-release-controller",))


if __name__ == "__main__":
    unittest.main()
