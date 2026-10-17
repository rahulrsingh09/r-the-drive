---
title: Build and workflow programs
description: Local characterization and execution boundaries for build configuration and release workflow programs.
---

# Build and workflow programs

The workflow files keep their existing triggers, action pins, permissions,
job dependencies, platform settings and opt-in automation gates. Their inline
programs implement the policies below. Editing these programs does not enable
release or deployment automation.

## Program boundaries

Release promotion constructs the exact candidate references for the selected
architectures, checks each candidate, and validates the returned lowercase
SHA-256 digest before emitting a successful output. Stable releases stamp the
selected candidates and publish version and latest tags. Pre-releases publish
the version tag. Development promotion checks both the current development
commit and the digest resolved by the moving development tag.

Candidate retention groups single-tag candidate versions by architecture,
sorts their creation timestamps and version IDs in descending order, and
retains ten AMD64 versions and three ARM64 versions. Single legacy development
tags are eligible for removal. Shared, moving, cache and unrelated tags remain
outside that selection. Listing or deletion errors stop the program.

Path gates treat failed Git comparisons as requiring validation. Audit runners
pass the report bytes and audit exit status to the existing advisory evaluator.
Environment preparation preserves the template while replacing the established
assignment patterns and appending a generated setup token. Patch compatibility
overlays accept an already compatible recipe and otherwise require one install
seam before changing the file.

The runtime health program launches the API in a new process group and tries
up to twenty probes, with a two-second timeout per probe and a two-second delay
after failures. Cleanup signals only that process group, waits up to five
seconds, then kills and reaps the process if necessary. Probe errors still run
cleanup.

## Build configuration

The API-only Docker recipe copies the actual crate and performs one locked
release build with two compiler jobs. It no longer compiles a synthetic crate
or touches selected source files to invalidate that cache. The separate Compose
build continues to use its existing build coordinator.

Vite classifies vendor modules by package path boundaries and applies the
existing React, router, then query precedence. Proxy environment precedence,
connection-reset filtering, and the Storybook alias and theme adapters keep
their characterized contracts.

Documentation pages, palette previews, Tailwind tokens, and lint declarations
contain fixed output and policy data. Their unchanged literals are reviewed
separately from program replacements; unchanged text is not evidence of
independent authorship.

## Local verification

```bash
node --test tools/build-config*.test.mjs tools/workflow-*.test.mjs
python3 tools/test_container_build.py
pnpm build
pnpm docs:check
```

Workflow fixtures record provider and build command arguments in disposable
directories. They never run release, upload, deletion, package installation or
provider commands. Health fixtures replace process, signal and clock boundaries
with inert implementations. Tests cover successful behavior and denial paths;
the workflow-control snapshot separately checks the unchanged YAML controls.

The workflow guard integration patch also updates the old mutation fixture's
source seam while preserving its assertions. Run the existing validation tests
after applying that patch during integration.
