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
use cadenza_ast::ast::{Arenas, Builder, Leaf, Struct, StructId};
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

/// The contract NAME and [`contract_id`] read from a contract's `descriptor()` RETURN VALUE — the canonical
/// value form `(: (record (= id <bytes>) (= name <string>) …) <type>)` that `cdz run --format binary-ast`
/// emits after COMPILING + EXECUTING a contract's exported `descriptor()`. This is the Option-B host
/// derivation (operator 2026-08-27): rather than reading `@!contract`/`@!input`/`@!output` pragmas, or
/// re-deriving the id in Rust, the tooling runs the contract and reads the folded descriptor record — the
/// guest's own self-reflection (`contract-descriptor(Ast.module, name, input, output)`, const-folded) is the
/// single source of truth for the id. `value` is the decoded value-form [`Arenas`]
/// (`cadenza_ast::codec::decode` of the emitted `--format binary-ast` bytes). `None` unless it is a record
/// carrying a 33-byte `id` field (a tagged [`Hash`]) and a string `name` field — a malformed / non-descriptor
/// value names no contract rather than panicking.
#[must_use]
pub fn id_name_from_descriptor(value: &Arenas) -> Option<(String, Hash)> {
    // The escaped value form is `(: <record> <type>)`; the record is the first child after the `:` head.
    let annotated = value.as_form(value.root, ":")?;
    let record = *annotated.first()?;
    // `(record (= <field> <value>) …)` — scan the `(= …)` field groups for `id` and `name`.
    let fields = value.as_form(record, "record")?;
    let mut id: Option<Hash> = None;
    let mut name: Option<String> = None;
    for &field in fields {
        let Some([field_name, field_value]) = value.as_form(field, "=") else {
            continue; // not a `(= name value)` field group — skip
        };
        match value.as_name(*field_name) {
            // The `id` field is the 33-byte tagged contract-id, as a `Bytes` leaf.
            Some("id") => id = bytes_leaf(value, *field_value).and_then(|b| Hash::try_from(b).ok()),
            // The `name` field is the contract name, as a string leaf.
            Some("name") => name = value.as_str(*field_value).map(str::to_string),
            _ => {}
        }
    }
    Some((name?, id?))
}

/// The contract NAME plus its INPUT and OUTPUT type names, read from a contract's `descriptor()` RETURN VALUE
/// — the tuple `xtask codegen` needs to generate the kernel `contract()` (`Contract::new(name, types, input,
/// output)`) under the Option-B path (operator 2026-08-27: codegen compiles+executes the module, reads the
/// descriptor, and generates Rust that calls `Contract::new` from it). Unlike [`id_name_from_descriptor`],
/// which reads the already-computed `id`, this reads the pieces `Contract::new` RE-DERIVES the id from, so the
/// Rust side stays the runtime `Contract` representation with no precomputed-id type change. The descriptor's
/// `input`/`output` fields are `Ast.encode(Ast.Name(<type>))` (the encoded AST of the type-name reference, as
/// `Bytes`), so each is `codec::decode`d back to its `Name` and its symbol taken. `None` unless the value is a
/// record with a string `name` and `Bytes` `input`/`output` fields that each decode to a `Name`.
#[must_use]
pub fn identity_from_descriptor(value: &Arenas) -> Option<(String, String, String)> {
    let annotated = value.as_form(value.root, ":")?;
    let record = *annotated.first()?;
    let fields = value.as_form(record, "record")?;
    let mut name: Option<String> = None;
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    for &field in fields {
        let Some([field_name, field_value]) = value.as_form(field, "=") else {
            continue;
        };
        match value.as_name(*field_name) {
            Some("name") => name = value.as_str(*field_value).map(str::to_string),
            Some("input") => input = decode_type_name(value, *field_value),
            Some("output") => output = decode_type_name(value, *field_value),
            _ => {}
        }
    }
    Some((name?, input?, output?))
}

/// A descriptor's `input`/`output` field — `Ast.encode(Ast.Name(<type>))` as a `Bytes` leaf — decoded back to
/// the type NAME (`Envelope`). Decodes the leaf's bytes to their `Ast` (`cadenza_ast::codec::decode`) and takes
/// the root `Name` symbol. `None` if the field is not such a `Bytes` leaf or does not decode to a `Name`.
fn decode_type_name(value: &Arenas, id: StructId) -> Option<String> {
    let bytes = bytes_leaf(value, id)?;
    let inner = codec::decode(bytes)?;
    inner.as_name(inner.root).map(str::to_string)
}

/// The raw bytes a `Bytes` leaf (`b"…"`) carries, or `None` for any other node — the reader for the
/// descriptor record's `id` field (a tagged [`Hash`] rendered as `Bytes`).
fn bytes_leaf(value: &Arenas, id: StructId) -> Option<&[u8]> {
    match value.get(id) {
        Struct::Atom(leaf) => match value.leaf(*leaf) {
            Leaf::Bytes(bytes) => Some(bytes),
            _ => None,
        },
        Struct::List(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        contract_declaration, contract_id, id_name_from_descriptor, identity_from_descriptor,
    };
    use crate::{Hash, HashTag};
    use cadenza_ast::ast::Leaf;
    use cadenza_ast::ast::{Builder, StructId};
    use std::sync::Arc;

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

    #[test]
    fn the_contract_id_byte_format_is_stable_a_golden() {
        // A GOLDEN pin on the exact contract-id text for a fixed declaration. The other tests here assert only
        // RELATIVE properties (reproducible, nominal, input/output-sensitive, comment-tolerant) — every one of
        // them still passes if the canonical encoding / tag / hash format shifts, because the shift moves all
        // ids together. But the contract-id is the identity the whole platform ROUTES on and PERSISTS, and a
        // guest self-reflecting via `Ast.module` folds this same declaration to match it (P4) — so an
        // unintended format change silently breaks byte-identity with every stored id and every guest. This
        // freezes the absolute output for `(contract "temp.celsius" (types (type Temp (Mk f64))) Temp Temp)`;
        // if it fails, the wire format changed — update the golden ONLY as a deliberate, re-hash-everything
        // decision, never as a reflexive "make the test pass."
        let id = contract_id("temp.celsius", temp_type, "Temp", "Temp");
        assert_eq!(id.tag(), Some(HashTag::Contract));
        assert_eq!(
            id.to_string(),
            "01UUXRcMG63Ct66Z4TP7l6QfY7pvktdISpoHyTdJVtS70"
        );
    }

    #[test]
    fn id_and_name_are_read_from_the_descriptor_value_form() {
        // The Option-B host derivation (operator 2026-08-27): after `cdz run --format binary-ast` executes a
        // contract's `descriptor()`, the host decodes the value form `(: (record (= ast b"…") (= id b"<tagged
        // 33 bytes>") (= name "<name>") …) <type>)` and reads the contract-id + name back out — no pragmas, no
        // Rust re-derivation. Build that exact shape (with an extra `ast` field the reader must SKIP) and
        // assert `id_name_from_descriptor` recovers the tagged `Hash` and the `String` name.
        let want_id = Hash::of(HashTag::Contract, b"a-contract-declaration");
        let mut b = Builder::new();
        let field = |b: &mut Builder, key: &str, val: StructId| -> StructId {
            let eq = b.name("=");
            let k = b.name(key);
            b.list(vec![eq, k, val])
        };
        let ast_val = b.atom_leaf(Leaf::Bytes(Arc::from(&b"module-ast-bytes"[..])));
        let field_ast = field(&mut b, "ast", ast_val);
        let id_val = b.atom_leaf(Leaf::Bytes(Arc::from(&want_id.as_bytes()[..])));
        let field_id = field(&mut b, "id", id_val);
        let name_val = b.atom_leaf(Leaf::Str(Arc::from("temp.celsius")));
        let field_name = field(&mut b, "name", name_val);
        let rec_head = b.name("record");
        let record = b.list(vec![rec_head, field_ast, field_id, field_name]);
        // A minimal `<type>` node — the reader only needs the `:`-envelope's FIRST child (the value).
        let ty_head = b.name("record");
        let ty = b.list(vec![ty_head]);
        let colon = b.name(":");
        let root = b.list(vec![colon, record, ty]);
        let arenas = b.finish(root);

        let (name, id) =
            id_name_from_descriptor(&arenas).expect("a well-formed descriptor value form");
        assert_eq!(name, "temp.celsius");
        assert_eq!(id, want_id);
        assert_eq!(
            id.to_string(),
            want_id.to_string(),
            "renders the SAME base62 id"
        );
    }

    #[test]
    fn name_input_output_are_read_from_the_descriptor_value_form() {
        // Option (b) codegen path: execute descriptor() + read (name, input, output) to generate the kernel
        // `Contract::new(name, types, input, output)`. The descriptor's input/output fields are
        // `Ast.encode(Ast.Name(<type>))` (Bytes), so `identity_from_descriptor` decodes them back to the
        // type-name strings. Build that shape (with real encoded Name references) and assert the read.
        let encode_name = |sym: &str| -> Vec<u8> {
            let mut nb = Builder::new();
            let n = nb.name(sym);
            let a = nb.finish(n);
            cadenza_ast::codec::encode(&cadenza_ast::canon::canonicalize(&a))
        };
        let mut b = Builder::new();
        let field = |b: &mut Builder, key: &str, val: StructId| -> StructId {
            let eq = b.name("=");
            let k = b.name(key);
            b.list(vec![eq, k, val])
        };
        let name_val = b.atom_leaf(Leaf::Str(Arc::from("cdz-platform.deliver")));
        let f_name = field(&mut b, "name", name_val);
        let in_val = b.atom_leaf(Leaf::Bytes(Arc::from(&encode_name("Envelope")[..])));
        let f_in = field(&mut b, "input", in_val);
        let out_val = b.atom_leaf(Leaf::Bytes(Arc::from(&encode_name("Outcome")[..])));
        let f_out = field(&mut b, "output", out_val);
        let rec_head = b.name("record");
        let record = b.list(vec![rec_head, f_name, f_in, f_out]);
        let ty_head = b.name("record");
        let ty = b.list(vec![ty_head]);
        let colon = b.name(":");
        let root = b.list(vec![colon, record, ty]);
        let arenas = b.finish(root);

        let (name, input, output) =
            identity_from_descriptor(&arenas).expect("a well-formed descriptor value form");
        assert_eq!(name, "cdz-platform.deliver");
        assert_eq!(input, "Envelope");
        assert_eq!(output, "Outcome");
    }

    #[test]
    fn a_non_descriptor_value_names_no_contract() {
        // A value that is not a `(: (record …) …)` with `id`+`name` fields → `None` (a rejected value, not a
        // panic) — the total-parse discipline: a malformed descriptor names no contract.
        let mut b = Builder::new();
        let root = b.atom_leaf(Leaf::Str(Arc::from("not a descriptor")));
        let arenas = b.finish(root);
        assert!(id_name_from_descriptor(&arenas).is_none());
    }
}
