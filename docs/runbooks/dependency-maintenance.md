# Dependency maintenance runbook

This runbook owns JavaScript, Rust, runtime, service-image, and CI dependency updates.

## Sources of truth

- `config/dependency-baselines.json` records Node, pnpm, Rust, PostgreSQL, TimescaleDB, Redis, Garage, and image baselines.
- `pnpm-workspace.yaml` owns shared JavaScript versions through the default catalog.
- `package.json` pins pnpm and the supported Node range.
- `rust-toolchain.toml` and `apps/api/Cargo.toml` pin Rust and the crate MSRV.
- Dockerfiles and Compose files pin stable patch tags plus multi-architecture manifest digests.

Keep Lucide, Iconify, and React Icons. They serve app-native, dynamic catalog, and specialized icon-family use cases respectively. Recharts, uPlot, and react-grid-layout are also intentional specialized dependencies.

## Routine update sequence

1. Work from a clean worktree and record direct dependency, lockfile, duplicate, bundle, crate, and image-size baselines.
2. Run `pnpm deps:check` before and after changes. Shared dependencies must use `catalog:` and imports must be declared by the package that owns them.
3. Group patch and minor updates. Keep major updates isolated so migrations and rollback are reviewable.
4. Run `pnpm install --frozen-lockfile`, peer checks, typecheck, lint, tests, production build, docs build, and Storybook build.
5. Run Cargo format, Clippy, all-target checks/tests, `cargo tree --duplicates`, SQLx prepare/check, and `cargo audit --deny warnings`. Preserve the raw audit results even when a documented exception applies.
6. Build and scan clean amd64 and arm64 images. Smoke-test development and production fresh installs.
7. For PostgreSQL majors, use dump/restore into a new volume only. Validate a second recovery-package restore before release.
8. Record results in a dated dependency-modernization report under `docs/`.

## Automation policy

Dependabot monitors Cargo, npm, GitHub Actions, Dockerfiles, and Compose. Patch/minor changes are grouped; majors remain separate. CI rejects catalog drift, undeclared/unused dependencies, peer failures, lockfile drift, unsupported runtime references, high/critical npm or Rust advisories, leaked secrets, and fixable high/critical container findings. Unfixed base-image findings stay in Trivy output and require review whenever the pinned base digest changes.

Do not add unbounded advisory exceptions. Any temporary exception needs an owner, upstream link, expiry, and removal condition. `config/dependency-audit-exceptions.json` is the executable register. Native Cargo/pnpm ignore lists must remain empty. The inherited security workflow prints raw JSON before `scripts/check-dependency-audits.mjs` checks exact advisory/package/version, expiry, and npm paths; it rejects unknown findings, scanner failures, and stale exceptions. Both production and development npm dependencies are scanned. A policy pass with exceptions is **not** a clean vulnerability scan.

## Advisory review — 2026-10-05

The [dated dependency review](../dependency-review-2026-10-05.md) records source evidence, reachability, and regression checks. No application API or encrypted-data format changed.

- `age 0.12.1` accepts the published `i18n-embed-fl 0.10.1`, which replaces `proc-macro-error2` with `proc-macro-error3`. The October 1 exception for `RUSTSEC-2026-0173` is removed, not extended.
- npm published `image-size 2.0.3` and `2.0.4` on September 14. The exact `2.0.4` override replaces the incomplete `2.0.2` patch and removes both image-size ignores. The retired patch file contains only a note because the inherited Dockerfile still copies that path; the container owner can remove the COPY and note together. There is no active local package patch.
- The remaining raw results are **one RSA vulnerability** and **two high npm findings** (`braces` and disputed `http-cache-semantics`). All have narrow exceptions below. There are no Cargo informational warnings or other npm findings in the reviewed lockfiles.
- Run `node --test scripts/check-dependency-audits.test.mjs scripts/image-size-security.test.mjs` after a frozen install. Parser tests cover both CommonJS/ESM public buffer and file APIs, including zero/undersized/truncated ICNS, HEIF and JXL data and valid image dimensions.
- Dependency install scripts are restricted to esbuild's binary setup. The unnecessary core-js banner/cache postinstall is explicitly denied. Re-review any change to the allowlist; do not auto-approve newly introduced scripts.

## Rust patch baseline — 2026-10-16

The API lockfile now selects `quinn-proto 0.11.18` (previously 0.11.16) and
`rustls 0.23.46` (previously 0.23.45). RustSec's published version ranges
confirm these releases cover the following advisories:

- Quinn: [RUSTSEC-2026-0338](https://rustsec.org/advisories/RUSTSEC-2026-0338.html),
  [0339](https://rustsec.org/advisories/RUSTSEC-2026-0339.html),
  [0340](https://rustsec.org/advisories/RUSTSEC-2026-0340.html),
  [0341](https://rustsec.org/advisories/RUSTSEC-2026-0341.html), and
  [0342](https://rustsec.org/advisories/RUSTSEC-2026-0342.html). The fixes span
  0.11.17 and 0.11.18; the selected version includes both.
- Rustls: [RUSTSEC-2026-0346](https://rustsec.org/advisories/RUSTSEC-2026-0346.html),
  [0347](https://rustsec.org/advisories/RUSTSEC-2026-0347.html),
  [0348](https://rustsec.org/advisories/RUSTSEC-2026-0348.html), and
  [0349](https://rustsec.org/advisories/RUSTSEC-2026-0349.html), each listing
  0.23.46 as the first patched release.

The accompanying lock resolution uses `getrandom 0.3.4` for `tempfile 3.27.0`,
within its declared `>=0.3.0, <0.5` requirement. Direct dependency requirements
and the audit exception register are unchanged. This scoped patch does not
establish a clean security audit: the existing RSA exception and npm findings
recorded above still require their scheduled review. Preserve raw scan results
and distinguish a policy pass with exceptions from a scan without findings.

## Maintenance register

This register records accepted maintenance debt that is not release-blocking.
Each entry needs an owner, evidence, a due date, and a closure check. Do not
use it to waive a security, privacy, data-integrity, or availability defect.

| Priority | Item                                                                      | Owner               | Evidence and current control                                                                                                                                                                                                                                                                                                                                                                                                                                                                               | Due date   | Closure evidence                                                                                                                                                                                                    |
| -------- | ------------------------------------------------------------------------- | ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| P3       | RSA vulnerability, private-key operation not reached: `RUSTSEC-2023-0071` | Release maintainer  | Rechecked October 5: the active reverse dependency is `openidconnect 4.0.1` → `rsa 0.9.10`. Provider ID-token verification reaches `RsaPublicKey::verify`, not RSA private-key signing/decryption. Panda's separate private-key operations use AWS-LC. [RustSec still reports no patched version](https://rustsec.org/advisories/RUSTSEC-2023-0071.html); the existing deadline is unchanged.                                                                                                          | 2026-12-31 | Remove the exception when a fixed release exists or RSA is removed. Any new caller/private-key use requires immediate review; rerun raw audit and reverse-tree checks.                                              |
| P3       | Build-tool stack exhaustion: `GHSA-vfj7-8cjw-p6xm`                        | Release maintainer  | `braces 3.0.3` is reached through Docusaurus → chokidar/micromatch, including fast-glob and webpack-dev-server. Reviewed repository-controlled glob patterns only; production serves static output, not the docs dev server. [GitHub reports no patched release](https://github.com/advisories/GHSA-vfj7-8cjw-p6xm), and registry `3.0.4` is absent despite npm audit suggesting it. Raw audit remains high.                                                                                               | 2026-11-05 | Upgrade to an actually published fix or replace the dependency. Untrusted glob input, an exposed docs dev server, or a new application path invalidates this exception; review before processing untrusted content. |
| P3       | Disputed update-client cache advisory: `GHSA-ch52-4w7c-c8xp`              | Release maintainer  | Docusaurus → update-notifier → latest-version → package-json → got → cacheable-request → `http-cache-semantics 4.2.0`. This is a registry client, not an application server/shared user cache; got caching is disabled by default. [The maintainer disputes the report](https://github.com/kornelski/http-cache-semantics/issues/56), while GitHub still lists no fix. Registry `4.2.1` is absent and published `4.3.0` leaves the reported max-stale branch unchanged, so no cosmetic upgrade is applied. | 2026-11-05 | Remove on withdrawal, a verified fix, or dependency removal. Reassess immediately if used for a shared cache or with application-user request headers.                                                              |
| P3       | Asset optimization                                                        | Frontend maintainer | No source-backed performance budget or image/asset optimization evidence is currently recorded. The artwork workflow validates fallback correctness, not total transfer-size optimization.                                                                                                                                                                                                                                                                                                                 | 2026-10-01 | Record a repeatable baseline, optimize the identified assets without visual regression, and attach desktop/mobile verification.                                                                                     |
| P3       | Unproven dead exports                                                     | Frontend maintainer | `pnpm deps:check` invokes Knip, but the current repository has no reviewed export-by-export disposition proving all reported candidates are safe to remove.                                                                                                                                                                                                                                                                                                                                                | 2026-10-01 | Attach reviewed Knip output, remove confirmed dead exports with focused tests, and document any intentional public/package exports.                                                                                 |

Review this register during dependency maintenance and release preparation. If a
date passes without closure, either resolve the entry or explicitly reassess its
priority and risk in a pull request; do not silently extend it.
