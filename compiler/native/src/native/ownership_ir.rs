//! Ownership actions resolved before machine-code emission.
use super::*;

/// Storage read when an ownership action executes. Homes are reloaded on the
/// failure edge: a failing call may already have replaced the caller's binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnershipSource {
    Value(ir::Value),
    Home(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Release {
    pub source: OwnershipSource,
    pub layout: LayoutId,
}

/// A modeled failure exits the current frame after these releases, returning a
/// placeholder of the function's ABI result type. Nested lowering helpers share
/// this path; they must not infer a different set of live owners.
#[derive(Debug, Clone, Default)]
pub struct FailurePath {
    pub releases: Vec<Release>,
}

/// Explicit ownership effects attached to a native instruction site.
#[derive(Debug, Clone, Default)]
pub struct OwnershipEffects {
    pub failure: FailurePath,
    pub retain_result: Option<LayoutId>,
    pub retain_source: Option<(ir::Value, LayoutId)>,
    pub release: Option<Release>,
}

pub(super) fn plan(
    function: &ir::Function,
    cleanup: &FailureCleanup,
    layouts: NativeLayouts<'_>,
) -> Vec<Vec<OwnershipEffects>> {
    let addressable = super::optimization::addressable_homes(function);
    function
        .blocks
        .iter()
        .enumerate()
        .map(|(block, body)| {
            body.instructions
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    let managed = |value| layouts.managed_layout(function.value_type(value));
                    let mut effects = OwnershipEffects::default();
                    effects.failure.releases = cleanup
                        .values
                        .get(&(block, index))
                        .into_iter()
                        .flatten()
                        .filter_map(|value| {
                            Some(Release {
                                source: function
                                    .values
                                    .hint(value.index())
                                    .filter(|home| addressable.contains(home))
                                    .map_or(OwnershipSource::Value(*value), OwnershipSource::Home),
                                layout: managed(*value)?,
                            })
                        })
                        .collect();
                    match &entry.instruction {
                        ir::Instruction::Portable(ir::PortableInstruction::Drop { value }) => {
                            effects.release = managed(*value).map(|layout| Release {
                                source: OwnershipSource::Value(*value),
                                layout,
                            });
                        }
                        ir::Instruction::Portable(ir::PortableInstruction::Move {
                            destination,
                            ..
                        })
                        | ir::Instruction::StringToBytes { destination, .. }
                        | ir::Instruction::UnboxValue { destination, .. } => {
                            effects.retain_result = managed(*destination)
                        }
                        ir::Instruction::WrapCallable { source, .. }
                        | ir::Instruction::BoxValue { source, .. }
                        | ir::Instruction::ContractView { source, .. } => {
                            effects.retain_source =
                                managed(*source).map(|layout| (*source, layout));
                        }
                        _ => {}
                    }
                    effects
                })
                .collect()
        })
        .collect()
}
