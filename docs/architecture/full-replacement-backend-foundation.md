---
title: Backend foundation replacement
---

The backend foundation keeps the existing application interfaces while changing
how configuration, startup, public errors, model decoding, and geofence lookup
execute. It uses the existing dependencies and database contracts.

## Configuration and startup

Configuration loading resolves key inputs as missing, inline, or file-backed
material before constructing the validated configuration. OIDC overrides use a
captured set of named environment inputs and separate text, boolean, and
credential decoding. Setup proofs pass through a validated token type; comparison
uses the existing AWS-LC constant-time primitive over fixed-size SHA-256 digests.

Origin validation distinguishes HTTPS origins from the explicit literal-IP LAN
policy. Address classification is shared by client-origin and server-bind checks.
Existing validation order, error messages, setup-token rules, configuration
names, and public record fields remain part of the contract.

Startup owns two successive states: a migrated and history-protected database,
then initialized application services. Key initialization, reconciliation,
default seeding, Redis admission, capture recovery, acquisition workers, and
maintenance tasks retain their required order. No application service is served
before its startup dependencies succeed.

## Errors, logs, and models

Public errors have a typed envelope. Status and error-code selection are separate
from the ownership and sanitization of the public message. Internal dependency
errors remain private, and capacity responses retain their retry headers.
Log formatting assembles a complete record before writing it; record content,
levels, field formatting, and exclusion of enclosing span fields are characterized.

Telemetry constructors use the declared optional-field defaults. A vocabulary
catalog supplies canonical telemetry state names and accepted aliases, including
Unicode lowercase behavior. Vehicle-history states retain their stricter,
case-sensitive vocabulary. Cost-profile decoding borrows the stored JSON value,
and schedule validation checks each interval against the preceding endpoint.
Geofence lookup materializes the owner-scoped spatial candidates and computes
one distance projection before selecting the nearest eligible result.

## Compatibility and validation

Existing tests and golden fixtures remain unchanged. Additional characterization
covers full public error envelopes and headers, environment and key-presence
truth tables, recovery and rate-limit error precedence, address classification,
Redis credential encoding, complete empty telemetry shape, state aliases,
charging predicates, schedule boundaries, exact log frames, and text-decoding
errors. Geofence tests use both extension implementations and a restricted reader
without history-write privileges.

Fixed scalar defaults, configuration record associations, and small predicates
that directly express an unchanged public or security truth function remain
explicit contracts. Their individual disposition is recorded in the review
inventory; they are not represented as new algorithms. Existing schema and
third-party serialization contracts also remain. This work does not change the
project's license or make a clean-room authorship claim.
