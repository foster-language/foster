# Build-host compiler

This build dependency of `foster-compiler-core` compiles the shared `compiler/src/lib.rs`
implementation with the `foster_bootstrap` build-host configuration.
`compiler/core/build.rs` uses it to check and compile the bundled Foster library before
the compiler core embeds that artifact. The build-host compiler reads library
source, avoiding a dependency cycle on the artifact it is producing. It includes
checking, shared SSA, and bytecode generation, but excludes the native backend and
has no native-backend or Cranelift dependencies. `foster-compiler` combines the core
with the separate native backend.

Keep its dependencies aligned with `compiler/core/Cargo.toml`. Core and library
source changes rebuild the generated artifacts; native backend edits do not.
Core and native tests consume the resulting artifacts.

Use `cargo check -p foster-bootstrap --lib` to check this build configuration.
`cargo test --test workspace` checks the dependency boundary; bundled-library tests
exercise the generated artifacts with VM and native consumers.
