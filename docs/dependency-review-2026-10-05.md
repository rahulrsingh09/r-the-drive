# Dependency advisory review — 2026-10-05

Documentation impact: internal documentation update required.

Scope: dependency manifests/lockfiles, the inherited security workflow, audit policy,
and dependency regressions. Application APIs and encrypted-data formats are unchanged.
The review started from upstream `d93c157fb1cc2a8d3e8b08b414033ea64eed04e0`.

## Results and published fixes

The baseline raw npm audit reports 30 findings: 12 high, 12 moderate, 6 low.
The inherited image-size ignore list hid two of the advisory entries. After the
targeted updates, the full and production npm audits report **two high findings**:
`braces` and `http-cache-semantics`. Cargo reports **one vulnerability**, RSA's
`RUSTSEC-2023-0071`, and **zero informational warnings**. The baseline also reported
the unmaintained `proc-macro-error2` warning `RUSTSEC-2026-0173`.
These are not zero-vulnerability results.

Versions were verified against the public npm/crates.io registries and the primary
advisory records, not inferred from an audit command's suggested version.

| Dependency                                                 | Reviewed update                 | Purpose                                                                                                                             |
| ---------------------------------------------------------- | ------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| `i18n-embed-fl`                                            | 0.10.0 → 0.10.1                 | Compatible with age 0.12.1's `^0.10` requirement; replaces unmaintained proc-macro-error2/attr2 with proc-macro-error3/attr3 3.1.1. |
| `image-size`                                               | patched 2.0.2 → published 2.0.4 | Replaces the incomplete patch for ICNS/HEIF/JXL infinite loops, including public CJS/ESM buffer and file APIs.                      |
| `vitest`, `@vitest/coverage-v8`, matching Vitest internals | 4.1.10 → 4.1.11                 | GHSA-82fw-gwwq-j7x9; keeps the existing major.                                                                                      |
| `fast-uri`                                                 | 3.1.6 → 3.1.8                   | Host confusion/URI parsing advisories, including GHSA-58mr-gqgx-xq4g and GHSA-hrr3-gc8f-f4qj.                                       |
| `undici`                                                   | 7.29.0 → 7.29.1                 | Published patch for the reported HTTP-client advisories.                                                                            |
| `brace-expansion`                                          | 5.0.9 → 5.0.12                  | Reported expansion denial-of-service advisories.                                                                                    |
| `joi`                                                      | 17.13.4 → 17.13.7               | Reported validation advisories; override restricted to major 17.                                                                    |
| `qs`                                                       | 6.15.3 → 6.16.0                 | Both reported query-string advisories.                                                                                              |
| `colord`                                                   | 2.9.3 → 2.9.4                   | GHSA-2wm5-q62r-hmrv.                                                                                                                |
| `postcss-selector-parser`                                  | 6.1.2 → 6.1.3                   | GHSA-w9m9-85wc-3x92; override restricted to major 6.                                                                                |
| `smol-toml`                                                | 1.7.0 → 1.7.1                   | GHSA-7w5x-hrqm-74c2.                                                                                                                |

The image-size registry publishes 2.0.3 and 2.0.4 on **2026-09-14**. Version 2.0.4
also adds ICO entry-count bounds. The old patch changed individual parser modules
but missed copies embedded in the public 2.0.2 distribution entry points.
Both public-module regression runs time out against that old patch; they complete
against 2.0.4. The old patch file is now an unapplied retirement note, retained
solely for the inherited Dockerfile COPY path. Remove that COPY and the note together.

## Remaining advisories and applicability

### RSA: no fixed release, public verification only

`cargo tree --locked -i rsa` gives:

```text
rsa 0.9.10
└── openidconnect 4.0.1
    └── panda-api
```

`apps/api/src/services/oidc.rs` calls `claims(&client.id_token_verifier(), ...)`.
The reviewed openidconnect `core/jwk/mod.rs` calls `core/crypto.rs::verify_rsa_signature`,
which constructs `rsa::RsaPublicKey` and calls `verify`. No RustCrypto RSA private
key is used on that path. The application's other private-key operations use
AWS-LC, a different implementation. Optional SQLx MySQL packages present in the
lockfile do not appear in the active RSA reverse tree.

RustSec's September 14 update explicitly states that neither stable 0.9.10 nor
0.10.0-rc.18 fixes Marvin; migration to crypto-bigint is not a fix. Keep this raw
finding visible. The existing **2026-12-31** deadline is unchanged.
Owner: release maintainer. Remove the exception on a verified fix or dependency
removal; any new RSA caller or private-key use requires immediate reassessment.

### Braces: no published fix, documentation build tooling

`pnpm why -r braces` identifies `braces 3.0.3` through Docusaurus → chokidar and
Docusaurus → micromatch (including fast-glob and webpack-dev-server middleware).
Deeply nested untrusted patterns can exhaust the stack. The application does not
expose a glob API; these paths process repository-controlled build/watch patterns.
The production image serves compiled static output, not the Docusaurus dev server.
Review incoming repository/configuration changes and do not expose that development
server or pass application-user patterns to these tools.

The npm audit suggestion `>=3.0.4` is not a published release as of this review;
the registry's latest is 3.0.3 and GitHub's primary advisory lists no patched version.
Owner: release maintainer. Exception expires **2026-11-05**. Replace it with a
published fix or remove the dependency. New callers or untrusted pattern input
invalidate the exception immediately.

### HTTP cache semantics: disputed, no verified fix

The only reverse path is Docusaurus → update-notifier → latest-version →
package-json → got → cacheable-request → `http-cache-semantics 4.2.0`.
This is an npm-registry client, not an application HTTP server or shared user cache.
The reviewed got 12.6.1 default has `cache: undefined`; package-json does not enable
caching or pass application-user cache-control headers.

GHSA-ch52-4w7c-c8xp describes shared-cache disclosure through client `max-stale`.
The maintainer disputes it, citing RFC 9111's treatment of Set-Cookie and
Cache-Control: private. The advisory remains active with no patched version.
The registry has no 4.2.1, despite npm audit suggesting it. Published 4.3.0 leaves
the reported stale-serving branch unchanged; its changed audit range is not
evidence of a fix. Retain the reviewed 4.2.0 instead of claiming remediation.

Owner: release maintainer. Exception expires **2026-11-05**. Remove on advisory
withdrawal, an independently verified fix, or dependency removal. Any use as a
shared cache or with application-user request headers requires immediate review.

## Enforcement, scripts, and verification

`config/dependency-audit-exceptions.json` records exact packages, versions, owners,
review dates, deadlines, source links and removal conditions. The evaluator checks
all raw findings, including Cargo warnings and development npm dependencies.
It fails on scanner errors, hidden Cargo ignores, unknown findings, expiry,
changed npm paths/versions and stale exceptions. Entries cannot exceed 90 days.
The dependency policy rejects native pnpm audit ignores and a return to the
retired image-size patch or unmaintained proc-macro-error chain.

The inherited workflow prints raw reports before evaluating exceptions and runs
the npm scan even if the Cargo audit fails. Its PR route inventory, secret/SAST
scans and scheduled/manual image scan requirements remain. Fork CI enablement
and new validation/sync workflows are owned separately; this review does not
establish that GitHub Actions ran.

Registry manifests and installed lifecycle scripts were reviewed. None of the
updated npm packages adds an install lifecycle hook. Only esbuild 0.28.1 remains
allowed: its postinstall selects/checks the platform binary, with versioned npm
fallback downloads and binary hash checks on fallback paths. The optional platform
package is lockfile-pinned. core-js 3.49.0's banner/temp-cache postinstall is denied.
The three changed Rust macro crates have no build.rs; their macros still execute
at compile time. Cargo dependency checksums and pnpm integrity records are retained.

Verification uses Rust 1.97.1, Node 24.18.0 and pnpm 11.15.1, with SQLX_OFFLINE=true:

- Frozen install and dependency policy/Knip checks.
- Raw full/production npm audits and Cargo audit with `--deny warnings`; bounded policy evaluation.
- Audit-policy regressions for expiry, path/version drift, new warnings, stale exceptions and scanner failures.
- Image-size: 15 fixtures through each of CJS/ESM buffer/file APIs, including valid and malformed formats; negative control against the retired patch.
- `cargo test --locked --test dependency_crypto`: synthetic age round trip, wrong identity, tampering and truncation.
- `cargo test --locked --lib services::oidc::tests` and locked default-binary build.
- Production web/docs build and docs validation.

No live credentials, cloud resources, database migrations, or SQLx metadata changes
are involved. Dependency checks do not replace container scanning or an independent
runtime security review.

## Primary sources

- [i18n-embed-fl 0.10.1 registry dependencies](https://crates.io/api/v1/crates/i18n-embed-fl/0.10.1/dependencies)
- [age 0.12.1 registry dependencies](https://crates.io/api/v1/crates/age/0.12.1/dependencies)
- [proc-macro-error2 RustSec advisory](https://rustsec.org/advisories/RUSTSEC-2026-0173.html)
- [RSA RustSec advisory](https://rustsec.org/advisories/RUSTSEC-2023-0071.html)
- [image-size registry publication metadata](https://registry.npmjs.org/image-size)
- [ICNS advisory](https://github.com/advisories/GHSA-w3rx-r6r6-pgpr) and [HEIF/JXL advisory](https://github.com/advisories/GHSA-5p2g-fcmc-qvqq)
- [braces advisory](https://github.com/advisories/GHSA-vfj7-8cjw-p6xm) and [registry](https://registry.npmjs.org/braces)
- [HTTP cache advisory](https://github.com/advisories/GHSA-ch52-4w7c-c8xp), [maintainer response](https://github.com/kornelski/http-cache-semantics/issues/56), and [registry](https://registry.npmjs.org/http-cache-semantics)
- [Public npm registry](https://registry.npmjs.org/) and [GitHub Advisory Database](https://github.com/advisories) for the remaining exact versions and findings above.
