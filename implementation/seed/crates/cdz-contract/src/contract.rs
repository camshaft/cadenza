//! The contract-id computation (`design/cadenza-platform.md` §1).
//!
//! A contract is a declared interaction — a name, an input type, an output type — and its identity is the
//! hash of that declaration. The declaration is a Cadenza value in its canonical binary form, so the
//! contract-id is reproducible from the declaration alone: two producers of the same declaration compute the
//! same id, with no shared state. That is what lets a build step (or the nix setup) turn a directory of
//! contract schemas into a name→hash mapping that agrees exactly with what the running platform routes on.
//!
//! Declaration shape: the value `(contract <name> (types <type-decl>…) <input-name> <output-name>)` — the
//! head `contract`, the contract's name as a string, a `(types …)` list of named Cadenza type declarations
//! (`(type Name …)` forms), then the names of the input and output types (each referencing a declared type).
//! Types are always named — no inline anonymous types — so recursive and mutually-referential types resolve
//! by name within the set (spec direction 2026-08-20). This crate builds that value with the canonical AST
//! (`cadenza-ast`) and hashes its canonical encoding — one canonical form shared with the compiler, never a
//! parallel or lossy re-encoding.

use crate::{Hash, HashTag};
use cadenza_ast::ast::{Builder, Leaf, StructId};
use cadenza_ast::{canon, codec};
use std::sync::Arc;

/// The canonical encoded declaration of a contract: the value
/// `(contract <name> (types <type-decl>…) <input> <output>)`, canonicalized and encoded with the one
/// canonical codec (§1/§12). The contract-id is the hash of exactly these bytes ([`contract_id`]), and the
/// platform's `Contract` stores them as its declaration — so building the declaration lives here once.
///
/// `types` builds the `(type Name …)` declarations into `b` (via the `cadenza-ast` builder) and returns the
/// `StructId` of each, and `input` / `output` name which of those declared types are the contract's input
/// and output. A type's body references any name in the set (including itself), so recursion resolves by
/// name; there are no inline anonymous types.
#[must_use]
pub fn contract_declaration(
    name: &str,
    types: impl FnOnce(&mut Builder) -> Vec<StructId>,
    input: &str,
    output: &str,
) -> Vec<u8> {
    let mut b = Builder::new();
    let head = b.name("contract");
    let name_node = b.atom_leaf(Leaf::Str(Arc::from(name)));
    // (types <type-decl>…) — the set of named Cadenza type declarations the input/output refer to.
    let type_decls = types(&mut b);
    let types_head = b.name("types");
    let types_node = b.list(std::iter::once(types_head).chain(type_decls).collect());
    // input/output are references to declared types, by name — no inline anonymous types (§1, so
    // recursive/mutually-referential types resolve within the named set).
    let input_ref = b.name(input);
    let output_ref = b.name(output);
    let root = b.list(vec![head, name_node, types_node, input_ref, output_ref]);
    let arenas = b.finish(root);
    // Canonicalize, then encode: the encoded bytes are what the contract-id is the hash of.
    let canonical = canon::canonicalize(&arenas).into_owned();
    codec::encode(&canonical)
}

/// The contract-id (§1): the [`Contract`](HashTag::Contract)-tagged [`Hash`] of a contract's canonical
/// declaration. A pure function of `(name, types, input, output)` — the sole identity contracts route on —
/// so two callers that declare the same contract compute the same id, and any difference in name, a type, or
/// the input/output reference gives a different id.
#[must_use]
pub fn contract_id(
    name: &str,
    types: impl FnOnce(&mut Builder) -> Vec<StructId>,
    input: &str,
    output: &str,
) -> Hash {
    Hash::of(
        HashTag::Contract,
        &contract_declaration(name, types, input, output),
    )
}

#[cfg(test)]
mod tests {
    use super::{contract_declaration, contract_id};
    use crate::HashTag;
    use cadenza_ast::ast::{Builder, StructId};

    /// Build a `(Head child…)` list form. A small helper because a builder call cannot nest another
    /// builder call in its arguments (each borrows the builder), so every node is bound first.
    fn form(b: &mut Builder, head: &str, children: Vec<StructId>) -> StructId {
        let head = b.name(head);
        b.list(std::iter::once(head).chain(children).collect())
    }

    /// A minimal named type: `(type Temp (Mk f64))`.
    fn temp_type(b: &mut Builder) -> Vec<StructId> {
        let f64 = b.name("f64");
        let mk = form(b, "Mk", vec![f64]);
        let temp = b.name("Temp");
        vec![form(b, "type", vec![temp, mk])]
    }

    /// A genuinely recursive type: `(type Expr (Lit Int64) (Add (Tuple Expr Expr)))`.
    fn expr_type(b: &mut Builder) -> Vec<StructId> {
        let int = b.name("Int64");
        let lit = form(b, "Lit", vec![int]);
        let (e1, e2) = (b.name("Expr"), b.name("Expr"));
        let tuple = form(b, "Tuple", vec![e1, e2]);
        let add = form(b, "Add", vec![tuple]);
        let name = b.name("Expr");
        vec![form(b, "type", vec![name, lit, add])]
    }

    #[test]
    fn id_is_reproducible_from_the_declaration_alone() {
        // The same contract declared twice yields the same id (a pure function of the declaration), and the
        // id is over exactly the declaration bytes.
        let a = contract_id("temp.celsius", temp_type, "Temp", "Temp");
        let b = contract_id("temp.celsius", temp_type, "Temp", "Temp");
        assert_eq!(a, b);
        assert_eq!(a.tag(), Some(HashTag::Contract));
        assert_eq!(
            a,
            crate::Hash::of(
                HashTag::Contract,
                &contract_declaration("temp.celsius", temp_type, "Temp", "Temp")
            )
        );
    }

    #[test]
    fn identity_is_nominal_same_shape_different_name_differs() {
        // Same types + shape, different name only: the name is part of the identity (the spec's own example).
        let celsius = contract_id("temp.celsius", temp_type, "Temp", "Temp");
        let fahrenheit = contract_id("temp.fahrenheit", temp_type, "Temp", "Temp");
        assert_ne!(celsius, fahrenheit);
    }

    #[test]
    fn a_recursive_type_hashes_stably_and_differs_from_another_type() {
        let a = contract_id("eval", expr_type, "Expr", "Expr");
        let b = contract_id("eval", expr_type, "Expr", "Expr");
        assert_eq!(a, b);
        assert_ne!(a, contract_id("eval", temp_type, "Temp", "Temp"));
    }

    #[test]
    fn a_different_input_or_output_reference_differs() {
        fn two_types(b: &mut Builder) -> Vec<StructId> {
            let int = b.name("Int64");
            let mk_a = form(b, "MkA", vec![int]);
            let name_a = b.name("A");
            let a = form(b, "type", vec![name_a, mk_a]);
            let boolean = b.name("Bool");
            let mk_b = form(b, "MkB", vec![boolean]);
            let name_b = b.name("B");
            let bt = form(b, "type", vec![name_b, mk_b]);
            vec![a, bt]
        }
        let base = contract_id("f", two_types, "A", "B");
        assert_ne!(base, contract_id("f", two_types, "B", "B"));
        assert_ne!(base, contract_id("f", two_types, "A", "A"));
    }
}
