# Repository validation tooling

Repository checks read the committed policy and source contracts before local
builds or release preparation. Their command names and imported helper APIs are
stable. This implementation replaces inherited validation and orchestration
logic while preserving the repository's provenance and GPL attribution.

## Policy evaluation

Architecture checks collect source files once per rule, count every pattern
match, enforce line and byte budgets, and traverse import dependencies with an
explicit stack. Test and story exclusions, package aliases, transport bans,
thresholds, and exception lists remain defined by the existing policy.

Route checks inventory mounted Rust route modules, the authorization matrix,
and the deprecated ownership helper ban. They are a source inventory, not a
substitute for handler authorization tests. Dependency checks retain catalog
requirements, toolchain pins, immutable action references, the install-script
allowlist, and bounded advisory exceptions. Audit evaluation keeps raw findings,
validates exception dates and exact package versions, and requires every pnpm
dependency path to remain inside its reviewed scope.

Documentation checks cover required pages, local links, sidebar membership,
routes, environment references, and deployment contracts. Release checks read
workflow jobs and evaluate candidate, promotion, digest, provenance, platform,
retention, and verification requirements. Some acceptance-runner contracts are
intentionally source-sensitive: changes to those runners must preserve their
checks or supply equivalent reviewed verification before integration.

## Migration identity and reset boundaries

Migration identity is computed from the original bytes with SHA-384. Public
versions starting at 28 retain their offset of 1,000,000; the deployed private
checkpoint keeps version 28. Catalog extension rejects changed, removed, or
reordered serialized identities. The public integrity command also validates
filenames, UTF-8 and LF encoding, contiguous versions, and Git history. Its
historical five-file-to-one-file baseline cutover remains narrowly defined.

Database reset still requires the development acknowledgement, the explicit
confirmation argument, a non-production environment, and a loopback target.
Upgrade verification requires an explicitly named disposable loopback database.
No validation test resets a host database.

## Synchronization and service lifecycle

Default synchronization reads the complete source set before writing drifted
targets. Check mode reports drift without writing. Repeating a successful sync
performs no additional writes. Tests exercise copies of the chart and dashboard
fixtures; the generated files in the checkout are not rewritten for validation.

The production supervisor prepares writable storage and the restore-agent key,
renders the origin gate, removes the gateway token from child environments, and
starts the services. A single lifecycle loop monitors nginx and the restore
agent both during normal operation and while the API is stopped for restoration.
The API restarts only after the restore marker clears. Failed key generation or
storage preparation prevents startup.

The origin renderer validates the token before creating a private staging file
and atomically replacing the nginx map. HTTP checks use a synthetic local backend
and preserve the application authorization header while removing the edge token.
The browser CSP probe restricts requests to its local test server.

## Verification

`pnpm test:scripts` includes the original script assertions plus characterization,
Git-history fixtures, command failure tests, and temporary supervisor processes.
Local-CI orchestration tests inject a command recorder: they verify gate order,
retry exhaustion, cleanup, and argument forwarding without executing deployment
or pull-request commands. Real local nginx, Compose rendering, browser CSP, and
container permission probes complement those tests.

Operational driver tests also record the exact image digest passed to Compose,
reuse the original owner token after restart, and verify mount preparation before
both source and target startup. Mutation cases remove each protected smoke-test
operation and require the release guard to reject it, including asynchronous
backup completion, both artifact stores, and restore error diagnostics.

Run the strict repository guards, `pnpm build`, `pnpm docs:check`, and
`pnpm docs:build` when changing this boundary. A new documentation page must
appear exactly once in the shared sidebar when integrated.
