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
//! ## Migration Status
//!
//! Phase 1 (Foundation) and Phase 2 (Source Tracking) are complete. The database
//! infrastructure is established with source file tracking. The existing
//! mutable `Compiler` and `EvalContext` architecture remains the primary
//! evaluation path.
//!
//! Future phases will:
//! - Phase 3: Make parsing a tracked function
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

    // Note: CadenzaDbImpl is not Send + Sync because Salsa databases use
    // thread-local storage for performance. In Phase 6, we'll create a
    // thread-safe wrapper for LSP integration that uses parking_lot::Mutex.
}
