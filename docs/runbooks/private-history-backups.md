# Protecting vehicle history and independent backups

Documentation impact: internal and user-facing documentation update.

This private fork preserves recorded trip, efficiency and normalized telemetry history.
Production startup installs `apps/api/src/private_deployment/history.sql` after
migrations and before ingestion. It revokes the app database role's DELETE and
TRUNCATE on vehicles, trips, telemetry, state periods, software versions and
battery capacity snapshots. Production vehicle deletion returns HTTP 409.
Development remains unchanged. Scheduled Timescale retention for telemetry,
trips, charge sessions and Rivian charge payloads is disabled. Diagnostic event
and database job logs keep their existing expiry; these are distinct from
durable driving and charging history.

Charge-session DELETE remains available for merging duplicates into a canonical
record; TRUNCATE is blocked. Recording and repair continue normally. A database
administrator still has the power to drop or rewrite data deliberately.

## Automatic copies and free storage bounds

When `HISTORY_BACKUP_TOKEN` is present, the existing backup scheduler creates a
full recovery package and the private module encrypts the entire package to
the original AGE X25519 identity before uploading it to a separate
`panda-backup-vault` Cloudflare Worker. The existing app gateway and
Cloudflare Access configuration are unchanged. Neither GitHub Actions nor
Cloudflare receives the private AGE key.

The receiver accepts POST `/backup` with a separate 64-character hexadecimal
bearer token, an age-format ciphertext and matching SHA-256 header. It exposes
no download, list or delete endpoint. Store the token as a Worker secret and
Northflank app secret, never in Git. Downloads require Cloudflare administrative
access. The app does not follow redirects. Do not log headers or archive contents.

There are 31 fixed daily slots, selected by the server's UTC date. A successful
upload replaces that day's slot in one write; no delete operation is needed.
Another backup on the same UTC day replaces that day's snapshot. An administrative
`baseline` key can be provisioned once; the Worker never writes it. Each object
is capped at 20 MiB. These 32 objects consume at most 640 MiB, below the 1 GB KV
free storage allowance. That allowance is account-wide: do not add unrelated
objects or assume other KV workloads have no impact.

Every snapshot contains the complete retained database history. Rotating snapshots
does **not** delete old trips from the live database or latest snapshot. With daily
backups, recovery can lose up to a day of newer records, or longer if backups fail.
This is not continuous replication or a zero-data-loss guarantee.

If encryption, size limits, quota, upload or receipt verification fails, the run
reports failure and keeps its cataloged local package. Local retention only runs
after successful remote publication. The existing scheduler does not retry a
failed daily run that day automatically: investigate and run a manual backup in
the owner's backup screen. The mirror forces a local copy even if local storage
was unchecked. Disabling scheduled backups also stops the mirror.

Do not enable paid storage or delete history to make an archive fit. If it
outgrows the cap, preserve existing copies and export to personal storage or
review another free option. The live database and local volume are also finite.

## Setup and recovery

1. Create a dedicated Workers KV namespace in the existing free account. Deploy
   `apps/private-gateway/src/backup-worker.mjs` with the `HISTORY_BACKUPS` binding,
   using `apps/private-gateway/wrangler.backup.example.json` as a template.
2. Generate a random 32-byte token encoded as 64 lowercase hexadecimal characters.
   Install `HISTORY_BACKUP_TOKEN` on the Worker and app. Preserve original Rivian,
   RSA, JWT and AGE keys. The receiver URL is pinned in the private Rust module;
   moving accounts requires a reviewed change.
3. Keep the original AGE key in a separate private recovery location. The encrypted
   Cloudflare copy is unusable without it. Never paste it into chat, GitHub or
   Cloudflare.
4. Verify a scheduled run succeeds. Download the ciphertext via the authenticated
   Cloudflare API, verify its metadata checksum, decrypt, and restore into an
   isolated database with no collector/network access. Compare migration identities
   and recorded trip/efficiency values.
5. Check the latest backup success and available space in the app periodically.
   No paid alerting or email/Slack notifications are configured.

Build `history_archive` from `apps/api` for an administrative baseline or recovery:

```sh
history_archive encrypt original-age-key.txt recovery.rma.tar.gz baseline.age
history_archive decrypt original-age-key.txt downloaded.age recovered.rma.tar.gz
```

Use private paths. The tool refuses to overwrite output and creates files with
mode 0600. A baseline encrypted from raw `pg_dump` decrypts to a dump rather than
a recovery package; record the input format. Never connect a test restore to Rivian.

See [native releases](native-northflank-release.md) for the separate fresh-backup,
data and original-key checks required before every production deployment.
