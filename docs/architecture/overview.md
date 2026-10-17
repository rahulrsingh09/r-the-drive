# Architecture Overview

## Audience

Engineering contributors making structural changes across the monorepo.

## Source Of Truth

This document is canonical for the repo-level architecture map. Update it when ownership, package boundaries, or major seams change.

## Repo Shape

```text
apps/
  api/          Rust API, ingestion, auth, database access, operational routes
  web/          React SPA, routes, layout, page composition
  docs/         Docusaurus site built directly from the canonical docs tree
packages/
  dashboards/   dashboard schema, defaults, renderer, widget registry
  hooks/        API client, auth store, React Query hooks
  types/        shared TypeScript contracts
  ui/           design system primitives, charts, tables, tokens
  config/       shared TS, ESLint, Tailwind config
compose/        production image, Compose topology, nginx, and development infrastructure
docs/           canonical documentation published through Docusaurus
scripts/        dev/build/docs utilities
```

## Structural Seams

- `apps/api` owns runtime truth, persistence, ingestion, and HTTP contracts.
- `packages/types` owns shared TypeScript API shapes consumed by the web app and packages.
- `packages/hooks` is the frontend data-access seam. Route and component code should not duplicate transport logic.
- `packages/dashboards` owns dashboard rendering and widget registration, not route-level page concerns.
- `packages/ui` owns shared visual primitives and tokens.
- `apps/web` owns route composition, page-specific UX, and integration of shared packages.
- `compose/Dockerfile` owns the unified production image containing the API, built SPA, nginx, backup tools, and the local restore supervisor. Extended Parallax acquisition runs as an integrated, isolated Tokio subsystem inside each vehicle worker; there is no second image or container. During an in-app restore nginx and the supervisor remain available while the API/ingestion process is replaced; development keeps infrastructure in `compose/docker-compose.dev.yml` while `scripts/dev.mjs` runs the API and restore supervisor as managed host processes.
- `compose/build-api.sh` supplies the dependency-cache and application-build phases inside that image build. Both phases use the same locked Cargo profile, binary list, job limit and ARM64 cross-compilation settings; only the application phase copies executable artifacts into the image staging directory.
- Migration `0032_stored_function_implementations.sql` replaces recursive charge-payload assembly with ordered JSON aggregation and separates legacy theme projection from explicit selection changes. Persisted fingerprint bytes, theme transitions and trigger permissions retain their existing contracts. Earlier migration files remain immutable; the effective catalog appends version `1000032`.
- The container `TZ` setting is limited to runtime/container behavior. Panda stores a separate global IANA application timezone in `system_config`; it drives user-facing date formatting, local-day grouping, and backup scheduling.
- Runtime logs use the common `[panda][LEVEL]` key-value prefix. Docker supplies the outer timestamp; nginx emits only failed edge requests so successful health, static, and proxied traffic is not duplicated.

`pnpm test:sql-contracts` compares the stored functions against their historical
implementations using synthetic data in a disposable PostgreSQL container with
no network access. It checks nested payloads and fingerprints, plus 864 theme
transitions including nullable fields and explicit selection/version changes.
The pinned TimescaleDB image must already be available locally; this test never
pulls an image or connects to an existing database.

## Change Triggers

Update this doc when:

- a new package/app is added
- a responsibility moves between packages
- a shared seam changes ownership
- a new canonical contributor entrypoint is introduced

## Adjacent Docs

- [`backend-data-flow.md`](./backend-data-flow.md)
- [`../frontend/dashboard-architecture.md`](../frontend/dashboard-architecture.md)
- [`../branding.md`](../branding.md)
- [`../contributing.md`](../contributing.md)
