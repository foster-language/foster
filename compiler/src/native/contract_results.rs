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

pub(super) fn collect_views(
    prepared: &NativeProgram<'_>,
) -> Result<HashMap<LayoutId, BTreeSet<LayoutId>>, FosterError> {
    fn record(
        actual: NativeType,
        expected: NativeType,
        env: NativeIrEnvironment<'_>,
        views: &mut HashMap<LayoutId, BTreeSet<LayoutId>>,
    ) {
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
                record(a, b, env, views);
            }
            return;
        }
        // Opaque boxes have their own ABI and own a handle to the original value.
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
        if !views.entry(b).or_default().insert(a) {
            return;
        }
        let mut fields =
            |a: &[crate::codegen::layout::physical::FieldLayout],
             b: &[crate::codegen::layout::physical::FieldLayout]| {
                for b in b {
                    if let Some(a) = a.iter().find(|a| a.name == b.name) {
                        record(
                            dispatch::field_native_type(a.value),
                            dispatch::field_native_type(b.value),
                            env,
                            views,
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
    let mut views = HashMap::new();
    loop {
        let before = views.clone();
        let mut env = prepared.environment();
        env.contract_views = &before;
        for function in &prepared.functions {
            for entry in function
                .ir
                .blocks
                .iter()
                .flat_map(|block| &block.instructions)
            {
                if let ir::Instruction::ContractView {
                    destination,
                    source,
                } = entry.instruction
                {
                    record(
                        function.ir.value_type(source),
                        function.ir.value_type(destination),
                        env,
                        &mut views,
                    );
                }
                if let ir::Instruction::Portable(Instruction::CallContractMethod {
                    destination,
                    receiver,
                    slot,
                    arguments,
                    ..
                }) = &entry.instruction
                {
                    let arguments = arguments
                        .iter()
                        .map(|argument| function.ir.value_type(*argument))
                        .collect::<Vec<_>>();
                    for candidate in contract_candidates(
                        *slot,
                        dereference_native_type(function.ir.value_type(*receiver), env)?,
                        &arguments,
                        env,
                    )? {
                        record(
                            env.function_types[&candidate.function].result,
                            function.ir.value_type(*destination),
                            env,
                            &mut views,
                        );
                    }
                }
            }
        }
        for (&expected, actuals) in &before {
            for &actual in actuals {
                if let Some(ancestors) = before.get(&actual) {
                    for &ancestor in ancestors {
                        record(
                            NativeType::Object(ancestor),
                            NativeType::Object(expected),
                            env,
                            &mut views,
                        );
                    }
                }
            }
        }
        if views == before {
            return Ok(views);
        }
    }
}

pub(super) fn erased_fields(prepared: &NativeProgram<'_>) -> BTreeSet<(LayoutId, String)> {
    let mut fields = BTreeSet::new();
    let opaque = prepared.layouts.opaque();
    for (&expected, actuals) in &prepared.contract_views {
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
                &prepared.physical_layouts.get(actual).kind,
                &prepared.physical_layouts.get(expected).kind,
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
    module: &mut ObjectModule,
    object: ClifValue,
    expected: LayoutId,
    results: &[NativeType],
    backend: &NativeBackend<'_>,
    mut lower: impl FnMut(
        &mut FunctionBuilder<'_>,
        &mut ObjectModule,
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
