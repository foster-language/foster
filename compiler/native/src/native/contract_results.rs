//! Contract views retain the original allocation and dispatch on its descriptor.
use super::*;

/// Source conformance has already been checked. This checks the native operations
/// needed to read a result through its contract, without selecting new storage.
pub(super) fn matches(
    actual: NativeType,
    expected: NativeType,
    env: NativeIrEnvironment<'_>,
) -> bool {
    let (NativeType::Object(a), NativeType::Object(b)) = (actual, expected) else {
        return false;
    };
    if matches!(env.layouts.get(a).kind, LayoutKind::Opaque)
        || matches!(env.layouts.get(b).kind, LayoutKind::Opaque)
    {
        return false;
    }
    fn visit(
        actual: NativeType,
        expected: NativeType,
        env: NativeIrEnvironment<'_>,
        seen: &mut HashSet<(NativeType, NativeType)>,
    ) -> bool {
        if actual == expected || erased_conversion(actual, expected, env.layouts).is_some() {
            return true;
        }
        let (NativeType::Object(a), NativeType::Object(b)) = (actual, expected) else {
            return false;
        };
        if !seen.insert((actual, expected)) {
            return true;
        }
        match (&env.layouts.get(a).kind, &env.layouts.get(b).kind) {
            (LayoutKind::Pointer { pointee: a, .. }, LayoutKind::Pointer { pointee: b, .. }) => {
                match (
                    native_verification_type(&env.program.metadata, env.layouts, a, None),
                    native_verification_type(&env.program.metadata, env.layouts, b, None),
                ) {
                    (Ok(a), Ok(b)) => visit(a, b, env, seen),
                    _ => false,
                }
            }
            (LayoutKind::Record { .. }, LayoutKind::Record { .. }) => {
                let (
                    PhysicalKind::Record { fields: a, .. },
                    PhysicalKind::Record { fields: b, .. },
                ) = (
                    &env.physical_layouts.get(a).kind,
                    &env.physical_layouts.get(b).kind,
                )
                else {
                    return false;
                };
                b.iter().all(|b| {
                    a.iter().find(|a| a.name == b.name).is_some_and(|a| {
                        a.ownership == b.ownership
                            && visit(
                                dispatch::field_native_type(a.value),
                                dispatch::field_native_type(b.value),
                                env,
                                seen,
                            )
                    })
                })
            }
            (
                LayoutKind::Variant {
                    variant_type: a_id, ..
                },
                LayoutKind::Variant {
                    variant_type: b_id, ..
                },
            ) if a_id == b_id => {
                let (
                    PhysicalKind::Variant {
                        alternatives: a, ..
                    },
                    PhysicalKind::Variant {
                        alternatives: b, ..
                    },
                ) = (
                    &env.physical_layouts.get(a).kind,
                    &env.physical_layouts.get(b).kind,
                )
                else {
                    return false;
                };
                a.len() == b.len()
                    && a.iter().zip(b).all(|(a, b)| {
                        a.tag == b.tag
                            && a.fields.len() == b.fields.len()
                            && a.fields.iter().zip(&b.fields).all(|(a, b)| {
                                a.ownership == b.ownership
                                    && visit(
                                        dispatch::field_native_type(a.value),
                                        dispatch::field_native_type(b.value),
                                        env,
                                        seen,
                                    )
                            })
                    })
            }
            _ => false,
        }
    }
    visit(actual, expected, env, &mut HashSet::new())
}

type Views = HashMap<LayoutId, BTreeSet<LayoutId>>;

#[derive(Default)]
struct PendingViews {
    additions: Views,
    pairs: Vec<(LayoutId, LayoutId)>,
}

impl PendingViews {
    fn record(&mut self, actual: NativeType, expected: NativeType, env: NativeIrEnvironment<'_>) {
        if actual == expected {
            return;
        }
        let (NativeType::Object(a), NativeType::Object(b)) = (actual, expected) else {
            return;
        };
        if let (LayoutKind::Pointer { pointee: a, .. }, LayoutKind::Pointer { pointee: b, .. }) =
            (&env.layouts.get(a).kind, &env.layouts.get(b).kind)
        {
            if let (Ok(a), Ok(b)) = (
                native_verification_type(&env.program.metadata, env.layouts, a, None),
                native_verification_type(&env.program.metadata, env.layouts, b, None),
            ) {
                self.record(a, b, env);
            }
            return;
        }
        // Opaque boxes have their own ABI and own a handle to the original value.
        if env
            .contract_views
            .get(&b)
            .is_some_and(|values| values.contains(&a))
            || self
                .additions
                .get(&b)
                .is_some_and(|values| values.contains(&a))
        {
            return;
        }
        if !matches(actual, expected, env)
            || matches!(
                env.layouts.get(a).kind,
                LayoutKind::Opaque | LayoutKind::Pointer { .. }
            )
            || matches!(
                env.layouts.get(b).kind,
                LayoutKind::Opaque | LayoutKind::Pointer { .. }
            )
        {
            return;
        }
        if !self.additions.entry(b).or_default().insert(a) {
            return;
        }
        self.pairs.push((b, a));
        let mut fields =
            |a: &[crate::codegen::layout::physical::FieldLayout],
             b: &[crate::codegen::layout::physical::FieldLayout]| {
                for b in b {
                    if let Some(a) = a.iter().find(|a| a.name == b.name) {
                        self.record(
                            dispatch::field_native_type(a.value),
                            dispatch::field_native_type(b.value),
                            env,
                        );
                    }
                }
            };
        match (
            &env.physical_layouts.get(a).kind,
            &env.physical_layouts.get(b).kind,
        ) {
            (PhysicalKind::Record { fields: a, .. }, PhysicalKind::Record { fields: b, .. }) => {
                fields(a, b)
            }
            (
                PhysicalKind::Variant {
                    alternatives: a, ..
                },
                PhysicalKind::Variant {
                    alternatives: b, ..
                },
            ) => {
                for (a, b) in a.iter().zip(b) {
                    fields(&a.fields, &b.fields);
                }
            }
            _ => {}
        }
    }
}

#[derive(Hash, PartialEq, Eq)]
struct ViewCall {
    slot: crate::types::DispatchSlot,
    receiver: NativeType,
    arguments: Vec<NativeType>,
    result: NativeType,
}

#[derive(Default)]
struct ViewWork {
    calls: usize,
    pairs: usize,
    rounds: usize,
}

pub(super) fn collect_views(
    functions: &[legalize::FunctionRepresentations],
    environment: NativeIrEnvironment<'_>,
) -> Result<Views, FosterError> {
    Ok(collect_views_with_work(functions, environment)?.0)
}

fn collect_views_with_work(
    functions: &[legalize::FunctionRepresentations],
    environment: NativeIrEnvironment<'_>,
) -> Result<(Views, ViewWork), FosterError> {
    let views = Views::new();
    let mut pending = PendingViews::default();
    let mut calls = Vec::new();
    let mut unique_calls = HashSet::new();
    let initial = NativeIrEnvironment {
        contract_views: &views,
        ..environment
    };
    // Read portable SSA once. Conversion requirements do not depend on discovered
    // views; only calls with a statically selected receiver need subscriptions.
    for function in functions {
        let ty = |value: ir::Value| function.inferred.types[value.index()];
        let mut conversion = |actual, expected| -> Result<(), FosterError> {
            if matches!(
                legalize::ReturnConversion::between(actual, expected, initial)?,
                Some(legalize::ReturnConversion::ContractView)
            ) {
                pending.record(actual, expected, initial);
            }
            Ok(())
        };
        conversion_arguments(
            &function.body.entry_arguments,
            &function.body.blocks[function.body.entry.0 as usize].parameters,
            function,
            &mut conversion,
        )?;
        for block in &function.body.blocks {
            match &block.terminator {
                ir::Terminator::Return(value) => conversion(ty(*value), function.result)?,
                ir::Terminator::Jump { target, arguments } => conversion_arguments(
                    arguments,
                    &function.body.blocks[target.0 as usize].parameters,
                    function,
                    &mut conversion,
                )?,
                ir::Terminator::Branch {
                    then_target,
                    then_arguments,
                    else_target,
                    else_arguments,
                    ..
                } => {
                    for (target, arguments) in
                        [(then_target, then_arguments), (else_target, else_arguments)]
                    {
                        conversion_arguments(
                            arguments,
                            &function.body.blocks[target.0 as usize].parameters,
                            function,
                            &mut conversion,
                        )?;
                    }
                }
                ir::Terminator::Unreachable => {}
            }
        }
        for entry in function
            .body
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
        {
            if let ir::Instruction::Portable(Instruction::CallContractMethod {
                destination,
                receiver,
                slot,
                arguments,
                ..
            }) = &entry.instruction
            {
                let call = ViewCall {
                    slot: *slot,
                    receiver: dereference_native_type(ty(*receiver), initial)?,
                    arguments: arguments.iter().map(|value| ty(*value)).collect(),
                    result: ty(*destination),
                };
                if unique_calls.insert((
                    call.slot,
                    call.receiver,
                    call.arguments.clone(),
                    call.result,
                )) {
                    calls.push(call);
                }
            }
        }
    }
    settle_views(pending, &calls, environment)
}

fn settle_views(
    mut pending: PendingViews,
    calls: &[ViewCall],
    environment: NativeIrEnvironment<'_>,
) -> Result<(Views, ViewWork), FosterError> {
    let mut views = Views::new();
    let mut parents = Views::new();
    let mut observers = HashMap::<LayoutId, Vec<usize>>::new();
    for (index, call) in calls.iter().enumerate() {
        if !specialization::dynamic_receiver(call.slot, call.receiver, environment)
            && let NativeType::Object(layout) = call.receiver
        {
            observers.entry(layout).or_default().push(index);
        }
    }
    let mut work = ViewWork::default();
    let mut queue = (0..calls.len()).collect::<BTreeSet<_>>();
    let mut delta = Vec::new();
    loop {
        work.rounds += 1;
        // Keep round semantics: calls see the previous round's complete relation.
        // New edges are buffered, so an ABI choice cannot depend on queue order.
        let env = NativeIrEnvironment {
            contract_views: &views,
            ..environment
        };
        for &index in &queue {
            work.calls += 1;
            crate::compiler::profile::count("native.views.call");
            let call = &calls[index];
            let candidates = contract_candidates(call.slot, call.receiver, &call.arguments, env)?;
            let converted = candidates
                .first()
                .map(|candidate| &env.function_types[&candidate.function].parameters[1..]);
            let candidates = if let Some(arguments) = converted
                && arguments != call.arguments.as_slice()
            {
                contract_candidates(call.slot, call.receiver, arguments, env)?
            } else {
                candidates
            };
            for candidate in candidates {
                pending.record(
                    env.function_types[&candidate.function].result,
                    call.result,
                    env,
                );
            }
        }
        // Each new edge participates in both directions of transitive closure.
        // Old/old compositions were handled when their last edge was introduced.
        for &(expected, actual) in &delta {
            work.pairs += 1;
            crate::compiler::profile::count("native.views.pair");
            if let Some(ancestors) = views.get(&actual) {
                for &ancestor in ancestors {
                    pending.record(
                        NativeType::Object(ancestor),
                        NativeType::Object(expected),
                        env,
                    );
                }
            }
            if let Some(contracts) = parents.get(&expected) {
                for &contract in contracts {
                    pending.record(
                        NativeType::Object(actual),
                        NativeType::Object(contract),
                        env,
                    );
                }
            }
        }
        if pending.pairs.is_empty() {
            return Ok((views, work));
        }
        queue.clear();
        delta = pending.pairs;
        for &(expected, actual) in &delta {
            views.entry(expected).or_default().insert(actual);
            parents.entry(actual).or_default().insert(expected);
            if let Some(affected) = observers.get(&expected) {
                queue.extend(affected);
            }
        }
        pending = PendingViews::default();
    }
}

// Empty tokens have no runtime storage and are retyped rather than converted.
fn conversion_arguments(
    arguments: &[ir::Value],
    parameters: &[ir::Value],
    function: &legalize::FunctionRepresentations,
    conversion: &mut impl FnMut(NativeType, NativeType) -> Result<(), FosterError>,
) -> Result<(), FosterError> {
    for (argument, parameter) in arguments.iter().zip(parameters) {
        if !function.inferred.empty.contains(argument)
            && !function.body.entry_seeds.contains(argument)
        {
            conversion(
                function.inferred.types[argument.index()],
                function.inferred.types[parameter.index()],
            )?;
        }
    }
    Ok(())
}

/// ContractView's generic SSA check only establishes pointer-shaped operands.
/// Native verification must also establish that descriptor-based access was planned.
pub(super) fn verify(
    function: &ir::Function,
    environment: NativeIrEnvironment<'_>,
) -> Result<(), FosterError> {
    for entry in function.blocks.iter().flat_map(|block| &block.instructions) {
        let ir::Instruction::ContractView {
            source,
            destination,
        } = entry.instruction
        else {
            continue;
        };
        let actual = function.value_type(source);
        let expected = function.value_type(destination);
        if !matches(actual, expected, environment) {
            return Err(native_error(format!(
                "invalid contract view in `{}`: {actual:?} to {expected:?}",
                function.name
            )));
        }
        let actual = dereference_native_type(actual, environment)?;
        let expected = dereference_native_type(expected, environment)?;
        if actual != expected
            && let (NativeType::Object(actual), NativeType::Object(expected)) = (actual, expected)
            && !matches!(environment.layouts.get(actual).kind, LayoutKind::Opaque)
            && !matches!(environment.layouts.get(expected).kind, LayoutKind::Opaque)
            && !environment
                .contract_views
                .get(&expected)
                .is_some_and(|layouts| layouts.contains(&actual))
        {
            return Err(native_error(format!(
                "unplanned contract view in `{}`: l{} to l{}",
                function.name, actual.0, expected.0
            )));
        }
    }
    Ok(())
}

pub(super) fn erased_fields(environment: NativeIrEnvironment<'_>) -> BTreeSet<(LayoutId, String)> {
    let mut fields = BTreeSet::new();
    let opaque = environment.layouts.opaque();
    for (&expected, actuals) in environment.contract_views {
        for &actual in actuals {
            let mut compare = |actual_fields: &[crate::codegen::layout::physical::FieldLayout], expected_fields: &[crate::codegen::layout::physical::FieldLayout]| {
                for expected in expected_fields {
                    if expected.value.pointee == Some(opaque)
                        && let Some(field) = actual_fields.iter().find(|field| field.name == expected.name)
                        && field.value.pointee != Some(opaque) {
                        fields.insert((actual, field.name.clone()));
                    }
                }
            };
            match (
                &environment.physical_layouts.get(actual).kind,
                &environment.physical_layouts.get(expected).kind,
            ) {
                (
                    PhysicalKind::Record { fields: a, .. },
                    PhysicalKind::Record { fields: b, .. },
                ) => compare(a, b),
                (
                    PhysicalKind::Variant {
                        alternatives: a, ..
                    },
                    PhysicalKind::Variant {
                        alternatives: b, ..
                    },
                ) => {
                    for (a, b) in a.iter().zip(b) {
                        compare(&a.fields, &b.fields);
                    }
                }
                _ => {}
            }
        }
    }
    fields
}

/// Select physical operations using the original allocation's descriptor. Only
/// layouts actually returned through this view need a branch; other reads stay static.
pub(super) fn with_layout(
    builder: &mut FunctionBuilder<'_>,
    module: &mut PhysicalModule,
    object: ClifValue,
    expected: LayoutId,
    results: &[NativeType],
    backend: &NativeBackend<'_>,
    mut lower: impl FnMut(
        &mut FunctionBuilder<'_>,
        &mut PhysicalModule,
        LayoutId,
    ) -> Result<Vec<ClifValue>, FosterError>,
) -> Result<Vec<ClifValue>, FosterError> {
    let Some(actuals) = backend.ir.contract_views.get(&expected) else {
        return lower(builder, module, expected);
    };
    let word = module.target_config().pointer_type();
    let descriptor = builder.ins().load(
        word,
        MemFlagsData::trusted(),
        object,
        backend.ir.physical_layouts.header().descriptor_offset as i32,
    );
    let join = builder.create_block();
    for ty in results {
        builder.append_block_param(join, cranelift_type(*ty, word));
    }
    for &actual in actuals {
        if actual == expected {
            continue;
        }
        let selected = builder.create_block();
        let next = builder.create_block();
        let data = module.declare_data_in_func(backend.objects.descriptors[&actual], builder.func);
        let expected_descriptor = builder.ins().symbol_value(word, data);
        let matches = builder
            .ins()
            .icmp(IntCC::Equal, descriptor, expected_descriptor);
        builder.ins().brif(matches, selected, &[], next, &[]);
        builder.switch_to_block(selected);
        let values = lower(builder, module, actual)?;
        builder.ins().jump(
            join,
            &values
                .iter()
                .map(|value| (*value).into())
                .collect::<Vec<_>>(),
        );
        builder.switch_to_block(next);
    }
    let values = lower(builder, module, expected)?;
    builder.ins().jump(
        join,
        &values
            .iter()
            .map(|value| (*value).into())
            .collect::<Vec<_>>(),
    );
    builder.switch_to_block(join);
    Ok(builder.block_params(join).to_vec())
}

#[cfg(test)]
mod tests;
