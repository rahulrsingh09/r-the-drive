---
title: Charging cost engine
description: Cost-source precedence, profile selection, interval allocation, and persistence contracts.
---

# Charging cost engine

The cost engine computes completed-session amounts from session corrections,
network preferences, provider evidence, and user rate cards. The implementation
is contained in `apps/api/src/services/cost.rs` and
`apps/api/src/models/cost_profile.rs`. Public signatures, profile serialization,
database schema, route authorization, and frontend behavior remain unchanged.

Documentation impact: internal architecture update.

## Calculation stages

`CostBasis` selects a source before performing arithmetic:

| Priority | Condition                                                            | Method                         | Profile retained |
| -------- | -------------------------------------------------------------------- | ------------------------------ | ---------------- |
| 1        | Session override is free                                             | `user_free`                    | No               |
| 2        | Session override is manual                                           | `user_manual`                  | No               |
| 3        | Network preference is free                                           | `network_free`                 | No               |
| 4        | Provider marks session free                                          | `rivian_free_session`          | No               |
| 5        | Explicit Rivian network flag, not home, and a provider amount exists | `rivian_paid_total`            | Yes, if resolved |
| 6        | An applicable profile exists                                         | `profile` or `profile_pending` | Yes              |
| 7        | No source applies                                                    | `unknown`                      | No               |

A zero provider amount is present evidence. Home sessions cannot select paid
provider totals, even when their network flag is true. A null manual amount is
representable by the pure computation function; the unchanged database constraint
rejects storing that manual override.

`BillableQuantity` separates energy, minutes, a flat session, free charging, and
a scheduled amount. Energy billing uses the greater of energy added and energy
used, substituting zero for missing totals. A nonpositive selected total cannot
produce an energy-based amount. The existing floating-point behavior, including
nonfinite inputs and signed zero, is preserved. Fees are added once, except that
a free profile always produces zero.

## Time-of-use allocation

`DailySchedule` holds the parsed timezone and periods. Measured readings use their
UTC timestamp converted to a local minute, then select the first matching period
and sum `kWh × rate` in input order. Unmatched minutes have rate zero. This helper
does not sort, clip, rescale, or validate the readings, and its caller owns
effective-date selection.

When no readings are available, `SessionWindow` enumerates local calendar days,
resolves each tariff interval, intersects it with the session, and computes a
duration-weighted rate. It multiplies that rate by the selected session energy.
Overlap durations use whole seconds converted to minutes. Missing end times use
the nonnegative duration; an explicit end takes precedence.

Civil-time boundaries retain the existing DST behavior: ambiguous boundaries
select the earliest occurrence, nonexistent boundaries skip the affected interval,
and minute 1440 denotes the next midnight. Period order and arithmetic order are
preserved. Invalid timezones, empty schedules, nonpositive energy, or a window
without overlap can leave a profile pending. A present but unusable measured
schedule does not automatically retry duration weighting.

## Profile and evidence lookup

`ProfileSource` visits a pinned profile, the linked geofence, and the vehicle
owner's default in that order. Each candidate must exist and be effective at the
session start. Effective dates are inclusive in the profile timezone; an invalid
timezone falls back to UTC for this date comparison.

Lookups remain lazy: selecting a valid earlier candidate avoids later queries.
The owner is the earliest owner membership selected by the existing vehicle
helper. Viewer preferences do not replace that owner's default. Explicit profile
and geofence IDs are trusted inputs to this internal seam; authorization remains
the responsibility of its existing callers.

The service converts stored Wh totals to kWh before pricing. An available
`energy_added_wh` takes precedence over `kwh_added`, including when its value is
zero. Stored duration takes precedence over a derived duration.

For closed sessions, energy evidence first uses positive SOC deltas from the
one-minute telemetry aggregate, with the maximum observed battery capacity in
the window. If that query returns no positive readings, stored charge-curve power
samples supply `kW / 60`. Curve samples retain their existing session-ID scope.
An evidence-query error is logged and falls back to duration weighting; open
sessions do not query interval evidence. These stages make no external requests.

## Persistence

`persist_cost_result` retains parameterized, change-guarded writes to the
canonical charge session followed by the owner's annotation. Existing geofence
and home annotations survive null incoming values. Viewer annotations remain
untouched. Owner annotations continue to write `USD`; profile currency and the
canonical session currency are not converted or rewritten.

The two writes remain sequential and are not an atomic transaction. If the
annotation write fails after the canonical update, a retry repairs the annotation.
Unchanged results do not update annotation clocks. History privileges, constraints,
and migrations are not relaxed by the engine.

## Compatibility evidence

`cost_contracts.rs` compares complete outputs against frozen baseline JSON:

- 913 pure cases cover precedence combinations, profile serialization, null and
  nonfinite numbers, float bits, energy and fee choices, durations, effective
  dates, malformed schedules, exact validation errors, and DST intervals.
- 35 database cases cover complete resolved profiles, full persisted session and
  annotation rows, owner selection, telemetry and curve precedence, schema errors,
  evidence-query failure, annotation failure/retry, and protected history writes.

Database tests run migrations and the production history policy in a unique
disposable TimescaleDB database under a restricted application role. They compare
repeated persisted rows with their actual timestamps to prove idempotence.
Snapshot files replace only database-generated clocks with a marker; validation
artifacts also retain the full clock values. All data is synthetic.

Run focused checks from `apps/api`:

```sh
cargo test --locked --offline --lib cost
cargo test --locked --offline --lib cost -- --include-ignored --test-threads=1
cargo check --locked --offline --all-targets --all-features
```

The second command requires the dedicated `COST_CONTRACT_DATABASE_URL` fixture.
It deliberately accepts only the `cost_fixture` bootstrap account and database
at the `db` service in the isolated cost-engine Compose network. The normal test
command reports this database test as ignored; that is not database validation.

This replacement makes no claim of clean-room implementation or exclusive
authorship. GPL-3.0-only licensing and the repository's attribution notices remain
in effect.
