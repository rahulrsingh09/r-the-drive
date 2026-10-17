# Maintenance commands and database coordination

This implementation replaces inherited maintenance and database coordination
routines while retaining the existing command names, output formats, schema
identities, and authorization rules. It remains part of the attributed project;
implementation replacement does not change its provenance or licensing.

Database command entry points await pool shutdown on both successful and failed
operations. A command's operation error remains its result. The argument decoder
in `db::maintenance` consumes options as a stream of switches and operands. Each
command specifies its accepted options and existing help text. Duplicate values
retain the last value; help stops parsing immediately unless it is being consumed
as an option's required operand. The sensor audit retains its separate historical
first-vehicle-option behavior.

Historical stamping and software version reconstruction consume bounded database
cursor pages. Stamping preserves chronological priority and commits each update
separately, so an error leaves earlier successful work in place. State reconstruction
uses one open interval, closing it on a normalized state change or a gap exceeding
600 seconds. Null samples remain unknown in the general state backfill; vehicle
history replay excludes samples without power state. Software runs close at the
first timestamp of the next distinct version, with a null end on the final run.

Trip energy projection shares one conversion from detector output to stored
energy, strategy, and efficiency columns. Odometer repair reads both endpoints
within each trip's update statement. It keeps existing endpoint values when the
corresponding telemetry value is absent. Sensor availability aggregates per
vehicle before grouping by model, including vehicles with no matching samples.
Payload compaction measures storage, pending identities, and duplicate counts in
one query. Window ranking selects the linked, oldest keeper before bounded
victim selection; alias and canonical-identity updates remain atomic with deletion.

The migration coordinator validates the public and legacy ledgers before choosing
fresh initialization, public continuation, legacy relocation, or redundant-ledger
removal. Both ledgers are read before validation when both exist, preserving error
precedence. The same transactional writer serves initialization, relocation,
restore, and baseline adoption. It validates identities, decodes checksums, and
inserts the ledger through parallel PostgreSQL arrays. Migration progress reporting
runs in the migration future instead of a detached background task.

The immutable SQL catalog, version namespace, descriptions, checksums, canonical
JSON digest, and baseline identity are unchanged. Baseline adoption still requires
schema equality, stopped writers, explicit confirmation, and verified backup
coverage. Temporary verification pools close before their disposable database is
dropped, including on errors. Timescale telemetry is disabled on each new
verification database before it is opened, so its reporter cannot hold the
database during teardown. Cleanup then disables new connections to that exact
command-created database before forced drop.

Vehicle membership, owner/manager restrictions, integration-key scope, and user
role checks retain their public contracts. Vehicle operations select a typed
access policy before the shared membership query, so rejected API keys never
reach the database. User role lookup evaluates the allowed roles in the same
query that reads the stored role, retaining the distinction between missing,
unrecognized, and insufficient roles. Prefix and complete-ledger validation use
one ordered comparison with the same error precedence.

The normal application connection path
continues to resolve trusted extension functions before temporary or public
objects. Maintenance commands do not grant privileges or remove history
protection. The historical rebuild and deletion-based backfills remain refused by
protected runtime roles.

Archive input remains bounded to 20 MiB. Encryption/decryption completes before a
new output file is created with mode 0600, and existing files are never overwritten.
Image export retains the optional-traffic gate; its transport tests call the private
transport routine on loopback without enabling optional production traffic.

Validation includes original frozen CLI/database/interruption contracts, catalog
and migration rollback tests, restricted-role geospatial tests, additional option
and authorization cases, isolated timeline databases, archive process round-trips,
and loopback image transport failures. No test requires a live provider or account.
