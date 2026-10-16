---
title: Install R:The Drive
description: Bring up the standard private R:The Drive Compose stack and create the first owner.
slug: /getting-started/install/
sidebar_label: Install R:The Drive
---

# Install R:The Drive

The [setup script](../../scripts/deploy.py) builds R:The Drive from this checkout, starts the private Docker Compose stack and guides you through creating the first owner account.

It installs **on the host where you run it**. It does not provision a server or
deploy to Northflank. If your Docker server is remote, connect to it first and
run the commands there; the helper rejects remote Docker contexts.

## Before you start

You will need Docker Compose v2, Git, Python 3, and a trusted host with persistent storage and enough resources for a Rust/frontend source build. Configure an authenticated HTTPS gateway on the Docker host forwarding to `http://127.0.0.1:8080`. The helper does not provision a domain, TLS certificate or gateway access policy. Read the [prerequisites](./prerequisites.md) and [secure deployment guide](./secure-deployment.md).

## Bring up R:The Drive

1. Clone the repository and enter it:

   ```bash
   git clone <repository-url> r-the-drive
   cd r-the-drive
   ```

2. Run setup with your exact gateway origin:

   ```bash
   python3 scripts/deploy.py --project r-the-drive --origin https://vehicle.example.com
   ```

3. Enter the first-owner email and a new password when prompted. The helper builds the image, generates database/cache credentials, provisions external RSA/AGE keys, starts healthy services and registers the owner locally. It removes the one-time proof after confirming setup is complete.

4. Open your gateway address, sign in and connect your Rivian account. Securely back up the `r-the-drive-keys` Docker volume separately using the [key-custody runbook](../runbooks/key-custody.md).

Generated configuration is ignored by Git, with owner-only permissions. Keys stay in a dedicated external volume mounted read-only in the app. The application port binds to loopback; database and Redis ports are not published.

Use `python3 scripts/deploy.py start` to retry startup or owner creation. The helper refuses an existing installation during setup and never deletes data or regenerates existing keys. If preparation stopped before key generation, inspect the retained state before retrying. See the [configuration guide](./configuration.md) for manual configuration and the [deployment guide](./deployment.md) for updates.

## Installer defaults and options

| Option | Default | Purpose |
| --- | --- | --- |
| Command | `setup` | Creates a fresh installation. `start` resumes the configuration saved in `.deploy/settings.json`. |
| `--origin` | Required for setup | Exact HTTPS gateway address, without a path, query or fragment. No domain, TLS certificate or access policy is created. |
| `--project` | `vehicle-history` | Namespaces Docker resources. The quick start explicitly selects `r-the-drive`, including its `r-the-drive-keys` volume. |
| `--port` | `8080` | Publishes the application on `127.0.0.1` at this port; accepts ports `1024`–`65535`. Match your gateway's forwarding port to it. |
| `--image` | Build this checkout as `<project>:local` | Alternatively pull an existing reviewed image pinned as `registry/image@sha256:DIGEST`. The script does not publish an image. |

These options configure **initial setup**. `start` reads saved settings; passing
new setup flags to `start` does not reconfigure the installation.

The helper explicitly enables production mode, sets the allowed HTTPS origin,
disables insecure LAN HTTP authentication, and uses UTC. It generates distinct
database and Redis passwords and a one-time owner-setup proof. The dedicated
key volume keeps signing and encryption keys out of database storage and is
mounted read-only in the app.

Docker Compose keeps this installation path independent of a hosting provider's
API. It starts the app, TimescaleDB/PostgreSQL and Redis on the same host, with
named volumes for database data, Redis state, backup files and cache. The
loopback application port and unpublished database/cache ports preserve the
authenticated gateway boundary. This is a single-host installation; the helper
does not configure replication, failover or off-host backups.

Configuration lives in `.env` and `.deploy/` with owner-only permissions. Keep
the generated `.deploy/compose.json` overlay and project name in subsequent
Compose commands: they supply the external key mount and select the correct
storage. See [logs and updates](./deployment.md#logs-and-updates) for commands.
For another host or provider, see
[deployment options and moving hosts](./deployment.md#deployment-options-and-moving-hosts).

Using Synology DSM? Follow the dedicated [Synology installation guide](./synology.md).
The DSM guide includes the setup-token template, first-boot delay, and shared
folder ACL checks required for a fresh install.

## Copy the standard Compose file

This is the exact production Compose file from the repository, included at build time so the documentation stays synchronized with the installation source. Copy it into `compose/docker-compose.yml` if you are working from a downloaded documentation bundle, or use the linked source file directly: [`compose/docker-compose.yml`](../../compose/docker-compose.yml).

```compose-include
compose/docker-compose.yml
```

Other Compose variants are available when you need them:

- [`docker-compose.build.yml`](../../compose/docker-compose.build.yml) — build the unified production image from the local checkout.
- [`docker-compose.dev.yml`](../../compose/docker-compose.dev.yml) — start development infrastructure for `pnpm dev:stack`.
- [`docker-compose.oidc-secret.yml`](../../compose/docker-compose.oidc-secret.yml) — optionally mount the OIDC client secret as a read-only file for the production container.

## Next steps

- [Connect your Rivian account](./rivian-account.md).
- [Check logs, update, or back up the stack](./deployment.md).
- [Understand privacy and third-party requests](../privacy.md).
