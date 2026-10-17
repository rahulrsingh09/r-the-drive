# Bootstrap validation

This page records validation by source revision. The latest completed workflow
replacement is documented first; earlier sections retain their original scope
and do not stand in for tests of later revisions.

## Complete workflow replacement validation

Validated locally on 2026-10-17. The combined runtime candidate is
`979e9f26a6352be91129e185dc80e9ae1798563b`. Subsequent test-only commits through
`6b1fe6dcbe194117549973e7be11b785a3aff062` update migration-related expectations
in eight existing test/golden files; they change no production implementation.
This validation record is a later documentation-only change. All evidence is
bound to the actual revision and executable used, rather than relabeling earlier
runs as tests of a later commit. Nothing was deployed or published.

The completed scope covers frontend state and rendering workflows, shared
API hooks, vehicle and telemetry acquisition, durable trip/charge processing,
query and calculation services, authentication and private security, maintenance,
backup/recovery, deployment helpers, and build/validation tooling. The R
interface retains its appearance and behavior. Unused layout and dashboard
assignment editing controls are removed; saved layouts remain viewable and
exportable, and chart-definition editing and favorites remain available.
Fixed schemas, protocol declarations, data, markup, equations, security
primitives and individually reviewed small adapters remain explicitly retained.
These are not represented as independently rewritten algorithms. GPL-3.0-only
and the accurate notices remain in place.

| Check                                | Result                                                                                                                                                                                                                                                                                                                               |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Combined runtime image               | 131 authenticated HTTP checks passed with synthetic R1T/R1S/R2 data, restricted database access, closed owner setup, vehicle authorization, session revocation and restart persistence.                                                                                                                                              |
| Actual guided installation           | The unchanged setup/start CLI passed on a separate fresh stack, including keys, owner registration, setup-proof removal, private permissions, persistence and restricted database privileges.                                                                                                                                        |
| Full browser interaction             | 157 passed, zero failed, skipped or flaky cases; the isolated browser container was removed.                                                                                                                                                                                                                                         |
| Rendered release image               | Ten desktop views and a 390×844 mobile view passed with zero browser or CSP errors and no horizontal overflow.                                                                                                                                                                                                                       |
| Frontend and workspace               | 1,838 web tests passed with one inherited live-API skip; 25 UI, four theme, 39 gateway and 24 documentation tests passed. Build, lint, type checking, dependency policy and architecture checks passed.                                                                                                                              |
| Scripts and build tooling            | 287 repository tests, 36 Node tooling tests and three Python tooling tests passed. Seven additional Python suites passed 83 tests, with one opt-in database unit skip covered by separate actual installation.                                                                                                                       |
| Rust and configured services         | 1,194 unique tests passed: 872 ordinary and 322 configured ignored. Strict all-target/all-feature Clippy passed; 28 normal binaries and 35 test executables are accounted for.                                                                                                                                                       |
| Visual comparison                    | 68 images reviewed: 43 exact, 24 requested branding/control-removal/copy changes, and one recorded rendering-precision frame. No masks or tolerance were added.                                                                                                                                                                      |
| Repository checks                    | Documentation/build/search, route-security inventory, bootstrap integrity, color, Compose and release-workflow checks passed.                                                                                                                                                                                                        |
| Privacy and historical compatibility | The full scan at `27f1671` covered 1,663 tracked files and 201 new commits; no known personal identifiers, retired project names or real secrets found. Fifteen synthetic fixtures were individually reviewed. All 32 historical migration files and the license/notice remain intact; the effective catalog contains 33 migrations. |
| Cleanup                              | Application, installer, browser and backend fixtures were removed. The backend audit verified 72 absent fixture scopes and 11 absent generated key/tmp directories; source-specific evidence and compilation caches remain.                                                                                                          |

The Rust test inventory and configured execution ledger distinguish ordinary
and normally ignored cases, repeated invocations and nested child tests.
The inventory contains 1,197 named tests across 35 executables, with no missing
original names or uncovered cases beyond the three explicit exclusions below.
Actual executions at `b4d56da`, `7655aa9` and `6b1fe6d` retain separate source,
binary and log hashes. Relinked binaries have distinct identities; no bitwise
reproducible-build claim is made. The 26 nested child executions are reconciled
without adding duplicate counts.
Two intentional exclusions generate historical capture fixtures. The third
attempts a legacy destructive demo refresh that protected history does not
permit; the corresponding denial regression remains required and passed.
The supported development refresh is a separate tested path. All 53 compiled
edge-session tests and all 14 original backup integration tests passed as part
of the configured security profile. No history permission or security assertion
was relaxed.

Validation found and fixed an HTTP compressed-stream completion regression:
the metric stream is now fused before being handed to the response body.
The original compressed-response contract and the optimized real HTTP checks
passed afterward. Additive migration expectations were updated precisely for
version 1000032; existing response/protection assertions and prior ledger rows
were preserved. The external maintenance oracle changes only its current-version
stdout line, with all other recorded values unchanged. The two candidate schema
fingerprint expectations were independently reproduced from the exact
protected-history backup archive in PostgreSQL 18. Among 1,975 schema items,
only the two intended stored-function definitions changed; all other schema
items and golden values were preserved. A separate fresh-schema control
differed only in one protected-history job scheduling flag, explaining why the
exact archive was necessary for that comparison.

Unoptimized recovery tests use the existing 16 MiB Rust test-thread stack
profile. The initial default-stack failures are preserved separately; no runtime
stack setting was changed. The optimized release image separately passed real
backup creation, checksum/length-checked download, admitted upload and versioned
preflight. All nine configured restore contracts passed, including candidate preparation,
rollback, durable state, activation/retry/finalization and cleanup. The historical
archive fixture required its existing restricted NOLOGIN ownership role;
restoring that fixture role left archive bytes and runtime permissions unchanged.
A domain network fixture was moved from a reserved address range to an explicit
private range after the outbound policy correctly rejected the original address.
A completed external-provider test encountered an open connection during
fixture teardown; its unchanged rerun and scoped cleanup passed.
A clock-sensitive backup expected-output header differed by three bytes when
a timestamp serialized at millisecond precision. The header matched the actual
response length and normalized bodies/state were identical; the original
contract passed unchanged on rerun. The initial failure remains recorded.
The artwork test uses the existing Python 3.12/Pillow environment after the
system Python lacked Pillow; no implementation or assertion was changed for it.

The final visual comparison includes desktop/mobile and light/dark cases.
One dark charge screenshot differs in 45 border pixels, each color channel by
at most one level; repeating identical candidate source produced 97 differing
border pixels. That observation is recorded as a capture-precision limitation,
not claimed as exact image parity. All 222 original assertion files are
accounted for: 206 byte-identical and 16 individually classified changes for
removed layout capabilities, product naming or unchanged-body hook mocks.

The local image ID is
`sha256:31b2f2d5877b3609532af4f3898ff7a3d5cf9fe1ee8007e42a181e3d95462f6b`.
Its revision label and embedded identity match the runtime candidate. The
source archive checksum is
`f5f79f74e8114d4b2efae8a27c6a13d03b36dfd4fbf9b4786aaeedf9388f0f75`.
The actual installer fixture maps a digest-shaped alias to this verified local
image and applies an external network-isolation overlay. Setup, provisioning,
key generation and authentication run unchanged. Registry pull/authentication
was not exercised; this Docker image ID is not a published registry digest.
The source Dockerfile build was exercised separately when building the image.

The setup entrypoint remains [scripts/deploy.py](./scripts/deploy.py).
It installs Docker Compose on the chosen host. The [README](./README.md#where-it-deploys)
explains defaults, reasons and the four setup steps. Cloud provisioning, DNS,
TLS and the authenticated gateway are supplied by the person operating it.
Tests used a disposable loopback HTTPS gateway and synthetic data only.
No real vehicle connection, production deployment, hosted workflow or external
S3 service was used. No new Cargo advisory, Semgrep or Trivy scan is claimed.
Known-pattern/Gitleaks scanning has bounded decoding and does not include OCR
or encrypted-file inspection; it is not proof of absence of every possible secret.

## Historical bootstrap validation

Validated locally on 2026-10-16 from source snapshot `f9482c7dc207adbdb2b43c3f247023f2e20323d6`. The repository has a new Git history, PANDA package/configuration/schema names and the existing R interface. No application deployment was performed.

The table records validation of the initial renamed snapshot. After the privacy cleanup, the workspace build, documentation checks, all 202 script tests, 116 affected web tests and 16 native deployment/guard tests were rerun successfully. The new Docker helper passed 10 safety tests, including real Compose configuration rendering and rejection of registration redirects. Actual offline key generation verified UID/GID 1001, directory permissions 0700 and key-file permissions 0600 in a disposable Docker volume. A complete deployment through a real HTTPS gateway has not been performed.

| Check | Result |
| --- | --- |
| Clean source baseline and renamed workspace builds | Passed; web, documentation and private gateway bundles |
| Lint and TypeScript checks | Passed |
| Documentation links, search indexes and final documentation build | Passed |
| Workspace tests | 903 web, 25 UI, 4 theme, 26 gateway and 8 documentation tests passed; one inherited web test remains skipped |
| Repository script tests | 202 passed; the final repository/workflow identity changes also passed 76 focused tests |
| Rust build and formatting | All binaries built; rustfmt passed |
| Rust library tests | 475 passed; database-dependent tests are handled separately below |
| Authentication integration and crypto regressions | 45 authentication tests and 2 crypto tests passed |
| Database-backed security and durability | 29 passed: migration ledger, vehicle authorization, key custody, protected history and trip checkpoints |
| Native release safeguards | 49 Python tests passed; repository identity changes also passed the native adapter and shell-guard suites |
| Actual nginx origin boundary | Passed with a synthetic backend |
| Browser suite | 134 passed with the repository's standard single-worker configuration, including desktop/mobile views and synthetic README previews |
| Route-security inventory | Passed for 38 route modules |
| Secret scanning | Gitleaks 8.30.1: no leaks; exceptions are narrowly matched, nonfunctional test fixtures |
| Dependency policy and npm audit | Passed policy; two inherited, time-bounded documentation-tooling exceptions remain in `config/dependency-audit-exceptions.json` |
| Compose and release-workflow contracts | Passed |
| Rename and migration-cache integrity | No retired project identifiers in tracked paths or bytes; 31 migration identities and every SQLx query hash verified |
| Source inventory and license | All source modules retained; only eight old layered logos, an obsolete domain file and historical scan suppressions removed. GPL license unchanged |

The initial browser run used two workers and had two timeouts; the complete rerun with one worker passed. A restore-space test was made deterministic by requesting an impossible reserve instead of racing a changing free-space measurement. Runtime reserve enforcement was unchanged.

This is not a completed production deployment or a fresh independent penetration test. Hosted workflows have not run. A local Cargo advisory scan, Semgrep scan and Trivy image scan were not performed in this bootstrap; their inherited workflow configuration is retained.

See [BOOTSTRAP.md](./BOOTSTRAP.md) for the fresh-database requirement, disabled automation, GitHub feature limitations and remaining deployment configuration. No production credentials, vehicle records, backups, access grants or deployment secrets were copied.

## Mainline module retention

Validated on 2026-10-16 after removing the unused predecessor sidebar layout
and its private notice components. The mainline R implementation, routes,
shared packages, Rust API, gateway and Compose runtime files are unchanged
from the sanitized snapshot.

- Workspace build, focused ESLint, documentation checks and bootstrap inventory passed.
- 25 R layout, route, reading and renewal tests passed.
- 53 frontend authentication and API-client tests passed.
- 26 gateway tests and 3 authentication-fixture policy tests passed.
- 13 browser tests passed with one worker, covering 390px/1280px, light/dark,
  route navigation, settings, chart editing, vehicle rotation and trip inspection.
- Security dependency audit confirmed all private-deployment modules remain wired;
  route-security inventory passed for 38 route modules.
- Gitleaks found no leaks in the exact staged source using the existing narrowly
  scoped fixture exceptions. Known personal identifiers and retired project
  names were absent from the source and binary-file scan.

The browser suite uses synthetic API fixtures. Live vehicle authentication and
a production deployment were not performed. These checks do not establish
independent authorship of retained implementation.

## R:The Drive publication

The product README, display name, installation guide and neutral repository
templates were updated on 2026-10-16. Validation passed: workspace build,
documentation checks and 8 documentation tests, 202 repository script tests,
106 affected web tests, 4 sign-in browser tests, 10 deployment-helper tests,
16 native deployment/guard tests and 25 cloud-adapter tests. Display-name
checks also passed on desktop and mobile against the local synthetic preview.
Compose and release-workflow contracts passed.

The publication source is checked by an external identifier scan and Gitleaks.
The retired-name denylist is no longer shipped in the repository, including
its former character-code representation. License and attribution remain
accurate in their dedicated files. Production and vehicle data were not changed.

The final encoded-identifier scan passed across 1,094 tracked files. A legacy
encoded test-password fixture was replaced with the existing password-hashing
helper; all 5 helper/role tests passed and the API test target compiled. The
database-dependent authorization fixture was not rerun in this pass. The
GPLv3 license bytes match the source license, the backend metadata explicitly
uses GPL-3.0-only, and NOTICE.md records attribution and the modification date.

## Functional rewrite readiness

The runtime candidate is `f07b2f6cd636e1ec24f614e93a10a9681d6f37c3`,
validated locally on 2026-10-16 and 2026-10-17 with synthetic accounts and
vehicle data. This section supersedes the earlier bootstrap's incomplete
integration gates; it does not turn those historical results into tests of the
current revision. This validation report is a later documentation-only change.
No production deployment or live vehicle connection was performed.

The implemented scope covers dashboard and API-client internals, telemetry
collection and durable trip/charge state, provider orchestration, vehicle and
access workflows, query and calculation services, enrichment, backup and
restore, and maintenance. The selected R interface, private security modules,
public wire/schema contracts and documented compatibility utilities remain.
GPL-3.0-only and the attribution in NOTICE.md are preserved.

| Validation | Result |
| --- | --- |
| Rust build and static checks | All binaries built; strict all-target/all-feature Clippy passed on the runtime candidate. |
| Ordinary Rust tests | 682 library and 10 binary tests passed on the candidate. All 66 ordinary integration tests have passing scoped evidence. |
| Configured Rust tests | 269 normally ignored tests have named passing evidence, including all nine final restore execution/security cases. Three intentional exclusions are explained below. |
| Whole application acceptance | 131 HTTP checks, 10 desktop views and a 390×844 mobile view passed against the exact candidate image, with zero browser runtime or Content Security Policy errors. Restart persistence and local backup/download/upload/preflight checks passed. |
| Guided installer | Actual `setup --image` and `start` passed on a separate fresh stack using the same image: generated keys, owner registration, setup-proof removal, private file permissions, saved-state persistence and restricted database privileges. |
| Browser and frontend | 1,406 web, 25 UI and 4 theme tests passed; one inherited web test remains skipped. Lint, type checking and the workspace build passed. |
| Repository and setup checks | All 202 script tests passed. Installer checks passed on Python 3.7 and 3.12, with the database-dependent unit entry explicitly skipped and actual setup covered separately. |
| Documentation | Documentation checks, architecture checks, documentation build and search indexing passed. |
| Privacy and license | 1,436 tracked files, including bounded decompression of gzip fixtures, contain no known personal identifiers or retired project names. GPL license bytes match the published baseline; new commit identities are neutral. |
| Secret scanning | Strict Gitleaks scanning of changes since the published baseline found no findings. Initial-snapshot nonfunctional test fixtures retain their separately reviewed status. |
| Cleanup | Final application, installer and restore containers, volumes and networks were removed. Installer and browser private temporary files were removed. |

The compiled Rust inventory contains 1,030 top-level tests: 758 ordinary and
272 normally ignored. The per-test evidence ledger distinguishes actual
execution revisions. Earlier frontend, installer and stable service results
are carried forward only after checking their relevant source files; they
were not all rerun in one command against the final commit. Final application
acceptance exercises the combined runtime. Nested authorization checks and
repeated scope runs are not added again to the inventory totals.

Two ignored entries generate historical capture fixtures and are intentionally
excluded. A third performs a legacy destructive demo refresh that protected
history does not permit; a passing regression verifies the permission denial.
Normal historical rebuilds that require protected deletion remain unsupported.
No history permission was relaxed to make these checks pass.

Restore coverage includes candidate preparation, failure rollback, successful
restore of nonempty history and worker restart. Scratch-database cleanup was
corrected after cancellation and busy-database regressions exposed a defect;
the eight original cases and four new PostgreSQL regressions pass. Other
corrections include durable charge completion, revoked-membership cost access,
and cursor metadata reuse. Full-output fixtures and original security assertions
remain in the evidence, including an assertion-backed correction to two legacy
voltage expectations that previously contained power values.

The final application image ID is
`sha256:2c619e12db189dc513a9720d57ab88aec05d881de6d57d654b26975df6bc0bd4`.
Its revision label and embedded release identity match the runtime commit.
The broad HTTPS/browser fixture and separate installer fixture used the same
source archive and image. The installer used an external test adapter to map
a digest-shaped alias to that verified local image and add network isolation;
its setup, provisioning, key generation and authentication code ran unchanged.
Registry pull/authentication was not exercised, and this image ID is not a
published registry manifest digest.

The deployment helper remains [scripts/deploy.py](./scripts/deploy.py). It
installs with local Docker Compose on the host where it runs; it does not
provision Northflank, a cloud server, DNS, TLS or an access gateway. The
[README](./README.md#deploy-in-four-steps) records the four installation steps,
defaults and reasons. Use new storage and credentials for a fresh installation.

This is local release-readiness evidence, not a production deployment, a live
provider certification or an independent penetration test. A user's real
HTTPS gateway remains outside the isolated acceptance fixture. Hosted
automation has not been enabled. No new Cargo advisory, Semgrep or Trivy scan
is claimed by this pass.
