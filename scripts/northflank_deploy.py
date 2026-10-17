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
READINESS_HASH = "a50ef273e600fad30d1295d5efbfecf4439f1b8aca6a413d8a7ec828bb7c6ee1"
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
WITH history(name, amount) AS (
 SELECT 'telemetry', count(*) FROM timeseries.telemetry UNION ALL
 SELECT 'trips', count(*) FROM panda.trips UNION ALL
 SELECT 'charges', count(*) FROM panda.charge_sessions UNION ALL
 SELECT 'statePeriods', count(*) FROM panda.vehicle_state_periods UNION ALL
 SELECT 'vehicles', count(*) FROM panda.vehicles UNION ALL
 SELECT 'users', count(*) FROM panda.users UNION ALL
 SELECT 'credentials', count(*) FROM panda.vehicle_credentials
), collectors AS (
 SELECT count(*) AS healthy FROM panda.vehicle_credentials c JOIN panda.vehicle_runtime_state r USING(vehicle_id)
 WHERE r.auth_state='authorized' AND r.worker_health='connected'
 AND r.collector_heartbeat_at >= now() - interval '120 seconds' AND r.trip_persistence_error IS NULL
), ledger AS (
 SELECT json_agg(json_build_object('version', version, 'checksum', encode(checksum,'hex'), 'success', success)
                 ORDER BY version) AS entries FROM public._sqlx_migrations
)
SELECT json_build_object('ledger', (SELECT entries FROM ledger),
 'counts', (SELECT json_object_agg(name,amount) FROM history),
 'collector', json_build_object('credentials', (SELECT amount FROM history WHERE name='credentials'),
                                'healthy', (SELECT healthy FROM collectors)),
 'schemaReady', to_regclass('panda.active_trip_checkpoints') IS NOT NULL
                AND to_regclass('panda.pending_trip_completions') IS NOT NULL);
COMMIT;
SQL
)
printf '%s\\n' "$result" | sed -n 's/^{/PANDA_AUDIT={/p'
"""


class Halt(Exception):
    """A public failure carries only a static reason code."""


def require(condition, reason):
    if condition:
        return
    raise Halt(reason)


def is_sha(value):
    return isinstance(value, str) and bool(SHA.fullmatch(value))


def matches(value, expected):
    return isinstance(value, dict) and all(value.get(key) == item for key, item in expected.items())


def digest(path):
    checksum = hashlib.sha256()
    with open(path, "rb") as handle:
        while True:
            data = handle.read(1024 * 1024)
            if not data:
                return checksum.hexdigest()
            checksum.update(data)


def outside_git(path):
    for candidate in [path] + list(path.parents):
        require(not (candidate / ".git").exists(), "control-path-in-git")


def trusted_path(value, *, directory=False, private=False):
    original = Path(value)
    require(original.is_absolute() and not original.is_symlink(), "noncanonical-path")
    resolved = original.resolve()
    info = resolved.stat()
    constraints = [(info.st_uid == os.getuid(), "path-owner"),
                   ((stat.S_ISDIR if directory else stat.S_ISREG)(info.st_mode), "path-type"),
                   (info.st_mode & (0o077 if private else 0o022) == 0, "path-permissions")]
    for valid, reason in constraints:
        require(valid, reason)
    return resolved


def private_directory(path):
    path.mkdir(mode=0o700)
    checked = trusted_path(str(path), directory=True, private=True)
    return checked


def read_private(path):
    file = trusted_path(str(path), private=True)
    with file.open() as source:
        return json.load(source)


def atomic_json(path, value):
    name = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", prefix=".write-", dir=path.parent, delete=False) as output:
            name = output.name
            output.write(json.dumps(value, sort_keys=True) + "\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(name, path)
        fd = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        with _descriptor(fd):
            os.fsync(fd)
    finally:
        if name is not None and os.path.exists(name):
            os.unlink(name)


@contextmanager
def _descriptor(fd):
    try:
        yield fd
    finally:
        os.close(fd)


@contextmanager
def deployment_lock(path):
    flags = os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW
    with _descriptor(os.open(path, flags, 0o600)) as descriptor:
        trusted_path(str(path), private=True)
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise Halt("deployment-locked") from None
        yield


class Config:
    def __init__(self, root):
        self.root = trusted_path(root, directory=True, private=True)
        outside_git(self.root)
        settings = read_private(self.root / "config.json")
        paths = {"node": False, "nf_cli": False, "readiness": False,
                 "nf_config_dir": True, "state_dir": True, "reviews_dir": True}
        for key, directory in paths.items():
            checked = trusted_path(settings[key], directory=directory, private=directory)
            outside_git(checked)
            setattr(self, key, checked)
        require(self.root in self.readiness.parents, "readiness-outside-control-root")
        require(digest(self.readiness) == READINESS_HASH, "readiness-pin-mismatch")
        self.github_cli = None
        if settings.get("github_cli"):
            self.github_cli = trusted_path(settings["github_cli"])
            outside_git(self.github_cli)
        self.state = self.state_dir / "deployed.json"
        self.origin, self.access = [self.https_origin(settings[key]) for key in ("origin_url", "access_url")]
        require(self.origin != self.access, "gate-urls-identical")

    @staticmethod
    def https_origin(value):
        location = urlsplit(value)
        invalid = (location.scheme != "https" or not location.hostname or location.username
                   or location.password or location.path not in ("", "/") or location.query
                   or location.fragment or location.port not in (None, 443))
        require(not invalid, "invalid-https-origin")
        return value.rstrip("/")


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def items(data, key):
    if isinstance(data, list):
        return data
    result = data[key]
    require(isinstance(result, list), "invalid-resource-list")
    if isinstance(data, dict):
        counts = [data[field] for field in ("total", "totalCount") if field in data]
        require(all(count == len(result) for count in counts), "incomplete-resource-list")
    return result


def validate_ledger(rows):
    require(isinstance(rows, list) and bool(rows), "missing-migration-ledger")
    versions = [0]
    for row in rows:
        require(isinstance(row, dict), "invalid-migration-ledger")
        version, checksum = row.get("version"), row.get("checksum")
        require(type(version) is int and version > versions[-1] and row.get("success") is True
                and isinstance(checksum, str) and CHECKSUM.fullmatch(checksum), "invalid-migration-ledger")
        versions.append(version)
    return rows


def validate_audit(audit):
    validate_ledger(audit["ledger"])
    require(audit.get("schemaReady") is True, "schema-not-ready")
    counts = audit["counts"]
    require(set(counts) == set(HISTORY), "invalid-history-counts")
    require(all(type(value) is int and value >= 0 for value in counts.values()), "invalid-history-counts")
    collector = audit["collector"]
    count = counts["credentials"]
    require(type(collector.get("healthy")) is int and matches(collector, {"credentials": count, "healthy": count}), "collector-not-ready")


def compare_audits(before, after, expected):
    for audit in (before, after):
        validate_audit(audit)
    checks = [(after["ledger"][:len(before["ledger"])] == before["ledger"], "migration-history-changed"),
              (after["ledger"] == expected, "unexpected-migrations"),
              (all(after["counts"][key] - before["counts"][key] >= 0 for key in HISTORY), "history-count-decreased"),
              (after["counts"]["credentials"] == before["counts"]["credentials"], "credential-count-changed")]
    for valid, message in checks:
        require(valid, message)


def runtime_hashes(runtime):
    require(isinstance(runtime, dict) and all(isinstance(item, str) for item in runtime.values()), "invalid-runtime")
    require(all(runtime.get(key) for key in KEYS), "missing-runtime-key")
    require(matches(runtime, {"PANDA_REQUIRE_GATEWAY": "true", "PANDA_ENV": "production"}), "runtime-gate-disabled")
    hashes = {}
    for key in runtime:
        hashes[key] = hashlib.sha256(runtime[key].encode()).hexdigest()
    return hashes


def free_policy(snapshot, allowed_jobs=()):
    app, redis, db = (snapshot[key] for key in ("app", "redis", "db"))
    services = items(snapshot["services"], "services")
    require(len(services) == 2 and {entry["id"] for entry in services} == {"telemetry-app", "telemetry-redis"}, "service-budget")
    require([entry["id"] for entry in items(snapshot["addons"], "addons")] == ["telemetry-db"], "addon-budget")
    require(sorted(entry["id"] for entry in items(snapshot["jobs"], "jobs")) == sorted(allowed_jobs), "job-budget")
    if allowed_jobs:
        details = snapshot["jobDetails"]
        require(set(details) == set(allowed_jobs), "incomplete-job-details")
        for job in details.values():
            require(job["billing"]["deploymentPlan"] == "nf-compute-20" and job["jobType"] == "manual"
                    and matches(job["settings"], {"backoffLimit": 0, "activeDeadlineSeconds": 720,
                            "runOnSourceChange": "never"}), "job-compute-budget")
    for service, name in ((app, "telemetry-app"), (redis, "telemetry-redis")):
        deployment = service["deployment"]
        require(service["id"] == name and service["billing"]["deploymentPlan"] == "nf-compute-20"
                and type(deployment["instances"]) is int and deployment["instances"] == 1
                and not deployment.get("autoscaling"), "compute-budget")
    require(app["billing"]["buildPlan"] == "nf-compute-400-16" and app["disabledCI"] is True, "build-budget-or-ci-enabled")
    require(matches(app["vcsData"], {"projectUrl": "https://github.com/" + REPOSITORY,
                                   "projectBranch": BRANCH}), "build-source-drift")
    require(all(snapshot["buildArguments"][key] in ({}, [])
                for key in ("buildArguments", "buildFiles", "dockerSecretMounts")), "build-secrets-present")
    spec = db["spec"]
    require(db["id"] == "telemetry-db" and spec["type"] == "postgresql"
            and matches(spec["config"]["deployment"], {"planId": "nf-compute-20", "replicas": 1, "storageSize": 6144})
            and spec["config"]["networking"]["externalAccessEnabled"] is False
            and not spec.get("pendingActions"), "database-budget-or-exposure")
    volumes = items(snapshot["volumes"], "volumes")
    owner = {"id": "telemetry-app", "type": "service"}
    require(len(volumes) == 1 and matches(volumes[0], {"id": "app-data", "owningObject": owner, "attachedObjects": [owner]})
            and volumes[0]["spec"]["storageSize"] == 6144, "volume-budget")
    require(bool(redis["ports"]) and all(port["public"] is False and not port.get("vpcAccessible")
                                       for port in redis["ports"]), "redis-exposed")
    ports = app["ports"]
    require(len(ports) == 1 and matches(ports[0], {"internalPort": 8080, "protocol": "HTTP"})
            and ports[0]["public"] is True and not ports[0].get("vpcAccessible")
            and not ports[0].get("domains"), "app-port-drift")
    usage = snapshot["billing"]["usage"]
    require(isinstance(usage, list) and bool(usage), "billing-unavailable")
    for row in usage:
        require(matches(row, {"currency": "usd", "total": 0}) and type(row.get("total")) in (int, float), "paid-usage")


def identity(app):
    deployment = app["deployment"]
    source = deployment["internal"]
    require(is_sha(source["deployedSHA"]) and matches(source, {"branch": BRANCH, "nfObjectId": "telemetry-app"}), "invalid-deployed-identity")
    image = deployment["imageUrl"]
    require(isinstance(image, str) and re.fullmatch(r"registry\.northflank\.com/northflank/service/[0-9a-f]+:[0-9a-f]+", image), "mutable-or-unknown-image")
    return dict(sha=source["deployedSHA"], image=image)


class Northflank:
    def __init__(self, config):
        self.config = config
        self.env = dict(PATH=str(config.node.parent) + ":/usr/bin:/bin", HOME=str(Path.home()), LANG="C.UTF-8")
        self.opener = build_opener(ProxyHandler({}), NoRedirect())
        require(self.command([str(config.node), "--version"]).strip() == "v24.18.0", "node-version")
        data = read_private(config.nf_config_dir / "config.json")
        selected = [row for row in data["contexts"] if row["name"] == data["current"]]
        require(len(selected) == 1, "nf-context")
        context = selected[0]
        require(context["host"].rstrip("/") == "https://api.northflank.com" and context["token"], "nf-context")
        self.token = context["token"]

    def command(self, argv, *, cli=False, timeout=120):
        environment = self.env.copy()
        if cli:
            environment.update(NF_CONFIG_DIR=str(self.config.nf_config_dir))
        options = dict(cwd=self.config.root, env=environment, input="", capture_output=True,
                       text=True, timeout=timeout)
        try:
            completed = subprocess.run(argv, **options)
        except (OSError, subprocess.TimeoutExpired):
            raise Halt("local-command-unavailable-or-timeout") from None
        require(completed.returncode == 0, "local-command-failed")
        return completed.stdout

    def cli(self, *args, timeout=120):
        arguments = [str(self.config.node), str(self.config.nf_cli)]
        arguments.extend(args)
        arguments.append("--quiet")
        return self.command(arguments, cli=True, timeout=timeout)

    def request(self, method, path, body=None):
        if method != "GET":
            require(method == "POST" and path == APP + "/build" and set(body) == {"sha"}
                    and is_sha(body["sha"]), "unapproved-api-write")
        headers = {"Authorization": "Bearer " + self.token, "Content-Type": "application/json"}
        payload = None if body is None else json.dumps(body).encode()
        request = Request("https://api.northflank.com" + path, data=payload, headers=headers, method=method)
        try:
            with self.opener.open(request, timeout=60) as response:
                value = json.load(response)
            paging = value.get("pagination", {})
            require(not any((paging.get("hasNextPage"), paging.get("next"), paging.get("totalPages", 1) > 1)), "incomplete-resource-list")
            return value["data"] if "data" in value else value
        except Halt:
            raise
        except Exception:
            raise Halt("northflank-request-failed") from None

    def readiness(self, sha):
        require(digest(self.config.readiness) == READINESS_HASH, "readiness-pin-mismatch")
        args = [str(self.config.node), str(self.config.readiness), "--state", str(self.config.state)]
        if self.config.github_cli:
            args += ["--github-cli", str(self.config.github_cli)]
        result = json.loads(self.command(args))
        rows = result.get("items")
        require(result.get("schemaVersion") == 1 and result.get("ready") is True
                and isinstance(rows, list) and len(rows) == 1, "ci-not-ready")
        expected = dict(sha=sha, repository=REPOSITORY, branch=BRANCH, id=f"{REPOSITORY}:{BRANCH}:{sha}")
        require(matches(rows[0], expected), "stale-or-foreign-sha")
        return rows[0]

    def snapshot(self):
        endpoints = [("services", PROJECT + "/services"), ("addons", PROJECT + "/addons"),
                     ("jobs", PROJECT + "/jobs"), ("volumes", PROJECT + "/volumes"),
                     ("app", APP), ("redis", PROJECT + "/services/telemetry-redis"),
                     ("db", PROJECT + "/addons/telemetry-db"), ("billing", "/v1/billing/usage"),
                     ("buildArguments", APP + "/build-arguments")]
        result = {}
        for name, endpoint in endpoints:
            result[name] = self.request("GET", endpoint)
        free_policy(result)
        return result

    def runtime(self):
        response = self.request("GET", APP + "/runtime-environment")
        return response["runtimeEnvironment"]

    def pin(self, sha):
        require(is_sha(sha), "invalid-pin")
        source = dict(id="telemetry-app", branch=BRANCH, buildSHA=sha)
        arguments = ["update", "service", "deployment", "--projectId", "panda-private",
                     "--serviceId", "telemetry-app", "--input", json.dumps({"internal": source})]
        self.cli(*arguments)

    def start_build(self, sha):
        response = self.request("POST", APP + "/build", dict(sha=sha))
        identifier = response["id"]
        require(isinstance(identifier, str) and re.fullmatch(r"[a-z0-9-]+", identifier), "invalid-build-id")
        return identifier

    def wait_build(self, build_id, sha):
        pending = {"PENDING", "QUEUED", "STARTING", "CLONING", "BUILDING", "UPLOADING", "IN_PROGRESS"}
        end = time.monotonic() + 1800
        while time.monotonic() < end:
            result = self.request("GET", APP + "/build/" + build_id)
            require(matches(result, {"id": build_id, "sha": sha}), "wrong-build")
            if result.get("concluded") is not True:
                require(result["status"] in pending, "build-failed-or-unknown")
                time.sleep(10)
                continue
            require(result.get("success") is True and result["status"] == "SUCCESS", "build-failed")
            return
        raise Halt("build-timeout")

    def wait_deployment(self, sha):
        end = time.monotonic() + 600
        while time.monotonic() < end:
            app = self.request("GET", APP)
            status = app["status"]["deployment"]["status"]
            require(status in {"COMPLETED", "PENDING", "RUNNING", "IN_PROGRESS"}, "deployment-failed")
            if status == "COMPLETED":
                current = identity(app)
                if current["sha"] == sha:
                    require(app["deployment"]["internal"]["buildSHA"] == sha, "pin-changed")
                    return app
            time.sleep(10)
        raise Halt("deployment-timeout")

    def remote(self, script):
        options = [("--projectId", "panda-private"), ("--serviceId", "telemetry-app"),
                   ("--shell-cmd", "sh -c"), ("--cmd", script)]
        args = ["exec", "service"]
        for pair in options:
            args.extend(pair)
        return self.cli(*args, timeout=600)

    @staticmethod
    def marker(output, name):
        evidence = []
        prefix = name + "="
        for line in output.splitlines():
            if line.startswith(prefix):
                evidence.append(line[len(prefix):])
        require(len(evidence) == 1, "missing-remote-evidence")
        return evidence.pop()

    def audit(self):
        document = self.marker(self.remote(AUDIT_SCRIPT), "PANDA_AUDIT")
        return json.loads(document)

    def backup(self, directory):
        stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
        remote = "/backups/pre-deploy-" + stamp + ".dump"
        script = REMOTE_PREFIX + f"""backup='{remote}'
test ! -e "$backup"
pg_dump --format=custom --no-owner --no-privileges --file="$backup" --dbname="$url"
pg_restore --list "$backup" >/dev/null
test -s "$backup"
metadata=$(stat -c '%s' "$backup")
checksum=$(sha256sum "$backup")
case "$metadata" in ''|*[!0-9]*|0) exit 1 ;; esac
printf 'PANDA_BACKUP=%s %s\\n' "${{checksum%% *}}" "$metadata"
"""
        evidence = self.marker(self.remote(script), "PANDA_BACKUP").split()
        require(len(evidence) == 2 and re.fullmatch(r"[0-9a-f]{64}", evidence[0])
                and evidence[1].isdigit() and int(evidence[1]) > 0, "invalid-backup-evidence")
        local = directory / "database.dump"
        args = ["download", "service", "file", "--projectId", "panda-private", "--serviceId", "telemetry-app",
                "--remotePath", remote, "--localPath", str(local)]
        self.cli(*args, timeout=600)
        require(local.is_file() and not local.is_symlink(), "backup-download-missing")
        local.chmod(0o600)
        expected_bytes = int(evidence[1])
        require(local.stat().st_size == expected_bytes and digest(local) == evidence[0], "backup-checksum-mismatch")
        return dict(remote=remote, local=str(local), sha256=evidence[0], bytes=expected_bytes, capturedAt=stamp)

    def probe(self, url, headers=None):
        request = Request(url, method="GET", headers={} if headers is None else headers)
        try:
            with self.opener.open(request, timeout=20) as response:
                status, location = response.status, response.headers.get("Location", "")
        except HTTPError as response:
            status, location = response.code, response.headers.get("Location", "")
        except Exception:
            raise Halt("gate-probe-unavailable") from None
        return status, location

    def gates(self, runtime):
        headers = {"X-Panda-Edge": runtime["PANDA_GATEWAY_TOKEN"]}
        tests = [("/health", headers, 200, "health-failed"),
                 ("/v1/vehicles", headers, 401, "app-auth-exposed"), ("/", None, 403, "origin-exposed")]
        for path, auth, expected, reason in tests:
            code = self.probe(self.config.origin + path, auth)[0] if auth else self.probe(self.config.origin + path)[0]
            require(code == expected, reason)
        code, redirect = self.probe(self.config.access + "/")
        target = urlsplit(redirect)
        blocked = code in (401, 403)
        if code in (302, 303, 307, 308):
            blocked = (target.scheme == "https" and (target.hostname or "").endswith(".cloudflareaccess.com")
                       and target.path.startswith("/cdn-cgi/access/login"))
        require(blocked, "cloudflare-access-exposed")


class ReleaseAttempt:
    def __init__(self, config, adapter, state, sha):
        self.config, self.adapter, self.state, self.sha = config, adapter, state, sha

    def save(self, name, value):
        atomic_json(self.directory / (name + ".json"), value)

    def phase(self, name, **evidence):
        self.state["attempt"].update(dict(phase=name, **evidence))
        atomic_json(self.config.state, self.state)

    def same_runtime(self):
        require(runtime_hashes(self.adapter.runtime()) == self.hashes, "runtime-changed")

    def prepare(self):
        review = read_private(self.config.reviews_dir / (self.sha + ".json"))
        require(matches(review, {"schemaVersion": 1, "sha": self.sha}) and review.get("approved") is True
                and is_sha(review.get("baseSha")), "missing-reviewed-promotion")
        self.expected = validate_ledger(review["migrationLedger"])
        self.adapter.readiness(self.sha)
        snapshot = self.adapter.snapshot()
        self.old = identity(snapshot["app"])
        require(self.old["sha"] == review["baseSha"] and self.old["sha"] != self.sha, "review-base-moved")
        require(snapshot["app"]["status"]["deployment"]["status"] == "COMPLETED", "deployment-already-active")
        builds = items(self.adapter.request("GET", APP + "/build"), "builds")
        require(all(build.get("concluded") is True for build in builds), "build-already-active")
        self.runtime = self.adapter.runtime()
        self.hashes = runtime_hashes(self.runtime)
        self.adapter.gates(self.runtime)
        self.before = self.adapter.audit()
        validate_audit(self.before)
        require(self.expected[:len(self.before["ledger"])] == self.before["ledger"], "review-ledger-mismatch")
        self.directory = private_directory(self.config.state_dir / self.sha)
        evidence = [("runtime-before", self.runtime), ("recovery", {"old": self.old, "runtimeHashes": self.hashes, "review": review}),
                    ("before", self.before)]
        for name, data in evidence:
            self.save(name, data)
        self.state["attempt"] = dict(sha=self.sha, phase="pin-started", directory=str(self.directory))

    def build(self):
        self.phase("pin-started")
        self.adapter.pin(self.old["sha"])
        require(identity(self.adapter.wait_deployment(self.old["sha"])) == self.old, "old-image-changed")
        self.adapter.snapshot()
        self.phase("build-started", readiness=self.adapter.readiness(self.sha))
        self.build_id = self.adapter.start_build(self.sha)
        self.phase("build-wait", buildId=self.build_id)
        self.adapter.wait_build(self.build_id, self.sha)

    def capture_backup(self):
        self.phase("backup-started")
        self.same_runtime()
        self.backup = self.adapter.backup(self.directory)
        self.save("backup", self.backup)
        self.fresh = self.adapter.audit()
        compare_audits(self.before, self.fresh, self.before["ledger"])
        self.save("before-deploy", self.fresh)
        current = self.adapter.snapshot()["app"]
        require(identity(current) == self.old and current["deployment"]["internal"]["buildSHA"] == self.old["sha"],
                "deployment-changed-during-build")
        self.same_runtime()
        self.adapter.gates(self.runtime)

    def activate(self):
        self.ready = self.adapter.readiness(self.sha)
        self.phase("deploy-started", readiness=self.ready, backup=self.backup)
        self.adapter.pin(self.sha)
        self.deployed = self.adapter.wait_deployment(self.sha)
        self.phase("verifying")
        require(identity(self.deployed)["image"] != self.old["image"], "deployed-image-unchanged")
        self.same_runtime()
        self.adapter.snapshot()
        self.adapter.gates(self.runtime)

    def verify(self):
        limit = time.monotonic() + 120
        while True:
            after = self.adapter.audit()
            self.save("after", after)
            try:
                compare_audits(self.fresh, after, self.expected)
            except Halt as error:
                retry = str(error) == "collector-not-ready" and time.monotonic() < limit
                if not retry:
                    raise
                time.sleep(10)
            else:
                break
        final = self.adapter.snapshot()["app"]
        require(identity(final) == identity(self.deployed) and final["deployment"]["internal"]["buildSHA"] == self.sha
                and final["status"]["deployment"]["status"] == "COMPLETED", "deployment-changed-during-verification")
        self.same_runtime()
        self.save("success", dict(sha=self.sha, old=self.old, deployed=identity(self.deployed), buildId=self.build_id,
                                 backup=self.backup, readiness=self.ready, runtimeHashes=self.hashes))
        self.state["deployedShas"].append(self.sha)
        self.state["attempt"] = None
        atomic_json(self.config.state, self.state)


def run(config, adapter, sha):
    require(is_sha(sha), "invalid-sha")
    with deployment_lock(config.state_dir / "deployment.lock"):
        state = read_private(config.state)
        require(state.get("schemaVersion") == 1 and isinstance(state.get("deployedShas"), list)
                and all(map(is_sha, state["deployedShas"])) and "attempt" in state, "invalid-state")
        require(state["attempt"] is None, "attempt-needs-reconciliation")
        if sha in state["deployedShas"]:
            return dict(status="already-deployed", sha=sha)
        attempt = ReleaseAttempt(config, adapter, state, sha)
        for transition in (attempt.prepare, attempt.build, attempt.capture_backup, attempt.activate, attempt.verify):
            transition()
        return dict(status="deployed", sha=sha)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ("--control-root", "--sha"):
        parser.add_argument(flag, required=True)
    args = parser.parse_args()
    os.umask(0o077)
    try:
        require(is_sha(args.sha), "invalid-sha")
        config = Config(args.control_root)
        require(config.root in Path(__file__).resolve().parents, "controller-not-installed")
        response = run(config, Northflank(config), args.sha)
    except Exception as error:
        response = {"status": "halted", "reason": str(error) if isinstance(error, Halt) else "controller-error"}
        print(json.dumps(response))
        return 1
    print(json.dumps(response))
    return 0


if __name__ == "__main__":
    sys.exit(main())
