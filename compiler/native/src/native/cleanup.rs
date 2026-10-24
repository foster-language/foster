//! Ownership retained at a modeled failure, before a result has been produced.
use super::*;
use std::ops::{Deref, DerefMut};

#[derive(Default)]
pub(super) struct FailureCleanup {
    pub values: HashMap<(usize, usize), Vec<ir::Value>>,
}

/// Keep the active instruction's cleanup with the builder so failures inside nested
/// lowering helpers use the same ownership boundary as direct calls and assertions.
#[derive(Clone, Copy)]
pub(super) enum CleanupSource {
    Value(ClifValue),
    Home(StackSlot),
}

pub(super) struct NativeBuilder<'a> {
    inner: cranelift_frontend::FunctionBuilder<'a>,
    pub cleanup: Vec<(CleanupSource, FuncId)>,
    /// Provisional borrowed execution-state handle, initially defined in entry.
    /// Finalization removes it or chooses a dominating acquisition block.
    pub failure_state: Option<ClifValue>,
    pub failure_reads: Vec<cranelift_codegen::ir::Inst>,
    pub uncached_failure: Option<cranelift_codegen::ir::FuncRef>,
}

impl<'a> NativeBuilder<'a> {
    pub fn new(
        function: &'a mut cranelift_codegen::ir::Function,
        context: &'a mut FunctionBuilderContext,
    ) -> Self {
        Self {
            inner: cranelift_frontend::FunctionBuilder::new(function, context),
            cleanup: Vec::new(),
            failure_state: None,
            failure_reads: Vec::new(),
            uncached_failure: None,
        }
    }

    pub fn finalize(self, config: cranelift_codegen::isa::TargetFrontendConfig) {
        if let Some(state) = self.failure_state {
            let cranelift_codegen::ir::ValueDef::Result(acquire, _) =
                self.inner.func.dfg.value_def(state)
            else {
                unreachable!("failure state is acquired by a call")
            };
            super::failure_cache::select(
                self.inner.func,
                acquire,
                &self.failure_reads,
                self.uncached_failure.expect("uncached status import"),
            );
        }
        self.inner.finalize(config);
    }
}

impl<'a> Deref for NativeBuilder<'a> {
    type Target = cranelift_frontend::FunctionBuilder<'a>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for NativeBuilder<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
