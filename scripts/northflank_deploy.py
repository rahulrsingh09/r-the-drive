#!/usr/bin/env python3
"""One-shot, locally installed deployment controller; Python standard library only."""

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile
import time
from contextlib import contextmanager
from datetime import datetime, timezone
from urllib.error import HTTPError
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener


PROJECT = "/v1/projects/panda-private"
APP = PROJECT + "/services/telemetry-app"
BRANCH = "mainline"
REPOSITORY = "your-org/r-the-drive"
READINESS_HASH = "b9ad78dbf864f911166401bc1c84fc8995baf682dd30d6c81be3eabc9f3540a2"
SHA = re.compile(r"[0-9a-f]{40}")
CHECKSUM = re.compile(r"[0-9a-f]{96}")
HISTORY = ("telemetry", "trips", "charges", "statePeriods", "vehicles", "users", "credentials")
KEYS = ("JWT_SECRET", "JWT_PUBLIC_KEY", "AGE_ENCRYPTION_KEY", "DATABASE_URL",
        "REDIS_URL", "PANDA_GATEWAY_TOKEN")
REMOTE_PREFIX = """set -eu
umask 077
url=$(printf '%s' "$DATABASE_URL" | sed 's/options=-c+/options=-c%20/g')
"""
AUDIT_SCRIPT = REMOTE_PREFIX + """result=$(psql "$url" -X -A -t -v ON_ERROR_STOP=1 <<'SQL'
BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;
SELECT json_build_object(
  'ledger', (SELECT json_agg(row_to_json(m) ORDER BY version)
    FROM (SELECT version, encode(checksum, 'hex') AS checksum, success
          FROM public._sqlx_migrations ORDER BY version) m),
  'counts', json_build_object(
    'telemetry', (SELECT count(*) FROM timeseries.telemetry),
    'trips', (SELECT count(*) FROM panda.trips),
    'charges', (SELECT count(*) FROM panda.charge_sessions),
    'statePeriods', (SELECT count(*) FROM panda.vehicle_state_periods),
    'vehicles', (SELECT count(*) FROM panda.vehicles),
    'users', (SELECT count(*) FROM panda.users),
    'credentials', (SELECT count(*) FROM panda.vehicle_credentials)),
  'collector', json_build_object(
    'credentials', (SELECT count(*) FROM panda.vehicle_credentials),
    'healthy', (SELECT count(*) FROM panda.vehicle_credentials c
      JOIN panda.vehicle_runtime_state r USING (vehicle_id)
      WHERE r.auth_state = 'authorized' AND r.worker_health = 'connected'
        AND r.collector_heartbeat_at >= now() - interval '120 seconds'
        AND r.trip_persistence_error IS NULL)),
  'schemaReady', to_regclass('panda.active_trip_checkpoints') IS NOT NULL
    AND to_regclass('panda.pending_trip_completions') IS NOT NULL);
COMMIT;
SQL
)
printf '%s\\n' "$result" | sed -n 's/^{/PANDA_AUDIT={/p'
"""


class Halt(Exception):
    """Only static, non-sensitive reason codes may cross the output boundary."""


def require(condition, reason):
    if not condition:
        raise Halt(reason)


def is_sha(value):
    return isinstance(value, str) and SHA.fullmatch(value) is not None


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def outside_git(path):
    require(not any((p / ".git").exists() for p in (path, *path.parents)),
            "control-path-in-git")


def trusted_path(value, *, directory=False, private=False):
    path = Path(value)
    require(path.is_absolute() and not path.is_symlink(), "noncanonical-path")
    path = path.resolve()
    info = path.stat()
    require(info.st_uid == os.getuid(), "path-owner")
    require(stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode),
            "path-type")
    require(not info.st_mode & (0o077 if private else 0o022), "path-permissions")
    return path


def private_directory(path):
    path.mkdir(mode=0o700)
    return trusted_path(str(path), directory=True, private=True)


def read_private(path):
    return json.loads(trusted_path(str(path), private=True).read_text())


def atomic_json(path, value):
    descriptor, temporary = tempfile.mkstemp(prefix=".write-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "w") as output:
            json.dump(value, output, sort_keys=True)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


@contextmanager
def deployment_lock(path):
    descriptor = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    try:
        trusted_path(str(path), private=True)
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise Halt("deployment-locked") from None
        yield
    finally:
        os.close(descriptor)


class Config:
    def __init__(self, root):
        self.root = trusted_path(root, directory=True, private=True)
        outside_git(self.root)
        data = read_private(self.root / "config.json")
        for name in ("node", "nf_cli", "readiness", "nf_config_dir", "state_dir", "reviews_dir"):
            directory = name.endswith("_dir")
            path = trusted_path(data[name], directory=directory, private=directory)
            outside_git(path)
            setattr(self, name, path)
        require(self.root in self.readiness.parents, "readiness-outside-control-root")
        require(digest(self.readiness) == READINESS_HASH, "readiness-pin-mismatch")
        self.github_cli = (trusted_path(data["github_cli"]) if data.get("github_cli") else None)
        if self.github_cli:
            outside_git(self.github_cli)
        self.state = self.state_dir / "deployed.json"
        self.origin = self.https_origin(data["origin_url"])
        self.access = self.https_origin(data["access_url"])
        require(self.origin != self.access, "gate-urls-identical")

    @staticmethod
    def https_origin(value):
        parts = urlsplit(value)
        require(parts.scheme == "https" and parts.hostname and not parts.username
                and not parts.password and parts.path in ("", "/")
                and not parts.query and not parts.fragment and parts.port in (None, 443),
                "invalid-https-origin")
        return value.rstrip("/")


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def items(data, key):
    rows = data if isinstance(data, list) else data[key]
    require(isinstance(rows, list), "invalid-resource-list")
    if isinstance(data, dict):
        for field in ("total", "totalCount"):
            if field in data:
                require(data[field] == len(rows), "incomplete-resource-list")
    return rows


def validate_ledger(rows):
    require(isinstance(rows, list) and len(rows) > 0, "missing-migration-ledger")
    previous = 0
    for row in rows:
        require(isinstance(row, dict) and type(row.get("version")) is int
                and row["version"] > previous and row.get("success") is True
                and isinstance(row.get("checksum"), str)
                and CHECKSUM.fullmatch(row["checksum"]), "invalid-migration-ledger")
        previous = row["version"]
    return rows


def validate_audit(audit):
    validate_ledger(audit["ledger"])
    require(audit.get("schemaReady") is True, "schema-not-ready")
    counts = audit["counts"]
    require(set(counts) == set(HISTORY) and all(type(v) is int and v >= 0
                                             for v in counts.values()), "invalid-history-counts")
    collector = audit["collector"]
    require(type(collector.get("healthy")) is int
            and collector.get("credentials") == counts["credentials"]
            and collector["healthy"] == counts["credentials"], "collector-not-ready")


def compare_audits(before, after, expected):
    validate_audit(before)
    validate_audit(after)
    require(after["ledger"][:len(before["ledger"])] == before["ledger"],
            "migration-history-changed")
    require(after["ledger"] == expected, "unexpected-migrations")
    require(all(after["counts"][key] >= before["counts"][key] for key in HISTORY),
            "history-count-decreased")
    require(after["counts"]["credentials"] == before["counts"]["credentials"],
            "credential-count-changed")


def runtime_hashes(runtime):
    require(isinstance(runtime, dict) and all(isinstance(v, str) for v in runtime.values()),
            "invalid-runtime")
    require(all(runtime.get(key) for key in KEYS), "missing-runtime-key")
    require(runtime.get("PANDA_REQUIRE_GATEWAY") == "true"
            and runtime.get("PANDA_ENV") == "production", "runtime-gate-disabled")
    return {key: hashlib.sha256(value.encode()).hexdigest() for key, value in runtime.items()}


def free_policy(snapshot, allowed_jobs=()):
    app, redis, db = snapshot["app"], snapshot["redis"], snapshot["db"]
    require({s["id"] for s in items(snapshot["services"], "services")}
            == {"telemetry-app", "telemetry-redis"}
            and len(items(snapshot["services"], "services")) == 2, "service-budget")
    require([a["id"] for a in items(snapshot["addons"], "addons")] == ["telemetry-db"],
            "addon-budget")
    jobs = items(snapshot["jobs"], "jobs")
    require(sorted(job["id"] for job in jobs) == sorted(allowed_jobs), "job-budget")
    if allowed_jobs:
        details = snapshot["jobDetails"]
        require(set(details) == set(allowed_jobs), "incomplete-job-details")
        require(all(job["billing"]["deploymentPlan"] == "nf-compute-20"
                    and job["jobType"] == "manual"
                    and job["settings"]["backoffLimit"] == 0
                    and job["settings"]["activeDeadlineSeconds"] == 720
                    and job["settings"]["runOnSourceChange"] == "never"
                    for job in details.values()), "job-compute-budget")
    for service, identity in ((app, "telemetry-app"), (redis, "telemetry-redis")):
        require(service["id"] == identity
                and service["billing"]["deploymentPlan"] == "nf-compute-20"
                and type(service["deployment"]["instances"]) is int
                and service["deployment"]["instances"] == 1
                and not service["deployment"].get("autoscaling"), "compute-budget")
    require(app["billing"]["buildPlan"] == "nf-compute-400-16"
            and app["disabledCI"] is True, "build-budget-or-ci-enabled")
    require(app["vcsData"]["projectUrl"] == "https://github.com/" + REPOSITORY
            and app["vcsData"]["projectBranch"] == BRANCH, "build-source-drift")
    for key in ("buildArguments", "buildFiles", "dockerSecretMounts"):
        require(snapshot["buildArguments"][key] in ({}, []), "build-secrets-present")
    spec = db["spec"]
    deployment = spec["config"]["deployment"]
    require(db["id"] == "telemetry-db" and spec["type"] == "postgresql"
            and deployment["planId"] == "nf-compute-20"
            and deployment["replicas"] == 1 and deployment["storageSize"] == 6144
            and spec["config"]["networking"]["externalAccessEnabled"] is False
            and not spec.get("pendingActions"), "database-budget-or-exposure")
    volumes = items(snapshot["volumes"], "volumes")
    require(len(volumes) == 1 and volumes[0]["id"] == "app-data"
            and volumes[0]["spec"]["storageSize"] == 6144
            and volumes[0]["owningObject"] == {"id": "telemetry-app", "type": "service"}
            and volumes[0]["attachedObjects"] == [{"id": "telemetry-app", "type": "service"}],
            "volume-budget")
    require(redis["ports"] and all(p["public"] is False and not p.get("vpcAccessible")
                                 for p in redis["ports"]), "redis-exposed")
    require(len(app["ports"]) == 1 and app["ports"][0]["internalPort"] == 8080
            and app["ports"][0]["protocol"] == "HTTP"
            and app["ports"][0]["public"] is True and not app["ports"][0].get("vpcAccessible")
            and not app["ports"][0].get("domains"), "app-port-drift")
    usage = snapshot["billing"]["usage"]
    require(isinstance(usage, list) and usage, "billing-unavailable")
    require(all(row.get("currency") == "usd" and type(row.get("total")) in (int, float)
                and row["total"] == 0 for row in usage), "paid-usage")


def identity(app):
    deployment = app["deployment"]
    internal = deployment["internal"]
    sha = internal["deployedSHA"]
    image = deployment["imageUrl"]
    require(is_sha(sha) and internal["branch"] == BRANCH
            and internal["nfObjectId"] == "telemetry-app", "invalid-deployed-identity")
    require(isinstance(image, str)
            and re.fullmatch(r"registry\.northflank\.com/northflank/service/[0-9a-f]+:[0-9a-f]+",
                             image), "mutable-or-unknown-image")
    return {"sha": sha, "image": image}


class Northflank:
    def __init__(self, config):
        self.config = config
        self.opener = build_opener(ProxyHandler({}), NoRedirect())
        self.env = {"PATH": str(config.node.parent) + ":/usr/bin:/bin",
                    "HOME": str(Path.home()), "LANG": "C.UTF-8"}
        require(self.command([str(config.node), "--version"]).strip() == "v24.18.0",
                "node-version")
        nf = read_private(config.nf_config_dir / "config.json")
        contexts = [c for c in nf["contexts"] if c["name"] == nf["current"]]
        require(len(contexts) == 1 and contexts[0]["host"].rstrip("/")
                == "https://api.northflank.com" and contexts[0]["token"], "nf-context")
        self.token = contexts[0]["token"]

    def command(self, argv, *, cli=False, timeout=120):
        env = dict(self.env)
        if cli:
            env["NF_CONFIG_DIR"] = str(self.config.nf_config_dir)
        try:
            result = subprocess.run(argv, cwd=self.config.root, env=env, input="",
                                    capture_output=True, text=True, timeout=timeout)
            require(result.returncode == 0, "local-command-failed")
            return result.stdout
        except (OSError, subprocess.TimeoutExpired):
            raise Halt("local-command-unavailable-or-timeout") from None

    def cli(self, *args, timeout=120):
        return self.command([str(self.config.node), str(self.config.nf_cli), *args,
                             "--quiet"], cli=True, timeout=timeout)

    def request(self, method, path, body=None):
        require(method == "GET" or (method == "POST" and path == APP + "/build"
                                   and set(body) == {"sha"} and is_sha(body["sha"])),
                "unapproved-api-write")
        request = Request("https://api.northflank.com" + path, method=method,
                          data=json.dumps(body).encode() if body is not None else None,
                          headers={"Authorization": "Bearer " + self.token,
                                   "Content-Type": "application/json"})
        try:
            with self.opener.open(request, timeout=60) as response:
                result = json.load(response)
            pagination = result.get("pagination", {})
            require(not pagination.get("hasNextPage") and not pagination.get("next")
                    and pagination.get("totalPages", 1) <= 1, "incomplete-resource-list")
            return result.get("data", result)
        except Halt:
            raise
        except Exception:
            raise Halt("northflank-request-failed") from None

    def readiness(self, sha):
        require(digest(self.config.readiness) == READINESS_HASH, "readiness-pin-mismatch")
        argv = [str(self.config.node), str(self.config.readiness),
                "--state", str(self.config.state)]
        if self.config.github_cli:
            argv.extend(["--github-cli", str(self.config.github_cli)])
        result = json.loads(self.command(argv))
        entries = result.get("items")
        require(result.get("schemaVersion") == 1 and result.get("ready") is True
                and isinstance(entries, list) and len(entries) == 1, "ci-not-ready")
        item = entries[0]
        require(item["sha"] == sha and item["repository"] == REPOSITORY
                and item["branch"] == BRANCH
                and item["id"] == f"{REPOSITORY}:{BRANCH}:{sha}", "stale-or-foreign-sha")
        return item

    def snapshot(self):
        paths = {"services": PROJECT + "/services", "addons": PROJECT + "/addons",
                 "jobs": PROJECT + "/jobs", "volumes": PROJECT + "/volumes",
                 "app": APP, "redis": PROJECT + "/services/telemetry-redis",
                 "db": PROJECT + "/addons/telemetry-db", "billing": "/v1/billing/usage",
                 "buildArguments": APP + "/build-arguments"}
        snapshot = {key: self.request("GET", path) for key, path in paths.items()}
        free_policy(snapshot)
        return snapshot

    def runtime(self):
        return self.request("GET", APP + "/runtime-environment")["runtimeEnvironment"]

    def pin(self, sha):
        require(is_sha(sha), "invalid-pin")
        self.cli("update", "service", "deployment", "--projectId", "panda-private",
                 "--serviceId", "telemetry-app", "--input",
                 json.dumps({"internal": {"id": "telemetry-app", "branch": BRANCH,
                                          "buildSHA": sha}}))

    def start_build(self, sha):
        result = self.request("POST", APP + "/build", {"sha": sha})
        build_id = result["id"]
        require(isinstance(build_id, str) and re.fullmatch(r"[a-z0-9-]+", build_id),
                "invalid-build-id")
        return build_id

    def wait_build(self, build_id, sha):
        deadline = time.monotonic() + 1800
        while time.monotonic() < deadline:
            build = self.request("GET", APP + "/build/" + build_id)
            require(build["id"] == build_id and build["sha"] == sha, "wrong-build")
            if build.get("concluded") is True:
                require(build.get("success") is True and build["status"] == "SUCCESS",
                        "build-failed")
                return
            require(build["status"] in ("PENDING", "QUEUED", "STARTING", "CLONING",
                                        "BUILDING", "UPLOADING", "IN_PROGRESS"),
                    "build-failed-or-unknown")
            time.sleep(10)
        raise Halt("build-timeout")

    def wait_deployment(self, sha):
        deadline = time.monotonic() + 600
        while time.monotonic() < deadline:
            app = self.request("GET", APP)
            status = app["status"]["deployment"]["status"]
            require(status in ("COMPLETED", "PENDING", "RUNNING", "IN_PROGRESS"), "deployment-failed")
            if status == "COMPLETED" and identity(app)["sha"] == sha:
                require(app["deployment"]["internal"]["buildSHA"] == sha, "pin-changed")
                return app
            time.sleep(10)
        raise Halt("deployment-timeout")

    def remote(self, script):
        return self.cli("exec", "service", "--projectId", "panda-private",
                        "--serviceId", "telemetry-app", "--shell-cmd", "sh -c",
                        "--cmd", script, timeout=600)

    @staticmethod
    def marker(output, name):
        matches = [line[len(name) + 1:] for line in output.splitlines()
                   if line.startswith(name + "=")]
        require(len(matches) == 1, "missing-remote-evidence")
        return matches[0]

    def audit(self):
        return json.loads(self.marker(self.remote(AUDIT_SCRIPT), "PANDA_AUDIT"))

    def backup(self, directory):
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
        remote = "/backups/pre-deploy-" + stamp + ".dump"
        script = REMOTE_PREFIX + f"""backup='{remote}'
test ! -e "$backup"
pg_dump --format=custom --no-owner --no-privileges --file="$backup" --dbname="$url"
pg_restore --list "$backup" >/dev/null
hash=$(sha256sum "$backup"); hash=${{hash%% *}}
bytes=$(wc -c < "$backup")
printf 'PANDA_BACKUP=%s %s\\n' "$hash" "$bytes"
"""
        evidence = self.marker(self.remote(script), "PANDA_BACKUP").split()
        require(len(evidence) == 2 and re.fullmatch(r"[0-9a-f]{64}", evidence[0])
                and evidence[1].isdigit() and int(evidence[1]) > 0, "invalid-backup-evidence")
        local = directory / "database.dump"
        self.cli("download", "service", "file", "--projectId", "panda-private",
                 "--serviceId", "telemetry-app", "--remotePath", remote,
                 "--localPath", str(local), timeout=600)
        require(not local.is_symlink() and local.is_file(), "backup-download-missing")
        local.chmod(0o600)
        require(local.stat().st_size == int(evidence[1]) and digest(local) == evidence[0],
                "backup-checksum-mismatch")
        return {"remote": remote, "local": str(local), "sha256": evidence[0],
                "bytes": int(evidence[1]), "capturedAt": stamp}

    def probe(self, url, headers=None):
        try:
            with self.opener.open(Request(url, headers=headers or {}, method="GET"),
                                  timeout=20) as response:
                return response.status, response.headers.get("Location", "")
        except HTTPError as error:
            return error.code, error.headers.get("Location", "")
        except Exception:
            raise Halt("gate-probe-unavailable") from None

    def gates(self, runtime):
        headers = {"X-Panda-Edge": runtime["PANDA_GATEWAY_TOKEN"]}
        require(self.probe(self.config.origin + "/health", headers)[0] == 200, "health-failed")
        require(self.probe(self.config.origin + "/v1/vehicles", headers)[0] == 401,
                "app-auth-exposed")
        require(self.probe(self.config.origin + "/")[0] == 403, "origin-exposed")
        code, location = self.probe(self.config.access + "/")
        target = urlsplit(location)
        access_redirect = (code in (302, 303, 307, 308) and target.scheme == "https"
                           and (target.hostname or "").endswith(".cloudflareaccess.com")
                           and target.path.startswith("/cdn-cgi/access/login"))
        require(code in (401, 403) or access_redirect, "cloudflare-access-exposed")


def run(config, adapter, sha):
    require(is_sha(sha), "invalid-sha")
    with deployment_lock(config.state_dir / "deployment.lock"):
        state = read_private(config.state)
        require(state.get("schemaVersion") == 1
                and isinstance(state.get("deployedShas"), list)
                and all(is_sha(s) for s in state["deployedShas"])
                and "attempt" in state, "invalid-state")
        require(state["attempt"] is None, "attempt-needs-reconciliation")
        if sha in state["deployedShas"]:
            return {"status": "already-deployed", "sha": sha}
        review = read_private(config.reviews_dir / (sha + ".json"))
        require(review.get("schemaVersion") == 1 and review.get("sha") == sha
                and review.get("approved") is True and is_sha(review.get("baseSha")),
                "missing-reviewed-promotion")
        expected = validate_ledger(review["migrationLedger"])
        adapter.readiness(sha)
        snapshot = adapter.snapshot()
        old = identity(snapshot["app"])
        require(old["sha"] == review["baseSha"] and old["sha"] != sha, "review-base-moved")
        require(snapshot["app"]["status"]["deployment"]["status"] == "COMPLETED",
                "deployment-already-active")
        builds = items(adapter.request("GET", APP + "/build"), "builds")
        require(all(build.get("concluded") is True for build in builds), "build-already-active")
        runtime = adapter.runtime()
        hashes = runtime_hashes(runtime)
        adapter.gates(runtime)
        before = adapter.audit()
        validate_audit(before)
        require(expected[:len(before["ledger"])] == before["ledger"], "review-ledger-mismatch")
        directory = private_directory(config.state_dir / sha)
        atomic_json(directory / "runtime-before.json", runtime)
        atomic_json(directory / "recovery.json", {"old": old, "runtimeHashes": hashes,
                                                "review": review})
        atomic_json(directory / "before.json", before)
        state["attempt"] = {"sha": sha, "phase": "pin-started", "directory": str(directory)}

        def phase(name, **evidence):
            state["attempt"].update(phase=name, **evidence)
            atomic_json(config.state, state)

        phase("pin-started")
        adapter.pin(old["sha"])
        pinned = adapter.wait_deployment(old["sha"])
        require(identity(pinned) == old, "old-image-changed")
        adapter.snapshot()
        ready = adapter.readiness(sha)
        phase("build-started", readiness=ready)
        build_id = adapter.start_build(sha)
        phase("build-wait", buildId=build_id)
        adapter.wait_build(build_id, sha)
        phase("backup-started")
        require(runtime_hashes(adapter.runtime()) == hashes, "runtime-changed")
        backup = adapter.backup(directory)
        atomic_json(directory / "backup.json", backup)
        fresh = adapter.audit()
        compare_audits(before, fresh, before["ledger"])
        atomic_json(directory / "before-deploy.json", fresh)
        current = adapter.snapshot()["app"]
        require(identity(current) == old
                and current["deployment"]["internal"]["buildSHA"] == old["sha"],
                "deployment-changed-during-build")
        require(runtime_hashes(adapter.runtime()) == hashes, "runtime-changed")
        adapter.gates(runtime)
        ready = adapter.readiness(sha)
        phase("deploy-started", readiness=ready, backup=backup)
        adapter.pin(sha)
        deployed = adapter.wait_deployment(sha)
        phase("verifying")
        require(identity(deployed)["image"] != old["image"], "deployed-image-unchanged")
        require(runtime_hashes(adapter.runtime()) == hashes, "runtime-changed")
        adapter.snapshot()
        adapter.gates(runtime)
        deadline = time.monotonic() + 120
        while True:
            after = adapter.audit()
            atomic_json(directory / "after.json", after)
            try:
                compare_audits(fresh, after, expected)
                break
            except Halt as error:
                if str(error) != "collector-not-ready" or time.monotonic() >= deadline:
                    raise
                time.sleep(10)
        final = adapter.snapshot()["app"]
        require(identity(final) == identity(deployed)
                and final["deployment"]["internal"]["buildSHA"] == sha
                and final["status"]["deployment"]["status"] == "COMPLETED",
                "deployment-changed-during-verification")
        require(runtime_hashes(adapter.runtime()) == hashes, "runtime-changed")
        atomic_json(directory / "success.json", {"sha": sha, "old": old,
                    "deployed": identity(deployed), "buildId": build_id, "backup": backup,
                    "readiness": ready, "runtimeHashes": hashes})
        state["deployedShas"].append(sha)
        state["attempt"] = None
        atomic_json(config.state, state)
        return {"status": "deployed", "sha": sha}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--control-root", required=True)
    parser.add_argument("--sha", required=True)
    args = parser.parse_args()
    os.umask(0o077)
    try:
        require(is_sha(args.sha), "invalid-sha")
        config = Config(args.control_root)
        require(config.root in Path(__file__).resolve().parents, "controller-not-installed")
        result = run(config, Northflank(config), args.sha)
        print(json.dumps(result))
        return 0
    except Exception as error:
        reason = str(error) if isinstance(error, Halt) else "controller-error"
        print(json.dumps({"status": "halted", "reason": reason}))
        return 1


if __name__ == "__main__":
    sys.exit(main())
