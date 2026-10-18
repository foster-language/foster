//! Workspace state and document lifecycle.
//!
//! Request implementations live in responsibility-specific child modules. Shared
//! source mapping, semantic resolution, and presentation helpers use explicit
//! imports so feature dependencies remain visible without changing workspace state.
use lsp_types::{InitializeParams, Uri};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

mod completion;
mod diagnostics;
mod document_symbols;
mod editing;
mod formatting;
mod hover;
mod navigation;
mod presentation;
mod semantic;
mod source;
mod workspace_symbols;

pub(super) use diagnostics::compiler_diagnostic;
pub(super) use presentation::callable_presentation;
pub(super) use source::{
    module_for_uri, path_to_uri, position_to_offset, source_for_uri, uri_to_path,
};

#[derive(Debug, Clone)]
pub(super) struct OpenDocument {
    pub(super) text: String,
    version: i32,
}

pub(super) struct Workspace {
    pub(super) root: Option<PathBuf>,
    pub(super) documents: HashMap<Uri, OpenDocument>,
    published: HashSet<Uri>,
    pub(super) compilations: super::compilation::CompilationCache,
}

impl Workspace {
    pub(super) fn new(params: &InitializeParams) -> Self {
        #[allow(deprecated)]
        let root = params
            .workspace_folders
            .as_ref()
            .and_then(|folders| folders.first())
            .and_then(|folder| uri_to_path(&folder.uri))
            .or_else(|| params.root_uri.as_ref().and_then(uri_to_path));
        let compilations = super::compilation::CompilationCache::default();
        compilations.symbol_index.borrow_mut().configure(params);
        Self {
            root,
            documents: HashMap::new(),
            published: HashSet::new(),
            compilations,
        }
    }

    pub(super) fn open(&mut self, uri: Uri, text: String, version: i32) {
        self.compilations.invalidate(&uri);
        self.documents.insert(uri, OpenDocument { text, version });
    }

    pub(super) fn change(&mut self, uri: Uri, text: String, version: i32) {
        self.open(uri, text, version);
    }

    pub(super) fn close(&mut self, uri: &Uri) {
        self.documents.remove(uri);
        self.compilations.close(uri, self.documents.keys());
    }

    pub(super) fn version(&self, uri: &Uri) -> Option<i32> {
        self.documents.get(uri).map(|document| document.version)
    }
}

#[cfg(test)]
mod tests;
