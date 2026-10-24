//! Host-native generation from checked Foster programs and verified shared SSA.
#![allow(clippy::result_large_err)]
use foster_compiler_core::{ast, codegen, compiler, error, hir, intrinsics, types};
#[cfg(test)]
use foster_compiler_core::{compile, entry};

mod native;
pub use native::*;

#[cfg(test)]
#[path = "test_support.rs"]
mod vm;
