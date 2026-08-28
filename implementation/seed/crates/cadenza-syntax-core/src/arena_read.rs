//! Shared arena read-helpers for the surface readers (`cedar`/`json`/`markdown`/`toml_surface`).
//!
//! Each surface reader walks a decoded `Arenas` to project it into that surface's model, and each grew
//! the same thin accessors independently — byte-identical `list_items`/`child_tail`/`str_leaf`/
//! `int_leaf`/`bool_leaf`. Lifted here as one set (a behavior-preserving dedup; ~-40 lines across the
//! four files) so a change to how a leaf/list is read from the arena happens in ONE place. Purely
//! structural reads over the shared `Arenas` — no surface-specific logic lives here.

use crate::ast::{Arenas, Leaf, StructId};

/// The children of a list node (`(a b c)` → `[a, b, c]`), or empty for a non-list.
pub fn list_items(a: &Arenas, id: StructId) -> Vec<StructId> {
    match a.get(id) {
        crate::ast::Struct::List(items) => items.clone(),
        _ => Vec::new(),
    }
}

/// The TAIL of a list node — its children after the head (`(head a b)` → `[a, b]`), or empty for a
/// non-list / a head-only list. Saturating so a zero-child list yields `[]` rather than panicking.
pub fn child_tail(a: &Arenas, id: StructId) -> Vec<StructId> {
    match a.get(id) {
        crate::ast::Struct::List(items) => items[1.min(items.len())..].to_vec(),
        _ => Vec::new(),
    }
}

/// The string a `Str`/`Name` leaf denotes (via the arena's `as_str` accessor), owned; `None` for a
/// non-string node.
pub fn str_leaf(a: &Arenas, id: StructId) -> Option<String> {
    a.as_str(id).map(str::to_string)
}

/// The `i64` an `Int` leaf denotes, or `None` for a non-int / out-of-`i64`-range value.
pub fn int_leaf(a: &Arenas, id: StructId) -> Option<i64> {
    match a.get(id) {
        crate::ast::Struct::Atom(l) => match a.leaf(*l) {
            Leaf::Int { value, .. } => value.to_i64(),
            _ => None,
        },
        _ => None,
    }
}

/// The `bool` a `Bool` leaf denotes, or `None` for a non-bool node.
pub fn bool_leaf(a: &Arenas, id: StructId) -> Option<bool> {
    match a.get(id) {
        crate::ast::Struct::Atom(l) => match a.leaf(*l) {
            Leaf::Bool(b) => Some(*b),
            _ => None,
        },
        _ => None,
    }
}
