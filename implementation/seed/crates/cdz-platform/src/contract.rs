//! Contracts — how everything communicates (`design/cadenza-platform.md` §1).
//!
//! A contract is a declared interaction: a name, the type of its input, and the type of its output. Its
//! identity is the hash of that declaration — the contract-id. Nothing communicates by string name or an
//! enumerated kind; a program references the exact contract it was built against, by hash, and routing is
//! an exact content-addressed lookup, never a string match or a version range.
//!
//! Three consequences the identity delivers (§1):
//! - Identity is nominal: the name is part of the hash, so `temp.celsius : Float -> Float` and
//!   `temp.fahrenheit : Float -> Float` have the same shape but different hashes and never route to each
//!   other.
//! - Identity is exact: a caller's contract-id must equal what an answerer declares — no compatibility
//!   check, no tolerant reader, no version field.
//! - Evolution is a new contract: changing an input or output type produces a different declaration and so
//!   a different hash.
//!
//! The input and output are Cadenza types, and the whole declaration is a Cadenza value in its canonical
//! binary form (§1/§12), so the contract-id is reproducible from the declaration alone. We build the
//! declaration with the language's canonical AST (`cadenza-ast`) and hash its canonical encoding — one
//! canonical form shared with the compiler, never a parallel or lossy re-encoding.
//!
//! Declaration shape: the value `(contract <name> <input-type> <output-type>)` — a list whose head is the
//! name `contract`, followed by the contract's name as a string, then the input and output type trees. The
//! runtime treats the payload against a contract as opaque bytes; what the types mean is the concern of the
//! programs on each end.

use crate::{Hash, Str};
use cadenza_ast::ast::{Arenas, Builder, Leaf, StructId};
use cadenza_ast::{canon, codec};
use std::sync::Arc;

/// A contract: a `(name, input type, output type)` declaration whose canonical encoding hashes to its
/// [`contract-id`](Contract::id) — the sole identity used to route (§1). Holds the declaration as a
/// canonical [`Arenas`], so `id` is a pure function of it and reproducible from the declaration alone.
#[derive(Clone)]
pub struct Contract {
    /// The contract's name — kept for the [`name`](Contract::name) accessor; it is also encoded inside the
    /// declaration (which is what the id is taken over).
    name: Str,
    /// The canonical declaration value `(contract <name> <input> <output>)`.
    declaration: Arenas,
}

impl Contract {
    /// Declare a contract from a `name` and its input/output types. The type closures build each type into
    /// the same arena as the declaration, using the `cadenza-ast` type builders (for example
    /// `|b| b.wit_type_prim("f64")` for a `Float`, or `|b| b.wit_type_list(b.wit_type_prim("u8"))`).
    ///
    /// The declaration is canonicalized immediately, so [`id`](Contract::id) and [`declaration`] are
    /// stable and reproducible.
    pub fn new(
        name: &str,
        input: impl FnOnce(&mut Builder) -> StructId,
        output: impl FnOnce(&mut Builder) -> StructId,
    ) -> Self {
        let mut b = Builder::new();
        let head = b.name("contract");
        let name_node = b.atom_leaf(Leaf::Str(Arc::from(name)));
        let input_ty = input(&mut b);
        let output_ty = output(&mut b);
        let root = b.list(vec![head, name_node, input_ty, output_ty]);
        let arenas = b.finish(root);
        // Canonicalize now so the stored declaration is the one canonical form its hash is taken over.
        Self {
            name: Str::from(name),
            declaration: canon::canonicalize(&arenas).into_owned(),
        }
    }

    /// The contract-id: the hash of the canonical declaration (§1). Reproducible from the declaration
    /// alone, so two contracts with equal declarations have equal ids, and any difference in name, input,
    /// or output type gives a different id.
    #[must_use]
    pub fn id(&self) -> Hash {
        Hash::of(&codec::encode(&self.declaration))
    }

    /// The contract's name.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// The canonical declaration value, for a consumer that wants to inspect or re-encode it.
    #[must_use]
    pub fn declaration(&self) -> &Arenas {
        &self.declaration
    }
}

#[cfg(test)]
mod tests {
    use super::Contract;

    // helpers building the two types the spec's example uses.
    fn float(b: &mut cadenza_ast::ast::Builder) -> cadenza_ast::ast::StructId {
        b.wit_type_prim("f64")
    }
    fn u8_list(b: &mut cadenza_ast::ast::Builder) -> cadenza_ast::ast::StructId {
        let elem = b.wit_type_prim("u8");
        b.wit_type_list(elem)
    }

    #[test]
    fn id_is_reproducible_from_the_declaration_alone() {
        // Building the same contract twice yields the same id (a pure function of the declaration).
        let a = Contract::new("temp.celsius", float, float);
        let b = Contract::new("temp.celsius", float, float);
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn identity_is_nominal_same_shape_different_name_differs() {
        // temp.celsius : Float -> Float and temp.fahrenheit : Float -> Float share a shape but not an id.
        let celsius = Contract::new("temp.celsius", float, float);
        let fahrenheit = Contract::new("temp.fahrenheit", float, float);
        assert_ne!(
            celsius.id(),
            fahrenheit.id(),
            "the name is part of the identity"
        );
    }

    #[test]
    fn evolution_is_a_new_contract_type_change_differs() {
        // Same name, but a different input or output type is a different contract (a different hash).
        let base = Contract::new("fetch", float, float);
        let diff_input = Contract::new("fetch", u8_list, float);
        let diff_output = Contract::new("fetch", float, u8_list);
        assert_ne!(base.id(), diff_input.id());
        assert_ne!(base.id(), diff_output.id());
        assert_ne!(diff_input.id(), diff_output.id());
    }

    #[test]
    fn name_reads_back() {
        let c = Contract::new("temp.celsius", float, float);
        assert_eq!(c.name(), "temp.celsius");
    }

    #[test]
    fn id_is_a_32_byte_hash_rendered_base64url() {
        let c = Contract::new("x", float, float);
        // it is a real content hash — 32 bytes, 43 base64url chars, and stable across calls.
        assert_eq!(c.id().as_bytes().len(), 32);
        assert_eq!(c.id().to_string().len(), 43);
        assert_eq!(c.id(), c.id());
    }
}
