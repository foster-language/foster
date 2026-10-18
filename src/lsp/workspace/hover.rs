//! Hover, signature help, and inlay-hint requests over semantic snapshots.
use super::Workspace;
use super::presentation::{declaration_hover, documented_hover, symbol_hover};
use super::semantic::{function_at, imported_declarations, self_type_owner_at, symbol_at};
use super::source::{identifier_at, module_for_uri, position_to_offset, qualifier_before};
use crate::lsp::byte_range_to_lsp;
use lsp_types::{
    Hover, HoverContents, InlayHint, InlayHintParams, MarkupContent, MarkupKind, SignatureHelp,
    SignatureHelpParams, TextDocumentPositionParams,
};

impl Workspace {
    pub(in crate::lsp) fn hover(&self, params: &TextDocumentPositionParams) -> Option<Hover> {
        let compilation = self.semantic_compilation_for(&params.text_document.uri)?;
        let module_id = module_for_uri(&compilation, &params.text_document.uri)?;
        let module = &compilation.hir.modules[module_id];
        let snapshot = self.semantic_snapshot(&compilation, &params.text_document.uri)?;
        let semantic_source = snapshot.semantic_source();
        let current_source = snapshot.current_source();
        let current_offset = position_to_offset(current_source, params.position)?;
        let (name, start) = identifier_at(current_source, current_offset)?;
        let range = byte_range_to_lsp(current_source, start..start + name.len());
        let qualifier = qualifier_before(current_source, start);
        let semantic_offset = snapshot.current_offset_to_semantic(current_offset)?;

        let value = if qualifier.is_none()
            && name == "Self"
            && let Some(symbol) = self_type_owner_at(&compilation, module_id, semantic_offset)
            && let Some(value) = symbol_hover(&compilation, symbol)
        {
            value
        } else if let Some(symbol) =
            symbol_at(&compilation, module_id, semantic_source, semantic_offset)
            && let Some(value) = symbol_hover(&compilation, symbol)
        {
            value
        } else if qualifier.is_none()
            && let Some(function_id) = function_at(&compilation, module_id, semantic_offset)
            && let Some((local_id, _)) = compilation
                .hir
                .locals
                .iter()
                .find(|(_, local)| local.function == function_id && local.name == name)
        {
            let ty = compilation.types.local_type(local_id)?;
            documented_hover(format!("{}: {}", name, compilation.types.display(ty)), None)
        } else if let Some(symbol) =
            symbol_at(&compilation, module_id, semantic_source, semantic_offset)
            && let Some(value) = symbol_hover(&compilation, symbol)
        {
            value
        } else {
            let target = qualifier
                .as_deref()
                .and_then(|qualifier| module.imports.get(qualifier).copied())
                .unwrap_or(module_id);
            declaration_hover(&compilation, target, name).or_else(|| {
                (qualifier.is_none()).then(|| {
                    imported_declarations(&compilation, module_id, name)
                        .into_iter()
                        .find_map(|(target, actual)| {
                            declaration_hover(&compilation, target, &actual)
                        })
                })?
            })?
        };

        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            }),
            range: Some(range),
        })
    }

    pub(in crate::lsp) fn signature_help(
        &self,
        params: &SignatureHelpParams,
    ) -> Option<SignatureHelp> {
        let uri = &params.text_document_position_params.text_document.uri;
        let compilation = self.semantic_compilation_for(uri)?;
        let snapshot = self.semantic_snapshot(&compilation, uri)?;
        let semantic_offset = snapshot
            .current_position_to_semantic_offset(params.text_document_position_params.position)?;
        let mut semantic_params = params.clone();
        semantic_params.text_document_position_params.position =
            byte_range_to_lsp(snapshot.semantic_source(), semantic_offset..semantic_offset).start;
        crate::lsp::hints::signature_help(&compilation, &semantic_params)
    }

    pub(in crate::lsp) fn inlay_hints(&self, params: &InlayHintParams) -> Option<Vec<InlayHint>> {
        match self.compile_for(&params.text_document.uri) {
            Ok(compilation) => crate::lsp::hints::inlay_hints(&compilation, params),
            Err(_) => {
                let compilation = self.semantic_compilation_for(&params.text_document.uri)?;
                let current = &self.documents.get(&params.text_document.uri)?.text;
                crate::lsp::hints::inlay_hints_for_unchanged_functions(
                    &compilation,
                    params,
                    current,
                )
            }
        }
    }
}
