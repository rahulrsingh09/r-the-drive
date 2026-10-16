---
title: Documentation overview
description: Choose the R:The Drive documentation path for installing, using, operating, or developing the project.
slug: /
sidebar_label: Overview
pagination_prev: null
pagination_next: guides/features
---

# R:The Drive documentation

R:The Drive's repository documentation is the source of truth for the application, its operation, and its implementation. Build the documentation locally from `docs/` with `pnpm docs:build`.

## Choose your path

| If you want to... | Start here |
|---|---|
| Understand the project, privacy model, security posture, or roadmap | **Overview** — continue through this section |
| Prepare, install, connect, and verify a new installation | [Getting Started](./guides/README.md) |
| Customize dashboards or control optional external services | [Using R:The Drive](./using-panda.md) |
| Update, secure, back up, recover, or troubleshoot an installation | [Operations](./operations.md) |
| Understand architecture or contribute code and documentation | [Development](./development.md) |
| Look up APIs, metrics, or dashboard data contracts | [Reference](./reference.md) |

## Common tasks

| Need | Start here |
|---|---|
| See what R:The Drive currently supports | [Features](./guides/features.md) |
| Understand privacy and external requests | [Privacy](./privacy.md) |
| Review deployment security | [Security](./security.md) |
| Customize an installed dashboard | [Using R:The Drive](./using-panda.md) |
| Choose or create an account theme | [Appearance and custom themes](./guides/themes.md) |
| Update or recover an installation | [Operations](./operations.md) |
| Understand the repository structure | [Architecture overview](./architecture/overview.md) |
| Build or change dashboards | [Dashboard architecture](./frontend/dashboard-architecture.md) |
| Understand dashboard query planning and data publication | [Dashboard data plan](./architecture/dashboard-data-plan.md) |
| Change dashboard editing and visibility behavior | [Dashboard session state](./architecture/dashboard-session-state.md) |
| Maintain dashboard import, layout, and expression validation | [Dashboard configuration](./architecture/rewrite-dashboard-configuration.md) |
| Trace requests, session renewal, and response validation | [API client](./architecture/rewrite-api-client.md) |
| Maintain recovery-file validation and safe extraction | [Recovery validation](./architecture/rewrite-recovery-validation.md) |
| Understand trip lifecycle and closure-motion decisions | [Trip domain](./architecture/rewrite-trip-domain.md) |
| Maintain charge reconciliation, identity, and repair | [Charge history](./architecture/rewrite-charge-history.md) |
| Trace route, location, and weather enrichment | [History enrichment](./architecture/rewrite-history-enrichment.md) |
| Maintain provider polling, connections, and shutdown | [Provider orchestration](./architecture/rewrite-provider-orchestration.md) |
| Follow telemetry persistence, checkpoints, and restart limits | [Durable collection](./architecture/rewrite-durable-collector.md) |
| Understand charge restart recovery and completion retries | [Durable charge capture](./architecture/durable-charge-capture.md) |
| Maintain backup admission, compatibility, and restore journals | [Recovery operations](./architecture/rewrite-recovery-operations.md) |
| Understand charging price selection and time-of-use calculations | [Charging costs](./architecture/rewrite-cost-engine.md) |
| Follow companion telemetry decoding, persistence and forwarding | [Companion telemetry](./architecture/rewrite-parallax-core.md) |
| Maintain chart validation and catalog reconciliation | [Chart contracts](./architecture/rewrite-chart-contracts.md) |
| Maintain trip history, detail projections and tag assignment | [Trip query workflows](./architecture/rewrite-trip-queries.md) |
| Follow charging summaries, curves and annotation updates | [Charging views](./architecture/rewrite-charging-views.md) |
| Maintain metric query planning and streamed results | [Metric queries](./architecture/rewrite-metrics-queries.md) |
| Trace battery, efficiency and parked-energy evidence | [Energy views](./architecture/rewrite-energy-views.md) |
| Maintain dashboard, chart, theme and preference workflows | [Personalization](./architecture/rewrite-personalization.md) |
| Follow places, pricing profiles and geocoder scheduling | [Places and profiles](./architecture/rewrite-places-profiles.md) |
| Maintain telemetry merging, storage and vehicle-state history | [Telemetry workflows](./architecture/telemetry-workflows.md) |
| Follow vehicle enrollment, invitations and artwork selection | [Vehicle workflows](./architecture/rewrite-vehicle-workflows.md) |
| Maintain external connection settings, cache and activity records | [External connections](./architecture/rewrite-external-integrations.md) |
| Follow Grafana queries, health, stewardship and capture summaries | [Operational read models](./architecture/rewrite-operational-read-models.md) |
| Follow charging payload identity and summary reconciliation | [Charge payload reconciliation](./architecture/rewrite-charge-payload-reconciliation.md) |
| Trace frontend request, map, and render failures | [Frontend error observability](./architecture/frontend-error-observability.md) |
| Build or manage charts | [Chart architecture](./frontend/chart-architecture.md) |
| Extend app themes or color authoring | [Theme architecture](./architecture/theming.md) |
| Verify chart migration and editor preview compatibility | [Chart compatibility testing](./frontend/chart-compatibility-testing.md) |
| Look up APIs, metrics, or dashboard data | [Reference](./reference.md) |

## Canonical entry points

- `README.md` introduces the project and provides the shortest installation summary.
- `AGENTS.md` defines repository policy for Codex and other repo-aware agents.
- `CLAUDE.md` provides companion command and execution guidance.
- `docs/branding.md` defines the shared visual system.
- This documentation site renders the complete `docs/` tree for easy navigation and review.

If behavior, structure, operations, or shared visual language changes, update its canonical document in the same pull request or create explicit tracked documentation debt.
