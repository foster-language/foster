use super::*;

fn plans(prepared: &NativeProgram<'_>, optimize: bool) -> Vec<legalize::FunctionRepresentations> {
    let shared = prepared.compilation.shared_program(optimize).unwrap();
    let (_, facts) = reference_results::retained_reference_flow(&shared).unwrap();
    prepared
        .instances
        .iter()
        .map(|instance| {
            legalize::plan_function(
                &prepared.program.bodies[&instance.key.function],
                &facts[&instance.key.function],
                &prepared.function_types[&instance.ir_function],
                &instance.key,
                prepared.environment(),
            )
            .unwrap()
        })
        .collect()
}

#[test]
fn view_worklist_matches_reference_rounds() {
    for source in [
        include_str!("../../../../tests/fixtures/programs/contract_owned_results.fos").to_owned(),
        include_str!("../../../../tests/fixtures/programs/contract_reference_results.fos")
            .to_owned(),
        crate::native::benchmark_sources::candidates(6),
        crate::native::benchmark_sources::views(12),
    ] {
        let compilation = crate::compile(&source).unwrap();
        assert_eq!(
            crate::vm::run(&compilation).unwrap(),
            crate::vm::Value::Integer(42)
        );
        for optimize in [false, true] {
            let prepared = prepare_with_options(&compilation, CompileOptions { optimize }).unwrap();
            let plans = plans(&prepared, optimize);
            let (actual, work) = collect_views_with_work(&plans, prepared.environment()).unwrap();
            assert_eq!(
                actual,
                reference_collect_views(&plans, prepared.environment()).unwrap()
            );
            assert_eq!(
                work.pairs,
                actual.values().map(BTreeSet::len).sum::<usize>()
            );
            prepared
                .compile_object(CompileOptions { optimize })
                .unwrap();
        }
    }
}

#[test]
fn view_worklist_revisits_only_dependent_calls_and_handles_cycles() {
    let compilation = crate::compile(&crate::native::benchmark_sources::candidates(3)).unwrap();
    let prepared = prepare(&compilation).unwrap();
    let env = prepared.environment();
    let item = |name| {
        env.layouts.layouts().iter().find(|layout| {
        matches!(&layout.kind, LayoutKind::Record { name: actual, .. } if actual == name)
    }).unwrap().id
    };
    let items = [item("Item0"), item("Item1"), item("Item2")];
    let slot = env
        .program
        .metadata
        .dispatch
        .iter()
        .find_map(|((nominal, slot), function)| {
            (env.program.functions[function].name == "Item0.build").then_some((*nominal, *slot))
        })
        .unwrap()
        .1;
    let empty = Views::new();
    let env = NativeIrEnvironment {
        contract_views: &empty,
        ..env
    };
    let result = |layout| {
        let candidates = contract_candidates(slot, NativeType::Object(layout), &[], env).unwrap();
        env.function_types[&candidates[0].function].result
    };
    let mut pending = PendingViews::default();
    pending.record(
        NativeType::Object(items[1]),
        NativeType::Object(items[0]),
        env,
    );
    pending.record(
        NativeType::Object(items[0]),
        NativeType::Object(items[1]),
        env,
    );
    pending.record(
        NativeType::Object(items[2]),
        NativeType::Object(items[1]),
        env,
    );
    let calls = [
        ViewCall {
            slot,
            receiver: NativeType::Object(items[0]),
            arguments: vec![],
            result: result(items[0]),
        },
        ViewCall {
            slot,
            receiver: NativeType::Object(items[2]),
            arguments: vec![],
            result: result(items[2]),
        },
    ];
    let (views, work) = settle_views(pending, &calls, env).unwrap();
    assert!(views[&items[0]].contains(&items[2]));
    let NativeType::Object(expected) = result(items[0]) else {
        unreachable!()
    };
    let NativeType::Object(actual) = result(items[2]) else {
        unreachable!()
    };
    assert!(
        views[&expected].contains(&actual),
        "a newly reachable implementation's result must propagate"
    );
    assert_eq!(
        work.calls, 4,
        "only the changed Item0 receiver is revisited, once per published round"
    );
    assert_eq!(work.pairs, views.values().map(BTreeSet::len).sum::<usize>());
}

#[test]
fn dynamic_view_calls_are_not_repeated_by_unrelated_view_growth() {
    let compilation = crate::compile(&crate::native::benchmark_sources::candidates(4)).unwrap();
    let prepared = prepare(&compilation).unwrap();
    let plans = plans(&prepared, false);
    let (_, work) = collect_views_with_work(&plans, prepared.environment()).unwrap();
    let mut unique = HashSet::new();
    for function in &plans {
        for entry in function
            .body
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
        {
            if let ir::Instruction::Portable(Instruction::CallContractMethod {
                slot,
                receiver,
                arguments,
                destination,
                ..
            }) = &entry.instruction
            {
                let ty = |value: ir::Value| function.inferred.types[value.index()];
                let receiver =
                    dereference_native_type(ty(*receiver), prepared.environment()).unwrap();
                assert!(specialization::dynamic_receiver(
                    *slot,
                    receiver,
                    prepared.environment()
                ));
                unique.insert((
                    *slot,
                    receiver,
                    arguments.iter().map(|value| ty(*value)).collect::<Vec<_>>(),
                    ty(*destination),
                ));
            }
        }
    }
    assert!(!unique.is_empty());
    assert_eq!(work.calls, unique.len());
    assert!(work.rounds >= 2);
}

#[test]
fn view_worklist_keeps_results_before_a_new_candidate_changes_argument_abi() {
    let compilation = crate::compile(&crate::native::benchmark_sources::candidates(3)).unwrap();
    let prepared = prepare(&compilation).unwrap();
    let original = prepared.environment();
    let item = |name| {
        original.layouts.layouts().iter().find(|layout| {
        matches!(&layout.kind, LayoutKind::Record { name: actual, .. } if actual == name)
    }).unwrap().id
    };
    let (actual, receiver) = (item("Item0"), item("Item2"));
    let slot = original
        .program
        .metadata
        .dispatch
        .iter()
        .find_map(|((_, slot), function)| {
            (original.program.functions[function].name == "Item0.build").then_some(*slot)
        })
        .unwrap();
    let function = |layout| {
        contract_candidates(slot, NativeType::Object(layout), &[], original).unwrap()[0].function
    };
    let (actual_function, receiver_function) = (function(actual), function(receiver));
    let result = prepared
        .instances
        .iter()
        .find_map(|instance| {
            (prepared.program.functions[&instance.key.function].name == "build")
                .then_some(prepared.function_types[&instance.ir_function].result)
        })
        .unwrap();
    // Exercise the native query graph directly: the later candidate accepts an
    // erased argument, so its lower layout ID changes the first-candidate ABI.
    let mut signatures = prepared.function_types.clone();
    signatures
        .get_mut(&actual_function)
        .unwrap()
        .parameters
        .push(NativeType::Object(original.layouts.opaque()));
    signatures
        .get_mut(&receiver_function)
        .unwrap()
        .parameters
        .push(NativeType::Int);
    let candidates = CandidateIndex::build(
        &compilation,
        &prepared.program,
        &prepared.layouts,
        &prepared.instance_ids,
        &signatures,
    );
    let empty = Views::new();
    let env = NativeIrEnvironment {
        candidates: &candidates,
        function_types: &signatures,
        contract_views: &empty,
        ..original
    };
    let mut pending = PendingViews::default();
    pending.record(
        NativeType::Object(actual),
        NativeType::Object(receiver),
        env,
    );
    let calls = [ViewCall {
        slot,
        receiver: NativeType::Object(receiver),
        arguments: vec![NativeType::Int],
        result,
    }];
    let (views, work) = settle_views(pending, &calls, env).unwrap();
    let NativeType::Object(result) = result else {
        unreachable!()
    };
    for function in [receiver_function, actual_function] {
        let NativeType::Object(layout) = signatures[&function].result else {
            unreachable!()
        };
        assert!(views[&result].contains(&layout));
    }
    assert_eq!(work.calls, 2);
}

fn reference_collect_views(
    functions: &[legalize::FunctionRepresentations],
    environment: NativeIrEnvironment<'_>,
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
        let mut env = environment;
        env.contract_views = &before;
        for function in functions {
            let ty = |value: ir::Value| function.inferred.types[value.index()];
            let mut conversion = |actual, expected| -> Result<(), FosterError> {
                if matches!(
                    legalize::ReturnConversion::between(actual, expected, env)?,
                    Some(legalize::ReturnConversion::ContractView)
                ) {
                    record(actual, expected, env, &mut views);
                }
                Ok(())
            };
            // Lowering inserts conversions on returns and control-flow edges.
            // Discover their storage requirements directly from inferred SSA.
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
                    let arguments = arguments
                        .iter()
                        .map(|argument| ty(*argument))
                        .collect::<Vec<_>>();
                    let receiver = dereference_native_type(ty(*receiver), env)?;
                    let candidates = contract_candidates(*slot, receiver, &arguments, env)?;
                    // Dispatch observes the argument ABI chosen by legalization,
                    // including boxing and callable wrappers, rather than the
                    // original source-value representations.
                    let arguments = candidates
                        .first()
                        .map_or(arguments.as_slice(), |candidate| {
                            &env.function_types[&candidate.function].parameters[1..]
                        });
                    for candidate in contract_candidates(*slot, receiver, arguments, env)? {
                        record(
                            env.function_types[&candidate.function].result,
                            ty(*destination),
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
