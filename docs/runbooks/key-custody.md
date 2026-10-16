# External application key custody

Production requires a complete externally managed RSA signing pair and age X25519
identity. It validates all three before connecting to PostgreSQL, checks that the
RSA pair can sign and verify RS256, and rejects database-backed key rows before
serving requests or starting workers. Only `PANDA_ENV=development` permits
automatic database key generation, on an empty installation. Unknown environment
names, partial bundles, empty values, unreadable files, and mismatches fail closed.

The three settings are `JWT_SECRET`, `JWT_PUBLIC_KEY`, and `AGE_ENCRYPTION_KEY`.
For each setting choose either its direct value or its `_FILE` alternative.
Files contain the actual PEM or one AGE secret identity, not escaped `\n` text.
An AGE file must contain only the identity and optional surrounding whitespace,
not the comment headers produced by some other key generators.

No paid secret manager is needed. Use owner-only files outside PostgreSQL,
Redis, recovery-package, cache, and source-control storage, or your host's securely
injected runtime secrets. Retain an independently protected backup of the bundle.
Do not paste private keys into shell arguments, tickets, chat, or logs, and do not
run shell tracing or print environment/config dumps containing secrets.

## New installation

Use the `panda-keys` binary from the same reviewed source as the app. It is
included at `/app/panda-keys` in source-built unified images. It also runs
from `apps/api` with `cargo run --locked --bin panda-keys -- ...`.
It never overwrites an existing directory or file and never prints key material.

On a trusted Unix host, select a **new** directory under an existing protected
parent outside the repository and application data:

```bash
panda-keys generate /secure/panda-keys
```

This creates `jwt_private.pem`, `jwt_public.pem`, and `age_key.txt` with mode
`0600`, in a directory with mode `0700`. Back them up separately before startup.
For a container, arrange ownership/ACLs so the app's UID/GID `1001:1001` can
read them; do not solve access errors by making the private keys world-readable.
When generating inside a container, mount the protected **parent** directory and
use `--entrypoint /app/panda-keys`; the command needs to create the bundle
directory itself. The parent must be writable by the selected container user.

For Compose file provisioning, set `PANDA_KEYS_SOURCE` in `.env` to that
absolute directory, omit the direct key values, and add the supplied overlay:

```bash
docker compose --env-file .env \
  -f compose/docker-compose.yml -f compose/docker-compose.keys.yml up -d
```

The overlay mounts the directory read-only under `/run/secrets/panda` and
supplies the three `_FILE` paths. It refuses a missing host source rather than
creating an empty directory. Include the overlay consistently for subsequent
Compose commands. Standard Compose without this overlay expects externally
injected values or file paths mounted through your own deployment configuration.

Set `PANDA_IMAGE` to the reviewed digest of an image built from the tested
source. There is no default image. To build locally, set
`PANDA_IMAGE=panda:local` and also include
`-f compose/docker-compose.build.yml`; this local tag is not a published release
or evidence of a security review.

For a personal hosted demo, provide all three values through the host's runtime
secret settings, preserve a separate offline copy, and keep real account
credentials out of the demo. Key custody does not depend on a paid integration.

The disposable `verify:fresh-install` procedure follows the production contract:
its caller-owned `--production-env` must supply all three external keys, or pass
`--keys-source /absolute/path/to/disposable-test-bundle` to mount the key overlay.
Use synthetic keys for verification, never production credentials. `--source-build`
selects `panda:local`; published-image tests should use `--image-ref` with the
reviewed digest. Release/fresh-install CI callers must provision this test bundle
before invoking the procedure; there is no production key-generation fallback.

## Existing database-backed installation

Do **not** run `generate`, clear `system_config`, replace the AGE key, or remove
encrypted credentials to make startup succeed. The old AGE identity is required
to read old ciphertext; preserving the original RSA pair also preserves its
verification identity. A partial or invalid original bundle requires recovery
from a protected raw backup, not automatic regeneration.

1. Stop every app, restore agent, worker, and old-version instance using this
   database, keeping PostgreSQL available. Take and verify a protected raw
   `pg_dump` and preserve the old deployment configuration. Sanitized downloadable
   recovery packages exclude installation keys and credentials; they are not a
   substitute for this migration backup.
2. Securely supply `DATABASE_URL` to the key utility's process. This maintenance
   utility needs that URL explicitly, even when normal Compose builds it from
   `POSTGRES_PASSWORD`. Do not put the URL in command history. The utility uses
   the existing schema and does not run or edit migrations.
3. Export the original bundle to a new protected directory:

   ```bash
   panda-keys export-db /secure/panda-original-keys
   ```

   Export is read-only against the database. It fails on incomplete or invalid
   key rows. If writing fails partway, the DB originals remain; preserve and
   inspect the partial files, then use a new directory for the next attempt.
4. Independently back up the exported bundle and verify that the intended
   runtime can read it. Keep the original files unmodified. This is a custody
   change, not a cryptographic rotation.
5. Explicitly migrate custody using those same files:

   ```bash
   panda-keys migrate-db /secure/panda-original-keys
   ```

   The transaction locks the key and encrypted-value tables, validates the
   supplied RSA pair against the original pair, compares the original AGE
   identities, and fully authenticates every stored encrypted Rivian credential,
   S3 secret, external-connection secret, and OIDC client secret. It then records
   only the public AGE recipient as `system_config.age_recipient` and removes
   only `jwt_private_key`, `jwt_public_key`, and `age_key`. Ciphertext bytes,
   telemetry/history, and unrelated settings are unchanged. Any error rolls
   back the database changes. Retrying after success is safe.
6. Provision the exported bundle externally and start the hardened image.
   Confirm startup reports `cryptographic_key_source=external`, repeat a restart,
   and test an isolated restore using the same bundle. Preserve the pre-migration
   backup under its stricter legacy-secret access policy.

Merely supplying matching external keys does not delete DB keys during normal
startup: it fails and directs the operator here. Migration takes database table
locks and may wait for other writers; it belongs in a maintenance window with
all application processes stopped. Locks cannot stop an old binary from storing
keys again after migration if that binary is restarted.

## Restarts, restores, and rotation

Every startup compares the AGE public recipient with the database binding. A
different AGE identity fails even when there are currently no credentials. On
an older externally keyed database with no binding, startup authenticates all
supported persisted ciphertexts before recording the public recipient. It never
substitutes a newly generated key.

Restoring a raw pre-migration dump restores its private key rows too: export or
recover the matching originals and repeat this migration before production
startup. Post-migration raw dumps need the independently backed-up AGE key.
Sanitized recovery packages exclude both the key rows and credential ciphertexts;
restoring one requires reconnecting providers. Keep old AGE keys for protected
raw backups and Redis state that still require them.

Logical key-row deletion is not secure erasure of PostgreSQL pages, WAL, database
snapshots, or older raw dumps. Those artifacts can still hold the original keys
beside ciphertext; protect them for their full retention period. This procedure
does not delete history, backups, or database files.

AGE rotation/re-encryption is not implemented by the custody utility. Do not
delete or edit the public binding as a rotation shortcut. Recover the original
identity if replacement was accidental; a deliberate rotation needs a separate
reviewed migration covering every encrypted store and retained backup.
RSA rotation after custody migration accepts a new complete matching pair but
invalidates old access-token signatures; plan session effects separately.
Runtime or host compromise can still read in-memory keys. External custody
reduces database-only compromise risk; it is not a security certification.
