---
title: Demo history generation
---

The demo generator populates the existing sanitized fourteen-day fixture for R1T,
R1S and R2. R2S and R2-S remain accepted generator aliases; the vehicle schema
continues to store the canonical R2 model. This change is an internal generation
replacement with unchanged entrypoint signatures and summary serialization.

## Generation stages

`services/demo_seed.rs` retains the transaction and validation boundary. The
private `demo_seed/history_plan.rs` builds route/time slots across the fixed
active-day calendar, distributes weather samples using quotient and remainder,
and selects the repeating AC/DC/unknown charge schedule. Trip and charge row
adapters retain their schema bindings, fixture constants and scalar unit
calculations. Weather samples and DC curve points are transferred as bounded
arrays for each parent record.

`demo_seed/telemetry.rs` loads the vehicle's ordered trip and completed charge
windows once. A forward interval cursor associates each sample with the earliest
covering window, using inclusive endpoints. A typed sample generator evaluates
location interpolation, battery decline, range, sensor sparsity, model-specific
closures and charging state. A bounded database calendar query preserves session-timezone and DST day
boundaries. Timestamp spacing preserves PostgreSQL's microsecond rounding;
the public seed anchor still discards nanoseconds.
The generated rows are written in the existing daily batches through an explicit
column list and PostgreSQL's typed record decoder. Other telemetry columns keep
their database defaults. No worker ingestion implementation is used or changed.

State periods are rebuilt from ordered trip and charge boundaries. Each preceding
end time contributes a sleep interval when the gap exceeds forty-five minutes,
then a ready interval. Positive drive/charge spans are retained. Open-ended
anchors reset the preceding-end state. Inserts preserve chronological order and
the database-generated duration field.

## Retained boundaries

Fresh-demo vehicle locking, creator admission and locking, empty-history checks,
fixture validation order, the caller-owned transaction and production history
protection are unchanged. Repeating fresh seeding rejects populated vehicles.
Protected refresh continues to fail rather than deleting history. The route
owner retains HTTP authentication, memberships, body limits and production
refresh admission. There are no new outbound calls, provider credentials,
configuration options, migrations or permissions.

Address upserts, software-version constants, latest-status/runtime-status writes
and summary count verification remain simple adapters. Their presentation,
nullable model closures and message text remain part of the captured contract.
The shared security, durable collector, charge checkpoint and recovery libraries
are retained unchanged. GPL-3.0-only and existing attribution remain unchanged.

## Characterization and limits

`services/__tests__/demo_generation.rs` compares full database rows and complete
serialized errors against compressed frozen fixtures captured before replacement.
The matrix includes canonical models and aliases, empty and sparse telemetry,
validation ordering, stored IEEE-754 bits, DST boundaries, repeat denial,
restricted-history denial, stage failure with
rollback/retry, concurrent seeding and terminated-connection recovery. Generated UUIDs are
mapped consistently across foreign keys; transaction-generated clocks are
normalized only by exact equality to the captured transaction clock. Fixed fixture UUIDs, integer
identities, fixed timestamps, float values, nulls and all other fields are retained.
Raw evidence is kept outside the source tree by the local runner. Evidence output
is optional; all frozen-output assertions also run without an evidence directory.

The legacy refresh integration test assumes DELETE access and is incompatible
with the production history policy. The fixture executes it separately and
records its existing permission-denied failure. Fresh seeding is covered directly
under the restricted role. HTTP/browser authorization, real provider accounts,
production workloads and release deployment are outside this generator test.
