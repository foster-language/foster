//! VM execution adapters used only by native backend regressions.
pub use foster_compiler_core::codegen::compile as compile_shared;
pub use foster_compiler_core::vm::*;
pub use foster_vm::{Machine, Value};

pub fn run(compilation: &crate::compiler::Compilation) -> Result<Value, crate::error::FosterError> {
    run_with_options(compilation, CompileOptions::default())
}

pub fn run_with_options(
    compilation: &crate::compiler::Compilation,
    options: CompileOptions,
) -> Result<Value, crate::error::FosterError> {
    let program = compile_with_options(compilation, options)?.into_verified()?;
    Machine::new(&program).run_main()
}
