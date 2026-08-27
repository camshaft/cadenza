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

/// The contract-id (§1) of a contract declared as a **Cadenza module** — the form a `.cdz` contract source
/// takes and a directory of contracts is hashed from. A contract module carries its identity in three module
/// pragmas (the language's `@!key arg` module attribute, "compatible with the rest of the language" —
/// operator 2026-08-23) alongside its `type` declarations, so `cdz convert` yields a module value shaped:
///
/// ```text
/// (do (pragma contract "cdz-platform.deliver")   ; the contract name (a string)
///     (pragma input Envelope)                    ; the input type, by name
///     (pragma output Outcome)                    ; the output type, by name
///     (type Envelope …) (type Outcome …) …)      ; the declared types
/// ```
///
/// This reads those pragmas and the `(type …)` forms, assembles the same
/// `(contract <name> (types …) <input> <output>)` declaration [`contract_id`] builds, and hashes it — so a
/// contract's id computed from its module source equals the one the platform computes from the same
/// declaration. `None` if the module is not a well-formed contract (not a `(do …)`, or missing the
/// `contract` / `input` / `output` pragma) — total, never a panic, so a malformed source is a rejected
/// contract, not a crash.
///
/// The module AST is what `cadenza_ast::codec::decode` yields from `cdz convert --to binary` of the source;
/// the caller supplies the decoded [`Arenas`] (this crate does no I/O and does not shell out).
#[must_use]
pub fn contract_id_from_module(module: &Arenas) -> Option<Hash> {
    contract_from_module(module).map(|(_name, id)| id)
}

/// A contract's declared **name** and its [`contract_id`], read from its module source — the pair a
/// directory→mapping tool emits (name → hash). Reads the `contract` / `input` / `output` pragmas and the
/// `(type …)` forms of the `(do …)` module, assembles the canonical `(contract <name> (types …) <input>
/// <output>)` declaration [`contract_id`] builds, and hashes it. `None` if the module is not a well-formed
/// contract (not a `(do …)`, or missing a required pragma). The name is borrowed from `module`.
#[must_use]
pub fn contract_from_module(module: &Arenas) -> Option<(&str, Hash)> {
    // The top-level forms — a module is a `(do <form>…)`; the forms are the children after the `do` head.
    // Each may be wrapped in `(comment …)` nodes (a `//` doc-comment parses to `(comment <text> <form>)`,
    // so a form preceded by comments is nested inside them), so every read peels the wrappers first — the
    // same way `xtask codegen` extracts the type declarations, so the CLI's ids match the platform's.
    let forms = module.as_form(module.root, "do")?;
    let name = module.as_str(pragma_arg(module, forms, "contract")?)?;
    let input = module.as_name(pragma_arg(module, forms, "input")?)?;
    let output = module.as_name(pragma_arg(module, forms, "output")?)?;
    // The declared types, grafted into the declaration builder so the assembled value is identical to the
    // one `contract_id` builds from a native `types` closure (canonicalization makes the two structurally
    // equal, so they hash the same).
    let type_forms: Vec<StructId> = forms
        .iter()
        .map(|&f| unwrap_comment(module, f))
        .filter(|&f| module.head_name(f) == Some("type"))
        .collect();
    let id = contract_id(
        name,
        |b| type_forms.iter().map(|&t| graft(b, module, t)).collect(),
        input,
        output,
    );
    Some((name, id))
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

/// The single argument of the `(pragma <key> <arg>)` form with the given `key` among `forms`, or `None` if
/// no such pragma (or it is not the two-element `key arg` shape). The module attribute `@!key arg` desugars
/// to exactly this form. Each form is peeled of any `(comment …)` wrappers first, so a commented pragma is
/// still found.
fn pragma_arg(module: &Arenas, forms: &[StructId], key: &str) -> Option<StructId> {
    forms.iter().copied().find_map(|f| {
        let f = unwrap_comment(module, f);
        let args = module.as_form(f, "pragma")?;
        (args.len() == 2 && module.as_name(args[0]) == Some(key)).then_some(args[1])
    })
}

/// Peel `(comment <text> <form>)` wrappers off a top-level form, returning the wrapped form — a `//`
/// doc-comment nests the following form inside a comment node (chained for consecutive comment lines), and
/// the wrapped form is the comment's last child. Mirrors `xtask codegen`'s comment-unwrapping so the two
/// extract the same declarations.
fn unwrap_comment(module: &Arenas, id: StructId) -> StructId {
    let mut id = id;
    while module.head_name(id) == Some("comment") {
        match module.get(id) {
            Struct::List(items) => match items.last() {
                Some(&last) if last != id => id = last,
                _ => break,
            },
            Struct::Atom(_) => break,
        }
    }
    id
}

/// Copy the subtree rooted at `id` in `src` into `b`, returning the new id — a structural deep copy, since
/// `cadenza-ast` has no cross-arena graft. Used to move a decoded `(type …)` declaration into the
/// declaration builder verbatim; canonicalization then makes a grafted node equal to a natively-built one.
fn graft(b: &mut Builder, src: &Arenas, id: StructId) -> StructId {
    match src.get(id) {
        Struct::Atom(leaf) => b.atom_leaf(src.leaf(*leaf).clone()),
        Struct::List(children) => {
            let kids: Vec<StructId> = children.iter().map(|&c| graft(b, src, c)).collect();
            b.list(kids)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        contract_declaration, contract_id, contract_id_from_module, id_name_from_descriptor,
        identity_from_descriptor,
    };
    use crate::{Hash, HashTag};
    use cadenza_ast::ast::{Arenas, Leaf};
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

    /// A `(pragma <key> <arg>)` form, as `cdz convert` yields from `@!key arg`. `arg_is_name` picks a name
    /// atom (`@!input Envelope`) vs a string leaf (`@!contract "…"`).
    fn pragma(b: &mut Builder, key: &str, arg: &str, arg_is_name: bool) -> StructId {
        let head = b.name("pragma");
        let key = b.name(key);
        let arg = if arg_is_name {
            b.name(arg)
        } else {
            b.atom_leaf(Leaf::Str(Arc::from(arg)))
        };
        b.list(vec![head, key, arg])
    }

    /// Assemble a contract module `(do (pragma contract …) (pragma input …) (pragma output …) <types>…)`
    /// with `temp_type`'s single `(type Temp …)` declaration — the shape `cdz convert` produces.
    fn temp_module(name: &str, input: &str, output: &str) -> Arenas {
        let mut b = Builder::new();
        let do_head = b.name("do");
        let c = pragma(&mut b, "contract", name, false);
        let i = pragma(&mut b, "input", input, true);
        let o = pragma(&mut b, "output", output, true);
        let mut forms = vec![do_head, c, i, o];
        forms.extend(temp_type(&mut b));
        let root = b.list(forms);
        b.finish(root)
    }

    #[test]
    fn id_from_a_module_matches_the_direct_computation() {
        // A contract read from its module source (pragmas + type decls) hashes to exactly what the direct
        // `contract_id` computes from the same name/types/input/output — so the CLI's directory→hash mapping
        // agrees with the platform. Grafting the decoded `(type …)` forms reproduces the declaration.
        let module = temp_module("temp.celsius", "Temp", "Temp");
        assert_eq!(
            contract_id_from_module(&module),
            Some(contract_id("temp.celsius", temp_type, "Temp", "Temp"))
        );
    }

    #[test]
    fn a_module_missing_a_required_pragma_is_none() {
        // No `contract` pragma → not a well-formed contract module → None (a rejected contract, not a panic).
        let mut b = Builder::new();
        let do_head = b.name("do");
        let i = pragma(&mut b, "input", "Temp", true);
        let o = pragma(&mut b, "output", "Temp", true);
        let mut forms = vec![do_head, i, o];
        forms.extend(temp_type(&mut b));
        let root = b.list(forms);
        let module = b.finish(root);
        assert_eq!(contract_id_from_module(&module), None);
    }

    #[test]
    fn a_non_module_value_is_none() {
        // A bare value that is not a `(do …)` module names no contract.
        let mut b = Builder::new();
        let root = b.atom_leaf(Leaf::Str(Arc::from("not a module")));
        let module = b.finish(root);
        assert_eq!(contract_id_from_module(&module), None);
    }

    #[test]
    fn the_input_output_reference_is_read_from_its_pragma() {
        // Two declared types, and the input/output pragmas select which is which — swapping them changes the
        // id (the same nominal-identity property, now driven by the module's pragmas).
        fn ab_module(input: &str, output: &str) -> Arenas {
            let mut b = Builder::new();
            let do_head = b.name("do");
            let c = pragma(&mut b, "contract", "f", false);
            let i = pragma(&mut b, "input", input, true);
            let o = pragma(&mut b, "output", output, true);
            // (type A (Mk Int64)) and (type B (Mk Bool))
            let ta = {
                let int = b.name("Int64");
                let mk = form(&mut b, "Mk", vec![int]);
                let name = b.name("A");
                form(&mut b, "type", vec![name, mk])
            };
            let tb = {
                let boolean = b.name("Bool");
                let mk = form(&mut b, "Mk", vec![boolean]);
                let name = b.name("B");
                form(&mut b, "type", vec![name, mk])
            };
            let root = b.list(vec![do_head, c, i, o, ta, tb]);
            b.finish(root)
        }
        let ab = contract_id_from_module(&ab_module("A", "B")).expect("well-formed");
        let ba = contract_id_from_module(&ab_module("B", "A")).expect("well-formed");
        assert_ne!(ab, ba);
    }

    #[test]
    fn comment_wrapped_pragmas_and_types_are_read_the_same() {
        // A `//` doc-comment parses to `(comment <text> <form>)`, nesting the form it precedes. The reader
        // must peel those wrappers, or a commented contract source (every real one has comments) computes a
        // different id than its clean equivalent. Wrap the contract pragma and the type decl in a comment
        // and assert the id is unchanged.
        let clean = temp_module("temp.celsius", "Temp", "Temp");
        let wrapped = {
            let mut b = Builder::new();
            let do_head = b.name("do");
            // (comment "doc" (pragma contract "temp.celsius"))
            let c = {
                let inner = pragma(&mut b, "contract", "temp.celsius", false);
                let head = b.name("comment");
                let text = b.atom_leaf(Leaf::Str(Arc::from("doc")));
                b.list(vec![head, text, inner])
            };
            let i = pragma(&mut b, "input", "Temp", true);
            let o = pragma(&mut b, "output", "Temp", true);
            // (comment "doc" (type Temp (Mk f64)))
            let ty = {
                let inner = temp_type(&mut b).remove(0);
                let head = b.name("comment");
                let text = b.atom_leaf(Leaf::Str(Arc::from("doc")));
                b.list(vec![head, text, inner])
            };
            let root = b.list(vec![do_head, c, i, o, ty]);
            b.finish(root)
        };
        assert_eq!(
            contract_id_from_module(&wrapped),
            contract_id_from_module(&clean),
            "comment wrappers must not change the contract-id"
        );
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
        // panic) — the total-parse contract, like `contract_id_from_module`.
        let mut b = Builder::new();
        let root = b.atom_leaf(Leaf::Str(Arc::from("not a descriptor")));
        let arenas = b.finish(root);
        assert!(id_name_from_descriptor(&arenas).is_none());
    }
}
