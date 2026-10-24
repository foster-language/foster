//! Machine-code emission consumes fully expanded physical native IR.
use super::*;

impl PhysicalProgram {
    /// Consume verified CFGs without consulting ownership or source-level facts.
    pub fn emit_object(mut self) -> Result<ObjectArtifact, FosterError> {
        for function in self.functions {
            let mut context = self.module.make_context();
            context.func = function.body;
            self.module
                .define_function(function.id, &mut context)
                .map_err(|error| {
                    native_error(format!(
                        "cannot compile physical native function {}: {error}",
                        function.id.as_u32()
                    ))
                })?;
        }
        let bytes =
            self.module.finish().emit().map_err(|error| {
                native_error(format!("cannot encode the native object: {error}"))
            })?;
        Ok(ObjectArtifact {
            bytes,
            result: self.result,
            accepts_arguments: self.accepts_arguments,
            runtime_strings: self.runtime_strings,
            releases_result: self.releases_result,
        })
    }
}
