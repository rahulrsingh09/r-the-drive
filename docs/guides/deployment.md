---
title: Deployment and updates
description: Choose a host, deploy, update, inspect, stop, and recover R:The Drive.
slug: /operations/deployment-and-updates/
sidebar_label: Deployment and updates
---

# Deployment and updates

The standard self-hosted stack runs TimescaleDB, Redis, and one unified R:The Drive container containing the API, web app, nginx origin, and backup tools. Only the unified app is published to the host, on `127.0.0.1:8080` by default. `PANDA_HOST_BIND_ADDRESS` explicitly overrides the host interface.

Place an authenticated HTTPS tunnel or identity-aware reverse proxy in front of the app and restrict direct port `8080` access with your host firewall. Never publish the API listener, database, or Redis directly.

## Deployment options and moving hosts

The [setup script](../../scripts/deploy.py) uses Docker Compose on the **current
host**. It does not create a server or select a cloud provider. Run it on the
Docker host itself; remote Docker contexts are unsupported because setup and
first-owner registration use the host's local application endpoint.

| Deployment path                  | What is included                                                                                                    | What you configure                                                                                                                             |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| Setup script on your Docker host | Source build or digest-pinned image pull, local services, persistent volumes, passwords, keys and first-owner setup | The host, persistent storage, authenticated HTTPS gateway, domain/TLS and off-host backups                                                     |
| Manual Compose installation      | The same application, database and Redis service definitions                                                        | Configuration, image selection, key provisioning, gateway and owner setup                                                                      |
| Northflank controller template   | Deployment checks and controlled promotion for an already configured Northflank installation                        | Repository/infrastructure identifiers, existing services and storage, secrets, protected controller configuration, review evidence and gateway |

The separate [`scripts/northflank_deploy.py`](../../scripts/northflank_deploy.py)
controller is not invoked by the installer. Its checked-in repository and
infrastructure settings are templates, not a ready-to-run deployment for this
repository. Do not run it against another installation without adapting and
validating its configuration. There is no `--provider` option or general cloud
provisioning script.

You can use the same installer for a fresh installation on another host meeting
the [prerequisites](./prerequisites.md). For a managed container platform, adapt
the service networking, TimescaleDB/PostgreSQL and Redis connections, persistent
storage, external keys and authenticated gateway to that platform. The existing
installer does not perform that conversion or select a hosting plan.

To move an **existing R:The Drive installation**, plan and test a restore rather
than rerunning fresh setup over its data. Preserve the deployment configuration
and compatible image version, verify a database backup, and transfer the
required persistent files and external keys securely. A raw database backup
needs its original AGE identity to decrypt saved credentials. Recovery packages
exclude credentials and installation keys, so that route requires reconnecting
providers. Follow [backup and restore](./backup-and-restore.md) and
[key custody](../runbooks/key-custody.md), stop collection on the old host for the
final transfer, and verify the new installation before switching the gateway.
The helper does not automate host migration.

## Initial deployment with the helper

Follow the [four-step installation guide](./getting-started.md). The helper
generates `.env`, `.deploy/settings.json`, `.deploy/compose.json` and the private
`.deploy/database-bootstrap-password` file, provisions the database, then starts
the stack and registers the owner locally. It retains these files and all
volumes on failure; `python3 scripts/deploy.py start` resumes startup using the
saved settings.

## Database bootstrap and runtime roles

The helper's generated overlay initializes TimescaleDB as `panda_bootstrap`.
Its password is a database-only Compose secret. It is not stored in `.env`,
passed in command arguments, or mounted in the application. Keep the private
file and overlay with the installation's configuration backups.

Before application startup, `scripts/deploy.py` calls its shared
`provision_database(settings)` function. This creates a restricted `panda`
login and installs `pgcrypto` and `timescaledb` in `public`, and `cube` and
`earthdistance` in `panda`. The application owns its database and schemas, but
has no superuser, role-creation, database-creation, replication or row-security
bypass privileges and inherits no other role. It uses its separate `.env`
password for migrations and runtime. Production startup continues to revoke
destructive history privileges and verify that those revocations take effect.

Provisioning commits role, schema and extension creation together. A retry
validates the existing role, ownership, extension locations and saved application
login; it does not rotate credentials, recreate data, change existing role
privileges, or restore revoked history permissions. A mismatched or legacy
installation fails closed and keeps its files and volumes.

Do not fix an older installation by disabling history protection or merely
changing the bootstrap role to `NOSUPERUSER`: fresh migration also requires
privileged extension installation. Before converting an existing installation,
verify its backups and keys, inventory database ownership and extensions, and
prepare a reviewed role/ownership migration for that database. This helper only
provisions its own fresh installations and does not automate that conversion.

## Manual initial deployment

These steps are an alternative to the helper. Do not overwrite the configuration
or omit the generated key overlay of an installation created by the helper.

1. Prefer the helper for a fresh production installation. For a manually managed database, provision the separate bootstrap and restricted application roles and extensions described above before application startup. The base Compose file alone initializes `POSTGRES_USER` as a PostgreSQL superuser; passing that same role to the production application is rejected by history protection.
2. Copy `compose/.env.example` to `.env`. Set the restricted application's database credentials separately from the database service's bootstrap credentials, using a database-only overlay or your platform's secret configuration. Set a separate strong Redis password, your exact public HTTPS `ALLOWED_ORIGINS` value, and a one-time `PANDA_SETUP_TOKEN` of at least 32 bytes. Never put bootstrap credentials in the application's `env_file`.
3. Provision the complete external RSA/AGE bundle using the [key-custody runbook](../runbooks/key-custody.md), and set `PANDA_IMAGE` to the reviewed digest built from the tested source. Existing DB-backed installs must export and migrate their original keys first. Start with your database credential overlay; mounted key files also require the `docker-compose.keys.yml` overlay:

   ```bash
   docker compose --env-file .env -f compose/docker-compose.yml -f compose.local.yml up -d
   ```

4. Verify it with the same overlay:

   ```bash
   docker compose --env-file .env -f compose/docker-compose.yml -f compose.local.yml ps
   curl http://localhost:8080/health
   ```

5. Configure an authenticated gateway that forwards to port `8080` and supports WebSockets.
6. Open the HTTPS address and create the first owner account.

## Persistent files

New installations use Docker-managed volumes:

| Compose volume key | Container path | Contents                                           |
| ------------------ | -------------- | -------------------------------------------------- |
| `panda-db`         | `/db`          | PostgreSQL data                                    |
| `panda-redis`      | `/data`        | Redis append-only state                            |
| `panda-backups`    | `/backups`     | Downloadable recovery packages                     |
| `panda-cache`      | `/data/cache`  | Application cache files, including vehicle artwork |

External key files are deliberately outside these volumes. Back them up separately
and restore the same AGE identity with raw database backups.

Compose prefixes these volume keys with the project name. With the quick start's
`--project r-the-drive`, database storage is `r-the-drive_panda-db`, for example.
The helper also creates the separate external volume `r-the-drive-keys` and
mounts it read-only at `/run/secrets/app`. A volume on the same host is not an
off-host disaster-recovery copy.

Do not delete Docker volumes during updates. Copy recovery packages off-host for disaster recovery. Existing installations that omit the four `*_SOURCE` variables continue using their existing `PANDA_DATA_DIR` bind paths.

If an existing installation uses bind paths, preserve its `PANDA_DATA_DIR`
and grant the application user write access to its backup and cache directories.
The app performs a writable-storage probe before starting and reports the exact
container path when the host ACL is insufficient.

## Synology DSM

Use the dedicated [Synology DSM installation guide](./synology.md). It uses the
same universal Compose file and Docker-managed volumes; no Synology-specific
Compose file or generator is required.

## Logs and updates

For a **helper installation**, use this shell function from the repository root
so every command retains the saved project and key overlay:

```bash
drive_compose() {
  docker compose \
    --project-name "$(python3 -c 'import json; print(json.load(open(".deploy/settings.json"))["project"])')" \
    --env-file .env \
    -f compose/docker-compose.yml -f .deploy/compose.json "$@"
}

drive_compose ps
drive_compose logs -f panda
```

Before updating, verify backups and read the migration notes below. For an
image-based update, set `PANDA_IMAGE` in `.env` to the reviewed new image digest,
then use `drive_compose pull` and `drive_compose up -d --wait`.
For a source-built installation, build the reviewed source with
`compose/Dockerfile`, give the image a new local tag, set `PANDA_IMAGE` to that
tag, and use `drive_compose up -d --wait`. The helper's `start` command does not
rebuild source or select a new application version for you.
Keep the image record in `.deploy/settings.json` aligned with your chosen image.
Do not rerun `setup` for an upgrade.

For a **manual installation**, the equivalent base commands are below. Retain
any key overlay and project name you selected during manual setup:

```bash
docker compose --env-file .env -f compose/docker-compose.yml logs -f
docker compose --env-file .env -f compose/docker-compose.yml logs -f panda
docker compose --env-file .env -f compose/docker-compose.yml pull
docker compose --env-file .env -f compose/docker-compose.yml up -d
```

The production nginx origin does not cache `index.html` or SPA fallbacks, so
new deployments can select the current Vite asset manifest. Existing files
under `/assets/` are content-hashed and receive one year of immutable caching;
vehicle fallback images retain a one-day cache. After the first upgrade that
introduced this policy, perform one hard refresh in each browser that may have
cached the previous SPA shell.

The app applies immutable, forward-only database migrations on startup. Set
`PANDA_IMAGE` to the reviewed digest of the new tested source image before
pulling. There is no automatic `latest` fallback. Existing pre-release installations must
complete the one-time explicit baseline
adoption in the [release database cutover runbook](../runbooks/release-database-cutover.md)
before starting the flattened public release; startup never edits migration
bookkeeping automatically.

The charge identity upgrade is health-first: the schema expansion completes
before the app binds, then the unified app container reports `/health` while a
resumable in-process worker backfills existing charge history in the
background. A healthy response does not mean that every vehicle's backfill is
complete. Monitor the structured `charge_payload_identity_backfill_started`,
`charge_payload_identity_backfill_progress`,
`charge_payload_identity_backfill_complete`, and
`charge_payload_identity_backfill_failed` events before declaring a populated
upgrade finished. The PostgreSQL
`panda.charge_payload_identity_backfill_status` row is the durable
checkpoint; a restart resumes rows whose `payload_fingerprint` is still null.
The later charge-identity helper migration only installs the canonical
fingerprint and identity-key functions used by ingestion, backfill, and
compaction; it does not start another backfill. Do not add a second backfill
container.

Before an upgrade, verify a recovery package and a raw `pg_dump`. If rollback
is required after the migration ledger advances, restore that pre-upgrade dump
using the previous image; reverting the image alone is not a safe rollback.

The PostgreSQL 18 image cannot reuse a PostgreSQL 16 data directory. Before upgrading an existing PostgreSQL 16 installation, create and verify a recovery package plus a raw `pg_dump`, stop the old stack, move its data directory aside, and restore into a newly initialized PostgreSQL 18 volume. Never point PostgreSQL 18 at the former PG16 directory. Follow the [backup and restore runbook](../runbooks/backup-restore.md) for the validation sequence.

Redis 8 can read the tested Redis 7 append-only snapshot format. Preserve a copy of `data/redis` before the upgrade. If Redis rejects the snapshot, start with an empty Redis directory; users will need to sign in again and external providers may need to reconnect, but PostgreSQL telemetry and configuration remain intact.

## Build from source

The helper already builds from source unless you pass `--image`. For a manual
Compose installation, use the build overlay below, retaining your key overlay
and project name:

```bash
PANDA_IMAGE=panda:local docker compose --env-file .env -f compose/docker-compose.yml -f compose/docker-compose.build.yml up -d --build
```

The image assigns mode `0644` to its two root-owned nginx configuration files,
so the runtime user (UID `1001`) can read them even when a restrictive checkout
or archive umask makes the source files `0600`. This applies only to the bundled
non-secret nginx configuration; external key and secret permissions are unchanged.

Local development continues to use `pnpm dev:stack` and
`compose/docker-compose.dev.yml`; production image consolidation does not
change that workflow. Extended Parallax acquisition is enabled inside the API
by default. Set `PARALLAX_ENABLED=false` only for emergency rollback; production
and development no longer launch a separate collector process or container.

## Stopping and recovery

For a helper installation, use `drive_compose down` with the function above.
For a manual installation, retain your chosen overlays and project name:

```bash
docker compose --env-file .env -f compose/docker-compose.yml down
```

`down` retains Docker-managed volumes and existing bind-mounted application
data. Do not add `--volumes` during a routine stop or update. See
[backup and restore](./backup-and-restore.md) before replacing or deleting any
persistent storage.
