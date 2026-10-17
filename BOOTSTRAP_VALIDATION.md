# Bootstrap validation

This page records historical bootstrap and bounded rewrite validation. Every
result applies to the source revision named in its section. The comprehensive
implementation replacement requires its own final validation; the results
below are not evidence that a later revision has passed.

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
