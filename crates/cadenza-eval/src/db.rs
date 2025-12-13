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
//! ## Migration Status
//!
//! This is Phase 1 of the Salsa migration. The database infrastructure is
//! established but not yet integrated into the evaluator. The existing
//! mutable `Compiler` and `EvalContext` architecture remains the primary
//! evaluation path.
//!
//! Future phases will:
//! - Phase 2: Add source file tracking as Salsa inputs
//! - Phase 3: Make parsing a tracked function
//! - Phase 4: Convert evaluation to tracked functions
//! - Phase 5: Make type inference a set of queries
//! - Phase 6: Wire LSP to query the database
//!
//! See `/docs/SALSA_MIGRATION.md` for the complete migration plan.

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
}
