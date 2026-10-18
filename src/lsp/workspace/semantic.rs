//! Shared symbol identity and callable/member resolution for workspace features.
use super::source::{find_name, identifier_at, qualifier_before};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SymbolIdentity {
    Local(crate::hir::LocalId),
    Constant(crate::hir::ConstantId),
    Function(crate::hir::FunctionId),
    Record(crate::hir::RecordId),
    RequiredMethod(crate::hir::RecordId, usize),
    Variant(crate::hir::VariantTypeId),
    Builtin(crate::intrinsics::Builtin),
}

fn expression_at(
    compilation: &crate::compiler::RecoveryCompilation,
    module: crate::hir::ModuleId,
    offset: usize,
) -> Option<crate::hir::ExprId> {
    compilation
        .hir
        .expression_spans
        .iter()
        .filter(|(expression, span)| {
            span.start <= offset
                && offset <= span.end
                && compilation
                    .hir
                    .expression_functions
                    .get(expression)
                    .is_some_and(|function| compilation.hir.functions[*function].module == module)
        })
        .min_by_key(|(_, span)| span.end.saturating_sub(span.start))
        .map(|(expression, _)| *expression)
}

pub(super) fn symbol_at(
    compilation: &crate::compiler::RecoveryCompilation,
    module_id: crate::hir::ModuleId,
    source: &str,
    offset: usize,
) -> Option<SymbolIdentity> {
    let (name, start) = identifier_at(source, offset)?;
    let module = &compilation.hir.modules[module_id];
    let qualifier = qualifier_before(source, start);
    for import in &module.imports_with_spans {
        if import.span.contains(&offset)
            && let Some(item) = &import.item_name
            && (name == import.name || item.rsplit('.').next() == Some(name))
        {
            return declaration_identity(compilation, import.target, item);
        }
    }
    // Recovery removes a failed body, but its parameter signatures remain valid.
    // Resolve simple parameter receivers from those signatures on a cold open too.
    if let Some(receiver) = qualifier.as_deref()
        && let Some(function) = function_at(compilation, module_id, offset)
        && compilation.hir.functions[function].body.is_empty()
        && let Some(index) = compilation.hir.functions[function]
            .parameters
            .iter()
            .position(|parameter| compilation.hir.locals[parameter.local].name == receiver)
        && let Some(ty) = compilation
            .types
            .function_type(function)
            .and_then(|signature| signature.parameters.get(index))
        && let Some(owner) = nominal_owner_from_type(&compilation.types, ty.ty)
        && let Some(target) = member_function_for_owner(compilation, owner, name)
    {
        return Some(SymbolIdentity::Function(target));
    }
    if let Some(expression) = expression_at(compilation, module_id, offset) {
        if let crate::hir::Expr::Call { callee, .. } = compilation.hir.expressions[expression]
            && let Some(function) = compilation
                .types
                .resolved_call(callee)
                .and_then(crate::types::ResolvedCall::function)
                .or_else(|| selected_function_for_callee(compilation, callee))
        {
            return Some(SymbolIdentity::Function(function));
        }
        let selected_function = selected_function_for_callee(compilation, expression);
        match &compilation.hir.expressions[expression] {
            crate::hir::Expr::Member {
                object,
                name: member,
            } if member == name => {
                if let Some(function) = selected_function {
                    return Some(SymbolIdentity::Function(function));
                }
                if let Some((record, method)) =
                    selected_requirement_for_callee(compilation, expression)
                {
                    return Some(SymbolIdentity::RequiredMethod(record, method));
                }
                if let Some(function) = member_function(compilation, *object, member) {
                    return Some(SymbolIdentity::Function(function));
                }
                if let Some((record, method)) = required_method(compilation, *object, member) {
                    return Some(SymbolIdentity::RequiredMethod(record, method));
                }
                // A stored member name is not a same-named local binding.
                return None;
            }
            crate::hir::Expr::Try {
                value,
                variant: Some(selected),
                ..
            } if selected == name => {
                if let Some(ty) = compilation.types.expression_type(*value)
                    && let crate::types::Type::Variant { variant, .. } = compilation.types.types[ty]
                {
                    return Some(SymbolIdentity::Variant(variant));
                }
            }
            crate::hir::Expr::Name(resolved) => match *resolved {
                crate::hir::ResolvedName::Local(local) => {
                    return Some(SymbolIdentity::Local(local));
                }
                crate::hir::ResolvedName::Constant(constant) => {
                    return Some(SymbolIdentity::Constant(constant));
                }
                crate::hir::ResolvedName::Function(function) => {
                    return Some(SymbolIdentity::Function(
                        selected_function.unwrap_or(function),
                    ));
                }
                crate::hir::ResolvedName::Record(record) => {
                    return Some(SymbolIdentity::Record(record));
                }
                crate::hir::ResolvedName::Variant(variant) => {
                    return Some(SymbolIdentity::Variant(
                        compilation.hir.variants[variant].parent,
                    ));
                }
                crate::hir::ResolvedName::Builtin(builtin) => {
                    return Some(SymbolIdentity::Builtin(builtin));
                }
                crate::hir::ResolvedName::Module(_) => {}
            },
            _ => {}
        }
    }
    if let Some(function) = function_at(compilation, module_id, offset) {
        let definition = &compilation.hir.functions[function];
        let declared_name = definition
            .name
            .rsplit('.')
            .next()
            .unwrap_or(&definition.name);
        if declared_name == name
            && find_name(source, definition.span.clone(), declared_name)
                .is_some_and(|span| span.start <= offset && offset <= span.end)
        {
            return Some(SymbolIdentity::Function(function));
        }
    }
    if qualifier.is_none()
        && let Some(function) = function_at(compilation, module_id, offset)
        && let Some((local, definition)) = compilation
            .hir
            .locals
            .iter()
            .find(|(_, local)| local.function == function && local.name == name)
    {
        // Explicit record labels share their spelling with locals but are not uses.
        if !definition.span.contains(&start)
            && crate::lexer::lex(&source[start + name.len()..])
                .ok()
                .and_then(|tokens| {
                    tokens
                        .into_iter()
                        .find(|token| !matches!(token.kind, crate::lexer::TokenKind::Newline))
                })
                .is_some_and(|token| matches!(token.kind, crate::lexer::TokenKind::Colon))
        {
            return None;
        }
        return Some(SymbolIdentity::Local(local));
    }
    let target = qualifier
        .as_deref()
        .and_then(|qualifier| module.imports.get(qualifier).copied())
        .unwrap_or(module_id);
    declaration_identity(compilation, target, name).or_else(|| {
        (qualifier.is_none()).then(|| {
            let mut matches = imported_declarations(compilation, module_id, name)
                .into_iter()
                .filter_map(|(target, actual)| declaration_identity(compilation, target, &actual));
            let first = matches.next()?;
            matches.next().is_none().then_some(first)
        })?
    })
}

pub(super) fn self_type_owner_at(
    compilation: &crate::compiler::RecoveryCompilation,
    module: crate::hir::ModuleId,
    offset: usize,
) -> Option<SymbolIdentity> {
    if let Some(function) = function_at(compilation, module, offset)
        && let Some(owner) = &compilation.hir.functions[function].owner
    {
        return declaration_identity(compilation, module, owner);
    }
    for (id, record) in compilation.hir.records.iter() {
        if record.module == module
            && record
                .methods
                .iter()
                .any(|method| method.span.contains(&offset))
        {
            return Some(SymbolIdentity::Record(id));
        }
    }
    for (id, variant) in compilation.hir.variant_types.iter() {
        if variant.module == module
            && variant
                .methods
                .iter()
                .any(|method| method.span.contains(&offset))
        {
            return Some(SymbolIdentity::Variant(id));
        }
    }
    None
}

fn declaration_identity(
    compilation: &crate::compiler::RecoveryCompilation,
    module: crate::hir::ModuleId,
    name: &str,
) -> Option<SymbolIdentity> {
    let definitions = &compilation.hir.modules[module];
    definitions
        .constants
        .get(name)
        .copied()
        .map(SymbolIdentity::Constant)
        .or_else(|| {
            definitions
                .functions
                .get(name)
                .and_then(|overloads| overloads.first())
                .copied()
                .map(SymbolIdentity::Function)
        })
        .or_else(|| {
            definitions
                .records
                .get(name)
                .copied()
                .map(SymbolIdentity::Record)
        })
        .or_else(|| {
            definitions
                .variant_types
                .get(name)
                .copied()
                .map(SymbolIdentity::Variant)
        })
}

pub(super) fn function_at(
    compilation: &crate::compiler::RecoveryCompilation,
    module: crate::hir::ModuleId,
    offset: usize,
) -> Option<crate::hir::FunctionId> {
    compilation
        .hir
        .functions
        .iter()
        .filter(|(_, function)| {
            function.module == module
                && function.span.start <= offset
                && offset <= function.span.end
        })
        .min_by_key(|(_, function)| function.span.end.saturating_sub(function.span.start))
        .map(|(id, _)| id)
}

pub(super) fn imported_declarations(
    compilation: &crate::compiler::RecoveryCompilation,
    module: crate::hir::ModuleId,
    name: &str,
) -> Vec<(crate::hir::ModuleId, String)> {
    let mut found = compilation
        .hir
        .visible_types(module, name)
        .iter()
        .map(|ty| {
            let (module, name) = compilation.hir.type_location(*ty);
            (module, name.to_owned())
        })
        .collect::<Vec<_>>();
    for value in compilation.hir.modules[module]
        .imported_values
        .get(name)
        .into_iter()
        .flatten()
    {
        match value {
            crate::hir::ResolvedName::Function(id) => found.push((
                compilation.hir.functions[*id].module,
                compilation.hir.functions[*id].name.clone(),
            )),
            crate::hir::ResolvedName::Constant(id) => found.push((
                compilation.hir.constants[*id].module,
                compilation.hir.constants[*id].name.clone(),
            )),
            _ => {}
        }
    }
    found.sort();
    found.dedup();
    found
}

pub(super) fn selected_function_for_callee(
    compilation: &crate::compiler::RecoveryCompilation,
    callee: crate::hir::ExprId,
) -> Option<crate::hir::FunctionId> {
    compilation.types.resolved_function_for_callee(callee)
}

pub(super) fn selected_requirement_for_callee(
    compilation: &crate::compiler::RecoveryCompilation,
    callee: crate::hir::ExprId,
) -> Option<(crate::hir::RecordId, usize)> {
    match compilation.types.resolved_call(callee)? {
        crate::types::ResolvedCall::ContractMethod { requirement, .. } => *requirement,
        _ => None,
    }
}

pub(super) fn member_function(
    compilation: &crate::compiler::RecoveryCompilation,
    object: crate::hir::ExprId,
    member: &str,
) -> Option<crate::hir::FunctionId> {
    let ty = compilation.types.expression_type(object)?;
    let owner = nominal_owner_from_type(&compilation.types, ty)?;
    member_function_for_owner(compilation, owner, member)
}

fn member_function_for_owner(
    compilation: &crate::compiler::RecoveryCompilation,
    owner: NominalOwner,
    member: &str,
) -> Option<crate::hir::FunctionId> {
    let (module, name) = match owner {
        NominalOwner::Record(record) => {
            let definition = &compilation.hir.records[record];
            (definition.module, definition.name.as_str())
        }
        NominalOwner::Variant(variant) => {
            let definition = &compilation.hir.variant_types[variant];
            (definition.module, definition.name.as_str())
        }
        NominalOwner::Bool => (compilation.hir.module_named("core.bool")?, "Bool"),
        NominalOwner::Int => (compilation.hir.module_named("core.int")?, "Int"),
        NominalOwner::Float => (compilation.hir.module_named("core.float")?, "Float"),
        NominalOwner::CodePoint => (
            compilation.hir.module_named("core.code_point")?,
            "CodePoint",
        ),
        NominalOwner::Byte => (compilation.hir.module_named("core.byte")?, "Byte"),
    };
    let qualified_name = format!("{name}.{member}");
    let function = compilation.hir.modules[module]
        .functions
        .get(&qualified_name)
        .and_then(|overloads| {
            let mut candidates = overloads.iter().copied().filter(|function| {
                let receiver_matches = compilation
                    .types
                    .function_type(*function)
                    .and_then(|signature| signature.parameters.first())
                    .is_some_and(|ty| {
                        matches!(
                            (owner, &compilation.types.types[ty.ty]),
                            (
                                NominalOwner::Record(expected),
                                crate::types::Type::Record { record, .. }
                            ) if expected == *record
                        ) || matches!(
                            (owner, &compilation.types.types[ty.ty]),
                            (
                                NominalOwner::Variant(expected),
                                crate::types::Type::Variant { variant, .. }
                            ) if expected == *variant
                        ) || matches!(
                            (owner, &compilation.types.types[ty.ty]),
                            (NominalOwner::Bool, crate::types::Type::Bool)
                                | (NominalOwner::Int, crate::types::Type::Int)
                                | (NominalOwner::Float, crate::types::Type::Float)
                                | (NominalOwner::CodePoint, crate::types::Type::CodePoint)
                                | (NominalOwner::Byte, crate::types::Type::Byte)
                        )
                    });
                receiver_matches && compilation.hir.functions[*function].receiver.is_some()
            });
            let first = candidates.next()?;
            candidates.next().is_none().then_some(first)
        })?;
    Some(function)
}

#[derive(Clone, Copy)]
enum NominalOwner {
    Record(crate::hir::RecordId),
    Variant(crate::hir::VariantTypeId),
    Bool,
    Int,
    Float,
    CodePoint,
    Byte,
}

fn nominal_owner_from_type(
    types: &crate::types::TypeInformation,
    ty: crate::types::TypeId,
) -> Option<NominalOwner> {
    match &types.types[ty] {
        crate::types::Type::Record { record, .. } => Some(NominalOwner::Record(*record)),
        crate::types::Type::Variant { variant, .. } => Some(NominalOwner::Variant(*variant)),
        crate::types::Type::Bool => Some(NominalOwner::Bool),
        crate::types::Type::Int => Some(NominalOwner::Int),
        crate::types::Type::Float => Some(NominalOwner::Float),
        crate::types::Type::CodePoint => Some(NominalOwner::CodePoint),
        crate::types::Type::Byte => Some(NominalOwner::Byte),
        crate::types::Type::Reference { value, .. } | crate::types::Type::Remote(value) => {
            nominal_owner_from_type(types, *value)
        }
        _ => None,
    }
}

pub(super) fn required_method(
    compilation: &crate::compiler::RecoveryCompilation,
    object: crate::hir::ExprId,
    member: &str,
) -> Option<(crate::hir::RecordId, usize)> {
    let ty = compilation.types.expression_type(object)?;
    let record = record_from_type(&compilation.types, ty)?;
    compilation.hir.records[record]
        .methods
        .iter()
        .position(|method| method.name == member)
        .map(|method| (record, method))
}

fn record_from_type(
    types: &crate::types::TypeInformation,
    ty: crate::types::TypeId,
) -> Option<crate::hir::RecordId> {
    match &types.types[ty] {
        crate::types::Type::Record { record, .. } => Some(*record),
        crate::types::Type::Reference { value, .. } | crate::types::Type::Remote(value) => {
            record_from_type(types, *value)
        }
        _ => None,
    }
}
