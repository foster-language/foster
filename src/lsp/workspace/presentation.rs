//! Documented hover content and Foster declaration/signature rendering.
use super::navigation::symbol_declaration;
use super::semantic::{
    SymbolIdentity, member_function, required_method, selected_function_for_callee,
    selected_requirement_for_callee,
};
use lsp_types::Location;

pub(super) fn symbol_hover(
    compilation: &crate::compiler::RecoveryCompilation,
    symbol: SymbolIdentity,
) -> Option<String> {
    match symbol {
        SymbolIdentity::Local(local) => compilation.types.local_type(local).map(|ty| {
            documented_hover(
                format!(
                    "{}: {}",
                    compilation.hir.locals[local].name,
                    compilation.types.display(ty)
                ),
                None,
            )
        }),
        SymbolIdentity::Constant(constant) => {
            let definition = &compilation.hir.constants[constant];
            compilation.types.constants.get(&constant).map(|ty| {
                documented_hover(
                    format!(
                        "const {}: {}",
                        definition.name,
                        compilation.types.display(*ty)
                    ),
                    definition.documentation.as_deref(),
                )
            })
        }
        SymbolIdentity::Function(function) => {
            let definition = &compilation.hir.functions[function];
            Some(documented_hover(
                function_signature(compilation, function, false),
                definition.documentation.as_deref(),
            ))
        }
        SymbolIdentity::Record(record) => {
            let definition = &compilation.hir.records[record];
            Some(documented_hover(
                record_signature(definition),
                definition.documentation.as_deref(),
            ))
        }
        SymbolIdentity::RequiredMethod(record, method) => {
            let method = &compilation.hir.records[record].methods[method];
            Some(documented_hover(
                method_requirement_signature(method),
                method.documentation.as_deref(),
            ))
        }
        SymbolIdentity::Variant(variant) => {
            let definition = &compilation.hir.variant_types[variant];
            Some(documented_hover(
                variant_signature(compilation, variant),
                definition.documentation.as_deref(),
            ))
        }
        SymbolIdentity::Builtin(builtin) => {
            let info = crate::lsp::builtins::info(builtin);
            Some(documented_hover(
                info.signature.to_owned(),
                Some(info.documentation),
            ))
        }
    }
}

pub(super) fn declaration_hover(
    compilation: &crate::compiler::RecoveryCompilation,
    module: crate::hir::ModuleId,
    name: &str,
) -> Option<String> {
    let definitions = &compilation.hir.modules[module];
    if let Some(constant) = definitions.constants.get(name) {
        let definition = &compilation.hir.constants[*constant];
        return Some(documented_hover(
            format!(
                "const {}: {}",
                definition.name,
                compilation
                    .types
                    .constants
                    .get(constant)
                    .map_or_else(|| "unknown".into(), |ty| compilation.types.display(*ty))
            ),
            definition.documentation.as_deref(),
        ));
    }
    if let Some(function) = definitions
        .functions
        .get(name)
        .and_then(|overloads| overloads.first())
        .copied()
    {
        let definition = &compilation.hir.functions[function];
        return Some(documented_hover(
            function_signature(compilation, function, false),
            definition.documentation.as_deref(),
        ));
    }
    if let Some(record) = definitions.records.get(name) {
        return Some(documented_hover(
            record_signature(&compilation.hir.records[*record]),
            compilation.hir.records[*record].documentation.as_deref(),
        ));
    }
    if let Some(variant) = definitions.variant_types.get(name) {
        return Some(documented_hover(
            variant_signature(compilation, *variant),
            compilation.hir.variant_types[*variant]
                .documentation
                .as_deref(),
        ));
    }
    None
}

pub(in crate::lsp) struct CallablePresentation {
    pub signature: String,
    pub parameters: Vec<String>,
    pub documentation: Option<String>,
    pub definition: Option<Location>,
}

pub(in crate::lsp) fn callable_presentation(
    compilation: &crate::compiler::RecoveryCompilation,
    callee: crate::hir::ExprId,
) -> Option<CallablePresentation> {
    let (function, receiver) = match &compilation.hir.expressions[callee] {
        crate::hir::Expr::Name(crate::hir::ResolvedName::Function(function)) => (
            selected_function_for_callee(compilation, callee).unwrap_or(*function),
            false,
        ),
        crate::hir::Expr::Member { object, name } => {
            if let Some(function) = selected_function_for_callee(compilation, callee) {
                (function, true)
            } else if let Some(function) = member_function(compilation, *object, name) {
                (function, true)
            } else {
                let (record, method_index) =
                    selected_requirement_for_callee(compilation, callee)
                        .or_else(|| required_method(compilation, *object, name))?;
                let method = &compilation.hir.records[record].methods[method_index];
                return Some(CallablePresentation {
                    signature: method_requirement_signature(method),
                    parameters: method
                        .parameters
                        .iter()
                        .skip(1)
                        .map(|parameter| parameter.name.clone())
                        .collect(),
                    documentation: method.documentation.clone(),
                    definition: symbol_declaration(
                        compilation,
                        SymbolIdentity::RequiredMethod(record, method_index),
                    ),
                });
            }
        }
        crate::hir::Expr::Name(crate::hir::ResolvedName::Builtin(builtin)) => {
            let info = crate::lsp::builtins::info(*builtin);
            return Some(CallablePresentation {
                signature: info.signature.to_owned(),
                parameters: info
                    .parameters
                    .iter()
                    .map(|name| (*name).to_owned())
                    .collect(),
                documentation: Some(info.documentation.to_owned()),
                definition: crate::lsp::builtins::definition_location(*builtin),
            });
        }
        _ => return None,
    };
    let definition = &compilation.hir.functions[function];
    Some(CallablePresentation {
        signature: function_signature(compilation, function, receiver),
        parameters: definition
            .parameters
            .iter()
            .skip(usize::from(receiver))
            .map(|parameter| compilation.hir.locals[parameter.local].name.clone())
            .collect(),
        documentation: definition.documentation.clone(),
        definition: symbol_declaration(compilation, SymbolIdentity::Function(function)),
    })
}

pub(super) fn function_signature(
    compilation: &crate::compiler::RecoveryCompilation,
    function_id: crate::hir::FunctionId,
    omit_receiver: bool,
) -> String {
    let function = &compilation.hir.functions[function_id];
    let signature = compilation.types.function_type(function_id);
    let skip = usize::from(omit_receiver || function.receiver.is_some());
    let parameters = function
        .parameters
        .iter()
        .enumerate()
        .skip(skip)
        .map(|(index, local)| {
            let ty = signature
                .and_then(|signature| signature.parameters.get(index))
                .map(|ty| compilation.types.display(ty.ty))
                .unwrap_or_else(|| "_".into());
            let consumed = signature
                .and_then(|signature| signature.parameters.get(index).map(|p| &p.mode))
                .is_some_and(|mode| *mode == crate::ast::ParameterMode::Consume);
            format!(
                "{}: {}{ty}",
                compilation.hir.locals[local.local].name,
                if consumed { "consume " } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let result = signature
        .map(|signature| compilation.types.display(signature.result))
        .unwrap_or_else(|| "()".into());
    let effects = signature.map_or_else(String::new, |signature| {
        display_effects(&signature.effects, signature.suspends)
    });
    let generics = angle_parameters(
        &function
            .type_parameters
            .iter()
            .filter(|name| !crate::types::is_reference_parameter(name))
            .map(|name| {
                let requirements = function
                    .constraints
                    .iter()
                    .filter(|constraint| &constraint.parameter == name)
                    .map(|constraint| display_type_expr(&constraint.requirement))
                    .collect::<Vec<_>>();
                if requirements.is_empty() {
                    name.clone()
                } else {
                    format!("{name} & {}", requirements.join(" & "))
                }
            })
            .collect::<Vec<_>>(),
    );
    let name = function.name.rsplit('.').next().unwrap_or(&function.name);
    let signature = format!(
        "{}{}func {name}{generics}({parameters}) -> {result}{effects}",
        if function.public { "pub " } else { "" },
        if function.owner.is_some() && function.receiver.is_none() {
            "static "
        } else {
            ""
        },
    );
    match &function.owner {
        Some(owner) => format!("impl {owner} {{\n    {signature}\n}}"),
        None => signature,
    }
}

pub(super) fn record_signature(record: &crate::hir::Record) -> String {
    let parameters = angle_parameters(&record.parameters);
    let compositions = if record.compositions.is_empty() {
        String::new()
    } else {
        format!(
            " & {}",
            record
                .compositions
                .iter()
                .map(display_type_expr)
                .collect::<Vec<_>>()
                .join(" & ")
        )
    };
    let mut members = record
        .fields
        .iter()
        .map(|field| {
            format!(
                "    {}{}: {}",
                if field.public { "pub " } else { "" },
                field.name,
                display_type_expr(&field.ty)
            )
        })
        .collect::<Vec<_>>();
    members.extend(
        record
            .methods
            .iter()
            .map(|method| format!("    {}", method_requirement_signature(method))),
    );
    let visibility = if record.public { "pub " } else { "" };
    if members.is_empty() && !record.compositions.is_empty() {
        return format!(
            "{visibility}type {}{parameters} ={compositions}",
            record.name
        );
    }
    let members = members.join("\n");
    let body_separator = if record.compositions.is_empty() {
        ""
    } else {
        " &"
    };
    format!(
        "{visibility}type {}{parameters} ={compositions}{body_separator} {{\n{members}\n}}",
        record.name
    )
}

fn method_requirement_signature(method: &crate::ast::MethodRequirement) -> String {
    let parameters = method
        .parameters
        .iter()
        .skip(usize::from(method.receiver))
        .map(|parameter| match &parameter.ty {
            Some(ty) => format!("{}: {}", parameter.name, display_type_expr(ty)),
            None => parameter.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let result = method
        .return_type
        .as_ref()
        .map(display_type_expr)
        .unwrap_or_else(|| "()".into());
    format!(
        "{}func {}{}({parameters}) -> {result}{}",
        if method.public { "pub " } else { "" },
        method.name,
        angle_parameters(&method.type_parameters),
        display_effects(&method.effects, method.suspends)
    )
}

fn variant_signature(
    compilation: &crate::compiler::RecoveryCompilation,
    variant_id: crate::hir::VariantTypeId,
) -> String {
    let variant = &compilation.hir.variant_types[variant_id];
    let parameters = angle_parameters(&variant.parameters);
    if variant.kind == crate::ast::VariantKind::Alias
        && variant.alternatives.len() == 1
        && variant.compositions.is_empty()
        && variant.methods.is_empty()
    {
        let member = compilation.hir.variants[variant.alternatives[0]]
            .member
            .as_ref()
            .expect("a type alias has a target type");
        return format!(
            "{}type {}{parameters} = {}",
            if variant.public { "pub " } else { "" },
            variant.name,
            display_type_expr(member)
        );
    }
    let alternatives = variant
        .alternatives
        .iter()
        .enumerate()
        .map(|(index, alternative)| {
            let alternative = &compilation.hir.variants[*alternative];
            let member = if let Some(member) = &alternative.member {
                display_type_expr(member)
            } else if alternative.payload.is_empty() {
                alternative.name.clone()
            } else {
                format!(
                    "{}({})",
                    alternative.name,
                    alternative
                        .payload
                        .iter()
                        .map(display_type_expr)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            if index == 0 && variant.kind == crate::ast::VariantKind::Enum {
                member
            } else if index == 0 {
                format!("    {member}")
            } else {
                format!("    | {member}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let compositions = variant
        .compositions
        .iter()
        .map(|composition| format!("    & {}", display_type_expr(composition)))
        .collect::<Vec<_>>();
    let body = if variant.methods.is_empty() {
        String::new()
    } else {
        format!(
            "\n    & {{\n{}\n    }}",
            variant
                .methods
                .iter()
                .map(|method| format!("        {}", method_requirement_signature(method)))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    let contracts = if compositions.is_empty() {
        String::new()
    } else {
        format!("\n{}", compositions.join("\n"))
    };
    let first_case_separator = if variant.kind == crate::ast::VariantKind::Enum {
        " "
    } else {
        "\n"
    };
    format!(
        "{}{} {}{parameters} ={first_case_separator}{alternatives}{contracts}{body}",
        if variant.public { "pub " } else { "" },
        if variant.kind == crate::ast::VariantKind::Enum {
            "enum"
        } else {
            "type"
        },
        variant.name
    )
}

fn angle_parameters(parameters: &[String]) -> String {
    if parameters.is_empty() {
        String::new()
    } else {
        format!("<{}>", parameters.join(", "))
    }
}

pub(super) fn display_type_expr(ty: &crate::ast::TypeExpr) -> String {
    match ty {
        crate::ast::TypeExpr::Unit => "()".into(),
        crate::ast::TypeExpr::Named(name, arguments) => {
            let arguments = arguments.iter().map(display_type_expr).collect::<Vec<_>>();
            format!(
                "{}{}",
                name.replace('.', "::"),
                angle_parameters(&arguments)
            )
        }
        crate::ast::TypeExpr::Intersection(members) => members
            .iter()
            .map(display_type_expr)
            .collect::<Vec<_>>()
            .join(" & "),
        crate::ast::TypeExpr::Reference { group, value } => {
            format!("ref[{group}] {}", display_type_expr(value))
        }
        crate::ast::TypeExpr::Function {
            parameters,
            parameter_modes,
            result,
            effects,
            suspends,
        } => {
            let parameters = parameters
                .iter()
                .zip(parameter_modes)
                .map(|(parameter, mode)| {
                    format!(
                        "{}{}",
                        if *mode == crate::ast::ParameterMode::Consume {
                            "consume "
                        } else {
                            ""
                        },
                        display_type_expr(parameter)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "func({parameters}) -> {}{}",
                display_type_expr(result),
                display_effects(effects, *suspends)
            )
        }
    }
}

fn display_effects(effects: &[crate::ast::Effect], suspends: bool) -> String {
    let mut values = effects
        .iter()
        .map(|effect| {
            let kind = match effect.kind {
                crate::ast::EffectKind::Read => "read",
                crate::ast::EffectKind::Mut => "mut",
                crate::ast::EffectKind::Reshape => "reshape",
                crate::ast::EffectKind::Consume => "consume",
            };
            format!("{kind} {}", effect.target)
        })
        .collect::<Vec<_>>();
    if suspends {
        values.push("suspend".into());
    }
    if values.is_empty() {
        String::new()
    } else {
        format!(" [{}]", values.join(", "))
    }
}

pub(super) fn documented_hover(signature: String, documentation: Option<&str>) -> String {
    let mut hover = format!("```foster\n{signature}\n```");
    if let Some(documentation) = documentation {
        hover.push_str("\n\n");
        hover.push_str(documentation);
    }
    hover
}
