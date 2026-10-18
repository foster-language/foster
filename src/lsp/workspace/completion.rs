//! Scope-aware completion, declaration candidates, and import edits.
use super::Workspace;
use super::semantic::function_at;
use super::source::{identifier_at, module_for_uri, position_to_offset, qualifier_before};
use crate::lsp::byte_range_to_lsp;
use lsp_types::{
    CompletionItem, CompletionItemKind, CompletionParams, CompletionResponse, Documentation,
    MarkupContent, MarkupKind, TextEdit,
};
use std::collections::{HashMap, HashSet};

impl Workspace {
    pub(in crate::lsp) fn completion(
        &self,
        params: &CompletionParams,
    ) -> Option<CompletionResponse> {
        let position = &params.text_document_position;
        let mut items = std::collections::BTreeMap::<String, CompletionItem>::new();
        let mut current_names = None;
        if let Some(document) = self.documents.get(&position.text_document.uri)
            && let Some(offset) = position_to_offset(&document.text, position.position)
        {
            if let Some(current) = self
                .editing_completions(&position.text_document.uri, &document.text, offset)
                .or_else(|| {
                    // Lexical damage elsewhere may prevent parsing the buffer. The
                    // existing mapping proves this entire function is unchanged.
                    let compilation = self.compilations.available(&position.text_document.uri)?;
                    let snapshot =
                        self.semantic_snapshot(&compilation, &position.text_document.uri)?;
                    let offset = snapshot.current_position_to_semantic_offset(position.position)?;
                    self.editing_completions(
                        &position.text_document.uri,
                        snapshot.semantic_source(),
                        offset,
                    )
                })
            {
                current_names = Some(current.keys().cloned().collect::<HashSet<_>>());
                items.extend(current);
            } else {
                return Some(CompletionResponse::Array(Vec::new()));
            }
            add_arguments_auto_import_completion(&document.text, offset, &mut items);
            let start = identifier_at(&document.text, offset).map_or(offset, |(_, start)| start);
            if qualifier_before(&document.text, start).is_none() {
                add_keyword_completions(&mut items);
            }
        }
        let compilation = match self.semantic_compilation_for(&position.text_document.uri) {
            Some(compilation) => compilation,
            None => {
                return (!items.is_empty())
                    .then(|| CompletionResponse::Array(items.into_values().collect()));
            }
        };
        let module_id = module_for_uri(&compilation, &position.text_document.uri)?;
        let module = &compilation.hir.modules[module_id];
        let snapshot = self.semantic_snapshot(&compilation, &position.text_document.uri)?;
        let source = snapshot.semantic_source();
        let Some(offset) = snapshot.current_position_to_semantic_offset(position.position) else {
            return Some(CompletionResponse::Array(items.into_values().collect()));
        };
        let start = identifier_at(source, offset).map_or(offset, |(_, start)| start);
        let qualifier = qualifier_before(source, start);

        if let Some(ref qualifier) = qualifier {
            if !add_associated_completions(&compilation, module_id, qualifier, &mut items)
                && let Some(target) = module.imports.get(qualifier)
            {
                add_module_completions(&compilation, *target, true, &mut items);
            }
        } else {
            add_keyword_completions(&mut items);
            if let Some(function) = function_at(&compilation, module_id, offset) {
                let mut counts = HashMap::new();
                for (_, local) in compilation
                    .hir
                    .locals
                    .iter()
                    .filter(|(_, local)| local.function == function)
                {
                    *counts.entry(local.name.as_str()).or_insert(0usize) += 1;
                }
                for (local, definition) in compilation.hir.locals.iter().filter(|(_, local)| {
                    local.function == function
                        && local.kind != crate::hir::LocalKind::CapturedValue
                        && !local.name.starts_with('$')
                }) {
                    if current_names
                        .as_ref()
                        .is_some_and(|names| !names.contains(&definition.name))
                    {
                        continue;
                    }
                    if current_names.is_some()
                        && items
                            .get(&definition.name)
                            .is_some_and(|item| item.kind != Some(CompletionItemKind::VARIABLE))
                    {
                        continue;
                    }
                    // Without a unique declaration identity, keep the current syntax
                    // candidate instead of attaching another shadowed local's type.
                    let detail = (counts[definition.name.as_str()] == 1)
                        .then(|| {
                            compilation
                                .types
                                .local_type(local)
                                .map(|ty| compilation.types.display(ty))
                        })
                        .flatten();
                    if let Some(item) = items.get_mut(&definition.name)
                        && item.detail.is_none()
                    {
                        item.detail = detail.clone();
                    }
                    insert_completion(
                        &mut items,
                        &definition.name,
                        CompletionItemKind::VARIABLE,
                        detail,
                    );
                }
            }
            add_module_completions(&compilation, module_id, false, &mut items);
            for name in module.imported_types.keys() {
                insert_completion(&mut items, name, CompletionItemKind::CLASS, None);
            }
            for name in module.imported_values.keys() {
                let constant = module.imported_values[name]
                    .iter()
                    .any(|value| matches!(value, crate::hir::ResolvedName::Constant(_)));
                insert_completion(
                    &mut items,
                    name,
                    if constant {
                        CompletionItemKind::CONSTANT
                    } else {
                        CompletionItemKind::FUNCTION
                    },
                    None,
                );
            }
            for import in &module.imports_with_spans {
                if import.item_name.is_some() {
                    continue;
                }
                insert_completion(
                    &mut items,
                    &import.name,
                    CompletionItemKind::MODULE,
                    Some(compilation.hir.modules[import.target].name.clone()),
                );
            }
            insert_completion(
                &mut items,
                "Never",
                CompletionItemKind::CLASS,
                Some("Uninhabited return type".into()),
            );
        }
        if qualifier.is_none()
            && let Some(names) = current_names
        {
            items.retain(|name, item| {
                names.contains(name)
                    || item.kind == Some(CompletionItemKind::KEYWORD)
                    || name == "Never"
                    || item.additional_text_edits.is_some()
            });
        }
        Some(CompletionResponse::Array(items.into_values().collect()))
    }
}

fn add_arguments_auto_import_completion(
    source: &str,
    offset: usize,
    items: &mut std::collections::BTreeMap<String, CompletionItem>,
) {
    let Some((prefix, _)) = identifier_at(source, offset) else {
        return;
    };
    if !"Arguments".starts_with(prefix) {
        return;
    }
    let Ok(program) = crate::parse(source) else {
        return;
    };
    let in_parameter_type = program.functions.iter().any(|function| {
        function.parameters.iter().any(|parameter| {
            parameter
                .type_span
                .as_ref()
                .is_some_and(|span| span.start <= offset && offset <= span.end)
        })
    });
    if !in_parameter_type {
        return;
    }
    let imported = program.imports.iter().any(|import| {
        !import.static_
            && (import.path == ["std", "process", "Arguments"]
                || (import.wildcard && import.path == ["std", "process"]))
    });
    let additional_text_edits = (!imported)
        .then(|| arguments_import_edit(source, &program))
        .flatten()
        .map(|edit| vec![edit]);
    items.insert(
        "Arguments".into(),
        CompletionItem {
            label: "Arguments".into(),
            kind: Some(CompletionItemKind::STRUCT),
            detail: Some("std.process.Arguments".into()),
            documentation: Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: "The executable name and command-line values supplied to `main`.".into(),
            })),
            sort_text: Some("0_Arguments".into()),
            insert_text: Some("Arguments".into()),
            additional_text_edits,
            ..CompletionItem::default()
        },
    );
}

fn arguments_import_edit(source: &str, program: &crate::ast::Program) -> Option<TextEdit> {
    let (offset, new_text) = if let Some(import) = program.imports.last() {
        (import.span.end, "\nimport std.process.Arguments")
    } else {
        let mut offset = 0;
        for token in crate::lexer::lex(source).ok()? {
            match token.kind {
                crate::lexer::TokenKind::Newline | crate::lexer::TokenKind::ModuleDocComment(_) => {
                    offset = token.range.end
                }
                _ => break,
            }
        }
        (offset, "import std.process.Arguments\n")
    };
    let position = byte_range_to_lsp(source, offset..offset).start;
    Some(TextEdit {
        range: lsp_types::Range::new(position, position),
        new_text: new_text.into(),
    })
}

pub(super) fn add_module_completions(
    compilation: &crate::compiler::RecoveryCompilation,
    module: crate::hir::ModuleId,
    public_only: bool,
    items: &mut std::collections::BTreeMap<String, CompletionItem>,
) {
    let definitions = &compilation.hir.modules[module];
    for (name, constant) in &definitions.constants {
        let definition = &compilation.hir.constants[*constant];
        if !public_only || definition.public {
            insert_documented_completion(
                items,
                name,
                CompletionItemKind::CONSTANT,
                compilation
                    .types
                    .constants
                    .get(constant)
                    .map(|ty| compilation.types.display(*ty)),
                definition.documentation.as_deref(),
            );
        }
    }
    for (name, overloads) in &definitions.functions {
        let Some(function) = overloads
            .iter()
            .copied()
            .find(|function| !public_only || compilation.hir.functions[*function].public)
        else {
            continue;
        };
        let definition = &compilation.hir.functions[function];
        if !definition.name.contains('$') {
            let detail = compilation.types.function_type(function).map(|signature| {
                format!(
                    "func({}) -> {}",
                    signature
                        .parameters
                        .iter()
                        .map(|ty| compilation.types.display(ty.ty))
                        .collect::<Vec<_>>()
                        .join(", "),
                    compilation.types.display(signature.result)
                )
            });
            insert_documented_completion(
                items,
                name,
                CompletionItemKind::FUNCTION,
                detail,
                definition.documentation.as_deref(),
            );
        }
    }
    for (name, record) in &definitions.records {
        if !public_only || compilation.hir.records[*record].public {
            insert_documented_completion(
                items,
                name,
                CompletionItemKind::STRUCT,
                Some("type".into()),
                compilation.hir.records[*record].documentation.as_deref(),
            );
        }
    }
    for (name, variant) in &definitions.variant_types {
        if !public_only || compilation.hir.variant_types[*variant].public {
            insert_documented_completion(
                items,
                name,
                if compilation.hir.variant_types[*variant].kind
                    == crate::ast::VariantKind::Enum
                {
                    CompletionItemKind::ENUM
                } else {
                    CompletionItemKind::INTERFACE
                },
                Some(
                    if compilation.hir.variant_types[*variant].kind
                        == crate::ast::VariantKind::Enum
                    {
                        "enum"
                    } else {
                        "type"
                    }
                    .into(),
                ),
                compilation.hir.variant_types[*variant]
                    .documentation
                    .as_deref(),
            );
        }
    }
}

fn add_associated_completions(
    compilation: &crate::compiler::RecoveryCompilation,
    current_module: crate::hir::ModuleId,
    type_name: &str,
    items: &mut std::collections::BTreeMap<String, CompletionItem>,
) -> bool {
    let types = compilation.hir.visible_types(current_module, type_name);
    let [ty] = types.as_slice() else {
        return false;
    };
    let (module, actual_name) = compilation.hir.type_location(*ty);
    let public_only = module != current_module;
    let prefix = format!("{actual_name}.");
    for (name, overloads) in &compilation.hir.modules[module].functions {
        let Some(member) = name.strip_prefix(&prefix) else {
            continue;
        };
        let Some(function) = overloads
            .iter()
            .copied()
            .find(|function| !public_only || compilation.hir.functions[*function].public)
        else {
            continue;
        };
        let definition = &compilation.hir.functions[function];
        let detail = compilation.types.function_type(function).map(|signature| {
            format!(
                "func({}) -> {}",
                signature
                    .parameters
                    .iter()
                    .map(|ty| compilation.types.display(ty.ty))
                    .collect::<Vec<_>>()
                    .join(", "),
                compilation.types.display(signature.result)
            )
        });
        insert_documented_completion(
            items,
            member,
            CompletionItemKind::FUNCTION,
            detail,
            definition.documentation.as_deref(),
        );
    }
    true
}

pub(super) fn insert_completion(
    items: &mut std::collections::BTreeMap<String, CompletionItem>,
    label: &str,
    kind: CompletionItemKind,
    detail: Option<String>,
) {
    items
        .entry(label.to_owned())
        .or_insert_with(|| CompletionItem {
            label: label.to_owned(),
            kind: Some(kind),
            detail,
            ..CompletionItem::default()
        });
}

pub(super) fn insert_documented_completion(
    items: &mut std::collections::BTreeMap<String, CompletionItem>,
    label: &str,
    kind: CompletionItemKind,
    detail: Option<String>,
    documentation: Option<&str>,
) {
    insert_completion(items, label, kind, detail.clone());
    if let Some(item) = items.get_mut(label) {
        if item.kind != Some(kind) {
            return;
        }
        if item.detail.is_none() {
            item.detail = detail;
        }
    }
    if let Some(documentation) = documentation
        && let Some(item) = items.get_mut(label)
    {
        item.documentation = Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: documentation.to_owned(),
        }));
    }
}

fn add_keyword_completions(items: &mut std::collections::BTreeMap<String, CompletionItem>) {
    for keyword in [
        "panic", "assert", "await", "branch", "break", "continue", "false", "func", "impl",
        "import", "let", "loop", "while", "for", "in", "move", "not", "pub", "ref", "remote",
        "return", "true", "type", "enum", "try",
    ] {
        insert_completion(items, keyword, CompletionItemKind::KEYWORD, None);
    }
}
