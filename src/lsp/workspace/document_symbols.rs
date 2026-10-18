//! Document outlines from current syntax, with semantic snapshot fallback.
use super::Workspace;
use super::source::{find_name, module_for_uri, uri_to_path};
use crate::lsp::byte_range_to_lsp;
use lsp_types::{DocumentSymbol, DocumentSymbolResponse, SymbolKind, Uri};

impl Workspace {
    pub(in crate::lsp) fn document_symbols(&self, uri: &Uri) -> Option<DocumentSymbolResponse> {
        let path = uri_to_path(uri)?;
        let source = self
            .documents
            .get(uri)
            .map(|document| document.text.clone())
            .or_else(|| std::fs::read_to_string(&path).ok())?;
        let program = match self.compilations.parse_document(&path, &source) {
            Ok(program) => program,
            Err(_) => return self.semantic_document_symbols(uri),
        };
        let mut symbols = Vec::new();
        for constant in &program.constants {
            symbols.push(symbol(
                &constant.name,
                SymbolKind::CONSTANT,
                &source,
                constant.span.clone(),
            ));
        }
        for record in &program.records {
            symbols.push(symbol(
                &record.name,
                SymbolKind::STRUCT,
                &source,
                record.span.clone(),
            ));
        }
        for variant in &program.variants {
            let kind = if variant.kind == crate::ast::VariantKind::Enum {
                SymbolKind::ENUM
            } else {
                SymbolKind::INTERFACE
            };
            symbols.push(symbol(&variant.name, kind, &source, variant.span.clone()));
        }
        for function in &program.functions {
            let name = function.name.clone();
            symbols.push(symbol(
                &name,
                SymbolKind::FUNCTION,
                &source,
                function.span.clone(),
            ));
        }
        Some(DocumentSymbolResponse::Nested(symbols))
    }

    fn semantic_document_symbols(&self, uri: &Uri) -> Option<DocumentSymbolResponse> {
        let compilation = self.semantic_compilation_for(uri)?;
        let module_id = module_for_uri(&compilation, uri)?;
        let module = &compilation.hir.modules[module_id];
        let source = compilation
            .package
            .module(&module.name)?
            .source
            .as_deref()?;
        let snapshot = self.semantic_snapshot(&compilation, uri)?;
        let mut symbols = Vec::new();
        for (_, constant) in compilation
            .hir
            .constants
            .iter()
            .filter(|(_, constant)| constant.module == module_id)
        {
            symbols.push(symbol(
                &constant.name,
                SymbolKind::CONSTANT,
                source,
                constant.span.clone(),
            ));
        }
        for (_, record) in compilation
            .hir
            .records
            .iter()
            .filter(|(_, record)| record.module == module_id)
        {
            symbols.push(symbol(
                &record.name,
                SymbolKind::STRUCT,
                source,
                record.span.clone(),
            ));
        }
        for (_, variant) in compilation
            .hir
            .variant_types
            .iter()
            .filter(|(_, variant)| variant.module == module_id)
        {
            symbols.push(symbol(
                &variant.name,
                if variant.kind == crate::ast::VariantKind::Enum {
                    SymbolKind::ENUM
                } else {
                    SymbolKind::INTERFACE
                },
                source,
                variant.span.clone(),
            ));
        }
        for (_, function) in
            compilation.hir.functions.iter().filter(|(_, function)| {
                function.module == module_id && !function.name.contains('$')
            })
        {
            let name = function.name.clone();
            symbols.push(symbol(
                &name,
                SymbolKind::FUNCTION,
                source,
                function.span.clone(),
            ));
        }
        symbols.retain_mut(|symbol| {
            let Some(range) = snapshot.semantic_range_to_current(symbol.range) else {
                return false;
            };
            let Some(selection_range) = snapshot.semantic_range_to_current(symbol.selection_range)
            else {
                return false;
            };
            symbol.range = range;
            symbol.selection_range = selection_range;
            true
        });
        Some(DocumentSymbolResponse::Nested(symbols))
    }
}

fn symbol(
    name: &str,
    kind: SymbolKind,
    source: &str,
    span: std::ops::Range<usize>,
) -> DocumentSymbol {
    let range = byte_range_to_lsp(source, span.clone());
    let selection = find_name(source, span, name.rsplit('.').next().unwrap_or(name))
        .map(|span| byte_range_to_lsp(source, span))
        .unwrap_or(range);
    #[allow(deprecated)]
    DocumentSymbol {
        name: name.to_owned(),
        detail: None,
        kind,
        tags: None,
        deprecated: None,
        range,
        selection_range: selection,
        children: None,
    }
}
