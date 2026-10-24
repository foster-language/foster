//! The immutable boundary between specialization/verification and native emission.
use super::*;

/// Logical types and parameter ownership retained after scalar/pointer legalization.
#[derive(Debug, Clone)]
pub struct LogicalSignature {
    pub captures: Vec<ExecutableType>,
    pub parameters: Vec<crate::types::Parameter<ExecutableType>>,
    pub result: ExecutableType,
}

/// One verified specialization. Only preparation can construct or modify it.
pub struct NativeFunction {
    pub(super) instance: NativeInstance,
    pub(super) ir: ir::Function,
    pub(super) mutable_parameter_homes: HashSet<u16>,
    pub(super) home_types: std::collections::BTreeMap<u16, NativeType>,
    ownership: Vec<Vec<OwnershipEffects>>,
    logical_signature: LogicalSignature,
    management: Vec<MemoryManagement>,
    // Logical evidence keyed by the original SSA identity, retaining CFG alternatives.
    logical_value_types: Vec<Vec<ExecutableType>>,
}

impl NativeFunction {
    pub fn ownership(&self) -> &[Vec<OwnershipEffects>] {
        &self.ownership
    }

    pub fn ir(&self) -> &ir::Function {
        &self.ir
    }
    pub fn source_function(&self) -> FunctionId {
        self.instance.key.function
    }
    pub fn specialization(&self) -> &crate::codegen::types::Specialization {
        &self.instance.key.substitutions
    }
    pub fn logical_signature(&self) -> &LogicalSignature {
        &self.logical_signature
    }
    /// Lifetime-management policy for each value in the legalized SSA function.
    pub fn management(&self) -> &[MemoryManagement] {
        &self.management
    }
    /// Verified logical alternatives for each original SSA value (before ABI lowering).
    pub fn logical_value_types(&self) -> &[Vec<ExecutableType>] {
        &self.logical_value_types
    }
    /// Source-level logical alternatives for an original SSA value.
    /// ABI-only temporaries have no separate Foster identity and return an empty slice.
    pub fn logical_types(&self, value: ir::Value) -> &[ExecutableType] {
        self.logical_value_types
            .get(value.index())
            .map_or(&[], Vec::as_slice)
    }
}

/// Verified specialized native SSA, ready for complete physical CFG lowering.
///
/// Rendering and object emission consume the same immutable functions, layouts, and call targets.
/// Physical lowering borrows nominal/source metadata. Its finished artifact is independent.
pub struct NativeProgram<'a> {
    pub(super) compilation: &'a Compilation,
    canonical: std::sync::Arc<crate::codegen::shared::SharedProgram>,
    optimized: bool,
    alternate: std::sync::OnceLock<Box<NativeProgram<'a>>>,
    pub(super) program: std::sync::Arc<Program>,
    pub(super) layouts: LayoutRegistry,
    pub(super) physical_layouts: PhysicalRegistry,
    pub(super) main: FunctionId,
    pub(super) instances: Vec<NativeInstance>,
    pub(super) instance_ids: HashMap<SpecializationKey, FunctionId>,
    pub(super) function_types: HashMap<FunctionId, ir::Signature>,
    pub(super) builtin_result_types: HashMap<crate::intrinsics::Builtin, ExecutableType>,
    pub(super) runtime_strings: Vec<String>,
    pub(super) runtime_string_indices: HashMap<u16, u64>,
    pub(super) runtime_literal_indices: HashMap<String, u64>,
    pub(super) functions: Vec<NativeFunction>,
    pub(super) contract_views: HashMap<LayoutId, BTreeSet<LayoutId>>,
    candidates: CandidateIndex,
}

/// Prepare once, then render or emit any number of objects without repeating specialization.
pub fn prepare(compilation: &Compilation) -> Result<NativeProgram<'_>, FosterError> {
    prepare_with_options(compilation, CompileOptions { optimize: false })
}

/// Prepare the requested optimization mode directly, without building a baseline variant.
pub fn prepare_with_options(
    compilation: &Compilation,
    options: CompileOptions,
) -> Result<NativeProgram<'_>, FosterError> {
    crate::compiler::profile::compilation("native.prepare", || {
        let shared = compilation.shared_program(false)?;
        crate::compiler::profile::measure("native.prepare", || {
            prepare_shared(compilation, shared, options.optimize)
        })
    })
}

fn prepare_shared(
    compilation: &Compilation,
    canonical: std::sync::Arc<crate::codegen::shared::SharedProgram>,
    optimized: bool,
) -> Result<NativeProgram<'_>, FosterError> {
    // Baseline preparation borrows the verified boundary. Optimized preparation
    // copies the mutable graph on write and shares unchanged analysis evidence.
    let optimized_shared = optimized
        .then(|| compilation.shared_program(true))
        .transpose()?;
    let shared = optimized_shared.as_deref().unwrap_or(canonical.as_ref());
    let (program, retained_facts) = reference_results::retained_reference_flow(shared)?;
    let facts = &retained_facts;
    let mut layouts =
        crate::compiler::profile::measure("native.clone_layouts", || shared.layouts().clone());
    if let Some(record) = program.metadata.string_record {
        layouts.instantiate_type(&ExecutableType::Record {
            record,
            arguments: Vec::new(),
        })?;
        layouts.instantiate_type(&ExecutableType::Bytes)?;
    }
    let main = program
        .metadata
        .main
        .ok_or_else(|| native_error("native compilation requires a `main` function"))?;
    let instances = crate::compiler::profile::measure("native.reachability", || {
        reachable_instances(compilation, &program, &program.bodies, main, facts)
    })?;
    let instance_ids = instances
        .iter()
        .map(|instance| (instance.key.clone(), instance.ir_function))
        .collect();
    let builtin_result_types = native_builtin_result_types(compilation)?;
    let function_types = crate::compiler::profile::measure("native.signatures", || {
        collect_function_types(
            compilation,
            &program,
            &instances,
            &builtin_result_types,
            &mut layouts,
            facts,
        )
    })?;
    // Projected mutable fields are addresses, not the objects stored at those addresses.
    // Materialize typed borrowed pointers before freezing the physical-layout registry.
    let field_types = layouts
        .layouts()
        .iter()
        .filter(|layout| layout.materialized)
        .flat_map(|layout| match &layout.kind {
            LayoutKind::Record { fields, .. } => fields
                .iter()
                .map(|field| field.ty.clone())
                .collect::<Vec<_>>(),
            LayoutKind::Variant { alternatives, .. } => alternatives
                .iter()
                .flat_map(|alternative| alternative.payload.iter().cloned())
                .collect(),
            _ => Vec::new(),
        })
        .collect::<BTreeSet<_>>();
    for ty in field_types {
        layouts.instantiate_type(&ExecutableType::Reference(Box::new(ty)))?;
    }
    let candidates = CandidateIndex::build(
        compilation,
        &program,
        &layouts,
        &instance_ids,
        &function_types,
    );
    let physical_layouts = crate::compiler::profile::measure("native.layouts", || {
        PhysicalRegistry::build(&layouts, TargetLayout::host())
    })
    .map_err(|error| native_error(format!("cannot calculate native object layouts: {error}")))?;
    validate_program(compilation, &program, &instances, &function_types, &layouts)?;
    let (runtime_strings, runtime_string_indices, runtime_literal_indices) =
        runtime_strings(&program);
    // Inference may inspect provisional layouts, but no native instructions are
    // emitted until contract views and their field storage have been settled.
    let empty_views = HashMap::new();
    let environment = NativeIrEnvironment {
        compilation,
        program: &program,
        function_types: &function_types,
        runtime_string_indices: &runtime_string_indices,
        runtime_literal_indices: &runtime_literal_indices,
        layouts: &layouts,
        physical_layouts: &physical_layouts,
        instances: &instance_ids,
        builtin_result_types: &builtin_result_types,
        contract_views: &empty_views,
        candidates: &candidates,
    };
    let function_types =
        reference_results::resolve_reference_results(&instances, environment, facts)?;
    let environment = NativeIrEnvironment {
        function_types: &function_types,
        ..environment
    };
    let representations = crate::compiler::profile::measure("native.representations", || {
        instances
            .iter()
            .map(|instance| {
                legalize::plan_function(
                    &program.bodies[&instance.key.function],
                    &facts[&instance.key.function],
                    &function_types[&instance.ir_function],
                    &instance.key,
                    environment,
                )
            })
            .collect::<Result<Vec<_>, FosterError>>()
    })?;
    let contract_views = super::contract_results::collect_views(&representations, environment)?;
    let environment = NativeIrEnvironment {
        contract_views: &contract_views,
        ..environment
    };
    let erased_fields = super::contract_results::erased_fields(environment);
    let physical_layouts = if erased_fields.is_empty() {
        physical_layouts
    } else {
        PhysicalRegistry::build_with_erased_fields(&layouts, TargetLayout::host(), &erased_fields)
            .map_err(|error| {
            native_error(format!("cannot calculate contract field layouts: {error}"))
        })?
    };
    let mut prepared = NativeProgram {
        canonical: canonical.clone(),
        optimized,
        alternate: std::sync::OnceLock::new(),
        compilation,
        program,
        layouts,
        physical_layouts,
        main,
        instances,
        instance_ids,
        function_types,
        builtin_result_types,
        runtime_strings,
        runtime_string_indices,
        runtime_literal_indices,
        functions: Vec::new(),
        contract_views,
        candidates,
    };
    for (instance, representations) in prepared.instances.iter().zip(representations) {
        let source = &prepared.program.functions[&instance.key.function];
        let source_states = &facts[&instance.key.function];
        let environment = prepared.environment();
        let (mut lowered, mut failure_cleanup) =
            crate::compiler::profile::measure("native.lower", || {
                lower_shared_to_native_ir(
                    representations,
                    source,
                    &prepared.function_types[&instance.ir_function],
                    &instance.key,
                    environment,
                )
            })?;
        if optimized {
            super::optimization::run(&mut lowered, &mut failure_cleanup);
        }
        lowered.verify(&prepared.function_types).map_err(|error| {
            native_error(format!("invalid native IR for `{}`: {error}", source.name))
        })?;
        super::contract_results::verify(&lowered, environment)?;
        for instruction in lowered.blocks.iter().flat_map(|block| &block.instructions) {
            if let ir::Instruction::RuntimeCall {
                helper, signature, ..
            } = &instruction.instruction
            {
                abi::verify_call(helper, signature).map_err(native_error)?;
            }
        }
        let layouts = NativeLayouts {
            metadata: &prepared.program.metadata,
            logical: &prepared.layouts,
            physical: &prepared.physical_layouts,
        };
        let management = lowered
            .values
            .iter()
            .map(|ty| layouts.management(*ty))
            .collect();
        let parameter_offset = lowered
            .parameters
            .len()
            .saturating_sub(source.parameters.len());
        let mut mutable_parameter_homes = lowered.parameters[parameter_offset..]
            .iter()
            .zip(source.parameters.iter().map(|p| &p.mutable))
            .filter_map(|(value, mutable)| {
                mutable
                    .then(|| lowered.values.hint(value.index()))
                    .flatten()
            })
            .collect::<HashSet<_>>();
        if compilation.hir.functions[instance.key.function]
            .receiver
            .is_some()
            && let Some(receiver) = lowered.parameters.get(parameter_offset)
            && let Some(home) = lowered.values.hint(receiver.index())
        {
            mutable_parameter_homes.insert(home);
        }
        let mut home_types = std::collections::BTreeMap::new();
        let addressable = super::optimization::addressable_homes(&lowered);
        for (value, home) in lowered.values.hints().enumerate() {
            if let Some(home) = home
                && (!optimized || addressable.contains(home))
            {
                home_types.entry(*home).or_insert(lowered.values[value]);
            }
        }
        let specialize = |ty: &ExecutableType| ty.specialize(&instance.key.substitutions);
        let logical_value_types = prepared.program.bodies[&instance.key.function]
            .values
            .iter()
            .enumerate()
            .map(|(index, _)| {
                source_states
                    .value_types(
                        prepared.program.bodies[&instance.key.function]
                            .values
                            .value(index)
                            .unwrap(),
                    )
                    .iter()
                    .map(specialize)
                    .collect()
            })
            .collect();
        let ownership = super::ownership_ir::plan(&lowered, &failure_cleanup, layouts);
        prepared.functions.push(NativeFunction {
            instance: instance.clone(),
            ir: lowered,
            mutable_parameter_homes,
            home_types,
            ownership,
            management,
            logical_signature: LogicalSignature {
                captures: source.capture_types.iter().map(specialize).collect(),
                parameters: source
                    .parameters
                    .iter()
                    .map(|p| crate::types::Parameter {
                        ty: specialize(&p.ty),
                        mode: p.mode,
                    })
                    .collect(),
                result: specialize(&source.result_type),
            },
            logical_value_types,
        });
    }
    Ok(prepared)
}

impl NativeProgram<'_> {
    /// Expand all ownership and failure behavior into verified target-level SSA.
    /// This performs no machine-code compilation. The result is independent of
    /// this program and the source compilation.
    pub fn physical_ir(&self) -> Result<PhysicalProgram, FosterError> {
        crate::compiler::profile::measure("native.physical", || {
            super::physical::lower(
                self,
                CompileOptions {
                    optimize: self.optimized,
                },
            )
        })
    }

    /// Shared logical metadata, independent of bytecode construction and native machine code.
    pub fn metadata(&self) -> &crate::codegen::metadata::ProgramMetadata {
        &self.program.metadata
    }

    pub fn functions(&self) -> &[NativeFunction] {
        &self.functions
    }
    pub fn layouts(&self) -> &LayoutRegistry {
        &self.layouts
    }
    pub fn physical_layouts(&self) -> &PhysicalRegistry {
        &self.physical_layouts
    }

    pub fn emit_ir(&self) -> String {
        use std::fmt::Write;
        let mut output = String::from("foster-codegen-ir 1\n\n");
        for function in &self.functions {
            writeln!(
                output,
                "; function #{} {:?}\n{}\n",
                function.source_function().into_raw().into_u32(),
                function.specialization(),
                function.ir
            )
            .unwrap();
            for (block, effects) in function.ownership.iter().enumerate() {
                for (instruction, effects) in effects.iter().enumerate() {
                    if effects.retain_result.is_some()
                        || effects.retain_source.is_some()
                        || effects.release.is_some()
                        || !effects.failure.releases.is_empty()
                    {
                        // Rendering must not expose randomized live-home map iteration.
                        // Keep the execution plan's cleanup order unchanged.
                        let mut effects = effects.clone();
                        effects
                            .failure
                            .releases
                            .sort_by_key(|release| match release.source {
                                OwnershipSource::Value(value) => (0, value.index()),
                                OwnershipSource::Home(home) => (1, usize::from(home)),
                            });
                        writeln!(output, "; ownership b{block}:{instruction} {effects:?}").unwrap();
                    }
                }
            }
        }
        output
    }

    pub fn compile_object(&self, options: CompileOptions) -> Result<ObjectArtifact, FosterError> {
        crate::compiler::profile::compilation("native.object", || {
            self.compile_object_inner(options)
        })
    }

    fn compile_object_inner(&self, options: CompileOptions) -> Result<ObjectArtifact, FosterError> {
        if options.optimize == self.optimized {
            let physical = self.physical_ir()?;
            return crate::compiler::profile::measure("native.emit", || physical.emit_object());
        }
        if self.alternate.get().is_none() {
            let prepared = crate::compiler::profile::measure("native.prepare", || {
                prepare_shared(self.compilation, self.canonical.clone(), options.optimize)
            })?;
            let _ = self.alternate.set(Box::new(prepared));
        }
        let physical = self.alternate.get().unwrap().physical_ir()?;
        crate::compiler::profile::measure("native.emit", || physical.emit_object())
    }

    pub fn build_executable(
        &self,
        output: impl AsRef<Path>,
        options: CompileOptions,
    ) -> Result<(), FosterError> {
        crate::compiler::profile::compilation("native.build", || {
            let object = self.compile_object(options)?;
            crate::compiler::profile::measure("native.link", || {
                runtime::link_executable(object, output.as_ref(), options)
            })
        })
    }

    pub(super) fn environment(&self) -> NativeIrEnvironment<'_> {
        NativeIrEnvironment {
            compilation: self.compilation,
            program: &self.program,
            function_types: &self.function_types,
            runtime_string_indices: &self.runtime_string_indices,
            runtime_literal_indices: &self.runtime_literal_indices,
            layouts: &self.layouts,
            physical_layouts: &self.physical_layouts,
            instances: &self.instance_ids,
            builtin_result_types: &self.builtin_result_types,
            contract_views: &self.contract_views,
            candidates: &self.candidates,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_storage_is_planned_from_portable_ssa_before_lowering() {
        let compilation = crate::compile(include_str!(
            "../../../../tests/fixtures/programs/contract_owned_results.fos"
        ))
        .unwrap();
        for optimize in [false, true] {
            let prepared = prepare_with_options(&compilation, CompileOptions { optimize }).unwrap();
            let shared = compilation.shared_program(optimize).unwrap();
            let (_, facts) = reference_results::retained_reference_flow(&shared).unwrap();
            let provisional =
                PhysicalRegistry::build(&prepared.layouts, TargetLayout::host()).unwrap();
            let empty = HashMap::new();
            let environment = NativeIrEnvironment {
                physical_layouts: &provisional,
                contract_views: &empty,
                ..prepared.environment()
            };
            let plans = prepared
                .instances
                .iter()
                .map(|instance| {
                    legalize::plan_function(
                        &prepared.program.bodies[&instance.key.function],
                        &facts[&instance.key.function],
                        &prepared.function_types[&instance.ir_function],
                        &instance.key,
                        environment,
                    )
                    .unwrap()
                })
                .collect::<Vec<_>>();
            assert!(
                plans
                    .iter()
                    .flat_map(|plan| &plan.body.blocks)
                    .flat_map(|block| &block.instructions)
                    .all(|entry| matches!(entry.instruction, ir::Instruction::Portable(_)))
            );
            let views = super::super::contract_results::collect_views(&plans, environment).unwrap();
            assert_eq!(views, prepared.contract_views);
            let environment = NativeIrEnvironment {
                contract_views: &views,
                ..environment
            };
            let erased = super::super::contract_results::erased_fields(environment);
            assert!(!erased.is_empty());
            assert_ne!(provisional, prepared.physical_layouts);
            assert_eq!(
                PhysicalRegistry::build_with_erased_fields(
                    &prepared.layouts,
                    TargetLayout::host(),
                    &erased
                )
                .unwrap(),
                prepared.physical_layouts
            );

            // Selecting physical storage must not change source-value representations.
            for (instance, plan) in prepared.instances.iter().zip(plans) {
                let final_plan = legalize::plan_function(
                    &prepared.program.bodies[&instance.key.function],
                    &facts[&instance.key.function],
                    &prepared.function_types[&instance.ir_function],
                    &instance.key,
                    prepared.environment(),
                )
                .unwrap();
                assert_eq!(plan.inferred, final_plan.inferred);
            }
        }
    }

    #[test]
    fn native_verification_rejects_unplanned_contract_views() {
        let compilation = crate::compile(include_str!(
            "../../../../tests/fixtures/programs/contract_owned_results.fos"
        ))
        .unwrap();
        for optimize in [false, true] {
            let prepared = prepare_with_options(&compilation, CompileOptions { optimize }).unwrap();
            let empty = HashMap::new();
            let environment = NativeIrEnvironment {
                contract_views: &empty,
                ..prepared.environment()
            };
            let function = prepared
                .functions
                .iter()
                .find(|function| {
                    function
                        .ir
                        .blocks
                        .iter()
                        .flat_map(|block| &block.instructions)
                        .any(|entry| {
                            matches!(entry.instruction, ir::Instruction::ContractView { .. })
                        })
                })
                .unwrap();
            function.ir.verify(&prepared.function_types).unwrap();
            let error =
                super::super::contract_results::verify(&function.ir, environment).unwrap_err();
            assert!(error.message.contains("unplanned contract view"), "{error}");
        }
    }

    #[test]
    fn native_cleanup_preserves_vm_and_avoids_nonaddressable_stack_homes() {
        for (index, source) in [
            include_str!("../../../../benchmarks/fibonacci.fos"),
            include_str!("../../../../benchmarks/native_runtime/branch_counter.fos"),
            include_str!("../../../../benchmarks/native_runtime/record_reuse.fos"),
            include_str!("../../../../benchmarks/native_runtime/list_push.fos"),
        ]
        .into_iter()
        .enumerate()
        {
            let compilation = crate::compile(source).unwrap();
            let before =
                crate::vm::encode_program(&crate::vm::compile(&compilation).unwrap()).unwrap();
            let prepared =
                prepare_with_options(&compilation, CompileOptions { optimize: true }).unwrap();
            prepared
                .compile_object(CompileOptions { optimize: true })
                .unwrap();
            assert_eq!(
                before,
                crate::vm::encode_program(&crate::vm::compile(&compilation).unwrap()).unwrap()
            );
            if index < 2 {
                assert!(
                    prepared
                        .functions()
                        .iter()
                        .all(|function| function.home_types.is_empty()),
                    "scalar workloads must not allocate addressable stack homes"
                );
            }
        }
    }

    #[test]
    fn shared_optimization_precedes_native_specialization_and_cleanup() {
        let compilation = crate::compile("func choose(flag: Bool) -> Int { return 20 + 22 if flag\n 1 / 0 }\nfunc main() -> Int { choose(true) }").unwrap();
        let baseline =
            prepare_with_options(&compilation, CompileOptions { optimize: false }).unwrap();
        let optimized =
            prepare_with_options(&compilation, CompileOptions { optimize: true }).unwrap();
        assert!(std::sync::Arc::ptr_eq(
            &baseline.program,
            baseline.canonical.program()
        ));
        assert!(!std::sync::Arc::ptr_eq(
            &optimized.program,
            optimized.canonical.program()
        ));
        assert!(optimized.functions.len() < baseline.functions.len());
        let main = optimized
            .functions
            .iter()
            .find(|function| function.source_function() == optimized.main)
            .unwrap();
        assert!(
            !main
                .ir
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
                .any(|entry| matches!(
                    entry.instruction,
                    ir::Instruction::Binary { .. } | ir::Instruction::Call { .. }
                ))
        );
        optimized
            .compile_object(CompileOptions { optimize: true })
            .unwrap();
        optimized
            .compile_object(CompileOptions { optimize: false })
            .unwrap();
        let alternate = optimized.alternate.get().unwrap() as *const _;
        optimized
            .compile_object(CompileOptions { optimize: false })
            .unwrap();
        assert_eq!(alternate, optimized.alternate.get().unwrap() as *const _);
    }

    #[test]
    fn one_prepared_program_renders_and_emits_without_mutating_its_ir() {
        let compilation = crate::compile(
            r#"
type Box<T> = { value: T }
func identity<T>(value: T) -> T { value }
func sum(left: Box<Int>, right: Box<Int>, extra: Int) -> Int {
    left.value + right.value + extra
}
func main() -> Int { sum(identity(Box { value: 20 }), Box { value: 20 }, 2) }
"#,
        )
        .unwrap();
        let prepared = prepare(&compilation).unwrap();
        let rendered = prepared.emit_ir();
        assert_eq!(rendered, super::super::emit_ir(&compilation).unwrap());
        let functions = prepared.functions.as_ptr();
        for optimize in [false, true] {
            let first = prepared
                .compile_object(CompileOptions { optimize })
                .unwrap();
            let second = prepared
                .compile_object(CompileOptions { optimize })
                .unwrap();
            assert!(
                first.bytes == second.bytes,
                "repeated emission changed object bytes"
            );
            assert_eq!(first.result, NativeType::Int);
            assert_eq!(prepared.functions.as_ptr(), functions);
            assert_eq!(prepared.emit_ir(), rendered);
        }
    }

    #[test]
    fn prepared_functions_retain_logical_identity_and_management_policy() {
        let compilation = crate::compile(
            r#"
type Box<T> = { value: T }
func identity<T>(value: T) -> T { value }
func main() -> Int {
    assert(identity(:hello) == :hello)
    assert(identity("hello") == "hello")
    identity(Box { value: 42 }).value
}
"#,
        )
        .unwrap();
        let prepared = prepare(&compilation).unwrap();
        let string = ExecutableType::Record {
            record: prepared.program.metadata.string_record.unwrap(),
            arguments: Vec::new(),
        };
        let symbol = ExecutableType::Record {
            record: prepared.program.metadata.symbol_record.unwrap(),
            arguments: Vec::new(),
        };
        for expected in [&string, &symbol] {
            let function = prepared
                .functions()
                .iter()
                .find(|function| {
                    function.ir.name.starts_with("identity")
                        && &function.logical_signature().result == expected
                })
                .unwrap();
            assert_eq!(function.ir.signature.result, NativeType::String);
            assert!(
                function
                    .logical_value_types()
                    .iter()
                    .any(|types| types.contains(expected))
            );
        }
        assert!(prepared.functions().iter().any(|function| {
            function
                .management()
                .iter()
                .any(|policy| matches!(policy, MemoryManagement::ManagedObject(_)))
        }));
        for function in prepared.functions() {
            for (ty, management) in function.ir.values.iter().zip(function.management()) {
                if *ty == NativeType::String {
                    assert!(matches!(management, MemoryManagement::ManagedObject(_)));
                }
            }
        }
        for function in prepared.functions() {
            assert_eq!(function.management().len(), function.ir.values.len());
            assert_eq!(
                prepared.program.functions[&function.source_function()]
                    .parameters
                    .iter()
                    .map(|p| p.mode)
                    .collect::<Vec<_>>(),
                function
                    .logical_signature()
                    .parameters
                    .iter()
                    .map(|p| p.mode)
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn missing_entry_is_rejected_by_preparation() {
        let compilation = crate::compile("func helper() -> Int { 42 }").unwrap();
        let error = prepare(&compilation).err().unwrap();
        assert!(error.message.contains("requires a `main` function"));
    }
}
