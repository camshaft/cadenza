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
//! Phase 2 introduces source file tracking and identifier interning using Salsa:
//!
//! - [`SourceFile`]: Salsa input for source text with path and content
//! - [`Identifier`]: Salsa interned type for deduplicated string identifiers
//!
//! ## Migration Status
//!
//! Phase 1 (Foundation) and Phase 2 (Source Tracking) are complete. The database
//! infrastructure is established with source file and identifier support. The
//! existing mutable `Compiler` and `EvalContext` architecture remains the primary
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
// Interned Types
// =============================================================================

/// An interned identifier.
///
/// Identifiers are deduplicated across the database, so two identifiers with
/// the same text will have the same identity. This makes equality checks very
/// cheap (just pointer comparison).
///
/// # Example
///
/// ```
/// use cadenza_eval::db::{CadenzaDbImpl, Identifier};
///
/// let db = CadenzaDbImpl::default();
/// let id1 = Identifier::new(&db, "foo".to_string());
/// let id2 = Identifier::new(&db, "foo".to_string());
/// 
/// // These are the same identifier (same pointer)
/// assert!(id1 == id2);
/// ```
#[salsa::interned]
pub struct Identifier<'db> {
    /// The text of the identifier.
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
///     // Implementation
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

    // Note: CadenzaDbImpl is not Send + Sync because Salsa databases use
    // thread-local storage for performance. In Phase 6, we'll create a
    // thread-safe wrapper for LSP integration that uses parking_lot::Mutex.

    // =============================================================================
    // SourceFile Tests
    // =============================================================================

    #[test]
    fn test_source_file_creation() {
        let db = CadenzaDbImpl::default();
        let source = SourceFile::new(&db, "test.cdz".to_string(), "let x = 1".to_string());
        
        assert_eq!(source.path(&db), "test.cdz");
        assert_eq!(source.text(&db), "let x = 1");
    }

    #[test]
    fn test_source_file_mutation() {
        let mut db = CadenzaDbImpl::default();
        let source = SourceFile::new(&db, "test.cdz".to_string(), "let x = 1".to_string());
        
        // Initial text
        assert_eq!(source.text(&db), "let x = 1");
        
        // Update the text
        source.set_text(&mut db).to("let x = 2".to_string());
        
        // Text should be updated
        assert_eq!(source.text(&db), "let x = 2");
        
        // Path should remain the same
        assert_eq!(source.path(&db), "test.cdz");
    }

    #[test]
    fn test_source_file_path_mutation() {
        let mut db = CadenzaDbImpl::default();
        let source = SourceFile::new(&db, "test.cdz".to_string(), "let x = 1".to_string());
        
        // Update the path
        source.set_path(&mut db).to("renamed.cdz".to_string());
        
        assert_eq!(source.path(&db), "renamed.cdz");
        assert_eq!(source.text(&db), "let x = 1");
    }

    // =============================================================================
    // Identifier Tests
    // =============================================================================

    #[test]
    fn test_identifier_creation() {
        let db = CadenzaDbImpl::default();
        let id = Identifier::new(&db, "foo".to_string());
        
        assert_eq!(id.text(&db), "foo");
    }

    #[test]
    fn test_identifier_interning() {
        let db = CadenzaDbImpl::default();
        let id1 = Identifier::new(&db, "foo".to_string());
        let id2 = Identifier::new(&db, "foo".to_string());
        
        // Same text should result in the same interned identifier
        assert!(id1 == id2);
    }

    #[test]
    fn test_identifier_different_texts() {
        let db = CadenzaDbImpl::default();
        let id1 = Identifier::new(&db, "foo".to_string());
        let id2 = Identifier::new(&db, "bar".to_string());
        
        // Different texts should result in different identifiers
        assert!(id1 != id2);
    }

    #[test]
    fn test_identifier_equality_cheap() {
        let db = CadenzaDbImpl::default();
        let id1 = Identifier::new(&db, "foo".to_string());
        let id2 = Identifier::new(&db, "foo".to_string());
        
        // Equality check should be cheap (pointer comparison)
        // This is ensured by Salsa's interning mechanism
        assert!(id1 == id2);
    }

    #[test]
    fn test_multiple_identifiers() {
        let db = CadenzaDbImpl::default();
        let id1 = Identifier::new(&db, "foo".to_string());
        let id2 = Identifier::new(&db, "bar".to_string());
        let id3 = Identifier::new(&db, "foo".to_string());
        let id4 = Identifier::new(&db, "baz".to_string());
        
        assert!(id1 == id3);
        assert!(id1 != id2);
        assert!(id1 != id4);
        assert!(id2 != id4);
    }
}
