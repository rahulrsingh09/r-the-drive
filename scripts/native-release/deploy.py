"""Pinned Northflank job: deploy an already built and backed-up exact commit."""

import json
import os
import re
import sys
import time
from urllib.error import HTTPError
from urllib.request import Request, ProxyHandler, HTTPRedirectHandler, build_opener

from northflank_deploy import free_policy, require, Halt

PROJECT = "/v1/projects/panda-private"
APP = PROJECT + "/services/telemetry-app"
REPOSITORY = "your-org/r-the-drive"
BRANCH = "mainline"


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        raise Halt("redirect-rejected")


def read_billing_usage(fetch_page):
    usage, seen = [], set()
    cursor = None
    for _ in range(100):
        result = fetch_page(cursor)
        pagination = result.get("pagination", {})
        rows = result.get("data", {}).get("usage")
        require(isinstance(rows, list) and type(pagination.get("hasNextPage")) is bool
                and type(pagination.get("count")) is int
                and pagination["count"] == len(rows), "invalid-billing-page")
        usage.extend(rows)
        if not pagination["hasNextPage"]:
            return {"usage": usage}
        cursor = pagination.get("cursor")
        require(rows and isinstance(cursor, str) and re.fullmatch("[0-9]{1,20}", cursor)
                and cursor not in seen, "invalid-billing-cursor")
        seen.add(cursor)
    raise Halt("billing-page-limit")


class Adapter:
    def __init__(self, token):
        self.token = token
        self.opener = build_opener(ProxyHandler({}), NoRedirect())

    def request(self, path, body=None, github=False):
        if github:
            require(body is None and path.startswith("/repos/" + REPOSITORY + "/"),
                    "unapproved-github-request")
            host = "https://api.github.com"
            headers = {"Accept": "application/vnd.github+json"}
        else:
            require(path.startswith(PROJECT + "/") or path == "/v1/billing/usage",
                    "unapproved-cloud-request")
            require(body is None or (path == APP + "/deployment"
                    and set(body) == {"internal"}
                    and set(body["internal"]) == {"buildId"}
                    and isinstance(body["internal"]["buildId"], str)
                    and re.fullmatch("[a-z0-9-]{3,54}", body["internal"]["buildId"])),
                    "unapproved-cloud-write")
            host = "https://api.northflank.com"
            headers = {"Authorization": "Bearer " + self.token, "Content-Type": "application/json"}
        def fetch_page(cursor=None):
            suffix = "?cursor=" + cursor if cursor is not None else ""
            req = Request(host + path + suffix, data=json.dumps(body).encode() if body else None,
                          headers=headers, method="POST" if body else "GET")
            with self.opener.open(req, timeout=30) as response:
                data = response.read(2 * 1024 * 1024 + 1)
            require(len(data) <= 2 * 1024 * 1024, "response-too-large")
            return json.loads(data)

        try:
            if not github and path == "/v1/billing/usage":
                return read_billing_usage(fetch_page)
            result = fetch_page()
            pagination = result.get("pagination", {})
            require(not pagination.get("hasNextPage") and not pagination.get("next")
                    and pagination.get("totalPages", 1) <= 1, "incomplete-response")
            return result if github else result.get("data", result)
        except Halt:
            raise
        except HTTPError as error:
            service = "github" if github else "northflank"
            operation = "deployment" if body is not None else "read"
            raise Halt("{}-{}-http-{}".format(service, operation, error.code)) from None
        except Exception:
            raise Halt("cloud-request-unavailable-or-uncertain") from None

    def snapshot(self):
        paths = {"app": APP, "redis": PROJECT + "/services/telemetry-redis",
                 "db": PROJECT + "/addons/telemetry-db",
                 "services": PROJECT + "/services", "addons": PROJECT + "/addons",
                 "jobs": PROJECT + "/jobs", "volumes": PROJECT + "/volumes",
                 "buildArguments": APP + "/build-arguments", "billing": "/v1/billing/usage"}
        snapshot = {name: self.request(path) for name, path in paths.items()}
        snapshot["jobDetails"] = {"verified-release-controller":
            self.request(PROJECT + "/jobs/verified-release-controller")}
        free_policy(snapshot, allowed_jobs=("verified-release-controller",))
        return snapshot

    def ci(self, sha, run_id):
        base = "/repos/" + REPOSITORY
        branch = self.request(base + "/branches/mainline", github=True)
        require(branch["commit"]["sha"] == sha and branch.get("protected") is True,
                "protected-branch-moved")
        run = self.request(base + "/actions/runs/" + run_id, github=True)
        require(str(run["id"]) == run_id and run["head_sha"] == sha
                and run["head_branch"] == BRANCH and run["event"] in ("push", "workflow_dispatch")
                and run["workflow_id"] == 0 and run["path"] == ".github/workflows/fork-ci.yml"
                and run["repository"]["id"] == 0
                and run["head_repository"]["id"] == 0
                and run["status"] == "completed" and run["conclusion"] == "success",
                "ci-not-successful")


def deploy(adapter, sha, run_id, build_id, attestation, *, now=time.time, sleep=time.sleep):
    require(isinstance(sha, str) and re.fullmatch("[0-9a-f]{40}", sha), "invalid-sha")
    require(isinstance(run_id, str) and re.fullmatch("[1-9][0-9]{0,19}", run_id), "invalid-run")
    require(isinstance(build_id, str) and re.fullmatch("[a-z0-9-]{3,54}", build_id), "invalid-build")
    matches = re.findall(r"^PANDA_NATIVE_backup_OK ([0-9a-f]{40}) ([0-9]{10})$",
                         attestation, re.MULTILINE)
    require(len(matches) == 1 and matches[0][0] == sha, "backup-not-attested")
    require(0 <= now() - int(matches[0][1]) < 120, "backup-not-fresh")
    snapshot = adapter.snapshot()
    app = snapshot["app"]
    require(app.get("serviceType") == "combined"
            and app.get("vcsData", {}).get("projectBranch") == BRANCH
            and app.get("vcsData", {}).get("projectUrl") == "https://github.com/" + REPOSITORY,
            "unexpected-build-source")
    require(app["status"]["deployment"]["status"] == "COMPLETED", "deployment-active")
    old_image = app["deployment"]["imageUrl"]
    build = adapter.request(APP + "/build/" + build_id)
    require(build.get("id") == build_id and build.get("sha") == sha
            and build.get("branch") == BRANCH and build.get("status") == "SUCCESS"
            and build.get("concluded") is True and build.get("success") is True,
            "exact-build-not-ready")
    adapter.ci(sha, run_id)
    require(0 <= now() - int(matches[0][1]) < 120, "backup-expired")
    adapter.request(APP + "/deployment", {"internal": {"buildId": build_id}})
    deadline = now() + 600
    while now() < deadline:
        app = adapter.request(APP)
        status = app["status"]["deployment"]["status"]
        if (status == "COMPLETED" and
                app["deployment"]["internal"].get("deployedSHA") == sha):
            require(app["deployment"]["imageUrl"] != old_image, "image-unchanged")
            adapter.snapshot()
            return {"status": "deployed-awaiting-verification", "sha": sha}
        require(status not in ("FAILED", "ERROR", "CRASHED"), "deployment-failed")
        sleep(5)
    raise Halt("deployment-timeout")


if __name__ == "__main__":
    try:
        result = deploy(Adapter(os.environ["NORTHFLANK_DEPLOYMENT_TOKEN"]),
                        os.environ["RELEASE_SHA"], os.environ["CI_RUN_ID"],
                        os.environ["BUILD_ID"], os.environ["BACKUP_ATTESTATION"])
        print(json.dumps(result))
    except Exception as error:
        print(json.dumps({"status": "halted", "reason": str(error) if isinstance(error, Halt)
                          else "controller-error"}))
        sys.exit(1)
