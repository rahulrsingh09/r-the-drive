# Authentication route workflow replacement

Documentation impact: internal architecture update required.

This change replaces the application workflows in `apps/api/src/routes/auth.rs`
against baseline `5651fdfeed91924af623cf7eb919c417e33628bb`. Existing licensing,
attribution, wire contracts and original tests remain. This is an implementation
replacement within an inherited codebase, not a clean-room authorship claim.

The three public router constructors retain their paths, methods, state type and
middleware integration. Their route registries now express the mapping as data.
No endpoint, dependency, schema, public signature or configuration flag is added.

## Workflow boundaries

| Boundary | Implementation and preserved guarantees |
| --- | --- |
| Owner enrollment | Validation precedes Argon2; the users table lock protects the empty-installation check, production setup proof, owner insert and audit. Account creation commits before auto-login. |
| Password login | Typed account data drives credential and account-policy decisions. Missing accounts still verify the vetted dummy Argon2 hash. Session issuance and success audit share one transaction. |
| Session creation | A materialized eligible-account query holds `FOR SHARE`; a returning insert yields the persisted family ID. Disabled or deleted accounts cannot receive a new session. |
| Cookie rotation | A shared family-lock operation precedes the token row lock. A consumed token revokes the family and commits before best-effort audit. Valid rotation consumes the parent and inserts the child in one SQL statement. |
| Password mutation | Explicit replace/initialize modes share the locked credential update, session revocation and audit transaction. Replacing requires the current password; initializing requires the previously verified OIDC operation. |
| Invitation activation | Password and OIDC validation remain distinct. A common activation operation creates the account and preferences, assigns viewer-only access with set-based inserts, attaches an OIDC identity when applicable, and records acceptance atomically. |
| OIDC resolution | A decision enum distinguishes touching an existing mapping, attaching an account, and provisioning. The advisory lock, account/identity row locks, invitation exclusion and active-owner lock remain inside the transaction. |
| OIDC callback | Redis consumption, browser/provider rejection and verified completion are separate phases. `GETDEL` occurs before binding checks; provider exchange and verification remain in the existing vetted OIDC service. |
| Responses | Shared cookie/session response assembly preserves status codes, payload fields, lifetime, path, SameSite, HttpOnly, Secure configuration and both OIDC completion cookies. |
| Preferences | Unit presets and validation use a field matrix. Storage projects the validated unit payload into the existing columns; theme selection and favorites retain their existing conflict-update rules. |

The first matching cookie in the first valid Cookie header remains authoritative.
Access tokens retain a 900-second advertised lifetime, refresh tokens retain a
30-day lifetime and 48-character alphanumeric form, and refresh hashes remain
SHA-256. OIDC state remains single-use with a 600-second Redis lifetime.

This file does not implement email-change delivery, email-verification delivery,
or forgotten-password reset endpoints. Verified email and initial-password setup
are part of its OIDC orchestration. It does not add a CSRF header protocol: the
existing SameSite cookie and state/browser binding rules remain, along with the
central router's CORS and bearer-authentication policies.

The preference update retains its historical ordering: validate theme before
units, persist units before theme in separate statements, then reread. A theme
palette change selects a built-in theme; retaining the palette preserves the
custom selection. Invitation vehicle lists prefer the join table over the legacy
single-vehicle field. Invitation denial precedence stays revoked, accepted,
then expired. Password lengths and favorite field limits remain UTF-8 byte limits.

## Explicit minimal retained units

These eight small bodies remain and require reviewer inspection rather than a
claim that all source text is newly authored:

- `argon2_hash`, `verify_password`, and `sha2_hash`: vetted primitive adapters;
  retain salt generation, default Argon2 parameters, digest and error contracts.
- `password_meets_minimum`: the fixed twelve-byte minimum comparison.
- `oidc_public_start_available`: the fixed enabled-owner boolean contract.
- `is_valid_map_style`: the exact seven-value public preference union.
- `default_theme_preferences`: the fixed dark/classic default payload.
- `issue_refresh_token`: the single-value compatibility projection from the
  session-creation function's existing generic executor interface.

The fixed route maps, SQL schema names, status/error strings, serialized field
names, cookie attributes and security audit event names also remain contractual.
The other 41 original production functions are traced in the review inventory to
the replaced workflows, including all private helpers and metadata routes.

## Validation

Original inline auth tests and `tests/auth_integration.rs` are preserved verbatim.
The separate `auth_full_replacement_tests.rs` module adds byte-level cookie and
preference characterization, malformed callback checks, account-policy denials,
and races for owner creation, password replacement, password/OIDC invitation
acceptance and refresh replay. The same assertions characterize the original
production implementation before replacement. An additional loopback HTTP test
checks cookie clearing, disallowed CORS origins, malformed bearer access and
the canonical rate-limit response header on the candidate. Candidate database
checks also cover concurrent OIDC provisioning and favorite merges, map-style
validation, and rejection of conflicting or disabled OIDC identities.

Tests use task-owned, fully migrated TimescaleDB and Redis fixtures on an
internal Docker network with no published ports. Compilation uses the existing
Rust tooling image, locked offline dependencies, two jobs and a private target
cache. The accompanying review report records actual commands, binary/source
identities, results and fixture cleanup; this page does not substitute for that
evidence.
