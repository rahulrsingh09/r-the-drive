# Bootstrap validation

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
