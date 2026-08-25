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
//! Declaration shape: the value `(contract <name> (types <type-decl>…) <input-name> <output-name>)` — the
//! head `contract`, then the contract's name as a string, then a `(types …)` list of named cadenza type
//! declarations (`(type Name …)` forms), then the names of the input and output types (each referencing a
//! declared type). Types are always named — no inline anonymous types — so recursive and mutually
//! referential types resolve by name within the set (spec direction 2026-08-20). The runtime treats the
//! payload against a contract as opaque bytes; what the types mean is the concern of the programs on each
//! end.

use crate::{Bytes, ContractId, Str};
use cadenza_ast::ast::{Builder, StructId};

/// A contract: a `(name, input type, output type)` declaration whose canonical encoding hashes to its
/// [`contract-id`](Contract::id) — the sole identity used to route (§1). The id is computed once at
/// construction, so it is a pure function of the declaration and reproducible from it alone.
#[derive(Clone)]
pub struct Contract {
    /// The contract-id: the hash of the canonical declaration, computed once at construction (the getter
    /// just returns it — hashing every call would be wasteful).
    id: ContractId,
    /// The contract's name — kept for the [`name`](Contract::name) accessor; it is also encoded inside the
    /// declaration (which is what the id is taken over).
    name: Str,
    /// The canonical declaration `(contract <name> (types …) <input> <output>)` in its encoded binary form. Stored
    /// as the encoded `Bytes` (an O(1) clone) rather than the `Arenas`, since encoding is exactly what the
    /// id is taken over; a consumer that wants the structured value decodes it with `cadenza_ast::codec`.
    declaration: Bytes,
}

impl Contract {
    /// Declare a contract from a `name`, a set of named cadenza type declarations, and the names of its
    /// input and output types.
    ///
    /// The input and output are cadenza types, and to represent any cadenza type — including recursive and
    /// mutually-referential ones — every type is named: `types` builds the `(type Name …)` declarations
    /// (via the `cadenza-ast` builder), and `input` / `output` name which of those declared types are the
    /// contract's input and output. A type's body references any name in the set (including itself), so
    /// recursion resolves by name; there are no inline anonymous types. A consequence is that a contract is
    /// consistent across declarations only if it uses the same type names — the names are part of the
    /// declaration, and therefore of the identity.
    ///
    /// `types` returns the `StructId` of each `(type Name …)` form it builds into the arena. For example, a
    /// recursive list type:
    /// ```ignore
    /// Contract::new(Str::from("sum"), |b| {
    ///     // (type Ints INil (ICons (Tuple Int64 Ints)))
    ///     let nil = b.name("INil");
    ///     let cons = b.list(vec![b.name("ICons"),
    ///         b.list(vec![b.name("Tuple"), b.name("Int64"), b.name("Ints")])]);
    ///     let ints = b.list(vec![b.name("type"), b.name("Ints"), nil, cons]);
    ///     vec![ints]
    /// }, "Ints", "Int64")
    /// ```
    ///
    /// The declaration is canonicalized and encoded immediately, so [`id`](Contract::id) and
    /// [`declaration`](Contract::declaration) are stable and reproducible.
    pub fn new(
        name: Str,
        types: impl FnOnce(&mut Builder) -> Vec<StructId>,
        input: &str,
        output: &str,
    ) -> Self {
        // The declaration build + canonical encoding lives once in `cdz-contract` (so it can also run as a
        // wasm component that turns a schema into a hash); build it once here, store the bytes, and take the
        // contract-id over exactly them. `ContractId::of` is `Hash::of(HashTag::Contract, …)`, the same hash
        // `cdz_contract::contract_id` computes — so the two agree by construction.
        let declaration = Bytes::from(cdz_contract::contract_declaration(
            name.as_str(),
            types,
            input,
            output,
        ));
        let id = ContractId::of(&declaration);
        Self {
            id,
            name,
            declaration,
        }
    }

    /// The contract-id: the hash of the canonical declaration (§1). Computed once at construction (above),
    /// so this is a cheap getter. Reproducible from the declaration alone, so two contracts with equal
    /// declarations have equal ids, and any difference in name, input, or output type gives a different id.
    #[must_use]
    pub fn id(&self) -> ContractId {
        self.id
    }

    /// The contract's name.
    #[must_use]
    pub fn name(&self) -> &Str {
        &self.name
    }

    /// The canonical declaration in its encoded binary form. Decode it with `cadenza_ast::codec` to
    /// inspect the structured value; the id is the hash of exactly these bytes.
    #[must_use]
    pub fn declaration(&self) -> &Bytes {
        &self.declaration
    }
}

#[cfg(test)]
mod tests {
    use super::Contract;
    use crate::Str;
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

    /// A genuinely recursive type: `(type Expr (Lit Int64) (Add (Tuple Expr Expr)))` — Expr refers to Expr.
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
        // Building the same contract twice yields the same id (a pure function of the declaration).
        let a = Contract::new(Str::from("temp.celsius"), temp_type, "Temp", "Temp");
        let b = Contract::new(Str::from("temp.celsius"), temp_type, "Temp", "Temp");
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn identity_is_nominal_same_shape_different_name_differs() {
        // temp.celsius and temp.fahrenheit share the exact same types + shape but differ only in name;
        // the name is part of the identity, so their ids differ (the spec's own example).
        let celsius = Contract::new(Str::from("temp.celsius"), temp_type, "Temp", "Temp");
        let fahrenheit = Contract::new(Str::from("temp.fahrenheit"), temp_type, "Temp", "Temp");
        assert_ne!(celsius.id(), fahrenheit.id());
    }

    #[test]
    fn a_recursive_type_is_representable_and_hashes_stably() {
        // A recursive Expr type resolves by name (Expr refers to Expr); the contract builds + hashes, and
        // the same recursive declaration is reproducible.
        let a = Contract::new(Str::from("eval"), expr_type, "Expr", "Expr");
        let b = Contract::new(Str::from("eval"), expr_type, "Expr", "Expr");
        assert_eq!(a.id(), b.id());
        // it differs from a contract over a different (non-recursive) type of the same name-shape.
        let temp = Contract::new(Str::from("eval"), temp_type, "Temp", "Temp");
        assert_ne!(a.id(), temp.id());
    }

    #[test]
    fn evolution_a_different_input_or_output_type_reference_differs() {
        // Two named types, then vary which one input/output reference — a different type reference is a
        // different contract (a different hash), even with the same name + same declared types.
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
        let base = Contract::new(Str::from("f"), two_types, "A", "B");
        let swapped_input = Contract::new(Str::from("f"), two_types, "B", "B");
        let swapped_output = Contract::new(Str::from("f"), two_types, "A", "A");
        assert_ne!(base.id(), swapped_input.id());
        assert_ne!(base.id(), swapped_output.id());
    }

    #[test]
    fn name_reads_back() {
        let c = Contract::new(Str::from("temp.celsius"), temp_type, "Temp", "Temp");
        assert_eq!(c.name().as_str(), "temp.celsius");
    }

    #[test]
    fn id_agrees_with_the_standalone_contract_id_computation() {
        // The collapse is faithful: a `Contract`'s id is exactly what `cdz_contract::contract_id` computes
        // from the same declaration, so the platform and the standalone tool (the nix name→hash mapping)
        // agree by construction. `Contract` derives its id via `cdz_contract::contract_declaration`, so this
        // pins that the two entry points cannot drift.
        let c = Contract::new(Str::from("temp.celsius"), temp_type, "Temp", "Temp");
        assert_eq!(
            c.id().hash(),
            cdz_contract::contract_id("temp.celsius", temp_type, "Temp", "Temp")
        );
    }

    #[test]
    fn id_is_byte_stable_a_golden_pin_on_the_declaration_encoding() {
        // A GOLDEN pin, complementing the relational tests above. Those prove the id is reproducible /
        // nominal / evolution-sensitive, but NONE pins its actual value — so a change to the canonical
        // declaration encoding (the `cadenza-ast` codec, a field order, a tag byte) would silently SHIFT
        // every contract-id while those relational tests still pass, breaking routing (already-registered
        // contracts stop matching, since routing is exact-hash equality, §1) and any userspace contract-id
        // builder (`DESIGN-compiler-primitives` P4) that must reproduce this exact byte-form. This asserts a
        // fixture contract's id is a specific value, so such drift fails loudly here. If it ever legitimately
        // changes, that is a deliberate contract-id flag-day — re-pin intentionally, do not paper over.
        let c = Contract::new(Str::from("temp.celsius"), temp_type, "Temp", "Temp");
        assert_eq!(
            c.id().hash().to_string(),
            "01UUXRcMG63Ct66Z4TP7l6QfY7pvktdISpoHyTdJVtS70",
            "the contract-id byte-form drifted (or this is a deliberate flag-day)"
        );
    }
    #[test]
    fn builtin_contract_ids_are_byte_stable_golden_pins_on_the_routing_keys() {
        // The built-in contracts' ids are the platform's live ROUTING KEYS — dispatch resolves an effect by
        // exact-hash equality on these (§1/§4). `id_is_byte_stable_…` above pins only a TEST fixture
        // (temp.celsius); a change to the canonical declaration encoding (the `cadenza-ast` codec, a field
        // order, the `HashTag::Contract` tag) would silently SHIFT every REAL contract-id — breaking routing
        // for already-deployed programs and diverging from any userspace contract-id builder
        // (`DESIGN-compiler-primitives` P4, which must reproduce these exact bytes) — while every relational
        // test still passed. This pins each built-in routing key's actual value so such drift fails loudly. If
        // one legitimately changes, that is a deliberate contract-id flag-day (a new contract, §1) — re-pin it
        // intentionally, do not paper over.
        assert_eq!(
            crate::deliver_contract().to_string(),
            "017nakNdnxPdydihXTT6ftU711SlTZ2ooWVkzbETVMNVs",
            "the deliver routing key drifted (or this is a deliberate flag-day)"
        );
        assert_eq!(
            crate::timer_contract().to_string(),
            "01hgs2SpFOf18sQCE9ovNcIiZrG29Spc3fOsWup5HTBFj",
            "the timer routing key drifted (or this is a deliberate flag-day)"
        );
        assert_eq!(
            crate::lifecycle_contract().to_string(),
            "010GbI5BHteX1jTUGN9R5pZchnUyETTRTtUGJ6nABYFfk",
            "the lifecycle routing key drifted (or this is a deliberate flag-day)"
        );
        assert_eq!(
            crate::spawned_contract().to_string(),
            "01h7MH6VU7juBXYTo8i6I6PQFLrMNhdIE8l2h2Lz7AwFl",
            "the spawned routing key drifted (or this is a deliberate flag-day)"
        );
        assert_eq!(
            crate::run_contract().to_string(),
            "012hF7m1p0cofeRmMDl5uXmtPB2pRRP0OvXQWzGYYjVbP",
            "the run routing key drifted (or this is a deliberate flag-day)"
        );
    }
}
