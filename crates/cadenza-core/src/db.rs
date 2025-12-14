//! Salsa database infrastructure for incremental compilation.
//!
//! This module defines the core database traits and implementations for the
//! Cadenza compiler using the Salsa framework. The database provides:
//!
//! - **On-demand computation**: Query only what you need, when you need it
//! - **Automatic incrementality**: Salsa tracks dependencies and recomputes only what changed
//! - **Extensibility**: Easy to add new queries without reinventing patterns
//!
//! ## Architecture
//!
//! The database is organized around a central `CadenzaDb` trait that all
//! queries operate on. Concrete implementations like `CadenzaDbImpl` provide
//! the storage backend.

// =============================================================================
// Input Types
// =============================================================================

/// A source file input containing the path and text content.
///
/// This is a Salsa input, meaning it can be mutated from outside the
/// database. When the text changes, Salsa automatically invalidates all
/// derived queries that depend on this source file.
///
/// # Example
///
/// ```ignore
/// use cadenza_core::{CadenzaDbImpl, SourceFile};
/// use salsa::Setter;
///
/// let mut db = CadenzaDbImpl::default();
/// let source = SourceFile::new(&db, "test.cdz".to_string(), "let x = 1".to_string());
///
/// // Later, update the source text
/// source.set_text(&mut db).to("let x = 2".to_string());
/// // All queries depending on this source are now invalidated
/// ```
#[salsa::input]
pub struct SourceFile {
    /// The path to the source file (e.g., "main.cdz", "lib/math.cdz").
    #[returns(ref)]
    pub path: String,

    /// The text content of the source file.
    #[returns(ref)]
    pub text: String,
}

// =============================================================================
// Tracked Types
// =============================================================================

/// A parsed file containing the concrete syntax tree (CST).
///
/// This is a Salsa tracked struct, meaning it is automatically memoized based
/// on its inputs. When the source file changes, Salsa will automatically
/// recompute this value and any queries that depend on it.
#[salsa::tracked]
pub struct ParsedFile<'db> {
    /// The source file that was parsed.
    pub source: SourceFile,

    /// The concrete syntax tree (CST) root node.
    #[returns(ref)]
    pub cst: cadenza_syntax::SyntaxNode,
}

// =============================================================================
// Accumulators
// =============================================================================

/// The severity level of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Severity {
    /// An error that prevents compilation or execution.
    Error,
    /// A warning that doesn't prevent compilation but indicates a potential issue.
    Warning,
    /// An informational hint or suggestion.
    Hint,
}

/// A related diagnostic that provides additional context or suggestions.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RelatedInformation {
    /// The source file containing the related information.
    pub source: SourceFile,

    /// The span in the source file.
    pub span: cadenza_syntax::span::Span,

    /// A message describing the relationship (e.g., "variable defined here").
    pub message: String,
}

/// A diagnostic message (error or warning) accumulated during compilation.
///
/// Diagnostics are collected using Salsa's accumulator pattern. Any tracked
/// function can emit diagnostics, and they can be collected after the query
/// completes.
#[salsa::accumulator]
pub struct Diagnostic {
    /// The source file where the diagnostic occurred.
    pub source: SourceFile,

    /// The severity level of this diagnostic.
    pub severity: Severity,

    /// The span in the source file where the diagnostic occurred.
    pub span: cadenza_syntax::span::Span,

    /// The diagnostic message.
    pub message: String,

    /// Related information providing additional context.
    pub related: Vec<RelatedInformation>,
}

// =============================================================================
// Tracked Functions
// =============================================================================

/// Parse a source file into a concrete syntax tree (CST).
///
/// This is a Salsa tracked function, meaning its result is automatically
/// memoized. When called with the same source file, it returns the cached
/// result. When the source file changes, Salsa automatically recomputes
/// the parse.
///
/// Parse errors are accumulated as diagnostics and can be retrieved using
/// `parse_file::accumulated::<Diagnostic>(db, source)`.
#[salsa::tracked]
pub fn parse_file(db: &dyn CadenzaDb, source: SourceFile) -> ParsedFile<'_> {
    use salsa::Accumulator;

    let text = source.text(db);
    let parse = cadenza_syntax::parse::parse(text);

    // Accumulate parse errors as diagnostics
    for error in &parse.errors {
        Diagnostic {
            source,
            severity: Severity::Error,
            span: error.span,
            message: error.message.clone(),
            related: vec![],
        }
        .accumulate(db);
    }

    ParsedFile::new(db, source, parse.syntax())
}

// =============================================================================
// Database Trait
// =============================================================================

/// The main database trait for Cadenza compiler queries.
///
/// This trait extends `salsa::Database` and serves as the interface for all
/// incremental queries in the Cadenza compiler.
#[salsa::db]
pub trait CadenzaDb: salsa::Database {}

/// The concrete database implementation for CLI and testing.
///
/// This struct provides the storage backend for Salsa queries.
#[salsa::db]
#[derive(Default, Clone)]
pub struct CadenzaDbImpl {
    storage: salsa::Storage<Self>,
}

#[salsa::db]
impl salsa::Database for CadenzaDbImpl {}

#[salsa::db]
impl CadenzaDb for CadenzaDbImpl {}

#[cfg(test)]
mod tests {
    use super::*;
    use salsa::Setter;

    #[test]
    fn test_db_creation() {
        let _db = CadenzaDbImpl::default();
    }

    #[test]
    fn test_source_file() {
        let mut db = CadenzaDbImpl::default();
        let source = SourceFile::new(&db, "test.cdz".to_string(), "let x = 1".to_string());

        assert_eq!(source.path(&db), "test.cdz");
        assert_eq!(source.text(&db), "let x = 1");

        // Test mutation
        source.set_text(&mut db).to("let x = 2".to_string());
        assert_eq!(source.text(&db), "let x = 2");
    }

    #[test]
    fn test_parse_file() {
        let db = CadenzaDbImpl::default();
        let source = SourceFile::new(&db, "test.cdz".to_string(), "let x = 1".to_string());

        let parsed = parse_file(&db, source);
        let cst = parsed.cst(&db);
        assert_eq!(cst.kind(), cadenza_syntax::token::Kind::Root);

        // Check that the source is correctly tracked
        assert!(parsed.source(&db) == source);

        // Check that there are no diagnostics for valid code
        let diagnostics = parse_file::accumulated::<Diagnostic>(&db, source);
        assert_eq!(diagnostics.len(), 0);
    }

    #[test]
    fn test_parse_file_with_errors() {
        let db = CadenzaDbImpl::default();
        let source = SourceFile::new(
            &db,
            "error.cdz".to_string(),
            "let x = \"unterminated".to_string(),
        );

        let _parsed = parse_file(&db, source);

        // Check that diagnostics were accumulated
        let diagnostics = parse_file::accumulated::<Diagnostic>(&db, source);
        assert!(
            !diagnostics.is_empty(),
            "Expected parse errors for unterminated string"
        );
    }

    #[test]
    fn test_parse_file_memoization() {
        let db = CadenzaDbImpl::default();
        let source = SourceFile::new(&db, "test.cdz".to_string(), "let x = 1".to_string());

        let parsed1 = parse_file(&db, source);
        let parsed2 = parse_file(&db, source);

        // Should return the same tracked value
        assert!(parsed1 == parsed2);
    }

    #[test]
    fn test_parse_file_invalidation() {
        let mut db = CadenzaDbImpl::default();
        let source = SourceFile::new(&db, "test.cdz".to_string(), "let x = 1".to_string());

        let parsed1 = parse_file(&db, source);
        let text1 = parsed1.cst(&db).text().to_string();

        // Modify the source
        source.set_text(&mut db).to("let y = 2".to_string());

        let parsed2 = parse_file(&db, source);
        let text2 = parsed2.cst(&db).text().to_string();

        // The CST should be different
        assert_ne!(text1, text2);
        assert!(text1.contains("x"));
        assert!(text2.contains("y"));
    }
}
