# Key custody and private deployment implementation replacement

Documentation impact: internal architecture update required.

This change replaces application workflows from baseline
`5651fdfeed91924af623cf7eb919c417e33628bb`. It is a derivative implementation
under the repository's existing GPL-3.0-only terms and attribution, not a
claim of independent or clean-room authorship. Internal `panda` names remain
wire, database, and command contracts for R:The Drive.

## Workflows

| Boundary | Implementation and preserved guarantees |
| --- | --- |
| Key sources | An ordered three-slot inventory drives reads, bulk development insertion, migration deletion, and bundle files. Production requires a complete external bundle and rejects retained database private keys. |
| Key validation | Construction validates the RSA pair and AGE identity. RSA remains 2048-bit AWS-LC generation, PKCS8/private and X509/public PEM, and RS256 challenge verification. AGE remains the existing X25519 implementation. No primitive or dependency version changes. |
| Custody transactions | Advisory and table locks precede inspection. Development generation is permitted only for empty custody with no binding or ciphertext. Migration authenticates every inventoried ciphertext before atomically removing the three original key rows. |
| Bundle files | A new 0700 directory reserves all three 0600 files before writing secrets. Existing paths, including symlinks, fail. Files, directory, and parent are synced before success. Failure never permits overwriting that path on retry. |
| Enrollment | The active-user lock and remote-vehicle advisory lock precede a materialized vehicle-lock query with a locked membership decision. Only owners and managers can replace existing credentials. Missing and disabled users retain distinct errors. |
| History | A fixed table of privilege denials drives revocation. Retention jobs for durable history are disabled. The transaction audits each denied privilege before committing. Charge repair retains DELETE; charge TRUNCATE and protected-history DELETE/TRUNCATE remain denied. |
| Provider authentication | One transport owns credential headers, the fixed destination, and response handling. Login and OTP share required-token extraction. CSRF refresh copies the prior bundle and replaces only the short-lived pair. |
| Images and optional traffic | Image retrieval and optional client creation terminate at the fixed denial. The unreachable image-fetch/retry implementation and its private response models were removed. Runtime settings cannot enable that traffic. |
| JSON encryption | Serialization streams into AGE; deserialization reads through AGE to JSON EOF, including authentication of the final chunk. Token wrappers keep their original formats and do not add validation that would reject older optional-session fields. |
| Backup transport | Raw recovery bytes retain the AGE archive format. Bounded asynchronous file reads precede encryption. Receipt and provider readers share a remaining-byte budget. Upload still requires 201, matching SHA256/length, and a valid daily slot. |
| Weather | Settings and updates share fixed-origin rules. Admission uses one clock sample in its atomic counter update, retaining the daily budget, five-second spacing, and six-attempt bound. Recovery locks stale running jobs before resetting them. |
| CLI | Operations are parsed before dispatch. DB operations explicitly close their pool on either success or error. Existing messages and arguments remain stable; no key material is printed. |

## Fixed operations requiring maintainer inspection

The following 17 original named bodies are retained explicitly. They are not
counted as replaced application workflows:

| Source | Exact bodies | Reason |
| --- | --- | --- |
| `private_deployment/keys.rs` | `BootstrappedKeys::validate`, `BootstrappedKeys::identity`, `validate_rsa_pair` | Required RSA/AGE validation sequence, fixed challenge and redacted failures; cryptography is delegated to the unchanged libraries. |
| `private_deployment/keys.rs` | `age_recipient`, `lock_ciphertexts` | Single fixed binding lookup and single table-lock operation. |
| `private_deployment/keys.rs` | non-Unix `write_key_bundle` | Mandatory fail-closed platform denial. The Unix implementation is replaced. |
| `private_deployment/history.rs` | `require_vehicle_deletion_allowed` | Fixed production-mode deletion denial and its public error. |
| `private_deployment/history_backup.rs` | `configured` | Presence of the existing environment variable. |
| `private_deployment/outbound.rs` | `optional_traffic_allowed`, `validate_http_destination`, `validate_ws_destination`, `validated_gateway_url` | Constant denial and exact-string destination contracts. |
| `private_deployment/outbound.rs` | `read_json` | Fixed 8 MiB bounded-reader/JSON adapter; its reader is replaced. |
| `ingestion/rivian_auth.rs` | `From<PolicyError>::from`, `format_gql_error` | Safe error conversion and the exact count-only redaction format. |
| `ingestion/session_store.rs` | `encrypt_tokens`, `decrypt_tokens` | Typed compatibility adapters to the replaced generic JSON encryption functions. |

The evidence inventory also lists 25 unchanged embedded closures, including
fixed redacted errors, parsing predicates, and closures inside these routines.
Each is identified separately and mapped to its enclosing workflow.

`src/keys.rs` is a pure reexport; `private_deployment/mod.rs` contains module
declarations. Public data declarations and wire query strings are contracts,
not executable replacements. The old `SessionSnapshot::from` projection is
replaced by direct snapshot comparison; the image-only authentication-error
scanner is removed along with the unreachable workflow.

## Validation and operational limits

Original inline assertions and fixtures remain byte-for-byte intact. Added
tests characterize wrong keys, corrupt/truncated authenticated streams,
optional session fields, CSRF failures and cancellation, malformed backup
receipts, bundle protections, enrollment locks, and weather recovery.
Original configured DB tests exercise migration rollback, all encrypted
columns, history rejection with SQLSTATE 42501, concurrent refresh
persistence, owner enrollment, and weather admission concurrency.

Tests use synthetic keys/accounts, loopback HTTP stubs, and disposable
PostgreSQL/TimescaleDB and Redis on an internal container network without
published ports. The CLI generation check never prints private material.
Cross-version tests read original baseline token and raw backup ciphertext.

Dropping a provider request or DB future releases its owned work/transaction.
Backup encryption runs in a bounded blocking task: cancellation may allow
that computation to finish, but it cannot continue into upload after the
caller is dropped. The local archive remains untouched on failure.

Linux is the validated platform. The unchanged non-Unix denial is listed
for inspection; no non-Unix toolchain build is claimed. Integrated release,
deployment, and live-provider validation belong to the parent workflow.
