//! Dispatch lookup over the frozen reachable specializations, in descriptor order.
use super::*;
use std::collections::BTreeMap;

struct CandidateGroup {
    implementation: FunctionId,
    // Arity includes the receiver. Each bucket is ordered by synthetic function ID.
    functions: BTreeMap<usize, Vec<FunctionId>>,
}

#[derive(Default)]
pub(in crate::native) struct CandidateIndex {
    slots: HashMap<crate::types::DispatchSlot, BTreeMap<crate::native::LayoutId, CandidateGroup>>,
}

#[derive(Default)]
struct LookupWork {
    groups: usize,
    signatures: usize,
}

impl CandidateIndex {
    pub(in crate::native) fn build(
        compilation: &Compilation,
        program: &Program,
        layouts: &LayoutRegistry,
        instances: &HashMap<SpecializationKey, FunctionId>,
        signatures: &HashMap<FunctionId, ir::Signature>,
    ) -> Self {
        let mut functions = HashMap::<_, BTreeMap<usize, Vec<FunctionId>>>::new();
        for (key, &function) in instances {
            let signature = &signatures[&function];
            let Some(&receiver) = signature.parameters.first() else {
                continue;
            };
            let receiver = if let NativeType::Object(layout) = receiver
                && let LayoutKind::Pointer { pointee, .. } = &layouts.get(layout).kind
            {
                let Ok(receiver) = crate::native::native_verification_type(
                    &program.metadata,
                    layouts,
                    pointee,
                    None,
                ) else {
                    continue;
                };
                receiver
            } else {
                receiver
            };
            functions
                .entry((key.function, receiver))
                .or_default()
                .entry(signature.parameters.len())
                .or_default()
                .push(function);
        }
        for arities in functions.values_mut() {
            for functions in arities.values_mut() {
                functions.sort_unstable_by_key(|function| function.into_raw().into_u32());
            }
        }
        let mut methods = HashMap::<_, Vec<_>>::new();
        for (&(nominal, slot), &implementation) in &program.metadata.dispatch {
            methods
                .entry(nominal)
                .or_default()
                .push((slot, implementation));
        }
        let mut index = Self::default();
        for layout in layouts
            .layouts()
            .iter()
            .filter(|layout| layout.materialized)
        {
            let concrete = match &layout.kind {
                LayoutKind::Record {
                    record, arguments, ..
                } => ExecutableType::Record {
                    record: *record,
                    arguments: arguments.clone(),
                },
                LayoutKind::Variant {
                    variant_type,
                    arguments,
                    ..
                } => ExecutableType::Variant {
                    variant: *variant_type,
                    arguments: arguments.clone(),
                },
                LayoutKind::Builtin { ty } => ty.clone(),
                _ => continue,
            };
            let Some(nominal) = nominal_id(&concrete, compilation) else {
                continue;
            };
            let Some(methods) = methods.get(&nominal) else {
                continue;
            };
            let receiver = if matches!(layout.kind, LayoutKind::Record { record, .. } if Some(record) == program.metadata.string_record)
            {
                NativeType::String
            } else {
                NativeType::Object(layout.id)
            };
            for &(slot, implementation) in methods {
                let Some(functions) = functions.get(&(implementation, receiver)) else {
                    continue;
                };
                index.slots.entry(slot).or_default().insert(
                    layout.id,
                    CandidateGroup {
                        implementation,
                        functions: functions.clone(),
                    },
                );
            }
        }
        index
    }

    fn lookup(
        &self,
        slot: crate::types::DispatchSlot,
        receiver: NativeType,
        arguments: &[NativeType],
        environment: NativeIrEnvironment<'_>,
    ) -> (Vec<ContractCandidate>, LookupWork) {
        let mut candidates = Vec::new();
        let mut work = LookupWork::default();
        let NativeType::Object(receiver_layout) = receiver else {
            return (candidates, work);
        };
        let Some(groups) = self.slots.get(&slot) else {
            return (candidates, work);
        };
        let mut select = |layout, group: &CandidateGroup| {
            work.groups += 1;
            crate::compiler::profile::count("native.candidates.group");
            let Some(functions) = group.functions.get(&(arguments.len() + 1)) else {
                return;
            };
            for &function in functions {
                work.signatures += 1;
                crate::compiler::profile::count("native.candidates.signature");
                if environment.function_types[&function].parameters[1..]
                    .iter()
                    .zip(arguments)
                    .all(|(expected, actual)| {
                        contract_argument_matches(*actual, *expected, environment)
                    })
                {
                    candidates.push(ContractCandidate {
                        layout,
                        implementation: group.implementation,
                        function,
                    });
                    break;
                }
            }
        };
        if dynamic_receiver(slot, receiver, environment) {
            for (&layout, group) in groups {
                select(layout, group);
            }
        } else {
            if let Some(group) = groups.get(&receiver_layout) {
                select(receiver_layout, group);
            }
            if let Some(actuals) = environment.contract_views.get(&receiver_layout) {
                for &actual in actuals.iter().filter(|actual| **actual != receiver_layout) {
                    if let Some(group) = groups.get(&actual) {
                        select(actual, group);
                    }
                }
            }
            candidates.sort_unstable_by_key(|candidate| candidate.layout);
        }
        (candidates, work)
    }
}

pub(in crate::native) fn dynamic_receiver(
    slot: crate::types::DispatchSlot,
    receiver: NativeType,
    environment: NativeIrEnvironment<'_>,
) -> bool {
    let NativeType::Object(layout) = receiver else {
        return false;
    };
    let nominal = match &environment.layouts.get(layout).kind {
        LayoutKind::Opaque => return true,
        LayoutKind::Record { record, .. } => Some(crate::types::NominalTypeId::Record(*record)),
        LayoutKind::Variant { variant_type, .. } => {
            Some(crate::types::NominalTypeId::Variant(*variant_type))
        }
        _ => None,
    };
    nominal.is_some_and(|nominal| {
        !environment
            .program
            .metadata
            .dispatch
            .contains_key(&(nominal, slot))
    })
}

pub(in crate::native) fn contract_candidates(
    slot: crate::types::DispatchSlot,
    receiver: NativeType,
    arguments: &[NativeType],
    environment: NativeIrEnvironment<'_>,
) -> Result<Vec<ContractCandidate>, FosterError> {
    Ok(environment
        .candidates
        .lookup(slot, receiver, arguments, environment)
        .0)
}

#[cfg(test)]
mod tests;
