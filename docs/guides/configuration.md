---
title: Configuration
description: Configure the minimal production values and optional Panda settings.
slug: /getting-started/configuration/
---

# Configuration

The standard Compose file reads the repository-root `.env` inside the unified app container. Keep real values out of Git and use a secret manager where your host supports one.

Start with [`compose/.env.example`](../../compose/.env.example). It lists the required deployment settings and external-key provisioning route. [`compose/.env.full.example`](../../compose/.env.full.example) is the complete override template, while the [environment-variable reference](../environment-variables.md) documents every supported value, default, and scope.

## Required for production

Set these before starting `compose/docker-compose.yml`:

- `POSTGRES_PASSWORD` — a strong database password; the app safely constructs its internal database URL from it.
- `REDIS_PASSWORD` — a separate strong password.
- `ALLOWED_ORIGINS` — the exact public HTTPS origin used by your authenticated gateway, such as `https://panda.example.net`.

Panda defaults to production mode; no production flag is required in `.env`.

Production requires an externally managed RSA signing pair and AGE identity:
`JWT_SECRET`, `JWT_PUBLIC_KEY`, and `AGE_ENCRYPTION_KEY`, each supplied as a
securely injected value or its mutually exclusive `_FILE` alternative. No paid
secret manager is required; separately stored, owner-readable mounted files work.
See the [key-custody runbook](../runbooks/key-custody.md) to generate keys for a new
database or export and migrate existing DB-backed keys without replacing them.
Missing/invalid bundles, mismatched RSA pairs, and unmigrated database keys fail
startup. The public AGE binding prevents accidental key replacement on restart.
Back up the bundle separately; losing the AGE key loses access to encrypted data.

Set `PANDA_IMAGE` to a reviewed digest reference built from the tested source.
Compose no longer defaults to a mutable `latest` image.

### Production first owner

Before the first production registration, set exactly one setup proof:

- `PANDA_SETUP_TOKEN` for a securely injected value, or
- `PANDA_SETUP_TOKEN_FILE` for a mounted secret file (preferred).
Synology uses the same `compose/.env.example` template and universal Compose
file as every other Docker host. The template leaves the setup token commented
out so an unclaimed production stack fails closed until a unique proof is
configured.

The proof must contain at least 32 bytes. The first registration sends it as
`setup_token`; the app uses it only while no user exists. The public setup
status intentionally reveals only whether a proof is required and available,
not its source or value. Without a proof, an unclaimed production stack stays
healthy but fails closed for registration. Remove or rotate the bootstrap secret
after the first owner is created. See the [environment reference](../environment-variables.md)
for the exact variable contract.

## Optional settings

Weather, geocoding, basemap, and Iconify policies are configured in **Settings > External Connections** and stored in the database. Do not add provider URLs or API keys to `.env`; custom connection secrets are encrypted with the installation age key and remain write-only. See [external connections](./external-connections.md).

- `PANDA_ORIGIN_PORT` changes the published app port from `8080`.
- `PANDA_HOST_BIND_ADDRESS` controls Docker's host-side published address;
  it defaults to `127.0.0.1` for a gateway on the same host. Explicit private
  interface publication still requires an authenticated gateway and firewall.
- `PANDA_BIND_ADDRESS` controls the application's internal listener and
  defaults to `127.0.0.1`; it is not the Docker host publication address. A
  non-loopback internal listener requires `ALLOW_PUBLIC_ORIGIN_BIND=true`.
- `PANDA_IMAGE` is required. Use the reviewed digest of the tested image;
  use `panda:local` only with the source-build overlay. IMAGE_TAG and
  PANDA_IMAGE_REGISTRY no longer select the production image.
- `BACKUP_DRIVER`, `BACKUP_ARTIFACT_DIR`, and `BACKUP_POLL_INTERVAL_SECONDS` tune recovery packages; normal Compose already uses `/backups`.
- `TZ` sets the Docker/container timezone for nginx and other runtime processes. It is separate from the shared user-facing application timezone configured under **Settings > Units**.
- Reconnect, telemetry-retention, logging, and rate-limit settings are available in the [complete reference](../environment-variables.md).

For optional single sign-on, follow the [OIDC single sign-on guide](./oidc-sso.md).
It explains the **Settings > Authentication** workflow, field-level environment
overrides, and the break-glass recovery path before password login is disabled.

## Local HTTP development

The local development stack is started with `pnpm dev:stack` and uses
`compose/docker-compose.dev.yml`. It deliberately sets:

```dotenv
PANDA_ENV=development
COOKIE_INSECURE=true
```

The development stack runs over HTTP. `COOKIE_INSECURE=true` makes the
`HttpOnly` refresh cookie usable by the browser on that HTTP origin, so a page
refresh can resume the session instead of requiring another login. This is not
an HTTPS substitute: leave the variable unset for the standard production
Compose stack, which keeps refresh cookies `Secure`. If you run the
production-like Compose file locally over HTTP, set both values in an
untracked local env file and do not reuse that file for production. Existing
local env files using `1` or `0` must be updated to `true` or `false`.

The active `.env` and `.env.local` files remain ignored by Git.
