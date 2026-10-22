//! Legalize shared SSA into native representations and ownership operations.
mod conversions;
mod edges;
mod function;
mod instructions;
pub(super) use conversions::{ErasedConversion, callable_conversion, erased_conversion};
pub(super) use function::inference_body;
pub(super) use function::lower_shared_to_native_ir;
