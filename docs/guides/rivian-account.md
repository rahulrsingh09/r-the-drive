---
title: Rivian account setup
description: Connect a Rivian account and handle MFA or authentication repair.
slug: /getting-started/rivian-account/
---

# Rivian account setup

After you create your Panda owner account, open **Settings → Vehicles** and choose **Add Vehicle**.

1. Enter the email address and password for your Rivian account.
2. Complete the one-time passcode (OTP) Rivian sends through its normal authentication flow.
3. Panda encrypts the credentials at rest, saves the vehicle, and starts its
   telemetry collector. The completion screen distinguishes that durable save
   from telemetry readiness.

If Rivian rejects the email, password, or verification code, Panda shows that specific correction beside the form. If the secure sign-in handoff expires before the vehicle is added, start again from the account step.

The first update can take a little while, especially if the vehicle is asleep.
You can open the dashboard while the collector starts. If startup reports that
it needs attention, use the vehicle health details and app logs; successfully
saving a vehicle does not require a container restart. Panda uses Rivian's
unofficial API and WebSocket behavior, so upstream changes can occasionally
require a project update.

## Telemetry-only vehicle access

This fork denies charging changes and departure/preconditioning schedule changes
in the backend GraphQL transport, including calls made outside the UI. Schedule
write routes return HTTP 403. Existing schedules, vehicle state, charging history,
and local trip data remain readable. Use the Rivian app to change the vehicle.
Login, OTP verification, vehicle discovery, and CSRF/session renewal remain enabled.

**Rivian tokens are not provider-enforced read-only credentials.** This is an
application restriction, not a limitation on a stolen token or a compromised
server. MFA does not revoke an already authenticated session.

Credential-bearing HTTP requests use only
`https://rivian.com/api/gql/gateway/graphql` and
`https://rivian.com/api/gql/chrg/user/graphql`. The
`RIVIAN_GRAPHQL_GATEWAY_URL` environment variable may be unset or exactly the
first URL; every other value fails closed. WebSocket session headers go only to
`wss://api.rivian.com/gql-consumer-subscriptions/graphql`. HTTP and WebSocket
redirects are rejected. HTTP clients cannot inherit a caller's proxy or redirect
configuration. Rivian HTTP responses have an 8 MiB size limit and 30 second timeout.
Unit tests can scope a literal-loopback HTTP mock; this capability is absent from
production binaries and cannot be enabled with an environment setting.

Optional location and media downloads are disabled in this fork, including
previously saved custom providers. See [external connections](external-connections.md).

## Estimated connection renewal

Panda records the time of each complete Rivian sign-in and shows an
estimated renewal date 180 days later. Starting seven days before that date,
the sidebar and **Settings → Vehicles** recommend refreshing the Rivian login.
The estimate is based on current behavior documented by unofficial Rivian
clients; Rivian does not publish a supported token-lifetime contract.

This reminder is preventive, not a statement that the connection has already
expired. A password change, account unlink, or Rivian-side revocation can still
require an earlier sign-in. Panda automatically renews the shorter-lived
CSRF and application-session pair without moving the 180-day estimate, but a
complete Rivian account renewal remains a user-assisted flow.

Choose **Refresh Rivian login** on the vehicle card. Completion waits for the
new credentials to be verified and for vehicle discovery to succeed. The
sidebar, vehicle list, health state, and live telemetry connection then refresh
without a browser reload. If credentials are saved but telemetry is still
starting, Panda keeps the new credentials and shows a recoverable waiting
state.

## Troubleshooting

- Watch the app logs with `docker compose --env-file .env -f compose/docker-compose.yml logs -f panda`.
- If authentication expires or is revoked, use **Refresh Rivian login** on the existing vehicle. Removing the vehicle is not required.
- If an OTP does not arrive, confirm the phone number on the Rivian account and retry from Settings.
- If the app reports temporary secure-session storage is unavailable, do not keep retrying Rivian credentials. Confirm the Panda container is healthy, then inspect its logs for `secure_session_store.unavailable`; this means the server cannot authenticate to or reach its Redis session store.

Rivian requests carry the information necessary to authenticate and collect vehicle data. See [privacy](../privacy.md) for the wider data-flow picture.
