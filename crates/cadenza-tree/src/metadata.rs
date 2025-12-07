//! Metadata tracking for syntax nodes.
//!
//! This module provides support for attaching metadata to nodes, including:
//! - Source file information
//! - Line number computation
//! - Arbitrary key-value pairs (future: AnyMap support)

use std::sync::Arc;

/// Information about a source file.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SourceFile {
    /// The name or path of the source file.
    pub path: Arc<str>,
    /// The complete source text.
    pub source: Arc<str>,
    /// Cached line start positions for efficient line lookup.
    line_starts: Arc<[usize]>,
}

impl SourceFile {
    /// Create a new source file.
    pub fn new(path: impl Into<Arc<str>>, source: impl Into<Arc<str>>) -> Self {
        let path = path.into();
        let source = source.into();
        let line_starts = compute_line_starts(&source);
        Self {
            path,
            source,
            line_starts,
        }
    }

    /// Get the line number (0-indexed) for a byte offset.
    pub fn line_number(&self, offset: usize) -> usize {
        match self.line_starts.binary_search(&offset) {
            Ok(line) => line,
            Err(line) => line.saturating_sub(1),
        }
    }

    /// Get the line and column (both 0-indexed) for a byte offset.
    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        let line = self.line_number(offset);
        let line_start = self.line_starts[line];
        let col = offset - line_start;
        (line, col)
    }

    /// Get the source text for a specific line (0-indexed).
    pub fn line_text(&self, line: usize) -> Option<&str> {
        let start = *self.line_starts.get(line)?;
        let end = self
            .line_starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.source.len());
        Some(&self.source[start..end])
    }
}

/// Compute line start positions for a source string.
fn compute_line_starts(source: &str) -> Arc<[usize]> {
    let mut line_starts = vec![0];
    for (i, c) in source.char_indices() {
        if c == '\n' {
            line_starts.push(i + 1);
        }
    }
    line_starts.into()
}

/// Metadata that can be attached to a syntax node.
///
/// This allows tracking additional information about nodes beyond their
/// basic structure.
#[derive(Debug, Clone, Default)]
pub struct NodeMetadata {
    /// The source file this node comes from, if any.
    pub source_file: Option<Arc<SourceFile>>,
    // Future: add AnyMap for arbitrary metadata
}

impl NodeMetadata {
    /// Create empty metadata.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create metadata with a source file.
    pub fn with_source_file(source_file: Arc<SourceFile>) -> Self {
        Self {
            source_file: Some(source_file),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_file_line_numbers() {
        let source = "line 0\nline 1\nline 2";
        let file = SourceFile::new("test.cdz", source);

        // First line
        assert_eq!(file.line_number(0), 0);
        assert_eq!(file.line_number(5), 0);

        // Second line (starts at byte 7)
        assert_eq!(file.line_number(7), 1);
        assert_eq!(file.line_number(12), 1);

        // Third line (starts at byte 14)
        assert_eq!(file.line_number(14), 2);
        assert_eq!(file.line_number(20), 2);
    }

    #[test]
    fn test_source_file_line_col() {
        let source = "abc\ndef\nghi";
        let file = SourceFile::new("test.cdz", source);

        assert_eq!(file.line_col(0), (0, 0)); // 'a'
        assert_eq!(file.line_col(2), (0, 2)); // 'c'
        assert_eq!(file.line_col(4), (1, 0)); // 'd'
        assert_eq!(file.line_col(8), (2, 0)); // 'g'
    }

    #[test]
    fn test_source_file_line_text() {
        let source = "line 0\nline 1\nline 2";
        let file = SourceFile::new("test.cdz", source);

        assert_eq!(file.line_text(0), Some("line 0\n"));
        assert_eq!(file.line_text(1), Some("line 1\n"));
        assert_eq!(file.line_text(2), Some("line 2"));
        assert_eq!(file.line_text(3), None);
    }
}
