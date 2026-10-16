# Recovery operations

Shared package admission, catalog recovery, compatibility planning, restore
journals and workspace ownership live in the recovery operations services.
The implementation preserves PANDA/panda, R:The Drive behavior and existing
GPL/NOTICE attribution. It is informed by the existing implementation; it is
not a clean-room implementation or an exclusive-authorship claim.

## Gzip integrity correction

Standalone admission previously accepted a complete tar archive when its
surrounding gzip stream lacked its eight-byte trailer. The regression fixture
produced `standalone admission accepted gzip trailer case 0` before the fix.
Corrupt trailers could also escape verification because tar EOF arrived first.

Admission now consumes gzip through its integrity trailer. This additional
decompression has a separate budget: the unused configured expanded-byte
allowance plus 64 KiB for tar padding. An archive with 128 KiB of trailing
expansion and no remaining content budget previously passed and now fails with
`RecoveryTooLarge`; ordinary 8 KiB padding remains accepted. Missing or corrupt
trailers fail as invalid packages.

This is an intentional correctness change, separate from parity refactoring.
Valid v1/v2/v3 formats, component checksums, database magic, migration metadata,
complete compressed-file checksums (including suffix bytes), and public
interfaces remain compatible. Malformed packages that relied on incomplete
gzip verification must be regenerated.
