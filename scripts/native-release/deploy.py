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


def matches(value, expected):
    return isinstance(value, dict) and all(value.get(key) == wanted for key, wanted in expected.items())


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        raise Halt("redirect-rejected")


def read_billing_usage(fetch_page):
    collected = []
    cursors = set()
    cursor = None
    pages = 0
    while pages < 100:
        page = fetch_page(cursor)
        pages += 1
        metadata = page.get("pagination", {})
        rows = page.get("data", {}).get("usage")
        valid = (isinstance(rows, list) and type(metadata.get("count")) is int
                 and type(metadata.get("hasNextPage")) is bool)
        require(valid and metadata["count"] == len(rows), "invalid-billing-page")
        collected += rows
        if metadata["hasNextPage"] is False:
            return {"usage": collected}
        cursor = metadata.get("cursor")
        require(bool(rows) and isinstance(cursor, str) and re.fullmatch(r"[0-9]{1,20}", cursor)
                and cursor not in cursors, "invalid-billing-cursor")
        cursors.add(cursor)
    raise Halt("billing-page-limit")


class Adapter:
    def __init__(self, token):
        self.opener, self.token = build_opener(ProxyHandler({}), NoRedirect()), token

    def request(self, path, body=None, github=False):
        if github:
            require(body is None and path.startswith("/repos/" + REPOSITORY + "/"), "unapproved-github-request")
            host, headers = "https://api.github.com", {"Accept": "application/vnd.github+json"}
        else:
            require(path.startswith(PROJECT + "/") or path == "/v1/billing/usage", "unapproved-cloud-request")
            if body is not None:
                valid = (path == APP + "/deployment" and set(body) == {"internal"}
                         and set(body["internal"]) == {"buildId"})
                build = body.get("internal", {}).get("buildId")
                require(valid and isinstance(build, str) and re.fullmatch(r"[a-z0-9-]{3,54}", build), "unapproved-cloud-write")
            host = "https://api.northflank.com"
            headers = {"Authorization": "Bearer " + self.token, "Content-Type": "application/json"}

        def page(cursor=None):
            endpoint = host + path
            if cursor is not None:
                endpoint += "?cursor=" + cursor
            request = Request(endpoint, headers=headers, method="POST" if body else "GET",
                              data=json.dumps(body).encode() if body else None)
            with self.opener.open(request, timeout=30) as response:
                payload = response.read(2 * 1024 * 1024 + 1)
            require(len(payload) <= 2 * 1024 * 1024, "response-too-large")
            return json.loads(payload)

        try:
            if path == "/v1/billing/usage" and not github:
                return read_billing_usage(page)
            response = page()
            pagination = response.get("pagination", {})
            incomplete = (pagination.get("hasNextPage") or pagination.get("next")
                          or pagination.get("totalPages", 1) > 1)
            require(not incomplete, "incomplete-response")
            return response if github else response.get("data", response)
        except Halt:
            raise
        except HTTPError as error:
            labels = ["github" if github else "northflank", "deployment" if body is not None else "read", "http", str(error.code)]
            raise Halt("-".join(labels)) from None
        except Exception:
            raise Halt("cloud-request-unavailable-or-uncertain") from None

    def snapshot(self):
        result = {}
        resources = [("app", "/services/telemetry-app"), ("redis", "/services/telemetry-redis"),
                     ("db", "/addons/telemetry-db"), ("services", "/services"),
                     ("addons", "/addons"), ("jobs", "/jobs"), ("volumes", "/volumes"),
                     ("buildArguments", "/services/telemetry-app/build-arguments")]
        for name, suffix in resources:
            result[name] = self.request(PROJECT + suffix)
        result["billing"] = self.request("/v1/billing/usage")
        job = "verified-release-controller"
        result["jobDetails"] = {job: self.request(PROJECT + "/jobs/" + job)}
        free_policy(result, allowed_jobs=(job,))
        return result

    def ci(self, sha, run_id):
        endpoint = "/repos/" + REPOSITORY
        branch = self.request(endpoint + "/branches/mainline", github=True)
        require(branch.get("protected") is True and branch["commit"]["sha"] == sha, "protected-branch-moved")
        run = self.request(endpoint + "/actions/runs/" + run_id, github=True)
        identity = {"head_sha": sha, "head_branch": BRANCH, "workflow_id": 0,
                    "path": ".github/workflows/fork-ci.yml", "status": "completed", "conclusion": "success"}
        require(matches(run, identity) and str(run["id"]) == run_id
                and run["event"] in ("push", "workflow_dispatch")
                and all(run[key]["id"] == 0 for key in ("repository", "head_repository")), "ci-not-successful")


def deploy(adapter, sha, run_id, build_id, attestation, *, now=time.time, sleep=time.sleep):
    for value, pattern, error in ((sha, r"[0-9a-f]{40}", "invalid-sha"),
                                  (run_id, r"[1-9][0-9]{0,19}", "invalid-run"),
                                  (build_id, r"[a-z0-9-]{3,54}", "invalid-build")):
        require(isinstance(value, str) and re.fullmatch(pattern, value), error)
    marker = r"^PANDA_NATIVE_backup_OK [0-9a-f]{40} [0-9]{10}$"
    evidence = [match.group(0).split(" ") for match in re.finditer(marker, attestation, re.MULTILINE)]
    require(len(evidence) == 1 and evidence[0][1] == sha, "backup-not-attested")
    captured = int(evidence[0][2])
    require(0 <= now() - captured < 120, "backup-not-fresh")
    app = adapter.snapshot()["app"]
    require(app.get("serviceType") == "combined" and matches(app.get("vcsData"),
            {"projectBranch": BRANCH, "projectUrl": "https://github.com/" + REPOSITORY}), "unexpected-build-source")
    require(app["status"]["deployment"]["status"] == "COMPLETED", "deployment-active")
    old_image = app["deployment"]["imageUrl"]
    build = adapter.request(APP + "/build/" + build_id)
    require(matches(build, {"id": build_id, "sha": sha, "branch": BRANCH, "status": "SUCCESS"})
            and build.get("concluded") is True and build.get("success") is True, "exact-build-not-ready")
    adapter.ci(sha, run_id)
    require(0 <= now() - captured < 120, "backup-expired")
    adapter.request(APP + "/deployment", {"internal": {"buildId": build_id}})
    end = now() + 600
    while now() < end:
        current = adapter.request(APP)
        status = current["status"]["deployment"]["status"]
        if status == "COMPLETED" and current["deployment"]["internal"].get("deployedSHA") == sha:
            require(current["deployment"]["imageUrl"] != old_image, "image-unchanged")
            adapter.snapshot()
            return {"status": "deployed-awaiting-verification", "sha": sha}
        require(status not in {"FAILED", "ERROR", "CRASHED"}, "deployment-failed")
        sleep(5)
    raise Halt("deployment-timeout")


def main():
    try:
        values = [os.environ[name] for name in ("RELEASE_SHA", "CI_RUN_ID", "BUILD_ID", "BACKUP_ATTESTATION")]
        result = deploy(Adapter(os.environ["NORTHFLANK_DEPLOYMENT_TOKEN"]), *values)
    except Exception as error:
        result = {"status": "halted", "reason": str(error) if isinstance(error, Halt) else "controller-error"}
        print(json.dumps(result))
        return 1
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
