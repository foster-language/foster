use super::*;

#[test]
fn indexed_candidates_match_reference_selection_and_order() {
    for source in [
        include_str!("../../../../../../tests/fixtures/programs/contract_owned_results.fos")
            .to_owned(),
        include_str!("../../../../../../tests/fixtures/programs/contract_reference_results.fos")
            .to_owned(),
        crate::native::benchmark_sources::candidates(6),
    ] {
        let compilation = crate::compile(&source).unwrap();
        for optimize in [false, true] {
            let prepared = crate::native::prepare_with_options(
                &compilation,
                crate::native::CompileOptions { optimize },
            )
            .unwrap();
            let env = prepared.environment();
            let mut queries = std::collections::HashSet::new();
            for (&slot, groups) in &env.candidates.slots {
                for (&layout, group) in groups {
                    for function in group.functions.values().flatten() {
                        let arguments = env.function_types[function].parameters[1..].to_vec();
                        queries.insert((slot, NativeType::Object(layout), arguments.clone()));
                        queries.insert((
                            slot,
                            NativeType::Object(env.layouts.opaque()),
                            arguments.clone(),
                        ));
                        queries.insert((slot, NativeType::Object(layout), vec![NativeType::Float]));
                        for (&expected, actuals) in env.contract_views {
                            if actuals.contains(&layout) {
                                queries.insert((
                                    slot,
                                    NativeType::Object(expected),
                                    arguments.clone(),
                                ));
                            }
                        }
                    }
                }
            }
            for (slot, receiver, arguments) in queries {
                assert_eq!(
                    contract_candidates(slot, receiver, &arguments, env).unwrap(),
                    reference_contract_candidates(slot, receiver, &arguments, env).unwrap(),
                    "{slot:?} {receiver:?} {arguments:?}"
                );
            }
        }
    }
}

#[test]
fn candidate_index_skips_unrelated_layouts_and_signatures() {
    for size in [2, 12] {
        let compilation =
            crate::compile(&crate::native::benchmark_sources::candidates(size)).unwrap();
        let prepared = crate::native::prepare(&compilation).unwrap();
        let env = prepared.environment();
        let (&slot, groups) = env
            .candidates
            .slots
            .iter()
            .find(|(_, groups)| {
                groups
                    .values()
                    .any(|group| env.program.functions[&group.implementation].name == "Item0.build")
            })
            .unwrap();
        let (&layout, _) = groups
            .iter()
            .find(|(_, group)| env.program.functions[&group.implementation].name == "Item0.build")
            .unwrap();
        let (candidates, work) = env
            .candidates
            .lookup(slot, NativeType::Object(layout), &[], env);
        assert_eq!(candidates.len(), 1);
        assert_eq!(work.groups, 1);
        assert_eq!(work.signatures, 1);
        assert!(groups.len() >= size);
        let (_, work) = env.candidates.lookup(
            slot,
            NativeType::Object(candidates[0].layout),
            &[NativeType::Int],
            env,
        );
        assert_eq!(work.groups, 1);
        assert_eq!(
            work.signatures, 0,
            "wrong-arity specializations must not be visited"
        );
    }
}

#[test]
fn indexed_candidates_keep_function_id_ties_and_layout_order() {
    let compilation = crate::compile(&crate::native::benchmark_sources::candidates(3)).unwrap();
    let prepared = crate::native::prepare(&compilation).unwrap();
    let env = prepared.environment();
    let (&slot, groups) = env
        .candidates
        .slots
        .iter()
        .find(|(_, groups)| groups.len() >= 3)
        .unwrap();
    let (&receiver, group) = groups.last_key_value().unwrap();
    let function = *group.functions.values().next().unwrap().first().unwrap();
    let mut instances = prepared.instance_ids.clone();
    let mut signatures = prepared.function_types.clone();
    let mut key = instances
        .iter()
        .find(|(_, target)| **target == function)
        .unwrap()
        .0
        .clone();
    key.substitutions = BTreeMap::from([("tie".to_owned(), ExecutableType::Integer)]).into();
    let extra = FunctionId::from_raw(RawIdx::from_u32(1_000_000));
    instances.insert(key, extra);
    signatures.insert(extra, signatures[&function].clone());
    let index = CandidateIndex::build(
        &compilation,
        &prepared.program,
        &prepared.layouts,
        &instances,
        &signatures,
    );
    let views = HashMap::from([(receiver, groups.keys().copied().collect())]);
    let env = NativeIrEnvironment {
        candidates: &index,
        instances: &instances,
        function_types: &signatures,
        contract_views: &views,
        ..env
    };
    let arguments = &signatures[&function].parameters[1..];
    let candidates =
        contract_candidates(slot, NativeType::Object(receiver), arguments, env).unwrap();
    assert_eq!(
        candidates,
        reference_contract_candidates(slot, NativeType::Object(receiver), arguments, env).unwrap()
    );
    assert!(
        candidates
            .windows(2)
            .all(|pair| pair[0].layout < pair[1].layout)
    );
    assert_eq!(
        candidates
            .iter()
            .find(|candidate| candidate.layout == receiver)
            .unwrap()
            .function,
        function
    );
}

fn reference_contract_candidates(
    slot: crate::types::DispatchSlot,
    receiver: NativeType,
    argument_types: &[NativeType],
    environment: NativeIrEnvironment<'_>,
) -> Result<Vec<ContractCandidate>, FosterError> {
    let receiver_layout = match receiver {
        NativeType::Object(layout) => layout,
        _ => return Ok(Vec::new()),
    };
    let receiver_nominal = match &environment.layouts.get(receiver_layout).kind {
        LayoutKind::Record { record, .. } => Some(crate::types::NominalTypeId::Record(*record)),
        LayoutKind::Variant { variant_type, .. } => {
            Some(crate::types::NominalTypeId::Variant(*variant_type))
        }
        _ => None,
    };
    let dynamic = matches!(
        environment.layouts.get(receiver_layout).kind,
        LayoutKind::Opaque
    ) || receiver_nominal.is_some_and(|nominal| {
        !environment
            .program
            .metadata
            .dispatch
            .contains_key(&(nominal, slot))
    });
    let mut candidates = Vec::new();
    for layout in environment
        .layouts
        .layouts()
        .iter()
        .filter(|layout| layout.materialized)
    {
        if !dynamic
            && layout.id != receiver_layout
            && !environment
                .contract_views
                .get(&receiver_layout)
                .is_some_and(|layouts| layouts.contains(&layout.id))
        {
            continue;
        }
        let concrete = match &layout.kind {
            LayoutKind::Record {
                record, arguments, ..
            } => crate::codegen::types::ExecutableType::Record {
                record: *record,
                arguments: arguments.clone(),
            },
            LayoutKind::Variant {
                variant_type,
                arguments,
                ..
            } => crate::codegen::types::ExecutableType::Variant {
                variant: *variant_type,
                arguments: arguments.clone(),
            },
            LayoutKind::Builtin { ty } => ty.clone(),
            _ => continue,
        };
        let Some(nominal) = nominal_id(&concrete, environment.compilation) else {
            continue;
        };
        let Some(implementation) = environment
            .program
            .metadata
            .dispatch
            .get(&(nominal, slot))
            .copied()
        else {
            continue;
        };
        let mut matching = environment
            .instances
            .iter()
            .filter_map(|(key, function)| {
                if key.function != implementation {
                    return None;
                }
                let signature = &environment.function_types[function];
                let receiver_type = if matches!(layout.kind, LayoutKind::Record { record, .. } if Some(record) == environment.program.metadata.string_record) { NativeType::String } else { NativeType::Object(layout.id) };
                (signature.parameters.first().is_some_and(|ty| {
                    crate::native::dereference_native_type(*ty, environment).is_ok_and(|ty| ty == receiver_type)
                })
                    && signature.parameters.len() == argument_types.len() + 1
                    && signature.parameters[1..].iter().zip(argument_types).all(|(expected, actual)| contract_argument_matches(*actual, *expected, environment)))
                .then_some(*function)
            })
            .collect::<Vec<_>>();
        matching.sort_unstable_by_key(|function| function.into_raw().into_u32());
        let Some(function) = matching.into_iter().next() else {
            continue;
        };
        candidates.push(ContractCandidate {
            layout: layout.id,
            implementation,
            function,
        });
    }
    Ok(candidates)
}
