# Native formatter artifact

This package compiles `tools/driver/format.fos` into the host formatter object and
exports its text constants. Its build dependencies include compiler core and the
native backend. Native backend edits rebuild this artifact without regenerating
any [embedded bytecode tools](../embedded/README.md).

When the Cargo host and target differ, native generation and linkage are skipped;
the driver uses the bytecode formatter. Parser validation, execution state, and
cancellation remain in the driver's formatting adapter.
