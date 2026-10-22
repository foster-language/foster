//! Retained storage layouts are independent of a reference result's source contract.
use super::*;

pub(super) fn retained_reference_flow(
    shared: &crate::codegen::shared::SharedProgram,
) -> Result<(std::sync::Arc<Program>, specialization::FlowFacts), FosterError> {
    use crate::codegen::flow::{Site, ValueFact};
    let mut program = shared.program.clone();
    let mut schemas = shared.schemas.clone();
    let mut facts = shared.facts.clone();
    for _ in 0..=schemas.len() {
        let mut changed = false;
        for (id, body) in &program.bodies {
            if !schemas[id].returns_reference {
                continue;
            }
            let returned = body
                .blocks
                .iter()
                .enumerate()
                .filter_map(|(block, body)| {
                    let ir::Terminator::Return(value) = body.terminator else {
                        return None;
                    };
                    match facts[id].at(
                        Site {
                            block: ir::Block(block as u32),
                            instruction: body.instructions.len(),
                        },
                        value,
                    ) {
                        ValueFact::Known(ty) if matches!(ty, ExecutableType::Reference(_)) => {
                            Some(ty.clone())
                        }
                        _ => None,
                    }
                })
                .collect::<Vec<_>>();
            if let Some(first) = returned.first()
                && returned.iter().all(|ty| ty == first)
                && schemas[id].result_type != *first
            {
                schemas.get_mut(id).unwrap().result_type = first.clone();
                changed = true;
            }
        }
        let mut replacements = Vec::new();
        for (id, body) in &program.bodies {
            for (block_index, block) in body.blocks.iter().enumerate() {
                for (index, entry) in block.instructions.iter().enumerate() {
                    let site = Site {
                        block: ir::Block(block_index as u32),
                        instruction: index,
                    };
                    if !facts[id].reachable(site) {
                        continue;
                    }
                    let ir::Instruction::Portable(instruction) = &entry.instruction else {
                        continue;
                    };
                    let mut replacement = instruction.clone();
                    let mut edited = false;
                    match &mut replacement {
                        Instruction::MakeWholeReference {
                            pointee_type,
                            object,
                            ..
                        } => {
                            if let ValueFact::Known(ty) = facts[id].at(site, *object) {
                                let ty = match ty {
                                    ExecutableType::Reference(value) => value.as_ref(),
                                    ty => ty,
                                };
                                if !matches!(ty, ExecutableType::Unknown) && *pointee_type != *ty {
                                    *pointee_type = ty.clone();
                                    edited = true;
                                }
                            }
                        }
                        Instruction::Call {
                            function,
                            specialization,
                            arguments,
                            ..
                        }
                        | Instruction::CallClosure {
                            function,
                            specialization,
                            arguments,
                            ..
                        } => {
                            let mut bindings = std::collections::BTreeMap::new();
                            for (parameter, argument) in
                                schemas[function].parameters.iter().zip(arguments)
                            {
                                if matches!(parameter.ty, ExecutableType::Reference(_))
                                    && let ValueFact::Known(actual) = facts[id].at(site, *argument)
                                {
                                    parameter.ty.infer_specialization(actual, &mut bindings);
                                }
                            }
                            let mut retained = specialization
                                .iter()
                                .cloned()
                                .collect::<std::collections::BTreeMap<_, _>>();
                            for (name, value) in bindings {
                                if !matches!(value, ExecutableType::Unknown) {
                                    retained.insert(name, value);
                                }
                            }
                            let retained = retained.into();
                            if *specialization != retained {
                                *specialization = retained;
                                edited = true;
                            }
                        }
                        Instruction::CallMethod {
                            function,
                            specialization,
                            receiver,
                            arguments,
                            ..
                        } => {
                            let mut bindings = std::collections::BTreeMap::new();
                            for (parameter, argument) in schemas[function]
                                .parameters
                                .iter()
                                .zip(std::iter::once(&*receiver).chain(arguments.iter()))
                            {
                                if matches!(parameter.ty, ExecutableType::Reference(_))
                                    && let ValueFact::Known(actual) = facts[id].at(site, *argument)
                                {
                                    parameter.ty.infer_specialization(actual, &mut bindings);
                                }
                            }
                            let mut retained = specialization
                                .iter()
                                .cloned()
                                .collect::<std::collections::BTreeMap<_, _>>();
                            for (name, value) in bindings {
                                if !matches!(value, ExecutableType::Unknown) {
                                    retained.insert(name, value);
                                }
                            }
                            let retained = retained.into();
                            if *specialization != retained {
                                *specialization = retained;
                                edited = true;
                            }
                        }
                        _ => {}
                    }
                    if edited {
                        replacements.push((*id, block_index, index, replacement));
                    }
                }
            }
        }
        changed |= !replacements.is_empty();
        if !changed {
            return Ok((program, facts));
        }
        for (id, block, index, replacement) in replacements {
            std::sync::Arc::make_mut(&mut program)
                .bodies
                .get_mut(&id)
                .unwrap()
                .blocks[block]
                .instructions[index]
                .instruction = ir::Instruction::Portable(replacement);
        }
        for (id, body) in &program.bodies {
            let evidence = crate::codegen::flow::analyze(
                &program.metadata,
                &schemas,
                &schemas[id],
                body,
                &shared.write_bindings[id],
            )?;
            facts.insert(*id, std::sync::Arc::new(evidence));
        }
    }
    Err(native_error("retained reference flow did not converge"))
}

impl NativeProgram<'_> {
    pub(super) fn resolve_reference_results(
        &mut self,
        facts: &specialization::FlowFacts,
    ) -> Result<(), FosterError> {
        let declared = self.function_types.clone();
        // A caller can forward another function's returned reference. Settle those
        // dependencies before emitting any caller; source conformance was checked
        // independently, so a contract annotation never selects a pointee layout.
        for _ in 0..=self.instances.len() {
            let mut updates = Vec::new();
            let environment = self.environment();
            for instance in &self.instances {
                let metadata = &self.program.functions[&instance.key.function];
                if !metadata.returns_reference {
                    continue;
                }
                let source = &self.program.bodies[&instance.key.function];
                let states = &facts[&instance.key.function];
                let body = legalize::inference_body(source, states, &instance.key, environment);
                let remote = verified_remote_calls(&body, states, &instance.key, environment)?;
                let signature = &self.function_types[&instance.ir_function];
                let inferred = infer_value_types(
                    &body,
                    &signature.parameters,
                    &instance.key,
                    environment,
                    &remote,
                )?;
                let placeholder = declared[&instance.ir_function].result;
                let results = body
                    .blocks
                    .iter()
                    .filter_map(|block| {
                        let ir::Terminator::Return(value) = block.terminator else {
                            return None;
                        };
                        Some(inferred.types[value.index()])
                    })
                    .collect::<Vec<_>>();
                let retained = results
                    .iter()
                    .copied()
                    .find(|ty| *ty != placeholder)
                    .or_else(|| results.iter().next().copied());
                let Some(retained) = retained else {
                    continue;
                };
                // Initial call results can still carry the declared contract while
                // a recursive callee is unresolved. The final IR verifier checks
                // every returned value against the settled signature.
                if results
                    .iter()
                    .any(|ty| *ty != retained && *ty != placeholder)
                    || dereference_native_type(retained, environment)? == retained
                {
                    continue;
                }
                if signature.result != retained {
                    updates.push((instance.ir_function, retained));
                }
            }
            if updates.is_empty() {
                return Ok(());
            }
            for (function, result) in updates {
                self.function_types.get_mut(&function).unwrap().result = result;
            }
        }
        Err(native_error(
            "reference result layout inference did not converge",
        ))
    }
}

#[cfg(test)]
mod tests {
    const SOURCE: &str =
        include_str!("../../../tests/fixtures/programs/contract_reference_results.fos");

    #[test]
    fn contract_reference_results_preserve_returned_storage() {
        let compilation = crate::compile(SOURCE).unwrap();
        for optimize in [false, true] {
            assert_eq!(
                crate::vm::run_with_options(&compilation, crate::vm::CompileOptions { optimize })
                    .unwrap(),
                crate::vm::Value::Integer(42)
            );
            crate::native::compile_object(&compilation, crate::native::CompileOptions { optimize })
                .unwrap();
        }
    }

    #[test]
    fn contract_reference_results_expose_only_the_return_contract() {
        let source = SOURCE.replace(
            "first_borrow.read() == 21",
            "first_borrow.extra == \"first\"",
        );
        let error = crate::compile(&source).unwrap_err();
        assert!(error.message.contains("extra"), "{error}");
        assert!(!error.message.contains("$reference:"));
    }

    #[test]
    fn contract_reference_results_reject_nonconforming_returns() {
        let source = r#"
type Value = { pub value: Int }
type Wrong = { pub text: String }
func invalid(item: ref[item] Wrong) -> ref[item] Value { ref item }
func main() -> Int { 42 }
"#;
        let error = crate::compile(source).unwrap_err();
        assert!(error.message.contains("value"), "{error}");
    }

    #[test]
    fn contract_reference_results_reject_frame_local_storage() {
        let source = r#"
type Value = { pub value: Int }
type Item = { pub extra: Int, pub value: Int }
func invalid(item: ref[item] Value) -> ref[item] Value {
    let local = Item { extra: 9, value: 42 }
    ref local
}
func main() -> Int { 42 }
"#;
        assert!(crate::compile(source).is_err());
    }
}
