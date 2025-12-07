//! Custom syntax tree implementation for Cadenza.
//!
//! This crate provides a lightweight, efficient syntax tree that replaces Rowan.
//! It's designed to be similar to Rowan's API for easy migration, while providing
//! additional features:
//! - Unified interning and metadata tracking
//! - Source file tracking per node
//! - Support for synthetic/virtual tokens
//! - Arbitrary metadata via AnyMap
//! - Line number computation
//!
//! # Architecture
//!
//! The tree follows a two-layer architecture:
//! - **Green Tree**: Immutable, interned tree structure (stored in arena)
//! - **Red Tree**: Typed wrappers that provide convenient access to the green tree
//!
//! This design minimizes allocations and enables efficient structural sharing.

mod green;
pub mod interner;
mod metadata;
mod red;
mod syntax_kind;
mod text;

pub use green::{Checkpoint, GreenNode, GreenNodeBuilder, GreenToken};
pub use interner::{Interned, InternedString, Storage, Strings};
pub use metadata::{NodeMetadata, SourceFile};
pub use red::{NodeOrToken, SyntaxElement, SyntaxNode, SyntaxToken};
pub use syntax_kind::SyntaxKind;
pub use text::SyntaxText;

/// Language trait that must be implemented by users of this tree.
///
/// This trait connects the tree to your specific token kinds and provides
/// conversions between your token type and the internal SyntaxKind.
pub trait Language: Sized + 'static {
    /// The token kind type for your language.
    type Kind: Copy + std::fmt::Debug + Eq + std::hash::Hash;

    /// Convert from a SyntaxKind to your token kind.
    fn kind_from_raw(raw: SyntaxKind) -> Self::Kind;

    /// Convert from your token kind to a SyntaxKind.
    fn kind_to_raw(kind: Self::Kind) -> SyntaxKind;
}
