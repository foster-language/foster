//! Definition locations, symbol references, and versioned rename edits.
use super::Workspace;
use super::semantic::{SymbolIdentity, imported_declarations, self_type_owner_at, symbol_at};
use super::source::{
    find_name, identifier_at, is_ident, module_for_uri, path_to_uri, qualifier_before,
};
use crate::lsp::byte_range_to_lsp;
use lsp_types::{
    DocumentChanges, Location, OneOf, OptionalVersionedTextDocumentIdentifier, ReferenceParams,
    RenameParams, TextDocumentEdit, TextDocumentPositionParams, TextEdit, Uri, WorkspaceEdit,
};

impl Workspace {
    pub(in crate::lsp) fn definition(
        &self,
        params: &TextDocumentPositionParams,
    ) -> Option<Location> {
        let compilation = self.semantic_compilation_for(&params.text_document.uri)?;
        let module_id = module_for_uri(&compilation, &params.text_document.uri)?;
        let module = &compilation.hir.modules[module_id];
        let snapshot = self.semantic_snapshot(&compilation, &params.text_document.uri)?;
        let source = snapshot.semantic_source();
        let offset = snapshot.current_position_to_semantic_offset(params.position)?;
        let location = (|| {
            if let Some(symbol) = symbol_at(&compilation, module_id, source, offset)
                && let Some(location) = symbol_declaration(&compilation, symbol)
            {
                return Some(location);
            }
            let (name, name_start) = identifier_at(source, offset)?;
            let qualifier = qualifier_before(source, name_start);
            if qualifier.is_none()
                && name == "Self"
                && let Some(symbol) = self_type_owner_at(&compilation, module_id, offset)
            {
                return symbol_declaration(&compilation, symbol);
            }

            if qualifier.is_none() && name == "CodePoint" {
                return embedded_module_location("core.code_point");
            }

            if qualifier.is_none()
                && let Some((function_id, function)) =
                    compilation.hir.functions.iter().find(|(_, function)| {
                        function.module == module_id
                            && function.span.start <= offset
                            && offset <= function.span.end
                    })
                && let Some((_, local)) = compilation
                    .hir
                    .locals
                    .iter()
                    .find(|(_, local)| local.function == function_id && local.name == name)
            {
                let span = find_name(source, function.span.clone(), &local.name)
                    .unwrap_or_else(|| local.span.clone());
                return Some(Location::new(
                    params.text_document.uri.clone(),
                    byte_range_to_lsp(source, span),
                ));
            }

            let target_module = qualifier
                .as_deref()
                .and_then(|qualifier| module.imports.get(qualifier).copied())
                .unwrap_or(module_id);
            if qualifier.is_none()
                && let Some(import) = module
                    .imports_with_spans
                    .iter()
                    .find(|import| import.item_name.is_none() && import.name == name)
            {
                return module_location(&compilation, import.target);
            }
            definition_in_module(&compilation, target_module, name).or_else(|| {
                imported_declarations(&compilation, module_id, name)
                    .into_iter()
                    .find_map(|(target, actual)| {
                        definition_in_module(&compilation, target, &actual)
                    })
            })
        })()?;
        self.remap_semantic_location(&compilation, location)
    }

    pub(in crate::lsp) fn references(&self, params: &ReferenceParams) -> Option<Vec<Location>> {
        let position = &params.text_document_position;
        let compilation = self.semantic_compilation_for(&position.text_document.uri)?;
        let module = module_for_uri(&compilation, &position.text_document.uri)?;
        let snapshot = self.semantic_snapshot(&compilation, &position.text_document.uri)?;
        let source = snapshot.semantic_source();
        let offset = snapshot.current_position_to_semantic_offset(position.position)?;
        let symbol = symbol_at(&compilation, module, source, offset)?;
        let mut locations = symbol_locations(&compilation, symbol);
        if !params.context.include_declaration {
            let declaration = symbol_declaration(&compilation, symbol);
            locations.retain(|location| Some(location) != declaration.as_ref());
        }
        Some(
            locations
                .into_iter()
                .filter_map(|location| self.remap_semantic_location(&compilation, location))
                .collect(),
        )
    }

    pub(in crate::lsp) fn rename(&self, params: &RenameParams) -> Option<WorkspaceEdit> {
        if !valid_identifier(&params.new_name) {
            return None;
        }
        let position = &params.text_document_position;
        let compilation = if self.compilations.snapshot_only {
            // Edits require a current package, even for a local rename.
            self.compile_for(&position.text_document.uri).ok()?
        } else {
            self.semantic_compilation_for(&position.text_document.uri)?
        };
        let module = module_for_uri(&compilation, &position.text_document.uri)?;
        let snapshot = self.semantic_snapshot(&compilation, &position.text_document.uri)?;
        let source = snapshot.semantic_source();
        let offset = snapshot.current_position_to_semantic_offset(position.position)?;
        let symbol = symbol_at(&compilation, module, source, offset)?;
        if matches!(symbol, SymbolIdentity::Builtin(_)) {
            return None;
        }
        if params.new_name == "Self"
            && matches!(
                symbol,
                SymbolIdentity::Record(_) | SymbolIdentity::Variant(_)
            )
        {
            return None;
        }
        if !matches!(symbol, SymbolIdentity::Local(_))
            && !self.compilation_sources_are_current(&compilation)
        {
            return None;
        }
        let shorthand = match symbol {
            SymbolIdentity::Local(local) => record_shorthand_spans(&compilation, local),
            _ => Vec::new(),
        };
        let mut grouped = std::collections::BTreeMap::<String, (Uri, Vec<TextEdit>)>::new();
        for semantic_location in symbol_locations(&compilation, symbol) {
            let new_text = if shorthand
                .iter()
                .any(|range| byte_range_to_lsp(source, range.clone()) == semantic_location.range)
            {
                let SymbolIdentity::Local(local) = symbol else {
                    unreachable!()
                };
                format!(
                    "{}: {}",
                    compilation.hir.locals[local].name, params.new_name
                )
            } else {
                params.new_name.clone()
            };
            let Some(location) = self.remap_semantic_location(&compilation, semantic_location)
            else {
                continue;
            };
            grouped
                .entry(location.uri.as_str().to_owned())
                .or_insert_with(|| (location.uri.clone(), Vec::new()))
                .1
                .push(TextEdit {
                    range: location.range,
                    new_text,
                });
        }
        let edits = grouped
            .into_values()
            .map(|(uri, edits)| TextDocumentEdit {
                text_document: OptionalVersionedTextDocumentIdentifier {
                    version: self.version(&uri),
                    uri,
                },
                edits: edits.into_iter().map(OneOf::Left).collect(),
            })
            .collect();
        Some(WorkspaceEdit {
            changes: None,
            document_changes: Some(DocumentChanges::Edits(edits)),
            change_annotations: None,
        })
    }
}

// A renamed shorthand local must retain the stored field's name.
fn record_shorthand_spans(
    compilation: &crate::compiler::RecoveryCompilation,
    local: crate::hir::LocalId,
) -> Vec<std::ops::Range<usize>> {
    use crate::hir::{self, visit::Visitor};
    struct Find {
        local: hir::LocalId,
        spans: Vec<std::ops::Range<usize>>,
    }
    impl Find {
        fn pattern(&mut self, hir: &hir::PackageHir, pattern: &hir::Pattern) {
            match pattern.unspanned() {
                hir::Pattern::Record { fields } => {
                    for (name, pattern) in fields {
                        if matches!(pattern.unspanned(), hir::Pattern::Binding(local) if *local == self.local)
                            && name == &hir.locals[self.local].name
                            && let Some(span) = pattern.span()
                        {
                            self.spans.push(span);
                        }
                        self.pattern(hir, pattern);
                    }
                }
                hir::Pattern::Variant { fields, .. } => {
                    for pattern in fields {
                        self.pattern(hir, pattern);
                    }
                }
                _ => {}
            }
        }
    }
    impl Visitor for Find {
        fn visit_statement(&mut self, hir: &hir::PackageHir, statement: &hir::Stmt) {
            if let hir::Stmt::Destructure { pattern, .. } = statement {
                self.pattern(hir, pattern);
            }
            hir::visit::walk_statement(self, hir, statement);
        }
        fn visit_expression(&mut self, hir: &hir::PackageHir, expression: hir::ExprId) {
            match &hir.expressions[expression] {
                hir::Expr::Branch { arms, .. } => {
                    for arm in arms {
                        if let hir::BranchTest::Pattern(pattern) = &arm.test {
                            self.pattern(hir, pattern);
                        }
                    }
                }
                hir::Expr::Record { fields, .. } => {
                    for (name, value) in fields {
                        if matches!(hir.expressions[*value], hir::Expr::Name(hir::ResolvedName::Local(local)) if local == self.local)
                            && name == &hir.locals[self.local].name
                            && let Some(span) = hir.expression_spans.get(value)
                        {
                            self.spans.push(span.clone());
                        }
                    }
                }
                _ => {}
            }
            hir::visit::walk_expression(self, hir, expression);
        }
    }
    let function = &compilation.hir.functions[compilation.hir.locals[local].function];
    let mut find = Find {
        local,
        spans: Vec::new(),
    };
    find.visit_block(&compilation.hir, &function.body);
    let Some(source) = compilation
        .package
        .module(&compilation.hir.modules[function.module].name)
        .and_then(|module| module.source.as_deref())
    else {
        return Vec::new();
    };
    let Ok(tokens) = crate::lexer::lex(source) else {
        return Vec::new();
    };
    find.spans.retain(|span| {
        source.get(span.clone()) == Some(compilation.hir.locals[local].name.as_str())
            && tokens
                .iter()
                .rev()
                .find(|token| {
                    token.range.end <= span.start
                        && !matches!(token.kind, crate::lexer::TokenKind::Newline)
                })
                .is_some_and(|token| {
                    matches!(
                        token.kind,
                        crate::lexer::TokenKind::LBrace | crate::lexer::TokenKind::Comma
                    )
                })
    });
    find.spans
}

fn symbol_locations(
    compilation: &crate::compiler::RecoveryCompilation,
    symbol: SymbolIdentity,
) -> Vec<Location> {
    let mut locations = Vec::new();
    for (module_id, module) in compilation.hir.modules.iter() {
        let Some(path) = module.source_path.as_deref() else {
            continue;
        };
        let Some(source) = compilation
            .package
            .module(&module.name)
            .and_then(|module| module.source.as_deref())
        else {
            continue;
        };
        let Some(uri) = path_to_uri(path.as_std_path()) else {
            continue;
        };
        let name = symbol_name(compilation, symbol);
        let source_name = name.rsplit('.').next().unwrap_or(name).to_owned();
        let ranges = identifier_ranges(source, &source_name).collect::<Vec<_>>();
        for range in ranges {
            let lookup_offset = source_name
                .rsplit_once('.')
                .map_or(range.start, |(_, member)| range.end - member.len());
            if symbol_at(compilation, module_id, source, lookup_offset) == Some(symbol) {
                let location_range = source_name
                    .rsplit_once('.')
                    .map_or(range.clone(), |(_, member)| {
                        range.end - member.len()..range.end
                    });
                locations.push(Location::new(
                    uri.clone(),
                    byte_range_to_lsp(source, location_range),
                ));
            }
        }
    }
    locations
}

fn symbol_name(compilation: &crate::compiler::RecoveryCompilation, symbol: SymbolIdentity) -> &str {
    match symbol {
        SymbolIdentity::Local(local) => &compilation.hir.locals[local].name,
        SymbolIdentity::Constant(constant) => &compilation.hir.constants[constant].name,
        SymbolIdentity::Function(function) => &compilation.hir.functions[function].name,
        SymbolIdentity::Record(record) => &compilation.hir.records[record].name,
        SymbolIdentity::RequiredMethod(record, method) => {
            &compilation.hir.records[record].methods[method].name
        }
        SymbolIdentity::Variant(variant) => &compilation.hir.variant_types[variant].name,
        SymbolIdentity::Builtin(builtin) => crate::lsp::builtins::info(builtin).name,
    }
}

pub(super) fn symbol_declaration(
    compilation: &crate::compiler::RecoveryCompilation,
    symbol: SymbolIdentity,
) -> Option<Location> {
    match symbol {
        SymbolIdentity::Local(local) => {
            let definition = &compilation.hir.locals[local];
            let function = &compilation.hir.functions[definition.function];
            location(
                compilation,
                function.module,
                definition.span.clone(),
                &definition.name,
            )
        }
        SymbolIdentity::Constant(constant) => {
            let definition = &compilation.hir.constants[constant];
            location(
                compilation,
                definition.module,
                definition.span.clone(),
                &definition.name,
            )
        }
        SymbolIdentity::Function(function) => {
            let definition = &compilation.hir.functions[function];
            let member = definition
                .name
                .rsplit_once('.')
                .map_or(definition.name.as_str(), |(_, member)| member);
            location(
                compilation,
                definition.module,
                definition.span.clone(),
                member,
            )
        }
        SymbolIdentity::Record(record) => {
            let definition = &compilation.hir.records[record];
            location(
                compilation,
                definition.module,
                definition.span.clone(),
                &definition.name,
            )
        }
        SymbolIdentity::RequiredMethod(record, method) => {
            let record = &compilation.hir.records[record];
            let method = &record.methods[method];
            location(
                compilation,
                record.module,
                method.span.clone(),
                &method.name,
            )
        }
        SymbolIdentity::Variant(variant) => {
            let definition = &compilation.hir.variant_types[variant];
            location(
                compilation,
                definition.module,
                definition.span.clone(),
                &definition.name,
            )
        }
        SymbolIdentity::Builtin(builtin) => crate::lsp::builtins::definition_location(builtin),
    }
}

fn identifier_ranges<'a>(
    source: &'a str,
    expected: &'a str,
) -> impl Iterator<Item = std::ops::Range<usize>> + 'a {
    let bytes = source.as_bytes();
    let mut index = 0;
    std::iter::from_fn(move || {
        while index < bytes.len() {
            if !is_ident(bytes[index]) {
                index += 1;
                continue;
            }
            let start = index;
            while index < bytes.len() && is_ident(bytes[index]) {
                index += 1;
            }
            if &source[start..index] == expected {
                return Some(start..index);
            }
        }
        None
    })
}

fn valid_identifier(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|first| first == b'_' || first.is_ascii_alphabetic())
        && bytes.all(is_ident)
}

fn definition_in_module(
    compilation: &crate::compiler::RecoveryCompilation,
    module: crate::hir::ModuleId,
    name: &str,
) -> Option<Location> {
    let definition = &compilation.hir.modules[module];
    let span = definition
        .constants
        .get(name)
        .map(|id| compilation.hir.constants[*id].span.clone())
        .or_else(|| {
            definition
                .functions
                .get(name)
                .and_then(|overloads| overloads.first())
                .map(|id| compilation.hir.functions[*id].span.clone())
        })
        .or_else(|| {
            definition
                .records
                .get(name)
                .map(|id| compilation.hir.records[*id].span.clone())
        })
        .or_else(|| {
            definition
                .variant_types
                .get(name)
                .map(|id| compilation.hir.variant_types[*id].span.clone())
        })?;
    location(compilation, module, span, name)
}

fn module_location(
    compilation: &crate::compiler::RecoveryCompilation,
    module: crate::hir::ModuleId,
) -> Option<Location> {
    location(compilation, module, 0..0, "")
}

fn embedded_module_location(module: &str) -> Option<Location> {
    let path = crate::package::embedded_source_path(module)?;
    Some(Location::new(
        path_to_uri(path.as_std_path())?,
        lsp_types::Range::default(),
    ))
}

fn location(
    compilation: &crate::compiler::RecoveryCompilation,
    module: crate::hir::ModuleId,
    span: std::ops::Range<usize>,
    name: &str,
) -> Option<Location> {
    let module = &compilation.hir.modules[module];
    let path = module.source_path.as_deref()?;
    let source = compilation
        .package
        .module(&module.name)?
        .source
        .as_deref()?;
    let span = find_name(source, span, name).unwrap_or(0..0);
    Some(Location::new(
        path_to_uri(path.as_std_path())?,
        byte_range_to_lsp(source, span),
    ))
}
