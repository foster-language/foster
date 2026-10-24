//! Shared schema conversion and analysis interfaces for executable backends.
use super::type_conversion::{self, Bytecode, Native};
use super::types::{ExecutableType, Specialization};
use crate::hir::PackageHir;
use crate::types::{TypeId, TypeInformation};

pub use type_conversion::{MAX_TYPE_DEPTH, NestingLimit, record_uses_dynamic_dispatch};

/// Convert a source type while preserving the native structural-dispatch metadata.
pub fn native_type(
    hir: &PackageHir,
    information: &TypeInformation,
    ty: TypeId,
    substitutions: &Specialization,
    depth: usize,
) -> Result<ExecutableType, NestingLimit> {
    type_conversion::convert::<Native>(hir, information, ty, substitutions, depth)
}

/// Convert aggregate arguments with the same schema policy as portable construction.
pub fn bytecode_type(
    hir: &PackageHir,
    information: &TypeInformation,
    ty: TypeId,
    substitutions: &Specialization,
    depth: usize,
) -> ExecutableType {
    type_conversion::convert::<Bytecode>(hir, information, ty, substitutions, depth)
        .unwrap_or_else(|never| match never {})
}

/// Exercise the shared scalar pass on backend IR in cross-backend regressions.
#[cfg(feature = "test-support")]
pub fn fold_scalars(
    function: &mut super::ir::Function,
    signatures: &std::collections::HashMap<crate::hir::FunctionId, super::ir::Signature>,
    constants: &[super::metadata::Constant],
) -> Result<usize, super::ir::VerifyError> {
    super::optimizer::optimize(function, signatures, constants).map(|report| report.folded_scalars)
}
