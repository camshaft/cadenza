//! Sum-type SYNTHESIS — a `(type NAME variant…)` declaration realized as ordinary records.
//!
//! The operator's directive (and the whole "records everywhere" through-line): a sum type IS a record
//! in scope. `(type Option (Some Int64) None)` binds `Option` to a RECORD whose FIELDS ARE ITS
//! VARIANTS. Then `Option.Some` is ORDINARY MEMBER ACCESS (the `Int64.max` path), `(Option.Some 5)`
//! is an ORDINARY APPLICATION dispatched by the variant record's meta channel, and `Option` in TYPE
//! position projects `(meta t)` — every one reusing the machinery already built for the prelude's
//! records, with NO name special-casing (`prelude-and-resolution.md` §Nothing Is Privileged By Name).
//! This module is the program-driven twin of `prelude::install`: it appends the sum's records as
//! ordinary arena nodes and returns the `name → occurrence` map the resolver consults.
//!
//! **What a sum record holds.** For `(type Option (Some Int64) None)`:
//! ```text
//! Option = (record ((meta t) (typeval (Sum Option <decl>)))   ; the type-value — `Option` in type pos
//!                   (Some    (record ((meta t) (-> Int64 (typeval (Sum Option <decl>))))))
//!                   (None    (record ((meta t) (typeval (Sum Option <decl>))))))
//! ```
//! - The sum record's `(meta t)` is the sum TYPE-VALUE `Ty::Sum { decl, name }` (encoded as the wire
//!   node `(Sum NAME <decl>)`, decoded by `resolve::decode_ty`). So `(: x Option)` reduces to `Ty::Sum`
//!   through the ordinary `(meta t)` projection, exactly as `(: e UInt8)` reduces to `Ty::Int`.
//! - Each VARIANT is a field whose value is a variant-CONSTRUCTOR record carrying its `(meta t)` — the
//!   constructor's TYPE. A PAYLOAD variant's type is the curried arrow `(-> P… Sum)` (read by
//!   `apply_type` when the constructor is applied); a NULLARY variant's type is the sum itself.
//!
//! **Identity is the declaration, not the name** (`type-system.md §A Nominal Type's Identity Is Its
//! Fully-Qualified Name`, §160): the `(meta t)` carries the TypeDecl's occurrence `<decl>`, so module
//! A's `Foo` and module B's `Foo` are distinct types (distinct declarations ⇒ distinct `decl` ids).
//! This is import-safe — package linking splices each file's arena in, so every declaration keeps its
//! own occurrence.
//!
//! **Deferred to the construction tick.** A variant constructor here carries ONLY `(meta t)` — its
//! TYPE. Its `(meta apply)` (the `sum-new` builder) and `(meta variant)` (the discriminant + sum-group
//! metadata the shared construct intrinsic reads, mirroring how `Wrap` reads its target width off the
//! solved type) arrive with construction. Until then a constructor TYPES but declines to CONSTRUCT —
//! the same "present, but not yet realized" state a prelude `unrealized` field has, reached by the same
//! generic member-access-then-decline path.

use crate::ast::{Arenas, CompoundCtor, IntValue, Leaf, Radix, Struct, StructId};
use crate::db::{TypeDecl, Variant};
use crate::prelude::{meta_field, push_atom, push_list};

/// Synthesize the record for every scanned `(type …)` declaration, appending them to `ast` as ordinary
/// nodes and RECORDING each on its `TypeDecl.synth`. Called at load AFTER the scan (which produced
/// `decls`) and AFTER the prelude install, so the sum records take `StructId`s above the program's and
/// the parent index (built next) covers them. Deterministic: a fixed function of the declarations.
///
/// There is NO name→record map: a `(type …)` resolves exactly like a `def` — by the ordinary top-level
/// lookup against `db.type_decls` (an occurrence-keyed Vec), so two same-named declarations each keep
/// their own record and identity (a name-keyed map could not — the second would clobber the first). A
/// declaration with an empty name (malformed `(type)`) still gets a record but binds no name.
///
/// Each variant field holds a CONSTRUCTOR value (a record carrying the constructor's `(meta t)` type),
/// never a pre-applied Sum value — even a NULLARY variant like `None` is a constructor (typed as the
/// sum), built only when APPLIED `(None unit)`. So the built-in sums the prelude installs (Option/Result
/// /Ordering/Sign, via the same path) likewise bind Constructor values, not pre-constructed Sum values.
//= spec/capabilities/core-semantics.md#a-sum-type-constructor-is-a-single-arity-function-producing-the-tagged-variant
//# The prelude MUST bind Constructor values only for sum type variants, not pre-applied Sum values.
pub fn synthesize(ast: &mut Arenas, decls: &mut [TypeDecl]) {
    for decl in decls.iter_mut() {
        let (record, ctors) = sum_record(ast, decl);
        decl.synth = Some(record);
        // Cache each variant's constructor occurrence (built just above) on the variant, so a later
        // per-arm ctor lookup is O(1) rather than an O(V) name-scan of the record's variant fields.
        for (variant, ctor) in decl.variants.iter_mut().zip(ctors) {
            variant.ctor = Some(ctor);
        }
    }
}

/// Build the BUILT-IN sum declarations — generic `Option` and `Result` — as ordinary `(type …)` arena
/// forms, scanned into [`TypeDecl`]s (exactly like a user declaration). Appends the declaration nodes to
/// `ast` and returns the decls; `Db::load` appends them to `db.type_decls` so `synthesize` builds their
/// records, and binds the names (`Option`/`Some`/`None`, `Result`/`Ok`/`Err`) in the prelude map. A
/// program uses bare `Some`/`None`/`Ok`/`Err` — the corpus surface — without declaring them; a user
/// `(type Option …)` shadows (top-level `type_decls` resolve before the prelude). Making these ordinary
/// declarations (not a bespoke built-in shape) keeps the ONE sum mechanism — nothing about Option is
/// privileged (`reference-compiler.md` §Nothing Is Privileged By Name).
pub fn prelude_decls(ast: &mut Arenas) -> Vec<TypeDecl> {
    // `(type Option (Some a) None)`.
    let option = type_form(ast, "Option", &[("Some", &["a"]), ("None", &[])]);
    // `(type Result (Ok a) (Err e))`.
    let result = type_form(ast, "Result", &[("Ok", &["a"]), ("Err", &["e"])]);
    // `(type Sign Neg Zero Pos)` — a MONOMORPHIC three-variant sum (all nullary), the sign of a number.
    // A closed prelude sum like Option/Result (§`Sign` is a prelude sum alongside Option/Result); a
    // program uses bare `Sign.Neg`/`Zero`/`Pos` without declaring it. Nothing privileged — the same
    // `type_form` + `scan_type_decl` path, just no type parameters.
    let sign = type_form(ast, "Sign", &[("Neg", &[]), ("Zero", &[]), ("Pos", &[])]);
    // `(type Ordering Less Equal Greater)` — the result of the three-way comparison (core-semantics.md §A
    // Total Order Is Observed Through A Three-Way Comparison). A monomorphic closed prelude sum like
    // `Sign`; the DISCRIMINANT ORDER is Less=0, Equal=1, Greater=2, which the comparison maps a `<`/`=`/`>`
    // ordering to. The comparison is NAMESPACED as `Ordering.of` (its associated function, below) — the
    // former top-level `compare`.
    let ordering = {
        let form = type_form(
            ast,
            "Ordering",
            &[("Less", &[]), ("Equal", &[]), ("Greater", &[])],
        );
        let mut decl = crate::db::scan_type_decl(ast, form).expect("built-in Ordering decl scans");
        // `Ordering.of` — the three-way comparison, set HERE at the Ordering declaration (prelude-defined),
        // the SAME `TypeDecl.associated` pattern as `Ast.module`; `sum_record` appends it to the synthesized
        // Ordering record. Replaces the former top-level `compare` (operator directive: prelude records with
        // associated functions, no bare globals).
        decl.associated = vec![crate::prelude::ordering_of_field(ast)];
        decl
    };
    // `(type Ast (Int BigInt) (Name String) (List (List Ast)))` — THE abstract-syntax-tree type
    // (`metaprogramming.md` §Quote Produces An AST Value; type-system.md §The Abstract Syntax Tree Type
    // Is An Ordinary Sum Type). A RECURSIVE, MONOMORPHIC prelude sum: a variant per syntactic form, each
    // with a CONCRETE or COMPOUND payload (`BigInt`, `String`, `(List Ast)`) — richer than the bare
    // type-parameter payloads `type_form` builds, so its variants are built with explicit payload
    // type-expression nodes. `quote`/`Ast.*` produce this value; a program reaches its variants ONLY
    // QUALIFIED (`Ast.Int` = `(. Ast Int)`), NOT bare — its variant names `Int`/`Name`/`List` collide with
    // prelude type/module names, and the `variant_ctor_index` build (`db.rs`) skips a prelude-colliding
    // variant so it never shadows the built-in. Nothing privileged — the same `scan_type_decl` +
    // `synthesize` path as Option/Result/Sign/Ordering, so `Ast` is CONSTRUCTED by the ordinary
    // variant-constructor mechanism (`Ast.Int` is member access + `sum-new`) and DECONSTRUCTED by the
    // ordinary variant-pattern match (`crate::quote::reify_pattern`) — no reflection primitive; a
    // compiler written in the language walks a program's `Ast` value exactly as it walks any sum.
    //= spec/capabilities/type-system.md#the-abstract-syntax-tree-is-an-ordinary-sum-type
    //# The AST sum type MUST be constructed and deconstructed by the same variant-construction and match mechanisms as any other sum type, so that a compiler written in the language walks a program as data with no reflection primitive.
    // The self-hosting-surface twin: `Ast` being an ordinary `(type …)` value is exactly what lets a
    // compiler authored in Cadenza examine a program without a foreign representation, and determining a
    // node's KIND (its variant discriminant) + obtaining its CHILDREN (the payload) is the ordinary
    // variant-pattern match — so the tree is walked structurally with no dedicated reflection.
    //= spec/capabilities/self-hosting-surface.md#a-program-s-syntax-tree-is-an-ordinary-value
    //# A program's abstract syntax tree MUST be expressible as an ordinary value of the language, so that a compiler authored in the language can examine a program without a foreign representation.
    //= spec/capabilities/self-hosting-surface.md#a-program-s-syntax-tree-is-an-ordinary-value
    //# A compiler MUST be able to determine a node's kind and obtain its children from that value, so that it can walk the tree structurally.
    let ast_decl = {
        let int_pay = push_atom(ast, Leaf::Name("BigInt".into()));
        let float_pay = push_atom(ast, Leaf::Name("Float64".into()));
        let bool_pay = push_atom(ast, Leaf::Name("Bool".into()));
        let str_pay = push_atom(ast, Leaf::Name("String".into()));
        let name_pay = push_atom(ast, Leaf::Name("String".into()));
        // `Bytes` — a raw byte-sequence LITERAL (`b"…"` / a `Bytes` value), the syntactic form for a
        // binary blob. Its payload is the `Bytes` type (not a `(List Int64)`), so a blob rides the AST +
        // its codec as ONE length-prefixed raw-bytes leaf (codec `KIND_BYTES`, tag 11 — already present)
        // rather than a node-per-byte list, cutting encode/decode overhead on the invoke wire format
        // (operator seq 113). Appended LAST so existing variant discriminants are unchanged (discs are
        // read BY NAME via `ast_variant_discs`, so order is display-only).
        let bytes_pay = push_atom(ast, Leaf::Name("Bytes".into()));
        // `Char` / `Symbol` — the remaining scalar syntax leaves (`#\a` char, `#"x"` symbol). Their
        // payloads are the ground `Char` / `Symbol` types (both nullary, like `String`). Appended LAST
        // (after `Bytes`) so existing discriminants are unchanged (discs are read BY NAME via
        // `ast_variant_discs`). Adding them makes IMPORT REFLECTION (`__ast__`) + `quote` TOTAL over
        // syntax leaves: a module containing a char/symbol literal reflects instead of declining (operator
        // directive — reflection must never bail, the same full-generality bar as the WIT surface). The
        // codec already carries these leaves (`KIND_CHAR`/`KIND_SYM` + `Leaf::Char`/`Leaf::Sym`), so no
        // byte-format change — only the guest-facing `Ast` VALUE sum gains the two variants.
        let char_pay = push_atom(ast, Leaf::Name("Char".into()));
        let sym_pay = push_atom(ast, Leaf::Name("Symbol".into()));
        // `(List Ast)` — the recursive list-of-Ast payload for the `List` variant.
        let list_head = push_atom(ast, Leaf::Name("List".into()));
        let ast_ref = push_atom(ast, Leaf::Name("Ast".into()));
        let list_ast = push_list(ast, vec![list_head, ast_ref]);
        // The native-compound-data OPTION-B variants (operator 2026-08-28: "end-to-end native collections
        // in the binary AST, no string heads for collections anywhere") — one reflected variant per
        // ctor-head leaf kind (20-26), so `quote`/`Ast.encode` of a collection produces a FIRST-CLASS
        // native ctor variant instead of a string/name-headed `List`. The 5 collection ctors carry
        // `(List Ast)` children (a record/map's children are `FieldPair` nodes); `FieldPair`/`Member` carry
        // `(Tuple Ast Ast)` (`(= key value)` and `(. obj key)` respectively). The generic `List` variant is
        // KEPT for a non-collection name-headed node (`(if …)`, `(fn …)`, an application). Appended LAST so
        // existing discriminants are unchanged (discs are read BY NAME via `ast_variant_discs`). The codec
        // already carries these leaf kinds (`KIND_*_CTOR`/`FIELD_PAIR`/`MEMBER` 20-26 + the `Leaf::Ctor`/
        // `FieldPair`/`Member` leaves), so no byte-format change here — this is the guest-facing `Ast` VALUE
        // sum gaining the seven variants. See `DESIGN-native-ast-compound-data.md` + type-system.md.
        // A fresh `(List Ast)` payload node per collection variant (own occurrence, mirrors `list_ast`).
        let list_ast_payload = |ast: &mut Arenas| {
            let h = push_atom(ast, Leaf::Name("List".into()));
            let a = push_atom(ast, Leaf::Name("Ast".into()));
            push_list(ast, vec![h, a])
        };
        // A fresh `(Tuple Ast Ast)` payload node — the `(= key value)` / `(. obj key)` pair shape.
        let tuple_ast_ast_payload = |ast: &mut Arenas| {
            let h = push_atom(ast, Leaf::Name("Tuple".into()));
            let a1 = push_atom(ast, Leaf::Name("Ast".into()));
            let a2 = push_atom(ast, Leaf::Name("Ast".into()));
            push_list(ast, vec![h, a1, a2])
        };
        let listctor_pay = list_ast_payload(&mut *ast);
        let tuplector_pay = list_ast_payload(&mut *ast);
        let recordctor_pay = list_ast_payload(&mut *ast);
        let mapctor_pay = list_ast_payload(&mut *ast);
        let setctor_pay = list_ast_payload(&mut *ast);
        let fieldpair_pay = tuple_ast_ast_payload(&mut *ast);
        let member_pay = tuple_ast_ast_payload(&mut *ast);
        // The `Ast` sum's variants follow the spec's enumeration order (`type-system.md` §The Abstract
        // Syntax Tree Is An Ordinary Sum Type: "an integer, a float, a string, a boolean, a name, and a
        // list of child nodes"). The variants: `Int` (BigInt — non-lossy quoted-integer storage), `Float`
        // (Float64), `Bool` (Bool), `Str` (String — a string LITERAL, distinct from `Name` which carries an
        // identifier), `Name` (String), `List` ((List Ast)) — the spec's set — plus `Bytes` (Bytes — a raw
        // byte-sequence literal `b"…"`, appended for compact binary-AST encoding, operator seq 113).
        // Discriminants are read BY NAME everywhere (`ast_variant_discs`), never positionally, so this order
        // is display-only and appending a variant never mis-tags an existing one.
        //
        // `Ast` is an ordinary `(type …)` sum built by the same `type_form_payloads`/`scan_type_decl` path
        // as any user sum — NOT a compiler-special-cased primitive — with exactly the spec's variant per
        // syntactic form and a `List` variant carrying a `(List Ast)` of the same type:
        //= spec/capabilities/type-system.md#the-abstract-syntax-tree-is-an-ordinary-sum-type
        //# The abstract syntax tree type MUST be an ordinary sum type of the language — a variant per syntactic form (an integer, a float, a string, a boolean, a name, and a list of child nodes) with the list variant carrying a list of the same type — rather than a primitive the type system special-cases.
        let form = type_form_payloads(
            ast,
            "Ast",
            &[
                ("Int", &[int_pay]),
                ("Float", &[float_pay]),
                ("Bool", &[bool_pay]),
                ("Str", &[str_pay]),
                ("Name", &[name_pay]),
                ("List", &[list_ast]),
                ("Bytes", &[bytes_pay]),
                ("Char", &[char_pay]),
                ("Symbol", &[sym_pay]),
                // Option-B native compound-ctor variants (mirror leaf kinds 20-26), appended LAST.
                ("ListCtor", &[listctor_pay]),
                ("TupleCtor", &[tuplector_pay]),
                ("RecordCtor", &[recordctor_pay]),
                ("MapCtor", &[mapctor_pay]),
                ("SetCtor", &[setctor_pay]),
                ("FieldPair", &[fieldpair_pay]),
                ("Member", &[member_pay]),
            ],
        );
        let mut decl = crate::db::scan_type_decl(ast, form).expect("built-in Ast decl scans");
        // Ast's ASSOCIATED FUNCTIONS (`Ast.module` self-reflection; `Ast.print`/`Ast.read` once those land)
        // — set HERE, at the Ast declaration, prelude-defined. `sum_record` appends `decl.associated` to the
        // synthesized Ast record, so these live in the PRELUDE like `bigint_module`'s fields — not a
        // `Db::load` post-synthesis special-case, and not found-by-name after the fact.
        decl.associated = crate::prelude::ast_associated_fields(ast);
        decl
    };
    // The plain prelude sums (no associated functions) scan straight through; `ast_decl` is ALREADY a
    // fully-formed `TypeDecl` (its associated functions were set at its declaration above), spliced in at
    // its canonical position — no scan-then-find-by-name.
    let mut decls: Vec<TypeDecl> = [option, result, sign]
        .into_iter()
        .filter_map(|item| crate::db::scan_type_decl(ast, item))
        .collect();
    // `ordering` and `ast_decl` are ALREADY fully-formed `TypeDecl`s (their associated functions —
    // `Ordering.of` / `Ast.module` — were set at their declarations above), spliced in at their positions.
    decls.push(ordering);
    decls.push(ast_decl);
    decls
}

/// Build a `(type NAME (V payload-node…)…)` declaration form from PRE-BUILT payload TYPE-EXPRESSION nodes
/// — the variant of [`type_form`] for a sum whose payloads are concrete or compound types (`Int64`,
/// `(List Ast)`), not bare type-parameter names. Each variant is `(vname payload-node…)`; a nullary
/// variant (`&[]`) is a bare `vname`.
fn type_form_payloads(ast: &mut Arenas, name: &str, variants: &[(&str, &[StructId])]) -> StructId {
    let head = push_atom(ast, Leaf::Name("type".into()));
    let name_occ = push_atom(ast, Leaf::Name(name.into()));
    let mut children = vec![head, name_occ];
    for (vname, payloads) in variants {
        if payloads.is_empty() {
            children.push(push_atom(ast, Leaf::Name((*vname).into())));
        } else {
            let mut vlist = vec![push_atom(ast, Leaf::Name((*vname).into()))];
            vlist.extend_from_slice(payloads);
            children.push(push_list(ast, vlist));
        }
    }
    push_list(ast, children)
}

/// The occurrence of the variant-constructor field named `vname` inside a synthesized sum `record` —
/// so the prelude map can bind a BARE variant name (`Some`) to its constructor. The record is `(record
/// ((meta t) …) [(meta apply) …] [(meta sum-decl) …] (= Some <ctor>) (= None <ctor>)…)`; a variant field
/// is the canonical `(= name <ctor>)` FieldPair (seq-276) — or the legacy bare `(name <ctor>)` 2-element
/// pair — whose name matches `vname`. `None` if not found (e.g. a meta field).
pub fn variant_ctor_field(ast: &Arenas, record: StructId, vname: &str) -> Option<StructId> {
    let Struct::List(children) = ast.get(record) else {
        return None;
    };
    for &field in children.iter().skip(1) {
        // seq-276: `sum_record` now emits each variant field as the canonical `(= name <ctor>)` FieldPair;
        // read that shape via `field_pair`, still accepting the legacy bare `(name <ctor>)` 2-element pair.
        let (name_id, ctor_id) = if let Some(kv) = ast.field_pair(field) {
            kv
        } else if let Struct::List(pair) = ast.get(field)
            && pair.len() == 2
        {
            (pair[0], pair[1])
        } else {
            continue;
        };
        if ast.as_name(name_id) == Some(vname) {
            return Some(ctor_id);
        }
    }
    None
}

/// Build a `(type NAME (V pay…)…)` declaration form in the arena and return its occurrence. Each
/// variant is `(vname payload-name…)` (a payload is a bare type-parameter name — lowercase, so the scan
/// reads it as an implicit generic). A nullary variant (`&[]` payloads) is a bare `vname`.
fn type_form(ast: &mut Arenas, name: &str, variants: &[(&str, &[&str])]) -> StructId {
    let head = push_atom(ast, Leaf::Name("type".into()));
    let name_occ = push_atom(ast, Leaf::Name(name.into()));
    let mut children = vec![head, name_occ];
    for (vname, payloads) in variants {
        if payloads.is_empty() {
            // Nullary variant — a bare name.
            children.push(push_atom(ast, Leaf::Name((*vname).into())));
        } else {
            // `(vname payload…)`.
            let mut vlist = vec![push_atom(ast, Leaf::Name((*vname).into()))];
            for p in *payloads {
                vlist.push(push_atom(ast, Leaf::Name((*p).into())));
            }
            children.push(push_list(ast, vlist));
        }
    }
    push_list(ast, children)
}

/// Build one sum's record: `(record ((meta t) <sum-typeval>) (<variant> <ctor>)…)`. The `(meta t)` is
/// the sum's own type-value; each variant is a field to its constructor record. A GENERIC sum (with
/// type params) ALSO gets `(meta apply)` = the `sum-ctor` intrinsic + `(meta sum-decl)` = its
/// declaration occurrence, so `(Option Int64)` in type position APPLIES the sum constructor to build
/// `Ty::Sum { decl, args: [Int64] }` — the same "a type ctor's `(meta apply)` builds a type" model as
/// `Int`/`Tuple`. A monomorphic sum needs no `(meta apply)` (it is never applied in type position).
///
/// Returns the record occurrence AND, in declaration order, each variant's constructor occurrence — so
/// `synthesize` can cache them on the variants (an O(1) later ctor lookup instead of a name-scan).
fn sum_record(ast: &mut Arenas, decl: &TypeDecl) -> (StructId, Vec<StructId>) {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let mut children = vec![head];
    let mut ctors = Vec::with_capacity(decl.variants.len());

    // `(meta t)` — the sum type-value, so `Option` used in type position reduces to `Ty::Sum` (with no
    // args — the base, un-applied form; a generic instantiation goes through `(meta apply)` below).
    let sum_ty = sum_typeval(ast, decl);
    children.push(meta_field(ast, "t", sum_ty));

    // A GENERIC sum is applyable in type position: `(meta apply)` = the shared `sum-ctor` intrinsic,
    // `(meta sum-decl)` = this declaration's occurrence (read at reduction to build `Ty::Sum{decl,args}`
    // — the analogue of a variant ctor's `(meta variant)` disc).
    if !decl.params.is_empty() {
        let builder = {
            let ih = push_atom(ast, Leaf::Name("intrinsic".into()));
            let who = push_atom(ast, Leaf::Name("sum-ctor".into()));
            push_list(ast, vec![ih, who])
        };
        children.push(meta_field(ast, "apply", builder));
        let decl_node = push_atom(
            ast,
            Leaf::Int {
                value: IntValue::from_i64(decl.occ.0 as i64),
                radix: Radix::Dec,
            },
        );
        children.push(meta_field(ast, "sum-decl", decl_node));
    }

    // One field per variant, its value the constructor record. `decl.variants` carries each variant's
    // payload TYPE occurrences (from the scan), so the constructor's arrow type reads them directly. The
    // variant's INDEX in declaration order is its DISCRIMINANT (`value-heap-runtime.md` §Sum).
    for (disc, variant) in decl.variants.iter().enumerate() {
        let ctor = variant_ctor(ast, decl, variant, disc as u32);
        ctors.push(ctor);
        let k = push_atom(ast, Leaf::Name(variant.name.clone().into()));
        children.push({
            let eq = push_atom(ast, Leaf::Name("=".into()));
            push_list(ast, vec![eq, k, ctor])
        });
    }

    // The `expect` ACCESSOR field — the unwrap-or-trap `∀params. (Sum params) → String → <payload0>`
    // (`Option.expect`/`Result.expect`, core-semantics.md §Requiring The Value Of An Optional Traps On
    // Absence). Present on a sum whose PRESENT variant (discriminant 0) carries exactly ONE payload — the
    // Option/Result shape — since expect unwraps that one payload; a nullary or multi-payload disc-0 has
    // no single value to yield, so no `expect` field (and a program's `(. T expect)` declines through the
    // ordinary closed-record projection, not a name special-case). Nothing is privileged BY NAME: the
    // field is added by SHAPE, so a user sum with the same disc-0-single-payload shape gets it too.
    if let Some(present) = decl.variants.first()
        && present.payloads.len() == 1
    {
        let expect_ty = expect_type_scheme(ast, decl, present.payloads[0]);
        let expect_op = expect_op_record(ast, expect_ty);
        let ek = push_atom(ast, Leaf::Name("expect".into()));
        children.push({
            let eq = push_atom(ast, Leaf::Name("=".into()));
            push_list(ast, vec![eq, ek, expect_op])
        });
    }

    // The `encode`/`decode` ACCESSOR fields — the binary bijection (`ast-encoding.md` §The Encoding Is A
    // Bijection With One Canonical Byte Form). Present on a sum that is a RECURSIVE NODE TREE — one with a
    // variant whose payload is `(List Self)`, the "tree of nodes, each a symbol applied to an ordered
    // sequence of child nodes" the encoding contract is defined over. Added by SHAPE, not by name (like
    // `expect`): the built-in `Ast` sum matches, and a user sum of the same recursive-list-of-self shape
    // would too. `encode : Sum → Bytes`, `decode : Bytes → (Result Sum e)` (total — a non-canonical byte
    // sequence yields `Err`, never a trap). A sum that is NOT such a tree gets no fields (and `(. T
    // encode)` declines through the ordinary closed-record projection, not a name special-case).
    if is_node_tree_sum(ast, decl) {
        let self_ty = sum_applied(ast, decl);
        let encode_ty = {
            let bytes = intrinsic_node(ast, "bytes-ty");
            arrow_type(ast, self_ty, bytes) // (-> Sum Bytes)
        };
        let encode_op = intrinsic_op_record(ast, encode_ty, "ast-encode");
        let ek = push_atom(ast, Leaf::Name("encode".into()));
        children.push({
            let eq = push_atom(ast, Leaf::Name("=".into()));
            push_list(ast, vec![eq, ek, encode_op])
        });

        // `decode : Bytes → (Result Sum e)` — total; `e` is a free error type (a fresh lambda param so the
        // caller unifies it), the sum reference re-built fresh so it does not share the encode occurrence.
        let decode_ty = decode_type_scheme(ast, decl);
        let decode_op = intrinsic_op_record(ast, decode_ty, "ast-decode");
        let dk = push_atom(ast, Leaf::Name("decode".into()));
        children.push({
            let eq = push_atom(ast, Leaf::Name("=".into()));
            push_list(ast, vec![eq, dk, decode_op])
        });
    }

    // ASSOCIATED FUNCTIONS the decl declares (`TypeDecl.associated`) — prelude-defined non-ctor member
    // fields appended to this sum's record, so a built-in prelude sum's namespaced operations (the built-in
    // `Ast` record's `Ast.module` self-reflection; later `Ast.print`/`Ast.read`) are reached as
    // `(. Type member)` like a ctor. Data-driven: append whatever the decl carries — only a prelude sum
    // with declared associated ops has any; a user sum's is empty. NOT a name test here (`prelude_decls`
    // attaches them to the built-in decl), so nothing is privileged by name in this generic builder.
    children.extend(decl.associated.iter().copied());

    (push_list(ast, children), ctors)
}

/// Whether `decl` is a RECURSIVE NODE TREE — a sum with a variant whose sole payload is `(List Self)`
/// (a homogeneous list of the sum itself). This is the structural shape the AST-encoding bijection is
/// defined over (`ast-encoding.md`: "a tree of nodes, each a symbol applied to an ordered sequence of
/// child nodes"), so a sum matching it carries the `encode`/`decode` fields. A SHAPE test, not a name
/// test — the built-in `Ast` matches (its `List` variant is `(List Ast)`), and nothing is privileged by
/// name. Reads the raw payload TYPE occurrence: a 2-element list `(h x)` with `h` = `List` (the built-in
/// list constructor) and `x` = the sum's own name.
fn is_node_tree_sum(ast: &Arenas, decl: &TypeDecl) -> bool {
    decl.variants.iter().any(|v| {
        v.payloads.len() == 1
            && matches!(ast.get(v.payloads[0]), Struct::List(items)
                if items.len() == 2
                    && ast.as_name(items[0]) == Some("List")
                    && ast.as_name(items[1]) == Some(decl.name.as_str()))
    })
}

/// An operation record `(record ((meta t) TYPE) ((meta apply) (intrinsic PRIM)))` — the operator-record
/// shape whose `(meta apply)` is the named intrinsic, so projecting the field and applying it rides the
/// ordinary `(meta apply)` dispatch to the intrinsic's lowering. The generic form of `expect_op_record`.
fn intrinsic_op_record(ast: &mut Arenas, type_scheme: StructId, prim: &str) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let t_field = meta_field(ast, "t", type_scheme);
    let builder = intrinsic_node(ast, prim);
    let apply_field = meta_field(ast, "apply", builder);
    push_list(ast, vec![head, t_field, apply_field])
}

/// `(intrinsic "NAME")` — a type-value / builder reference node. The sums-local twin of the same helper
/// in `prelude` (a bare name would mis-resolve inside the record being built).
fn intrinsic_node(ast: &mut Arenas, name: &str) -> StructId {
    let ih = push_atom(ast, Leaf::Name("intrinsic".into()));
    let who = push_atom(ast, Leaf::Name(name.into()));
    push_list(ast, vec![ih, who])
}

/// `(-> L R)` — a function-type expression node.
fn arrow_type(ast: &mut Arenas, l: StructId, r: StructId) -> StructId {
    let arrow = push_atom(ast, Leaf::Name("->".into()));
    push_list(ast, vec![arrow, l, r])
}

/// The `decode` field's type — `Bytes → (Result Sum e)`, wrapped in a `(fn (e) …)` type-lambda so the
/// error type `e` is a fresh scheme variable the caller unifies (the corpus discards it as `(Err _)`).
/// `Sum` is the sum's applied type-value (`sum_applied`); `Result` is the built-in generic sum.
fn decode_type_scheme(ast: &mut Arenas, decl: &TypeDecl) -> StructId {
    let result_sum_e = {
        let result = push_atom(ast, Leaf::Name("Result".into()));
        let sum = sum_applied(ast, decl);
        let e = push_atom(ast, Leaf::Name("e".into()));
        push_list(ast, vec![result, sum, e])
    };
    let bytes = intrinsic_node(ast, "bytes-ty");
    let body = arrow_type(ast, bytes, result_sum_e); // (-> Bytes (Result Sum e))
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let e_param = push_atom(ast, Leaf::Name("e".into()));
    let params = push_list(ast, vec![e_param]);
    push_list(ast, vec![fn_head, params, body])
}

/// The `expect` field's type scheme — `∀params. (Sum params) → String → <payload0>` — as a `(meta t)`
/// type expression. For a GENERIC sum a type-LAMBDA over the sum's params (so `scheme_of` reads a
/// polymorphic scheme, `(Option.expect (Some 5) "m")` unifying the payload param to `Int64`); the body
/// is `(-> (Sum params) (-> String <payload0>))`. `payload0` is the disc-0 variant's single payload TYPE
/// occurrence (COPIED fresh so a `a` re-resolves to the lambda param), the value `expect` yields.
fn expect_type_scheme(ast: &mut Arenas, decl: &TypeDecl, payload0: StructId) -> StructId {
    // `(-> (Sum params) (-> String <payload0>))`.
    let ret = copy_subtree(ast, payload0);
    let string = {
        let ih = push_atom(ast, Leaf::Name("intrinsic".into()));
        let who = push_atom(ast, Leaf::Name("String".into()));
        push_list(ast, vec![ih, who])
    };
    let arrow2 = push_atom(ast, Leaf::Name("->".into()));
    let inner = push_list(ast, vec![arrow2, string, ret]); // (-> String <payload0>)
    let sum = sum_applied(ast, decl); // (Sum params) or the bare typeval
    let arrow1 = push_atom(ast, Leaf::Name("->".into()));
    let body = push_list(ast, vec![arrow1, sum, inner]); // (-> (Sum params) (-> String payload0))
    if decl.params.is_empty() {
        return body;
    }
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let param_atoms: Vec<StructId> = decl
        .params
        .iter()
        .map(|p| push_atom(ast, Leaf::Name(p.clone().into())))
        .collect();
    let params_list = push_list(ast, param_atoms);
    push_list(ast, vec![fn_head, params_list, body])
}

/// The `expect` operation record: `(record ((meta t) TYPE-SCHEME) ((meta apply) (intrinsic sum-expect)))`
/// — the same operator-record shape as a variant constructor, but its `(meta apply)` is the `sum-expect`
/// intrinsic (`Prim::SumExpect`). Projecting `(. Option expect)` gives this record; applying it dispatches
/// through the ordinary `(meta apply)` path to the unwrap-or-trap lowering.
fn expect_op_record(ast: &mut Arenas, type_scheme: StructId) -> StructId {
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let t_field = meta_field(ast, "t", type_scheme);
    let builder = {
        let ih = push_atom(ast, Leaf::Name("intrinsic".into()));
        let who = push_atom(ast, Leaf::Name("sum-expect".into()));
        push_list(ast, vec![ih, who])
    };
    let apply_field = meta_field(ast, "apply", builder);
    push_list(ast, vec![head, t_field, apply_field])
}

/// A variant constructor record. It carries THREE meta channels — the SAME shape an operator record
/// has, so a variant application rides the ordinary `(meta apply)` dispatch:
///  - `(meta t)` — the constructor's TYPE, the curried arrow `(-> payload… Sum)` (bare `Sum` for a
///    nullary variant). Read by `apply_type` when the constructor is applied.
///  - `(meta apply)` — the `(intrinsic sum-new)` builder (`Prim::SumNew`). Applying the constructor
///    projects this and lowers to `sum-new(disc, payload)`.
///  - `(meta variant)` — the DISCRIMINANT (an integer literal), the one datum the shared `sum-new`
///    intrinsic reads at lowering to know WHICH variant it is building (the analogue of `Wrap` reading
///    its target width off the solved type — one prim, the specific value in the metadata, no
///    per-variant prim). The owning sum + payload arity are recovered from the ctor's `(meta t)`, so
///    the discriminant is all this channel needs.
fn variant_ctor(ast: &mut Arenas, decl: &TypeDecl, variant: &Variant, disc: u32) -> StructId {
    // The record head is the NATIVE ctor-LEAF `Leaf::Ctor(Record)` (unshadowable, recognized by kind — the
    // NAME `record` is a shadowable alias); it resolves structurally via `compound_ctor_leaf` (the M3
    // reader-flip removed the legacy `"record"` string-head dual-read).
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let ctor_ty = ctor_type_scheme(ast, decl, variant);
    let t_field = meta_field(ast, "t", ctor_ty);
    // `(meta apply)` = the shared sum-new intrinsic.
    let builder = {
        let ih = push_atom(ast, Leaf::Name("intrinsic".into()));
        let who = push_atom(ast, Leaf::Name("sum-new".into()));
        push_list(ast, vec![ih, who])
    };
    let apply_field = meta_field(ast, "apply", builder);
    // `(meta variant)` = the discriminant, an integer literal.
    let disc_node = push_atom(
        ast,
        Leaf::Int {
            value: IntValue::from_i64(disc as i64),
            radix: Radix::Dec,
        },
    );
    let variant_field = meta_field(ast, "variant", disc_node);
    push_list(ast, vec![head, t_field, apply_field, variant_field])
}

/// The constructor's type EXPRESSION as a `(meta t)` — for a GENERIC sum, a type-LAMBDA over the sum's
/// params so it reads as a polymorphic scheme; for a MONOMORPHIC sum, the bare arrow. `Some` of `(type
/// Option (Some a) None)` gets `(fn (a) (-> a (Option a)))` — a scheme `∀a. a → Option a`, so
/// `(Option.Some 5)` unifies `a = Int64` and yields `Option Int64` through the ordinary `scheme_of` +
/// `apply_type` machinery (the same generic path an operator's `(meta t)` takes). A monomorphic variant
/// keeps `(-> payload… Sum)` (no lambda, `params` empty).
fn ctor_type_scheme(ast: &mut Arenas, decl: &TypeDecl, variant: &Variant) -> StructId {
    let body = ctor_arrow(ast, decl, variant);
    if decl.params.is_empty() {
        return body;
    }
    // `(fn (a b…) <arrow>)` — the params, in first-appearance order, quantified over the ctor type.
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    let param_atoms: Vec<StructId> = decl
        .params
        .iter()
        .map(|p| push_atom(ast, Leaf::Name(p.clone().into())))
        .collect();
    let params_list = push_list(ast, param_atoms);
    push_list(ast, vec![fn_head, params_list, body])
}

/// The constructor's ARROW type `(-> payload… <sum>)`. A NULLARY variant (no payloads) IS the sum type
/// directly. A PAYLOAD variant `(Some a)` is `(-> a <sum>)`; multiple payloads curry (`(Cons a b)` →
/// `(-> a (-> b <sum>))`). The result `<sum>` is the sum APPLIED to its params — `(Option a)` for a
/// generic sum (so it reduces to `Ty::Sum{args:[a]}` under the lambda), or the bare sum type-value for a
/// monomorphic sum. Each payload TYPE occurrence is COPIED fresh (a structural copy re-resolving in the
/// synthesized scope — a lowercase `a` re-resolves to the lambda param; a single-parent invariant too).
fn ctor_arrow(ast: &mut Arenas, decl: &TypeDecl, variant: &Variant) -> StructId {
    // Innermost: the sum the constructor produces — applied to its params when generic.
    let mut ty = sum_applied(ast, decl);
    // Wrap right-to-left in `(-> payload …)` so the arrow curries in declaration order.
    for &payload in variant.payloads.iter().rev() {
        let arrow = push_atom(ast, Leaf::Name("->".into()));
        let p = copy_payload_type(ast, payload, decl);
        ty = push_list(ast, vec![arrow, p, ty]);
    }
    ty
}

/// Copy a payload TYPE occurrence for a constructor arrow, rewriting a BARE SELF-REFERENCE in a GENERIC
/// sum to carry the declaration's own type parameters. In `(type Tree (Leaf a) (Branch (Tuple Tree
/// Tree)))` the bare `Tree` in the payload is the sum applied to its OWN params — `(Tree a)` — exactly
/// as the constructor's RESULT type is (`sum_applied`). Without this rewrite a bare `Tree` reduced to the
/// args-LESS `Ty::Sum{args:[]}`, which does not unify with the `(Tree a)` a `Branch` value carries →
/// `cannot unify Tree with (Tree Int64)`. This is the generic analogue of a MONOMORPHIC sum's bare
/// self-reference (`(Tuple Int64 IntList)`), which needs no rewrite (no params to apply); so the rewrite
/// fires ONLY when the sum has params AND the payload names the sum bare (an already-applied `(Tree X)`
/// or an unrelated type is copied verbatim by `copy_subtree`). Descends compound payloads (`(Tuple Tree
/// Tree)`, `(List Tree)`, `(Option Tree)`) so a self-reference at any depth is rewritten.
fn copy_payload_type(ast: &mut Arenas, node: StructId, decl: &TypeDecl) -> StructId {
    if decl.params.is_empty() {
        // Monomorphic — a bare self-reference is already the whole type; nothing to apply.
        return copy_subtree(ast, node);
    }
    match ast.get(node).clone() {
        // A bare atom naming THIS sum — rewrite `Tree` → `(Tree a b…)` (the decl's params), the same
        // form `sum_applied` builds for the result type. Any other name is copied verbatim.
        Struct::Atom(lid) => {
            if let Leaf::Name(n) = ast.leaf(lid).clone()
                && n.as_ref() == decl.name.as_str()
            {
                return sum_applied(ast, decl);
            }
            copy_subtree(ast, node)
        }
        // A compound type expr — descend so a self-reference nested in `(Tuple …)`/`(List …)`/`(Option
        // …)` is rewritten too. CRUCIAL: if this list is ALREADY an application of the sum to arguments
        // (`(Tree a)` — head atom == the sum name), the head must be copied VERBATIM, not re-applied:
        // rewriting it would produce `((Tree a) a)`. So a self-named head is left as the bare application
        // head; only the ARGUMENTS are descended (they may hold deeper self-refs). Any other head
        // (`Tuple`/`List`/`Option`/`->`) is an ordinary type-ctor whose children all get the rewrite.
        Struct::List(children) => {
            let head_is_self = children
                .first()
                .and_then(|&c| ast.as_name(c).map(|n| n == decl.name))
                .unwrap_or(false);
            let copied: Vec<StructId> = children
                .iter()
                .enumerate()
                .map(|(i, &c)| {
                    if head_is_self && i == 0 {
                        copy_subtree(ast, c) // the application head — verbatim, not re-applied
                    } else {
                        copy_payload_type(ast, c, decl)
                    }
                })
                .collect();
            push_list(ast, copied)
        }
    }
}

/// The sum the constructor produces, as a type expression: for a GENERIC sum, the sum NAME applied to
/// its params — `(Option a)` — an ordinary type-constructor application that reduces to `Ty::Sum{decl,
/// args:[a]}` (the `NAME` atom re-resolves to the synthesized sum record, whose `(meta apply)` is the
/// `sum-ctor` intrinsic). For a MONOMORPHIC sum, the bare `(typeval (Sum NAME <decl>))`.
fn sum_applied(ast: &mut Arenas, decl: &TypeDecl) -> StructId {
    if decl.params.is_empty() {
        return sum_typeval(ast, decl);
    }
    // `(NAME a b…)` — the sum name applied to its params; `NAME` re-resolves to the sum record.
    let name = push_atom(ast, Leaf::Name(decl.name.clone().into()));
    let mut items = vec![name];
    for p in &decl.params {
        items.push(push_atom(ast, Leaf::Name(p.clone().into())));
    }
    push_list(ast, items)
}

/// The sum's type-value as an arena node: `(typeval (Sum NAME <decl>))`. The dual of
/// `resolve::decode_ty`'s `Sum` arm and `eval::encode_ty`'s `Sum` arm — the declaration occurrence is
/// the identity (an integer literal in the wire form), the name is for rendering.
fn sum_typeval(ast: &mut Arenas, decl: &TypeDecl) -> StructId {
    let tv_head = push_atom(ast, Leaf::Name("typeval".into()));
    let sum_head = push_atom(ast, Leaf::Name("Sum".into()));
    let nm = push_atom(ast, Leaf::Name(decl.name.clone().into()));
    let d = push_atom(
        ast,
        Leaf::Int {
            value: IntValue::from_i64(decl.occ.0 as i64),
            radix: Radix::Dec,
        },
    );
    let sum = push_list(ast, vec![sum_head, nm, d]);
    push_list(ast, vec![tv_head, sum])
}

/// Structurally COPY the subtree rooted at `node`, returning the copy's root. A NAME atom is copied
/// fresh (so it re-resolves against the copy's scope, not the original's — the same reason
/// `eval::beta_reduce` copies name atoms); a CONSTANT atom (int/bool/…) is self-contained and SHARED;
/// a list is copied with its children copied. This lets a synthesized constructor type reference a
/// user-written payload type without the shared occurrence acquiring a second parent (which would break
/// the single-parent invariant the scope walk relies on).
fn copy_subtree(ast: &mut Arenas, node: StructId) -> StructId {
    match ast.get(node).clone() {
        Struct::Atom(lid) => match ast.leaf(lid).clone() {
            Leaf::Name(_) => {
                let leaf = ast.leaf(lid).clone();
                push_atom(ast, leaf)
            }
            // A constant leaf resolves to its own value regardless of scope — share it.
            _ => node,
        },
        Struct::List(children) => {
            let copied: Vec<StructId> = children.iter().map(|&c| copy_subtree(ast, c)).collect();
            push_list(ast, copied)
        }
    }
}
