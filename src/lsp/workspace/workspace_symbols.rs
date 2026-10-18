//! Workspace symbol search over indexed files and unsaved document overlays.
use super::Workspace;

impl Workspace {
    pub(in crate::lsp) fn workspace_symbols(
        &self,
        query: &str,
    ) -> Vec<lsp_types::SymbolInformation> {
        let files = std::sync::Arc::clone(&self.compilations.symbol_files.borrow());
        let mut symbols = Vec::new();
        for (uri, entries) in files.iter() {
            if crate::compiler::cancellation::is_cancelled() {
                return Vec::new();
            }
            if !self.documents.contains_key(uri) {
                symbols.extend(entries.iter().filter_map(|symbol| {
                    Some((super::super::symbols::rank(symbol, query)?, symbol.clone()))
                }));
            }
        }
        // The overlay entirely replaces its disk entry, including deleted or
        // malformed declarations. Search never resurrects stale source positions.
        for (uri, document) in &self.documents {
            if crate::compiler::cancellation::is_cancelled() {
                return Vec::new();
            }
            symbols.extend(
                super::super::symbols::source_symbols(uri, &document.text)
                    .into_iter()
                    .filter_map(|symbol| {
                        Some((super::super::symbols::rank(&symbol, query)?, symbol))
                    }),
            );
        }
        symbols.sort_by(|(left_rank, left), (right_rank, right)| {
            left_rank
                .cmp(right_rank)
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.location.uri.as_str().cmp(right.location.uri.as_str()))
                .then_with(|| left.location.range.start.cmp(&right.location.range.start))
        });
        symbols.dedup_by(|(_, left), (_, right)| {
            left.name == right.name && left.location == right.location
        });
        symbols
            .into_iter()
            .take(256)
            .map(|(_, symbol)| symbol)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{InitializeParams, Position, Uri};

    #[test]
    fn workspace_search_uses_unsaved_names_and_positions_without_checking() {
        let mut workspace = Workspace::new(&InitializeParams::default());
        workspace.compilations.snapshot_only = true;
        let uri: Uri = "file:///symbols-test.fos".parse().unwrap();
        let source =
            "/// buildValue documentation\nfunc buildValue() -> Int { 0 }\nfunc unrelated() {}\n";
        workspace.open(uri.clone(), source.into(), 1);
        let results = workspace.workspace_symbols("bV");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "buildValue");
        assert_eq!(results[0].location.range.start, Position::new(1, 5));
        workspace.change(uri.clone(), "\nfunc renamed() { 0 }".into(), 2);
        assert!(workspace.workspace_symbols("buildValue").is_empty());
        assert_eq!(
            workspace.workspace_symbols("RENAMED")[0]
                .location
                .range
                .start
                .line,
            1
        );
        workspace.close(&uri);
        assert!(workspace.workspace_symbols("").is_empty());
    }
}
