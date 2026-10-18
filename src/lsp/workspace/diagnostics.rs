//! Diagnostic conversion and generation-checked publication of diagnostics and snapshots.
use super::Workspace;
use super::source::{path_to_uri, uri_to_path};
use crate::lsp::{byte_range_to_lsp, error_diagnostic, publish};
use crossbeam_channel::Sender;
use lsp_server::Message;
use lsp_types::{Diagnostic, DiagnosticSeverity, Location, NumberOrString, Position, Uri};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::path::Path;

impl Workspace {
    pub(in crate::lsp) fn publish_diagnostics(
        &mut self,
        sender: &Sender<Message>,
        expected_generation: u64,
        generation: &std::sync::atomic::AtomicU64,
        publication: Option<&crate::lsp::SnapshotMailbox>,
    ) -> Result<(), Box<dyn Error>> {
        let Some(symbols) = self.compilations.symbol_index.borrow_mut().refresh() else {
            return Ok(());
        };
        *self.compilations.symbol_files.borrow_mut() = std::sync::Arc::new(symbols);
        if generation.load(std::sync::atomic::Ordering::Acquire) != expected_generation {
            return Ok(());
        }
        // Search results become available before semantic checking finishes.
        self.publish_snapshot(publication, expected_generation);
        let mut next_by_uri = HashMap::<String, (Uri, Vec<Diagnostic>, Option<i32>)>::new();
        for (focus_uri, document) in &self.documents {
            if crate::compiler::cancellation::is_cancelled() {
                return Ok(());
            }
            match self.compile_for(focus_uri) {
                Ok(compilation) => {
                    for (_, module) in compilation.hir.modules.iter() {
                        let Some(path) = module.source_path.as_deref() else {
                            continue;
                        };
                        if !self.should_publish_diagnostics_for(path.as_std_path()) {
                            continue;
                        }
                        let Some(uri) = path_to_uri(path.as_std_path()) else {
                            continue;
                        };
                        let source = compilation
                            .package
                            .module(&module.name)
                            .and_then(|module| module.source.as_deref())
                            .unwrap_or_default();
                        let mut diagnostics = compilation
                            .diagnostics
                            .iter()
                            .filter(|diagnostic| {
                                diagnostic.source_module.as_deref() == Some(&module.name)
                            })
                            .map(|diagnostic| compiler_diagnostic(source, &uri, diagnostic))
                            .collect::<Vec<_>>();
                        diagnostics.extend(
                            self.compilations
                                .parse_diagnostics(path.as_std_path())
                                .into_iter()
                                .map(|error| error_diagnostic(source, &uri, error)),
                        );
                        next_by_uri.insert(
                            uri.as_str().to_owned(),
                            (uri.clone(), diagnostics, self.version(&uri)),
                        );
                    }
                }
                Err(error) => {
                    let mut diagnostics = uri_to_path(focus_uri)
                        .map(|path| self.compilations.parse_diagnostics(&path))
                        .unwrap_or_default()
                        .into_iter()
                        .map(|error| error_diagnostic(&document.text, focus_uri, error))
                        .collect::<Vec<_>>();
                    diagnostics.push(error_diagnostic(&document.text, focus_uri, error));
                    next_by_uri.insert(
                        focus_uri.as_str().to_owned(),
                        (focus_uri.clone(), diagnostics, Some(document.version)),
                    );
                }
            }
        }
        let next = next_by_uri.into_values().collect::<Vec<_>>();

        if generation.load(std::sync::atomic::Ordering::Acquire) != expected_generation
            || crate::compiler::cancellation::is_cancelled()
        {
            return Ok(());
        }

        // Publish semantics before diagnostics, so a client reacting to diagnostics
        // can immediately query the corresponding snapshot.
        self.publish_snapshot(publication, expected_generation);
        let next_uris = next
            .iter()
            .map(|(uri, _, _)| uri.as_str().to_owned())
            .collect::<HashSet<_>>();
        for uri in self
            .published
            .iter()
            .filter(|uri| !next_uris.contains(uri.as_str()))
        {
            publish(sender, uri.clone(), Vec::new(), self.version(uri))?;
        }
        for (uri, diagnostics, version) in &next {
            let mut diagnostics = diagnostics.clone();
            for diagnostic in &mut diagnostics {
                diagnostic.data = Some(serde_json::json!({ "fosterVersion": version }));
            }
            publish(sender, uri.clone(), diagnostics, *version)?;
        }
        self.published = next.into_iter().map(|(uri, _, _)| uri).collect();
        Ok(())
    }

    fn publish_snapshot(
        &self,
        publication: Option<&crate::lsp::SnapshotMailbox>,
        expected_generation: u64,
    ) {
        if let Some(publication) = publication {
            let next = std::sync::Arc::new(self.compilations.published());
            let previous = publication
                .lock()
                .unwrap()
                .replace((expected_generation, next));
            // Releasing a large retired compilation must not hold the mailbox lock.
            drop(previous);
        }
    }

    pub(super) fn should_publish_diagnostics_for(&self, path: &Path) -> bool {
        self.root
            .as_deref()
            .is_some_and(|root| path.starts_with(root))
            || self
                .documents
                .keys()
                .filter_map(uri_to_path)
                .any(|document| document == path)
    }
}

pub(in crate::lsp) fn compiler_diagnostic(
    source: &str,
    uri: &Uri,
    diagnostic: &crate::diagnostic::Diagnostic,
) -> Diagnostic {
    let range = diagnostic
        .labels
        .iter()
        .find(|label| label.primary)
        .or_else(|| diagnostic.labels.first())
        .map_or_else(
            || lsp_types::Range::new(Position::new(0, 0), Position::new(0, 1)),
            |label| byte_range_to_lsp(source, label.range.clone()),
        );
    Diagnostic {
        range,
        severity: Some(match diagnostic.severity {
            crate::diagnostic::Severity::Error => DiagnosticSeverity::ERROR,
            crate::diagnostic::Severity::Warning => DiagnosticSeverity::WARNING,
        }),
        code: diagnostic.code.clone().map(NumberOrString::String),
        code_description: None,
        source: Some("foster".into()),
        message: lsp_diagnostic_message(diagnostic),
        related_information: {
            let related = diagnostic
                .labels
                .iter()
                .filter(|label| !label.primary)
                .map(|label| lsp_types::DiagnosticRelatedInformation {
                    location: Location::new(
                        uri.clone(),
                        byte_range_to_lsp(source, label.range.clone()),
                    ),
                    message: label.message.clone(),
                })
                .collect::<Vec<_>>();
            (!related.is_empty()).then_some(related)
        },
        tags: None,
        data: None,
    }
}

fn lsp_diagnostic_message(diagnostic: &crate::diagnostic::Diagnostic) -> String {
    let mut message = diagnostic.message.clone();
    for label in diagnostic.labels.iter().filter(|label| !label.primary) {
        message.push('\n');
        message.push_str(&label.message);
    }
    for note in &diagnostic.notes {
        message.push_str("\nnote: ");
        message.push_str(note);
    }
    if let Some(help) = &diagnostic.help {
        message.push_str("\nhelp: ");
        message.push_str(help);
    }
    message
}
