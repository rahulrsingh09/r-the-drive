# Local Northflank deployment controller

> Bootstrap status: this is an inherited operational template. No deployment, upstream synchronization or publication is configured for PANDA. See [BOOTSTRAP.md](../../BOOTSTRAP.md) before using it.

Documentation impact: internal and documentation-site update required.

This is the legacy manual recovery controller. Automatic releases now use the
[GitHub/Northflank pipeline](native-northflank-release.md). Keep the old KiRoom
deployment trigger disabled. The installation and polling instructions below
describe the former design, not the current automatic release path.

`scripts/northflank_deploy.py` is a one-shot Python 3 standard-library controller
for the existing `panda-private` project. Install a reviewed copy outside Git.
Invoke that installed file with an absolute control root and a full lowercase SHA.
It does not install polling, promote branches, push Git refs, create resources,
change CI triggers, or write runtime variables. The operator installs the reviewed files outside Git and enables the separate
KiRoom triggers only after dry-run verification.

The daily agent reviews and promotes candidates to the protected default.
This controller only consumes successful default-branch CI through the installed
[`fork-deploy-readiness.mjs`](../../scripts/fork-deploy-readiness.mjs) contract.
Its byte hash is pinned in the Python source:
`c113d52b70432ca88a9b9473bc5af7e1c2236e7a46e3b519acb942013bbe0205`.
Changing that checker requires a reviewed controller update and installation.
Candidate repository code, package managers, hooks, CI artifacts and commit text
never execute locally in the credential-bearing controller.

## Installation contract

Use Python 3.7 or later on Linux with `fcntl`, and the pinned Node `24.18.0`.
The Python tests also run on Python 3.7. Keep the Node installation and Northflank
CLI dependency tree under the deployer's control; the controller checks the Node
version but does not independently hash the CLI's transitive dependencies.

The fixed control root contains:

```text
/absolute/panda-control/
  config.json
  northflank_deploy.py
  fork-deploy-readiness.mjs
```

Create the root, state, review and Northflank-config directories with mode `0700`.
Use `0600` for config files, review receipts, deployment state and recovery files.
They must belong to the invoking deployer and be outside every Git checkout.
All configured paths must be absolute. Leaf symlinks are rejected; an existing
host alias such as `/home` resolving to `/local/home` is allowed. Install the
controller and checker directly; do not point them at a candidate checkout.

Example `config.json` (substitute the two deployment-specific HTTPS origins and
the chosen state/review paths locally):

```json
{
  "node": "/home/deployer/.local/share/mise/installs/node/24.18.0/bin/node",
  "nf_cli": "/home/deployer/projects/panda-tooling/northflank-cli/node_modules/@northflank/cli/dist/cli.js",
  "nf_config_dir": "/home/deployer/.config/northflank-panda",
  "readiness": "/absolute/panda-control/fork-deploy-readiness.mjs",
  "github_cli": "/absolute/trusted/bin/gh",
  "state_dir": "/absolute/panda-state",
  "reviews_dir": "/absolute/panda-reviews",
  "origin_url": "https://your-northflank-origin.code.run",
  "access_url": "https://your-protected-worker.workers.dev"
}
```

The Northflank directory must already contain `config.json` with its selected
`current` context, matching entry in `contexts`, token, and host
`https://api.northflank.com`. Do not copy token values into this controller config.
Only the API adapter reads that token. The CLI receives `NF_CONFIG_DIR`; it reads
the same local context. Subprocess environments start from an allowlist of
`PATH`, `HOME`, and `LANG`. The readiness subprocess receives no Northflank config
environment, inherited token, proxy override or `NODE_OPTIONS`.
Northflank build arguments, build files and Docker secret mounts must all be
empty, including inherited values. No production secrets are sent to GitHub or
passed in the build request. The deployed runtime retains its original secrets.

The optional `github_cli` path enables authenticated metadata reads through the
existing host's saved GitHub CLI login. Configure it for this installation to
avoid the shared anonymous API quota. The trusted checker calls only
`gh api --hostname github.com --method GET` for this repository, without a shell.
It never extracts or prints the token. The CLI gets an allowlisted environment
and reads its existing local login; inherited `GH_TOKEN`, `GITHUB_TOKEN`,
`GH_CONFIG_DIR`, proxy variables, and cloud secrets are not forwarded.
An expired login or CLI error stops readiness without an anonymous fallback.
The CLI executable must be an absolute canonical regular file owned by the
deployer and not writable by group or others.

Configure the trigger source with the same canonical path:

```sh
node /absolute/panda-control/fork-deploy-readiness.mjs \
  --state /absolute/panda-state/deployed.json \
  --github-cli /absolute/trusted/bin/gh
```

Omitting `github_cli` retains the unauthenticated public transport. Both
transports enforce the same repository, protection, exact-SHA and CI checks.

Initialize `<state_dir>/deployed.json` only after independently verifying the
currently deployed SHA:

```json
{
  "schemaVersion": 1,
  "deployedShas": ["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
  "attempt": null
}
```

Replace the synthetic SHA above with the verified deployed commit. Never seed a
candidate as successful before deployment. The same file is also accepted by the
readiness checker; its extra fields are ignored by that checker.

For each reviewed promotion, write `<reviews_dir>/<full-candidate-sha>.json`
with these fields:

| Field | Required evidence |
| --- | --- |
| `schemaVersion` | Integer `1` |
| `sha` | Exact full candidate SHA |
| `baseSha` | Exact currently deployed SHA |
| `approved` | Boolean `true`, recorded by the trusted reviewer |
| `migrationLedger` | Complete expected post-deployment ordered array of `{version, checksum, success}` |

Each migration version is a positive integer, checksum is the lowercase 96-digit
SQLx SHA-384 checksum, and success is boolean `true`. Record the entire catalog
from reviewed migration evidence, preserving the currently deployed ledger as
an unchanged prefix. Missing, reordered, changed or extra migrations fail closed.
Do not derive approval by executing a candidate script with deployment credentials.
The receipt is local review evidence; public CI metadata alone does not prove
that source and migration changes were reviewed.

The known `df97d50` to `de6a85f` upgrade retains private version `28` and adds
upstream versions `1000028`, `1000029`, `1000030`, moving 28 ledger rows to 31.
Do not hard-code these row counts for later upgrades; use the complete reviewed
catalog. The application upgrade to `de6a85f` was verified on 2026-10-06: all 31
migrations succeeded, the original 28-entry prefix and runtime keys were unchanged,
and the connected Rivian collector resumed. This controller is installed separately.

After installation, the fixed command is:

```sh
/usr/bin/python3 -B /absolute/panda-control/northflank_deploy.py \
  --control-root /absolute/panda-control \
  --sha <full-40-character-lowercase-sha>
```

Supply the SHA as one structured argument from the ready item. Do not interpolate
repository text into a shell script. Maintain one state directory and one lock for
this deployment. Independently launched administrative changes do not acquire
that lock; serialize them with this controller.

## Deployment sequence

1. Acquire a nonblocking `fcntl` lock. Validate the state and receipt. A previously
   successful SHA returns `already-deployed` without cloud work. Any outstanding
   attempt blocks all new deployments until deliberate reconciliation.
2. Recheck exact-SHA readiness and inspect services, add-ons, jobs, volumes,
   build arguments and `/v1/billing/usage`. Require an idle app and no unfinished
   builds. Verify origin/Access gates, runtime configuration and database evidence.
3. Privately retain the original runtime environment, SHA, immutable image,
   runtime SHA-256 hashes, review receipt, SQLx ledger and history counts.
   Persist `pin-started` before invoking the CLI.
4. Pin the **currently deployed** SHA through `update service deployment`, wait
   for completion, and verify that the old image is unchanged. Recheck the resource
   policy and readiness immediately before starting the exact candidate build:
   `POST /v1/projects/panda-private/services/telemetry-app/build` with only
   `{"sha":"<full-sha>"}`. Do not select `latest`.
5. Poll `/services/telemetry-app/build/<build-id>` for the exact SHA and explicit
   concluded success, with a 30-minute deadline. Continue polling during the
   documented `QUEUED`, `PENDING`, `STARTING`, `CLONING`, `BUILDING`, `UPLOADING`,
   and `IN_PROGRESS` states. Failed, unschedulable and unknown states halt;
   `SUCCESS` alone is insufficient without both `concluded: true` and
   `success: true`. No failed or uncertain POST is retried automatically.
6. After build success, take a **fresh** remote custom-format `pg_dump` under
   `/backups/pre-deploy-<UTC>.dump`. The fixed script uses `set -eu`, `umask 077`,
   normalizes `options=-c+` to `options=-c%20`, refuses to overwrite a dump, and
   runs `pg_restore --list`. Download it through `download service file`; verify
   byte count and SHA-256 locally. A missing or mismatched dump blocks deployment.
7. Capture fresh database counts/ledger, recheck resources, the old deployed
   identity, original runtime hashes and access gates, then reread exact-SHA CI
   readiness immediately before the deployment call. Persist `deploy-started`.
8. Deploy using the same fixed CLI command with the approved candidate SHA:

   ```text
   update service deployment --projectId panda-private
     --serviceId telemetry-app
     --input {"internal":{"id":"telemetry-app","branch":"mainline","buildSHA":"<full-sha>"}}
     --quiet
   ```

9. Wait up to ten minutes for completed deployment of that pinned SHA. Verify
   the new immutable image, unchanged runtime/key hashes, resource policy, gates,
   schema, full expected ledger and data counts. A collector reconnect may take
   up to two additional minutes; only its read-only verification is retried.
   Check the deployed identity again before committing success.
10. Write private success evidence, then atomically replace `deployed.json` with
    the appended successful SHA and cleared attempt. Writes use a same-directory
    temporary file, `fsync`, rename, and directory `fsync`.

There is no transaction spanning GitHub, Northflank and the local filesystem.
Rereads bound the race window; they cannot prevent an independent administrator
from changing state after a check. Process death or a lost response can leave an
attempt requiring reconciliation even when the cloud operation succeeded.

## Resource and verification policy

The policy is deliberately fixed to the existing allocation:

- Exactly `telemetry-app` and `telemetry-redis`, each one `nf-compute-20` instance.
- App build plan `nf-compute-400-16`; app `disabledCI` must be boolean `true`.
- Exactly one PostgreSQL add-on `telemetry-db`, one `nf-compute-20` replica,
  6144 MiB, no pending actions, and external access disabled.
- Exactly one 6144 MiB `app-data` volume owned by and attached only to the app.
- No jobs; Redis has no public/VPC port. The app has only its existing public
  HTTP port 8080, without additional custom domains or VPC access.
- Nonempty billing usage with explicit zero USD totals. Missing, truncated,
  unknown or changed required fields stop the controller.

These checks enforce a configured allocation and observed zero usage; they do not
guarantee future provider pricing or prevent charges from another administrator.
The controller never upgrades plans, scales resources or creates a paid fallback.
It does not delete old images, remote dumps or local recovery files. Manage
capacity and retention separately after checking the recovery requirements.

Database probes run one repeatable-read, read-only transaction via `psql -X`,
using runtime `DATABASE_URL` without printing it. The migration ledger is
**`public._sqlx_migrations`**, not `panda._sqlx_migrations`. App tables are in
`panda`; raw telemetry is in `timeseries.telemetry`. Verification records only
the ordered migration version/checksum/success and aggregate counts of telemetry,
trips, charging sessions, state periods, vehicles, users and credential rows.
Counts cannot decrease, credential count must remain exact, and all credentialed
vehicles must report authorized/connected collectors with fresh heartbeats and
no trip persistence error. The private checkpoint/pending-completion tables
must still exist. No credential ciphertext, location, VIN or vehicle command is
selected or sent by these probes.

HTTP probes use GET without following redirects:

- Origin `/health` with `X-Panda-Edge`: `200`.
- Origin `/v1/vehicles` with that gateway token but **no app token**: `401`.
- Origin `/` without gateway token: `403`.
- Workers origin `/` without credentials: `401`/`403`, or an HTTPS redirect to
  a `*.cloudflareaccess.com/cdn-cgi/access/login...` destination.

The gateway token is sent only to the configured origin, never to the Workers
URL or a redirect destination. These are smoke checks for these routes, not
proof of every authentication path. There are no enrollment attempts, real
credential tests, auth bypasses or car commands.

## Failure and recovery

Stdout contains only a small JSON status (`deployed`, `already-deployed`, or
`halted` with a static reason). Halts exit nonzero. Raw CLI/API output, exception
details, runtime secrets and database URLs are never emitted.

`deployed.json` retains the last started phase and build/backup evidence.
`<state_dir>/<sha>/` retains original runtime, old image/SHA, review, baseline and
fresh audits, backup evidence and the dump. A failure after the candidate
deployment request may already have run migrations. It is never marked successful
and never retried automatically, even for a different queued SHA.

Before manually clearing `attempt`, hold the same `deployment.lock` and determine
whether a build, pin or migration began. Reconcile the live image, ledger, keys,
counts and gates with the private evidence. If the intended deployment completed,
perform the full verification before recording it successful. Otherwise follow
the separate administrator recovery procedure. Keep the failed attempt directory
as evidence; a fresh attempt for the same SHA requires deliberately archiving
that directory as well as reconciling state. There is no reset/retry/rollback
switch that discards uncertainty.

Rollback after forward migrations requires the **database backup, original
RSA/JWT and AGE keys, and old immutable image together**. Never perform an
automatic image-only rollback. Do not rotate keys, delete real data or restore a
production backup into a runnable test app as part of this controller.
Fresh production-backup restore validation is a separate parent task:
`pg_restore --list` and download hash verification prove archive readability and
transfer integrity, not a successful restore.

## Tests

All controller tests use synthetic fixtures and mock adapters. They do not read
the real Northflank config or contact the cloud.

```sh
python3 -B -m unittest discover -s scripts -p test_northflank_deploy.py -v
node --test scripts/northflank-deploy.test.mjs scripts/fork-deploy-readiness.test.mjs
```

`northflank-deploy.test.mjs` invokes the Python suite, so the existing
`pnpm test:scripts` CI step discovers it without a workflow change. Coverage
includes resource/payment/exposure drift, stale SHAs before build and deploy,
bad build/backup evidence, failed/uncertain deployments, runtime key changes,
ledger/checksum/count regressions, idempotency, lock contention, atomic state
writes, subprocess credential isolation and fail-closed HTTP probes.

## KiRoom coordination in this installation

The former daily KiRoom upstream trigger is disabled. GitHub now checks upstream
daily at **12:00 PM Pacific time**, following PST/PDT automatically. See
[private fork maintenance](private-fork-maintenance.md) for the active schedule
and [the optional review runbook](kiroom-upstream-review.md) for exceptions.

The separate 15-minute deployment trigger is retired and must remain disabled.
GitHub's successful-workflow event starts the native Northflank release workflow.
KiRoom is optional for exception review; routine updates, deployment and the
running app operate independently.

The public `/health` endpoint is intentionally available to Northflank probes.
The origin denial test targets `/`, while `/v1/vehicles` requires app authentication
even with the gateway header. Live read-only preflight verified these routes,
the singular `/build` API, and `IN_PROGRESS` rollout status; synthetic regression
tests also cover those provider contracts. The controller never changes gateway
policy to make a test pass.

New commits containing only this maintenance automation still pass the same
protected-branch checks. Controller/checker installation remains a separate,
reviewed action; the daily update agent cannot replace its own deployment policy.
