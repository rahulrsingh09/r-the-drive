# Repository bootstrap

The repository began from mainline snapshot `f9482c7dc207adbdb2b43c3f247023f2e20323d6`. The application is named **R:The Drive**, with repository name `r-the-drive`. Its sanitized export uses fresh Git history and neutral author metadata; local credentials, vehicle data, backups and generated output are excluded.

Subsequent implementation work replaces application workflows, rendering
controllers, collection and query services, security execution, recovery, and
repository tooling while preserving their public contracts. The maintained R
appearance, chart output, saved records, and security guarantees remain the
compatibility targets. Dashboard layout editing is removed from the interface;
existing layouts remain available for viewing and export. Fixed schema and
protocol definitions, presentation markup, standard primitives, assets, and
required attribution are retained explicitly.

## Isolation and data compatibility

Use a fresh database, Redis namespace, signing keys, encryption identity and gateway credentials. The SQL schema, migration-chain identity, package names, log targets, API-key prefix and configuration variables now use the panda namespace. Existing databases and backups require an explicitly designed migration; do not run this build against an existing installation. The new migration catalog is pinned in `config/native-release-catalog.json`.

## Security controls

- Authentication, secure cookies, refresh-token families, password hashing and rate limits.
- Vehicle authorization, role checks and read-only vehicle connectivity.
- External encryption-key custody and encrypted vehicle credentials.
- Cloudflare identity verification, private origin binding and nginx security headers.
- Trip checkpoints, history protection, backup vault and restore verification.
- Route-security inventory, dependency policy and audits, secret scanning, SAST and image scanning.
- Release admission requiring an exact tested commit, current backup and data-retention checks.

## Repository controls

`your-org/r-the-drive` is a deployment-template placeholder. Repository and workflow IDs remain zero until release automation is configured and verified. Account-specific repository ownership is kept in Git remote configuration rather than checked-in source. `mainline` is the local default branch. CODEOWNERS lists the security boundaries but requires a real owning team before enforcement. No deployment secrets are copied.

The requested strict checks, administrator enforcement, and force-push/deletion restrictions are saved in `.github/bootstrap-protection.json`. These are templates, not enforced hosted settings. Local Gitleaks scanning and the retained CI scanner configuration do not establish hosted secret-scanning availability.

Keep Actions disabled until the account's free allocation or spending limit is confirmed. Configure repository identities, run `fork-ci.yml` and `security.yml`, and apply the protection policy when supported. Do not treat checked-in workflow files as evidence of a successful hosted run.

Release, publishing and upstream automation additionally require the `PANDA_ENABLE_RELEASE_AUTOMATION` repository variable. Their templates require new deployment identities and protected environments. The `.invalid` domains are deliberate placeholders, including the backup receiver. Native deployment adapters still require private-repository metadata authentication and admission-policy changes before activation.

Run `pnpm bootstrap:check`, `pnpm verify`, `pnpm build`, the backend security integration suites and the browser suite before enabling deployment.

## Validation

See [BOOTSTRAP_VALIDATION.md](./BOOTSTRAP_VALIDATION.md) for the completed local checks and their limits.

## Licensing and provenance

The GPL-3.0-only license is preserved in LICENSE. Incorporated source includes work by GitHub user bballdavis and contributors, with private deployment and R interface work maintained by the private-fork maintainers. Subsequent implementation replacements do not erase that provenance or establish independent authorship of retained material.

See [NOTICE.md](./NOTICE.md) for attribution and the dated modification notice.
