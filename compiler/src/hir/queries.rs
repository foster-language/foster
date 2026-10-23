use crate::ast;

use super::{ExprId, LocalId, PackageHir};

/// Borrowed parameters need their caller's place when the result exposes it or
/// the function can mutate it. The source parameter already names that group.
pub(crate) fn parameter_requires_storage(
    name: &str,
    result: Option<&ast::TypeExpr>,
    effects: &[ast::Effect],
) -> bool {
    type_exposes_group(result, name)
        || effects.iter().any(|effect| {
            effect.target.root == name
                && matches!(effect.kind, ast::EffectKind::Mut | ast::EffectKind::Reshape)
        })
}

pub(crate) fn type_exposes_group(ty: Option<&ast::TypeExpr>, group: &str) -> bool {
    match ty {
        Some(ast::TypeExpr::Unit) => false,
        Some(ast::TypeExpr::Reference {
            group: found,
            value,
        }) => found == group || type_exposes_group(Some(value), group),
        Some(ast::TypeExpr::Named(_, arguments)) => arguments
            .iter()
            .any(|argument| type_exposes_group(Some(argument), group)),
        Some(ast::TypeExpr::Intersection(members)) => members
            .iter()
            .any(|member| type_exposes_group(Some(member), group)),
        Some(ast::TypeExpr::Function {
            parameters,
            result,
            effects,
            ..
        }) => {
            parameters
                .iter()
                .any(|parameter| type_exposes_group(Some(parameter), group))
                || type_exposes_group(Some(result), group)
                || effects.iter().any(|effect| effect.target.root == group)
        }
        None => false,
    }
}

pub(super) fn expression_uses_local(hir: &PackageHir, expression: ExprId, local: LocalId) -> bool {
    struct LocalUse {
        target: LocalId,
        found: bool,
    }

    impl super::visit::Visitor for LocalUse {
        fn visit_local_use(&mut self, local: LocalId) {
            self.found |= local == self.target;
        }
    }

    let mut visitor = LocalUse {
        target: local,
        found: false,
    };
    super::visit::Visitor::visit_expression(&mut visitor, hir, expression);
    visitor.found
}

/// Whether a borrowed method needs the caller's whole storage place.
/// Keep ordinary cursor reshaping distinct from replacing the receiver binding.
pub(crate) fn receiver_requires_storage(hir: &PackageHir, function: super::FunctionId) -> bool {
    receiver_storage(hir, function, &mut std::collections::HashSet::new())
}

fn receiver_storage(
    hir: &PackageHir,
    function: super::FunctionId,
    checked: &mut std::collections::HashSet<super::FunctionId>,
) -> bool {
    if !checked.insert(function) {
        return false;
    }
    let definition = &hir.functions[function];
    let Some(receiver) = definition.receiver else {
        return false;
    };
    let name = &hir.locals[receiver].name;
    if definition
        .effects
        .iter()
        .any(|effect| effect.kind == ast::EffectKind::Consume && effect.target.root == *name)
    {
        return false;
    }
    // Compiled interfaces retain the resolved receiver reference. Intrinsic
    // buffer writes need the allocation place despite having no body.
    if matches!(
        definition.parameters.first().and_then(|p| p.ty.as_ref()),
        Some(ast::TypeExpr::Reference { .. })
    ) || type_exposes_group(definition.return_type.as_ref(), name)
        || definition.intrinsic.as_deref() == Some("buffer.write")
    {
        return true;
    }
    struct Storage<'a> {
        function: super::FunctionId,
        receiver: LocalId,
        found: bool,
        visited: std::collections::HashSet<ExprId>,
        checked: &'a mut std::collections::HashSet<super::FunctionId>,
    }
    impl super::visit::Visitor for Storage<'_> {
        fn visit_expression(&mut self, hir: &PackageHir, expression: ExprId) {
            if self.found || !self.visited.insert(expression) {
                return;
            }
            let is_self = |id| matches!(hir.expressions[id], super::Expr::Name(super::ResolvedName::Local(local)) if local == self.receiver);
            match &hir.expressions[expression] {
                super::Expr::Reference(place) if is_self(*place) => self.found = true,
                super::Expr::Closure { captures, .. } => {
                    self.found = captures.iter().any(|capture| {
                        capture.local == self.receiver && capture.mode == super::CaptureMode::Ref
                    });
                }
                super::Expr::Call { callee, .. } => {
                    if let super::Expr::Member { object, name } = &hir.expressions[*callee]
                        && is_self(*object)
                        && let Some(owner) = &hir.functions[self.function].owner
                    {
                        self.found = hir
                            .functions_named(
                                hir.functions[self.function].module,
                                &format!("{owner}.{name}"),
                            )
                            .iter()
                            .any(|target| receiver_storage(hir, *target, self.checked));
                    }
                }
                _ => {}
            }
            super::visit::walk_expression(self, hir, expression);
        }
        fn visit_statement(&mut self, hir: &PackageHir, statement: &super::Stmt) {
            self.found |=
                matches!(statement, super::Stmt::Assign { local, .. } if *local == self.receiver);
            if !self.found {
                super::visit::walk_statement(self, hir, statement);
            }
        }
    }
    let mut storage = Storage {
        function,
        receiver,
        found: false,
        visited: std::collections::HashSet::new(),
        checked,
    };
    super::visit::Visitor::visit_block(&mut storage, hir, &definition.body);
    storage.found
}
