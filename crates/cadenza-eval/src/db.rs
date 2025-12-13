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
//!
//! ### Database Trait
//!
//! The `CadenzaDb` trait serves as the interface for all Salsa queries:
//!
//! ```ignore
//! #[salsa::db]
//! pub trait CadenzaDb: salsa::Database {}
//! ```
//!
//! ### Database Implementation
//!
//! `CadenzaDbImpl` is the concrete implementation used for CLI and testing:
//!
//! ```ignore
//! #[salsa::db]
//! pub struct CadenzaDbImpl {
//!     storage: salsa::Storage<Self>,
//! }
//! ```
//!
//! ## Phase 2: Source Tracking
//!
//! Phase 2 introduces source file tracking using Salsa:
//!
//! - [`SourceFile`]: Salsa input for source text with path and content
//!
//! Note: String interning continues to use the existing efficient `InternedString`
//! implementation in `interner.rs`, which provides zero-allocation lookups and
//! cached parsing for integer/float literals.
//!
//! ## Phase 3: Parsing
//!
//! Phase 3 makes parsing a tracked function with diagnostic accumulation:
//!
//! - [`ParsedFile`]: Tracked struct holding parsed CST and source reference
//! - [`parse_file`]: Tracked function that parses source text into CST
//! - [`Diagnostic`]: Accumulator for collecting parse errors and warnings
//!
//! ## Migration Status
//!
//! Phase 1 (Foundation), Phase 2 (Source Tracking), and Phase 3 (Parsing) are complete.
//! The database infrastructure is established with source file tracking and parsing.
//! The existing mutable `Compiler` and `EvalContext` architecture remains the primary
//! evaluation path.
//!
//! Future phases will:
//! - Phase 4: Convert evaluation to tracked functions
//! - Phase 5: Make type inference a set of queries
//! - Phase 6: Wire LSP to query the database
//!
//! See `/docs/SALSA_MIGRATION.md` for the complete migration plan.

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
/// ```
/// use cadenza_eval::db::{CadenzaDbImpl, SourceFile};
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
///
/// # Example
///
/// ```
/// use cadenza_eval::db::{CadenzaDbImpl, SourceFile, parse_file};
///
/// let db = CadenzaDbImpl::default();
/// let source = SourceFile::new(&db, "test.cdz".to_string(), "let x = 1".to_string());
/// let parsed = parse_file(&db, source);
///
/// // Access the CST
/// let cst = parsed.cst(&db);
/// ```
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

/// A diagnostic message (error or warning) accumulated during compilation.
///
/// Diagnostics are collected using Salsa's accumulator pattern. Any tracked
/// function can emit diagnostics, and they can be collected after the query
/// completes.
///
/// # Example
///
/// ```ignore
/// #[salsa::tracked]
/// fn some_query(db: &dyn CadenzaDb, input: SomeInput) -> Result {
///     use salsa::Accumulator;
///
///     // Emit a diagnostic
///     Diagnostic {
///         span,
///         message: "Parse error: unexpected token".to_string(),
///     }.accumulate(db);
///
///     // Continue processing...
/// }
/// ```
#[salsa::accumulator]
pub struct Diagnostic {
    /// The span in the source file where the diagnostic occurred.
    pub span: cadenza_syntax::span::Span,

    /// The diagnostic message.
    pub message: String,
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
///
/// # Example
///
/// ```
/// use cadenza_eval::db::{CadenzaDbImpl, SourceFile, parse_file, Diagnostic};
///
/// let db = CadenzaDbImpl::default();
/// let source = SourceFile::new(&db, "test.cdz".to_string(), "let x = 1".to_string());
///
/// // Parse the file
/// let parsed = parse_file(&db, source);
/// let cst = parsed.cst(&db);
///
/// // Check for diagnostics
/// let diagnostics = parse_file::accumulated::<Diagnostic>(&db, source);
/// assert_eq!(diagnostics.len(), 0);
/// ```
#[salsa::tracked]
pub fn parse_file(db: &dyn CadenzaDb, source: SourceFile) -> ParsedFile<'_> {
    use salsa::Accumulator;

    let text = source.text(db);
    let parse = cadenza_syntax::parse::parse(text);

    // Accumulate parse errors as diagnostics
    for error in &parse.errors {
        Diagnostic {
            span: error.span,
            message: error.message.clone(),
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
/// incremental queries in the Cadenza compiler. As we migrate functionality
/// to Salsa, tracked functions and queries will be defined against this trait.
///
/// # Example
///
/// ```ignore
/// #[salsa::tracked]
/// pub fn parse_file(db: &dyn CadenzaDb, source: SourceFile) -> ParsedFile<'_> {
///     // Implementation will be added in Phase 3
/// }
/// ```
#[salsa::db]
pub trait CadenzaDb: salsa::Database {}

/// The concrete database implementation for CLI and testing.
///
/// This struct provides the storage backend for Salsa queries. It's used
/// directly in the CLI and test code. For LSP integration, a thread-safe
/// wrapper will be created in Phase 6.
///
/// # Example
///
/// ```
/// use cadenza_eval::db::CadenzaDbImpl;
///
/// let db = CadenzaDbImpl::default();
/// // Use db for queries once they're implemented
/// ```
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
        // Database should be created successfully
    }

    #[test]
    fn test_db_implements_traits() {
        fn check_db<T: CadenzaDb>(_db: &T) {}
        let db = CadenzaDbImpl::default();
        check_db(&db);
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

        // Parse the file
        let parsed = parse_file(&db, source);

        // Check that we got a CST back
        let cst = parsed.cst(&db);
        assert_eq!(cst.kind(), cadenza_syntax::token::Kind::Root);

        // Check that the source is correctly tracked
        // Note: Using assert! with == because Salsa tracked types don't implement Debug
        assert!(parsed.source(&db) == source);

        // Check that there are no diagnostics for valid code
        let diagnostics = parse_file::accumulated::<Diagnostic>(&db, source);
        assert_eq!(diagnostics.len(), 0);
    }

    #[test]
    fn test_parse_file_with_errors() {
        let db = CadenzaDbImpl::default();
        // Unterminated string should cause parse error
        let source = SourceFile::new(
            &db,
            "error.cdz".to_string(),
            "let x = \"unterminated".to_string(),
        );

        // Parse the file
        let _parsed = parse_file(&db, source);

        // Check that diagnostics were accumulated
        let diagnostics = parse_file::accumulated::<Diagnostic>(&db, source);
        assert!(
            !diagnostics.is_empty(),
            "Expected parse errors for unterminated string"
        );

        // Check that the diagnostic contains useful information
        let first_diagnostic = &diagnostics[0];
        assert!(!first_diagnostic.message.is_empty());
    }

    #[test]
    fn test_parse_file_memoization() {
        let db = CadenzaDbImpl::default();
        let source = SourceFile::new(&db, "test.cdz".to_string(), "let x = 1".to_string());

        // Parse twice
        let parsed1 = parse_file(&db, source);
        let parsed2 = parse_file(&db, source);

        // Should return the same tracked value
        // Note: Using assert! with == because Salsa tracked types don't implement Debug
        assert!(parsed1 == parsed2);
    }

    #[test]
    fn test_parse_file_invalidation() {
        let mut db = CadenzaDbImpl::default();
        let source = SourceFile::new(&db, "test.cdz".to_string(), "let x = 1".to_string());

        // Parse the initial version
        let parsed1 = parse_file(&db, source);
        let text1 = parsed1.cst(&db).text().to_string();

        // Modify the source
        source.set_text(&mut db).to("let y = 2".to_string());

        // Parse again - should get a different result
        let parsed2 = parse_file(&db, source);
        let text2 = parsed2.cst(&db).text().to_string();

        // The CST should be different (different text content)
        assert_ne!(text1, text2);
        assert!(text1.contains("x"), "Expected 'x' in: {}", text1);
        assert!(text2.contains("y"), "Expected 'y' in: {}", text2);
    }

    // Note: CadenzaDbImpl is not Send + Sync because Salsa databases use
    // thread-local storage for performance. In Phase 6, we'll create a
    // thread-safe wrapper for LSP integration that uses parking_lot::Mutex.
}
