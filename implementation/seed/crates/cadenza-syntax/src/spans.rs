//! The span side-table: `StructId -> source span`.
//!
//! Spans live BESIDE the arena, never in it — the binary AST is span-free so formatting cannot
//! change its bytes. The table is total by construction: the parser records one span per structure
//! occurrence as it creates it, and occurrences are never deduplicated, so every `StructId` maps to
//! exactly the source range it came from (the two `x`s in `x + x` have distinct ids and distinct
//! spans, even though both resolve to one interned leaf). This is the source-tracking substrate
//! diagnostics and every later analysis (types, …) key off.

use crate::ast::StructId;
use crate::span::Span;

/// A source file identifier. `0` is the anonymous/single-file default; a multi-file driver assigns
/// distinct ids. Kept tiny and out of the arena — purely diagnostic metadata.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct FileId(pub u32);

/// Maps each structure occurrence to where it came from in source. Indexed by `StructId` position,
/// so lookup is O(1) and the table is exactly as long as the structure arena.
#[derive(Clone, Debug, Default)]
pub struct SpanTable {
    file: FileId,
    spans: Vec<Span>,
}

impl SpanTable {
    pub fn new(file: FileId) -> SpanTable {
        SpanTable { file, spans: Vec::new() }
    }

    /// Record the span for the next occurrence. The parser calls this once per created `StructId`,
    /// in id order, so `spans[id]` is that occurrence's span.
    pub fn push(&mut self, span: Span) {
        self.spans.push(span);
    }

    /// The source span of an occurrence, if recorded.
    pub fn get(&self, id: StructId) -> Option<Span> {
        self.spans.get(id.0 as usize).copied()
    }

    /// The file these spans are in.
    pub fn file(&self) -> FileId {
        self.file
    }

    pub fn len(&self) -> usize {
        self.spans.len()
    }
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }
}
