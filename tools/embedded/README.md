# Embedded tool artifacts

Each subdirectory is a Cargo package producing one `BYTECODE` constant. The packages
share `build.rs` and `lib.rs`; each build script watches only its Foster source and
the library. They depend on compiler core without native generation or Cranelift.
A source change regenerates only its tool. Compiler core changes rebuild all tools.

The [driver sources](../driver/README.md) contain the implementation. The C bridge
source lives in [cbind](../cbind/README.md). The separate [native formatter](../native-formatter/README.md)
package builds the host formatter object from the same formatting policy.

The root driver has no compiler build dependency and consumes these artifacts as
ordinary dependencies. Cross builds keep the bytecode formatter fallback.
