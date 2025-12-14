//! Cadenza Core - HIR-based incremental compilation with Salsa
//!
//! This crate provides the core compiler infrastructure for Cadenza using a
//! High-level Intermediate Representation (HIR) and the Salsa framework for
//! incremental computation.
//!
//! ## Architecture Overview
//!
//! The compilation pipeline follows these stages:
//!
//! 1. **Source → AST**: Parse source code into Abstract Syntax Tree (CST from cadenza-syntax)
//! 2. **AST → HIR**: Lower AST to High-level Intermediate Representation with span tracking
//! 3. **HIR Evaluation**: Evaluate/expand HIR (macro expansion, compile-time evaluation)
//! 4. **Type Inference**: Infer types on expanded HIR
//! 5. **LSP Queries**: Operate on expanded HIR for IDE features
//!
//! ## Key Components
//!
//! - **Database** (`db`): Salsa database infrastructure for incremental queries
//! - **HIR** (`hir`): High-level Intermediate Representation definitions
//! - **Lower** (`lower`): AST → HIR lowering with span preservation
//! - **Eval** (`eval`): HIR evaluation and macro expansion (TODO)
//! - **Queries** (`queries`): LSP and type inference queries (TODO)
//!
//! ## Design Principles
//!
//! 1. **Span Tracking**: Every HIR node preserves source spans for error reporting
//! 2. **Incremental**: Salsa tracks dependencies and recomputes only what changed
//! 3. **Pure Functions**: Queries are pure - no mutation, deterministic results
//! 4. **Post-Expansion LSP**: LSP operates on evaluated/expanded HIR to see generated code
//!
//! See `/docs/SALSA_MIGRATION.md` for the complete architecture and migration plan.

pub mod db;
pub mod hir;

// Re-export commonly used types
pub use db::{CadenzaDb, CadenzaDbImpl, Diagnostic, ParsedFile, Severity, SourceFile};
