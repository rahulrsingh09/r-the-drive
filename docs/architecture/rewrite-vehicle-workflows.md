# Vehicle workflow stages

Documentation impact: Documentation site update required.

Vehicle HTTP routes retain their paths, extractors, wire structs, error mapping,
private authorization calls, and encrypted session adapters. Private modules under
`routes/vehicles` coordinate enrollment, sign-in delivery, defaults, membership
acceptance, and artwork selection.

## Enrollment and connection delivery

`ConnectionExchange` distinguishes a pending OTP challenge from an authenticated
provider session. `DiscoveredSession` must finish vehicle discovery before it can
publish the encrypted connect session. OTP completion publishes that session before
consuming its challenge. Challenge ownership, provider request policy, five-minute
Redis TTLs, encryption, and HTTP error mapping retain their existing adapters.

`VerifiedEnrollment` carries the selected provider vehicle and requested local
settings. Persistence acquires the existing active-user, remote-ID, and membership
locks before choosing an existing vehicle or creating one. Both paths apply local
user settings; only creation stores credentials and initializes runtime state.
`DefaultSelection` applies the existing conditional-first-default or explicit-switch
rules within the caller's transaction. `SavedEnrollment` exists only after commit
and determines the supervisor notification and response. Artwork queueing and Redis
cleanup remain after commit, so cleanup failure cannot undo a saved vehicle.
Credential refresh uses the same runtime initializer, retaining its own membership
locks, credential encryption, transaction, and readiness wait.

## Membership transitions

Member addition resolves a validated email into a registered user or an invitation.
A `MembershipGrant` writes the role and ensures user settings. Direct member addition
retains two separate writes: a settings failure can leave the membership committed;
retry fills the missing settings. Invitation acceptance instead runs these writes
inside its existing transaction.

`LockedInvitation` reads the invitation and accepting user under the original
`FOR UPDATE` lock. It rejects revoked, accepted, expired, and mismatched-email states
in that order before writing membership, settings, and acceptance. Repeated or
concurrent acceptance retains its existing rejection. Invitation issuance shares
one token/hash/expiry persistence stage while retaining each endpoint's validation
order, the fourteen-day member-add expiry, and explicit invite expiry clamping.

## Artwork selection

`ArtworkSelection` indexes the first row for each normalized placement and design
in one traversal. Preferred-design lookup falls back to the first placement row;
URL resolution happens after that choice. An unmirrored first candidate therefore
still produces null even if a later row has a local URL. The `all` array retains
query order and includes only rows with a local mirror key.

Overlay projection indexes the first mirror for each exact source URL. A later
valid duplicate cannot replace an invalid first match. `CacheInventory` counts
remote base images and stored overlay mirrors, retaining checksum checks and the
ready/failed/pending precedence. Asset delivery, immutable/private cache headers,
corruption responses, and the prohibition on optional outbound traffic remain
unchanged.

## Retained boundaries and verification

Simple settings/name/member-list/update/removal/revocation handlers, scalar
canonicalization, vehicle-list/status/raw-telemetry projections, and read-only
schedule adapters remain compatible implementations. Schedule mutations still
return 403 after authentication, including malformed and oversized bodies. This
change does not claim to replace these retained adapters, the provider client,
crypto, private security, demo generator, durable charge capture, or recovery
libraries. GPL-3.0-only and existing NOTICE attribution are unchanged.

`vehicles/__tests__` contains the separately committed original-implementation
characterization. Seven restricted-role database tests compare full HTTP status,
content type, cache headers, JSON/text bodies, and complete relevant relational
rows. Fixed fixtures exercise role isolation, nulls and units, body limits,
ordered invitation errors, repeat operations, concurrent enrollment/acceptance,
rollback, partial failure/retry, local corruption, and pending-login cancellation.
Generated IDs/token derivatives and generated clocks are explicitly normalized;
full raw evidence remains in the external review directory. Credential ciphertext
has separate complete-row replay checks and decrypted contract assertions.

The fixture creates new disposable databases, installs extensions with a bootstrap
account, applies current migrations including 0031 as a restricted application
role, and enables the unchanged production history policy. No history-delete or
truncate grant is added. Provider responses are scripted on loopback through the
existing test gateway policy. Real accounts, cloud resources, release behavior,
and performance at production scale are outside this verification.
