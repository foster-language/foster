//! Preserve incoming storage layouts for borrowed structural contracts.
use super::*;
use crate::ast;

pub(super) fn lower(hir: &mut hir::PackageHir) -> Result<(), FosterError> {
    let mut plans = Vec::new();
    {
        let mut checker = Checker::new(hir);
        checker.declare_signatures()?;
        for (function_id, function) in hir.functions.iter() {
            for (index, parameter) in function.parameters.iter().enumerate() {
                if function.receiver == Some(parameter.local) {
                    continue;
                }
                let Some(annotation) = &parameter.ty else {
                    continue;
                };
                let Ty::Reference(group, pointee) =
                    checker.functions[&function_id].parameters[index].ty.clone()
                else {
                    continue;
                };
                if checker.reference_parameter_contract(&pointee)? {
                    let requirement = match annotation {
                        ast::TypeExpr::Reference { value, .. } => value.as_ref().clone(),
                        annotation => annotation.clone(),
                    };
                    let name = format!(
                        "$reference:{}:{}:{}:{index}",
                        hir.modules[function.module].name, function.name, function.span.start
                    );
                    plans.push((function_id, index, name, group, requirement));
                }
            }
        }
    }
    for (function_id, index, name, group, requirement) in plans {
        let function = &mut hir.functions[function_id];
        function.type_parameters.push(name.clone());
        function.constraints.push(ast::TypeConstraint {
            parameter: name.clone(),
            requirement,
        });
        function.parameters[index].ty = Some(ast::TypeExpr::Reference {
            group,
            value: Box::new(ast::TypeExpr::Named(name, vec![])),
        });
    }
    // Closure lowering precedes this pass; captures inherit the enclosing bounds.
    let mut closures = hir
        .expressions
        .iter()
        .filter_map(|(id, expression)| match expression {
            hir::Expr::Closure { function, .. } => Some((hir.expression_functions[&id], *function)),
            _ => None,
        })
        .collect::<Vec<_>>();
    closures.sort_by_key(|(parent, _)| parent.into_raw().into_u32());
    for (parent, child) in closures {
        let inherited = hir.functions[parent]
            .constraints
            .iter()
            .filter(|constraint| crate::types::is_reference_parameter(&constraint.parameter))
            .cloned()
            .collect::<Vec<_>>();
        for constraint in inherited {
            let child = &mut hir.functions[child];
            if !child.type_parameters.contains(&constraint.parameter) {
                child.type_parameters.push(constraint.parameter.clone());
                child.constraints.push(constraint);
            }
        }
    }
    Ok(())
}

impl Checker<'_> {
    fn reference_parameter_contract(&mut self, ty: &Ty) -> Result<bool, FosterError> {
        match ty {
            Ty::Intersection(_) => Ok(true),
            Ty::Record(record, arguments) => {
                let definition = &self.hir.records[*record];
                if definition.intrinsic || definition.fields.iter().any(|field| !field.public) {
                    return Ok(false);
                }
                Ok(self
                    .effective_record_fields(*record, arguments)?
                    .iter()
                    .all(|field| field.public))
            }
            _ => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    const SOURCE: &str =
        include_str!("../../../tests/fixtures/programs/contract_reference_parameters.fos");

    fn check(source: &str) {
        let compilation = crate::compile(source).unwrap();
        assert_eq!(
            crate::vm::run(&compilation).unwrap(),
            crate::vm::Value::Integer(42)
        );
        for optimize in [false, true] {
            crate::native::compile_object(&compilation, crate::native::CompileOptions { optimize })
                .unwrap();
        }
    }

    #[test]
    fn reference_contract_parameters_preserve_argument_storage() {
        check(SOURCE);
    }

    #[test]
    fn reference_contract_parameters_preserve_field_layouts() {
        check(include_str!(
            "../../../tests/fixtures/programs/structural_reference_fields.fos"
        ));
    }

    #[test]
    fn reference_contract_parameters_restrict_body_members() {
        let source = SOURCE.replacen("item.replace(21)", "item.extra()\n()", 1);
        let error = crate::compile(&source).unwrap_err();
        assert!(!error.message.contains("$reference:"));
    }

    #[test]
    fn reference_contract_parameters_reject_nonconforming_arguments() {
        let source = SOURCE.replace("pair(holder.first, holder.second)", "update(42)");
        let error = crate::compile(&source).unwrap_err();
        assert!(!error.message.contains("$reference:"));
    }

    #[test]
    fn reference_contract_parameters_reject_replacing_an_unknown_shape() {
        let error = crate::compile(
            r#"
type Value = { pub value: Int }
type Extended = { pub value: Int, pub extra: Int }
func replace(item: ref[item] Value) -> () {
    item = Value { value: 42 }
    ()
}
func main() -> Int {
    let item = Extended { value: 1, extra: 9 }
    replace(item)
    item.extra
}
"#,
        )
        .unwrap_err();
        assert!(
            error
                .message
                .contains("assigned value has an incompatible type")
                || error.message.contains("type mismatch"),
            "{error}"
        );
        assert!(!error.message.contains("$reference:"));
    }

    #[test]
    fn reference_contract_parameters_support_aliases_and_intersections() {
        for (declaration, annotation) in [
            ("type Alias = Replace", "Alias"),
            ("type More = & Replace & {}", "More"),
            ("type Marker = { pub value: Int }", "Replace & Marker"),
        ] {
            let source =
                format!("{SOURCE}\n{declaration}").replace("] Replace", &format!("] {annotation}"));
            check(&source);
        }
    }

    #[test]
    fn reference_contract_parameters_support_required_methods() {
        let source = SOURCE.replace("type Replace =", "pub type Replace =").replace("func main() -> Int", r#"
pub type Updater = { pub func apply(item: ref[item] Replace) -> () [reshape item] }
pub type Implementation = & Updater & {}
impl Implementation {
    pub func apply(item: ref[item] Replace) -> () [reshape item] { update(item) }
}
func invoke(updater: Updater, item: ref[item] Replace) -> () [reshape item] {
    updater.apply(item)
}
func main() -> Int"#).replace("pair(holder.first, holder.second)", "let updater = Implementation {}\ninvoke(updater, holder.first)\ninvoke(updater, holder.second)");
        check(&source);
    }
}
