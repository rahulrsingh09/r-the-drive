# Recovery extraction correctness

R:The Drive preserves PANDA recovery formats, public interfaces, GPL-3.0-only,
and NOTICE attribution. The implementation is informed by existing code; no
clean-room or exclusive-authorship claim is made.

Synthetic regressions demonstrate that the prior extractor accepted a missing
gzip trailer, followed a preexisting destination parent symlink, and retained
created directories after a later file collision. Extraction now checks gzip
integrity and the complete compressed digest before acceptance, pins its output
directory, refuses symlinked member parents, and rolls back created files and
directories on failure. The configured resource limits remain in force.

Catalog path validation now checks every component before creating directories.
A regression verifies that a rejected late component has no creation side effects.
Manifest/component validation remains owned by the shared `backups` module.
No tests perform a live restore or access a provider or database.
