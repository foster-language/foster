# Rust workspace

Foster has eight primary Rust packages, a build-host bootstrap compiler, and six
embedded-tool artifact packages in one
repository. The installed command is `foster`; the other primary packages expose
reusable implementation libraries.

Each implementation area has a short entry guide: [driver](../src/README.md),
[compiler](../compiler/README.md), [bytecode](../bytecode/README.md),
[VM](../vm/README.md), [runtime](../runtime/README.md), and [host](../host/README.md).
The [repository map](../README.md#working-in-the-repository) also points to optional
packages, editor integration, component tests, benchmarks, and research.

| Package | Source | Responsibility |
| --- | --- | --- |
| `foster` | `src/` | Toolchain driver: commands, project discovery and dependency resolution, formatting, executable packaging, documentation, language server, debugger, and C bridge builds |
| `foster-compiler` | `compiler/facade.rs` | Compiler API combining shared compilation with native generation |
| `foster-compiler-core` | `compiler/src/`, manifest in `compiler/core/` | Parsing, checking, compiled libraries, shared SSA/optimization, and bytecode generation |
| `foster-compiler-native` | `compiler/native/src/` | Native specialization, layout planning, verification, Cranelift emission, and executable linking |
| `foster-vm` | `vm/src/` | Execution state, values, builtin handlers, and bytecode execution |
| `foster-bytecode` | `bytecode/src/` | Executable identities and types, instructions, metadata, intrinsic contracts, encoding/decoding, linking, and validation |
| `foster-native-runtime` | `runtime/src/` | Runtime support linked into native Foster executables |
| `foster-host` | `host/src/` | Platform services, processes, foreign calls and callbacks, and shared remote lifecycle control |
| `foster-bootstrap` | `compiler/bootstrap/` | Build-host compiler used to generate the embedded library before the ordinary compiler builds |
| `foster-tool-*` | `tools/embedded/` | Five independent bytecode artifacts for initialization, projects, formatting, documentation, and the C bridge |
| `foster-native-formatter` | `tools/native-formatter/` | Native formatter object and constants, generated independently of bytecode tools |

## Dependency boundaries

The VM depends on bytecode and host services. Neither bytecode nor host services
has a production dependency on the compiler or toolchain driver. The native
runtime depends on host services.

The compiler core depends on bytecode and host contracts. It does not depend on VM
execution in production. The toolchain driver combines compilation and execution
and exposes its orchestration API from `src/lib.rs` and `src/vm.rs`.

The native backend depends on the core's checked compilation and read-only shared
SSA interfaces. The core and build-host bootstrap compiler have no dependency on
the native backend or Cranelift. Bootstrap compiles the same core source without
its generated library; the core build script generates and embeds those bundles.
Native-only edits leave bootstrap, bundled libraries, and all five embedded bytecode
tools unchanged. Each bytecode artifact has its own build script instance and depends
only on compiler core. The native formatter has a separate build unit that depends
on core and native generation. The driver build script only selects the host formatter
configuration; it has no compiler build dependency. Changing one tool source rebuilds
only its artifact (both formatter artifacts when formatting policy changes).

Bytecode also owns the executable SSA representation and logical flow engine
used by both the compiler and bytecode verifier. Keeping this shared validation
code together prevents compilation and decoding from checking different contracts.
Source AST, HIR declarations, semantic types, and ownership analysis remain in the
compiler. Executable IDs use declaration-independent markers; the compiler maps
those identities to its declaration arenas.

Intrinsic descriptions contain data, not pointers into the VM. The VM registers
its own handlers and tests their consistency with the shared execution contracts.

The project manager implements the compiler's `package::ProjectInputs` interface.
It resolves manifests and dependency locations; the compiler consumes those
locations, resolves imported names, and checks visibility and semantics. The
formatter uses the compiler parser. The debugger requests bytecode plus its local
register map through `vm::compile_debug`.

Project creation, manifest validation, library discovery, dependency traversal,
formatting policy, and documentation page generation are written in Foster under [`tools/driver`](../tools/driver/README.md).
The Rust build compiles these tools into embedded bytecode; the driver loads them once per process. Rust adapters retain
platform path ancestry, CLI parsing, parser diagnostics, and compiler-facing data
conversion. Documentation adapters supply resolved signatures, type links, and
Markdown HTML; Foster assembles and writes the site. Installed commands do not
depend on a source checkout.

Some unit tests use a **development-only** compiler dependency to produce real
executable fixtures. Bytecode tests transfer those fixtures through the serialized
format. This does not add the compiler to an ordinary VM or bytecode build.

## Build and test

From the repository root:

```powershell
cargo build --release --bin foster
cargo check -p foster-vm
cargo check -p foster-bytecode
cargo test --workspace
cargo test --test agent_documentation
python tools/integration_tests.py --suite language runtime --jobs 2
cargo fmt --all -- --check
```

The workspace default members include all eight primary packages and six artifact packages.
Existing CLI commands,
Foster package layout, and executable formats are unchanged. Native runtime source
assembly follows the owning packages when producing its cached standalone runtime.

Integration source is grouped by component while Cargo target names remain
registered in the root manifest. See [tests](../tests/README.md) for focused runs
and [CI selection](../tools/ci/README.md) for change-based coverage. Shared
compiler/runtime/ABI/library changes run the broad correctness gates.
