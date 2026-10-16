# Recovery artifact validation and extraction

R:The Drive keeps the PANDA recovery formats and existing public service interfaces.
This implementation retains the project's GPL-3.0-only and NOTICE attribution; it
is a rewrite informed by the existing implementation, without a clean-room or
exclusive-authorship claim.

## Admission and extraction

`backups` remains the authority for recovery v1/v2/v3 manifests, component hashes
and sizes, database magic, migration metadata, and the returned package checksum.
`recovery_extract` requires this admission before creating extraction output. Its
second pass couples a compressed-byte digest stream, member admission, and an
output transaction. Acceptance requires the complete compressed stream to match
the admitted digest. Replacing a package between these passes cannot silently
substitute different extracted content.

Member admission preserves the allowlist, regular-file/directory restriction,
portable traversal rejection, duplicate rejection, and configured compressed,
expanded, per-member, metadata, member-count, compression-ratio, and deadline
limits. Each write checks the configured free-space reserve. Buffers remain
64 KiB; file contents are streamed. Names used for admission and extraction share
the same slash normalization. Non-UTF-8 names, nonempty directory payloads, and
ambiguous duplicate directory spellings fail closed.

Output is rooted in a pinned directory handle. Parent components are opened
without following symlinks, files are created exclusively, and failures remove
only files and directories created by that extraction, in reverse order.
Preexisting files and directories are preserved. Cleanup records share path
ancestors and release them iteratively, avoiding quadratic prefix storage and
recursive destruction for deep names. Directory traversal checks the deadline
at each component. Cleanup is best effort when the
filesystem itself refuses removal; the enclosing `RecoveryWorkspace` retains its
separate lifetime-based cleanup responsibility.

Tar end-of-archive is not sufficient for acceptance: extraction drains the gzip
decoder to validate its CRC and size trailer, then consumes the remaining
compressed input to finish the package digest. Decompressed bytes after tar EOF
may consume the remaining expanded-byte budget plus at most 64 KiB of padding;
this keeps footer verification from admitting a hidden expansion bomb. Trailing bytes retain their
existing package-checksum semantics; they are not interpreted as another archive.

## Artifact files and inspection

`artifact_files` admits the entire portable catalog path before creating parent
directories. An admitted location is then resolved through directory handles.
The existing `.rma.tar.gz` suffix, managed staging exceptions, remote UUID names,
upload timestamp/UUID names, restore/host-restore paths, and publication behavior
remain unchanged. Catalog opens and deletes refuse symlinked parents and final
symlinks. Staging publication retains its existing rename semantics, including
replacement at the same service-selected final name.

Staged stream copies apply checked byte accounting and disk-reserve admission
before each write. EOF, flush, and publication complete the copy; read errors,
timeouts, and cancellation leave the temporary-file owner responsible for cleanup.
Byte staging does not imply recovery-format validation: the calling service must
still invoke the existing package validator.

Dump inspection extracts into `RecoveryWorkspace`, probes `pg_restore`, and drains
schema output and diagnostics concurrently. The probe and inspection share one
configured inspection timeout; extraction has its own bounded phase. The probe
discards its output. Schema detection preserves the existing case-sensitive
substring contract, including markers split across reads. Diagnostics retain only
the first 8 KiB while draining all output. A failed child cannot return successful
inspection flags, and dropping inspection kills the child process.

## Boundaries and validation

Synthetic tests cover all supported manifest versions, malformed JSON, missing
components, changed checksums and magic, traversal and special entry types,
resource limits, gzip corruption and truncation, partial extraction, destination
symlinks, read failures, cancellation, publication failure, path naming, and child
process failures. No test performs a restore or accesses a database or provider.

The shared `backups` validator and `RecoveryWorkspace` were not rewritten here.
Direct validation callers still use that validator's existing archive parser and
multi-pass file reads. Its hidden tar-extension allocation and concurrent
in-place modification behavior remain shared-module responsibilities. This
extractor's final digest detects differing admission/extraction bytes; it does
not make the validator's own hash and manifest passes an immutable snapshot.
The configured artifact/workspace root is trusted filesystem authority; this
change confines relative members beneath the opened destination. Windows
junction handling follows the existing capability library and needs execution
on Windows in addition to Linux tests.
