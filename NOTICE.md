# Copyright, attribution and modification notice

R:The Drive is distributed under **GNU GPL version 3 only**
(`GPL-3.0-only`). The complete license is in [LICENSE](./LICENSE).

The application includes implementation by **bballdavis and contributors**,
together with the R interface, private-deployment security and other work
maintained by this project's contributors. The product name does not replace
the authorship of retained implementation.

This version was modified on **2026-10-16**. Changes include R:The Drive branding,
the R interface, security and deployment integrations, a guided installation
script, documentation, removal of an unused layout and publication cleanup.
Further changes replace dashboard data, editing and configuration internals,
the API request pipeline, telemetry processing helpers and recovery validation.
They also correct session, charge-timestamp and recovery-file handling defects.
Additional replacements cover trip lifecycle, charge-history reconciliation,
history enrichment and provider connection orchestration, with corrections for
connection cancellation, diagnostic text handling and OTA response decoding.
Telemetry collection stages and charge restart reconstruction are also updated,
along with consistent geospatial database connection settings.
Backup admission, compatibility checks and restore journals use bounded
validation, atomic journal publication and transactional catalog updates,
with cancellation cleanup corrections. Charging price selection and interval
pricing are also replaced while preserving their characterized outputs.
Companion telemetry processing, chart validation and catalog reconciliation
have new internal stages. Browser schema initialization avoids dynamic code
under the existing Content Security Policy.
Charge detector state and completion records now commit with trip checkpoints,
with durable retries and session pins during history maintenance.
Trip, charging, metric and energy views use new query and projection workflows.
Dashboard, chart, theme, settings, place and pricing-profile operations use
new mutation and reconciliation stages behind the retained public interfaces.
Telemetry projection, duplicate suppression, storage and state-history rotation
use typed policies. Vehicle enrollment, invitation and artwork selection, plus
external connection settings, cache traversal and activity recording, use new stages.
Grafana queries and operational telemetry summaries use new selection and aggregation.
Charging payload resolution and summary persistence use new transactional plans.
Backup publication, import, scheduling and S3 transfer operations use new stages.
Maintenance uses bounded scans and new repair selection; synthetic history uses
typed timeline and telemetry generation.
Restore activation, rollback and operational-history replay use explicit states.
Weather scheduling, history reconciliation and charge backfill admission use
new job and transition stages.
See [BOOTSTRAP.md](./BOOTSTRAP.md) and
[BOOTSTRAP_VALIDATION.md](./BOOTSTRAP_VALIDATION.md) for source and validation details.

Existing copyright notices and dependency licenses remain applicable.
Third-party dependencies and assets retain their respective notices.
