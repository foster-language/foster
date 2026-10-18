//! Source lookup, snapshot position mapping, identifiers, and file URI conversion.
use super::Workspace;
use crate::lsp::snapshot::SemanticSnapshot;
use lsp_types::{Location, Position, Uri};
use std::path::{Path, PathBuf};
use std::str::FromStr;

impl Workspace {
    pub(super) fn semantic_snapshot<'a>(
        &'a self,
        compilation: &'a crate::compiler::RecoveryCompilation,
        uri: &Uri,
    ) -> Option<SemanticSnapshot<'a>> {
        let semantic_source = source_for_uri(compilation, uri)?;
        let current_source = self
            .documents
            .get(uri)
            .map_or(semantic_source, |document| document.text.as_str());
        SemanticSnapshot::new(compilation, uri, current_source)
    }

    pub(super) fn remap_semantic_location(
        &self,
        compilation: &crate::compiler::RecoveryCompilation,
        mut location: Location,
    ) -> Option<Location> {
        let Some(document) = self.documents.get(&location.uri) else {
            return Some(location);
        };
        let snapshot = SemanticSnapshot::new(compilation, &location.uri, &document.text)?;
        location.range = snapshot.semantic_range_to_current(location.range)?;
        Some(location)
    }

    pub(super) fn compilation_sources_are_current(
        &self,
        compilation: &crate::compiler::RecoveryCompilation,
    ) -> bool {
        self.documents.iter().all(|(uri, document)| {
            source_for_uri(compilation, uri).is_none_or(|source| source == document.text)
        })
    }
}

pub(in crate::lsp) fn module_for_uri(
    compilation: &crate::compiler::RecoveryCompilation,
    uri: &Uri,
) -> Option<crate::hir::ModuleId> {
    let path = uri_to_path(uri)?;
    compilation.hir.modules.iter().find_map(|(id, module)| {
        module
            .source_path
            .as_ref()
            .is_some_and(|source| source.as_std_path() == path)
            .then_some(id)
    })
}

pub(in crate::lsp) fn source_for_uri<'a>(
    compilation: &'a crate::compiler::RecoveryCompilation,
    uri: &Uri,
) -> Option<&'a str> {
    let module = module_for_uri(compilation, uri)?;
    compilation
        .package
        .module(&compilation.hir.modules[module].name)?
        .source
        .as_deref()
}

pub(super) fn find_name(
    source: &str,
    span: std::ops::Range<usize>,
    name: &str,
) -> Option<std::ops::Range<usize>> {
    let start = span.start.min(source.len());
    let end = span.end.min(source.len());
    let relative = source.get(start..end)?.find(name)?;
    Some(start + relative..start + relative + name.len())
}

pub(super) fn identifier_at(source: &str, offset: usize) -> Option<(&str, usize)> {
    let offset = offset.min(source.len());
    let identifier = |character: char| {
        character == '_'
            || character == '?'
            || character.is_alphabetic()
            || character.is_ascii_digit()
    };
    let mut start = offset;
    for (index, character) in source.get(..offset)?.char_indices().rev() {
        if !identifier(character) {
            break;
        }
        start = index;
    }
    let mut end = offset;
    for (index, character) in source.get(offset..)?.char_indices() {
        if !identifier(character) {
            break;
        }
        end = offset + index + character.len_utf8();
    }
    (start < end).then(|| (&source[start..end], start))
}

pub(super) fn qualifier_before(source: &str, start: usize) -> Option<String> {
    let bytes = source.as_bytes();
    let mut end = start;
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    if end >= 2 && &bytes[end - 2..end] == b"::" {
        end -= 2;
    } else if end > 0 && bytes[end - 1] == b'.' {
        end -= 1;
    } else {
        return None;
    }
    let mut begin = end;
    while begin > 0 && is_ident(bytes[begin - 1]) {
        begin -= 1;
    }
    (begin < end).then(|| source[begin..end].to_owned())
}

pub(super) fn is_ident(byte: u8) -> bool {
    byte == b'_' || byte == b'?' || byte.is_ascii_alphanumeric()
}

pub(in crate::lsp) fn position_to_offset(source: &str, position: Position) -> Option<usize> {
    let line = source.split_inclusive('\n').nth(position.line as usize)?;
    let line_start = source.as_ptr() as usize;
    let start = line.as_ptr() as usize - line_start;
    let mut utf16 = 0_u32;
    for (offset, character) in line.char_indices() {
        if utf16 >= position.character {
            return Some(start + offset);
        }
        utf16 += character.len_utf16() as u32;
    }
    Some(start + line.len())
}

pub(in crate::lsp) fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let value = uri.as_str().strip_prefix("file:///")?;
    let decoded = percent_decode(value)?;
    #[cfg(windows)]
    let decoded = decoded.replace('/', "\\");
    Some(PathBuf::from(decoded))
}

pub(in crate::lsp) fn path_to_uri(path: &Path) -> Option<Uri> {
    let value = path
        .to_string_lossy()
        .replace('\\', "/")
        .replace(' ', "%20");
    Uri::from_str(&format!("file:///{value}")).ok()
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = value.get(index + 1..index + 3)?;
            result.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            result.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(result).ok()
}
