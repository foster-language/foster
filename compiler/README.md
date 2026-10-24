# Foster compiler

`foster-compiler` exposes the compiler API through `facade.rs`. Its implementation
has two dependencies:

- `foster-compiler-core` (`core/Cargo.toml`, shared source in `src/`) owns parsing,
  checking, compiled libraries, shared SSA/optimization, and bytecode generation.
- `foster-compiler-native` (`native/`) consumes checked compilations and immutable
  shared SSA to plan layouts, specialize, lower, verify, and emit native objects.
  Cranelift is confined to this crate. `NativeProgram::physical_ir()` expands ownership
  and failure exits into verified target-level CFGs. `PhysicalProgram::emit_object()`
  consumes those graphs without source metadata or ownership bookkeeping.

[`bootstrap/`](bootstrap/README.md) compiles the core sources without their generated
library. `core/build.rs` uses it to generate the bundles embedded by the core.
Native source edits rebuild the backend without rebuilding bootstrap or those
bundles. The driver still regenerates its native formatter when its compiler changes.

Executable contracts come from [bytecode](../bytecode/README.md) and platform
contracts from [host](../host/README.md). VM execution is a development dependency.

From the repository root:

```text
cargo test -p foster-compiler-core -p foster-compiler-native --lib
cargo test -p foster-compiler
python tools/integration_tests.py --suite language --jobs 2
cargo test --test backend_parity
```

Start with the [workspace boundaries](../documentation/workspace.md) and
[development policy](../documentation/development-policy.md). Compiler and shared
contract changes run all component suites in CI.
