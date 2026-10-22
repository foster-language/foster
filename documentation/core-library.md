# Standard library and intrinsic boundary

Use the [library guide](../library/README.md) to choose APIs and
`foster documentation library --serve` for signatures, effects, ownership, and
failure behavior. The `.fos` declarations and comments are authoritative.

The library implements text algorithms, collections, parsers, calendar arithmetic,
random distributions, and resource wrappers in Foster. Runtime intrinsics supply
storage primitives, execution services, and operating-system access. Host-facing
intrinsics are private; public Foster wrappers validate inputs and return typed
errors. See [host providers](host-providers.md) for embedding configuration and
[the contract inventory](library-contract-audit.md) for supported compositions.

## Compiler intrinsics

Editor navigation targets this table for builtins without a Foster implementation
body. These entries describe the runtime boundary; applications should use the
public library wrappers where available.

| Intrinsic | Purpose |
| --- | --- |
| `print`, `println` | Write values to standard output, without or with a trailing newline |
| `code_point`, `from_code_point` | Explicit widening and checked construction of `CodePoint`; ordinary widening applies in `Int` contexts and integer operators |
| `parse_float` | Parse a binary64 floating-point value from text |
| `FloatHost.format` | Format a binary64 value as round-trippable scalar text |
| `FsHost.read_bytes`, `FsHost.write_bytes` | Perform whole-file binary operations; Foster wrappers implement UTF-8 text reads and writes |
| `FsHost.list_directory` | List directory entries |
| `FsHost.exists`, `FsHost.is_file`, `FsHost.is_directory` | Query host filesystem paths |
| `FsHost.create_directory`, `FsHost.create_directory_all` | Create one directory or a directory tree |
| `FsHost.remove_file`, `FsHost.remove_directory` | Remove one file or one empty directory |
| `FsHost.rename`, `FsHost.copy_file` | Move or copy filesystem entries |
| `PathHost.join`, `PathHost.parent`, `PathHost.file_name`, `PathHost.extension` | Apply host path rules |
| `PathHost.canonicalize` | Resolve a host filesystem location |
| `EnvHost.current_directory` | Read the process working directory |
| `TcpHost.listen`, `TcpHost.connect`, `TcpHost.accept` | Establish TCP resources |
| `TcpHost.read_bytes`, `TcpHost.write_bytes` | Operate on TCP connections; Foster wrappers implement UTF-8 text reads and writes |
| `TcpHost.set_timeout` | Configure TCP connection timeouts |
| `TcpHost.wait_readable`, `TcpHost.wait_writable`, `TcpHost.wait_accept` | Wait for advisory connection or listener readiness with a timeout |
| `TcpHost.close_listener`, `TcpHost.close_connection` | Close TCP resources |
| `TimeHost.wall_now` | Read canonical Unix seconds and fractional nanoseconds from the wall clock |
| `TimeHost.monotonic_now` | Read host-context-relative monotonic nanoseconds for elapsed measurement |
| `RandomHost.bytes` | Read a bounded number of bytes from the operating system's secure entropy source |
