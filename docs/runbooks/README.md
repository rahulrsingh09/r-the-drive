---
title: Runbooks
description: Repeatable Panda maintenance, publishing, recovery, release, and repair procedures.
slug: /runbooks/
---

# Runbooks

- [Release database cutover](./release-database-cutover.md)

## Audience

Maintainers and agents handling recurring maintenance, publishing, or troubleshooting work.

## Source Of Truth

This directory is canonical for operational and process runbooks. Update it when a recurring task needs stable step-by-step guidance.

## Current Runbooks

- [`documentation-maintenance.md`](./documentation-maintenance.md)
  How to keep canonical docs structured, validate the Docusaurus site, and publish safely.
- [`vehicle-history-rebuild.md`](./vehicle-history-rebuild.md)
  How the one-off vehicle history rebuild works, what it replays, and how post-replay trip enrichment is restored.
- [`secure-deployment.md`](./secure-deployment.md)
  Required authenticated-gateway posture, production secret requirements, and verification steps for shared instances.
- [`key-custody.md`](./key-custody.md)
  External key provisioning, preserving legacy keys during migration, and recovery requirements.
- [`private-fork-maintenance.md`](./private-fork-maintenance.md)
  Reviewed upstream integration and credential-free CI readiness for a local deployment consumer.
- [`release-images.md`](./release-images.md)
  Calendar Version releases, public GHCR images, pre-release images from `dev`, and recovery steps.
- [`backup-restore.md`](./backup-restore.md)
  Recovery-package validation, clean-install restore, and incident recovery steps.
- [`dev-harness.md`](./dev-harness.md)
  Refresh a disposable production clone from a verified recovery package and test an immutable dev image.
- [`dependency-maintenance.md`](./dependency-maintenance.md)
  Catalog ownership, upgrade grouping, runtime baselines, audits, and release qualification.
- [`rivian-connection-renewal.md`](./rivian-connection-renewal.md)
  Interpret the advisory 180-day timer, renew credentials safely, and verify live recovery.
- [`charge-payload-cleanup.md`](./charge-payload-cleanup.md)
  Diagnose and compact redundant charging payload evidence without broad or automatic deletion.
- [`r2-ingestion-diagnostics.md`](./r2-ingestion-diagnostics.md)
  Trace R2 acquisition, typed decoding, canonical status, and sparse trips.

## When To Add A Runbook

Add a runbook when:

- a maintainer task is repeated often enough to justify stable steps
- an operational workflow is easy to forget or easy to do inconsistently
- a change introduces new recovery, publishing, or verification steps

- [Local Northflank deployment controller](local-northflank-controller.md)
- [GitHub and Northflank release pipeline](native-northflank-release.md)
- [Vehicle history protection and encrypted backups](private-history-backups.md)
- [KiRoom upstream review procedure](kiroom-upstream-review.md)
