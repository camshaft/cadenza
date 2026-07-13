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

use crate::ast::{Arenas, IntValue, Leaf, Radix, Struct, StructId};
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
    // `(type Ordering Less Equal Greater)` — the result of the three-way `compare` (core-semantics.md §A
    // Total Order Is Observed Through A Three-Way Comparison). A monomorphic closed prelude sum like
    // `Sign`; the DISCRIMINANT ORDER is Less=0, Equal=1, Greater=2, which `compare` maps a `<`/`=`/`>`
    // ordering to. `compare` itself is a prelude operator (see `prelude::install`).
    let ordering = type_form(
        ast,
        "Ordering",
        &[("Less", &[]), ("Equal", &[]), ("Greater", &[])],
    );
    [option, result, sign, ordering]
        .into_iter()
        .filter_map(|item| crate::db::scan_type_decl(ast, item))
        .collect()
}

/// The occurrence of the variant-constructor field named `vname` inside a synthesized sum `record` —
/// so the prelude map can bind a BARE variant name (`Some`) to its constructor. The record is `(record
/// ((meta t) …) [(meta apply) …] [(meta sum-decl) …] (Some <ctor>) (None <ctor>)…)`; a variant field is
/// a 2-element `(name <ctor>)` list whose name matches `vname`. `None` if not found (e.g. a meta field).
pub fn variant_ctor_field(ast: &Arenas, record: StructId, vname: &str) -> Option<StructId> {
    let Struct::List(children) = ast.get(record) else {
        return None;
    };
    for &field in children.iter().skip(1) {
        if let Struct::List(pair) = ast.get(field)
            && pair.len() == 2
            && ast.as_name(pair[0]) == Some(vname)
        {
            return Some(pair[1]);
        }
    }
    None
}

/// Build a `(type NAME (V pay…)…)` declaration form in the arena and return its occurrence. Each
/// variant is `(vname payload-name…)` (a payload is a bare type-parameter name — lowercase, so the scan
/// reads it as an implicit generic). A nullary variant (`&[]` payloads) is a bare `vname`.
fn type_form(ast: &mut Arenas, name: &str, variants: &[(&str, &[&str])]) -> StructId {
    let head = push_atom(ast, Leaf::Name("type".to_string()));
    let name_occ = push_atom(ast, Leaf::Name(name.to_string()));
    let mut children = vec![head, name_occ];
    for (vname, payloads) in variants {
        if payloads.is_empty() {
            // Nullary variant — a bare name.
            children.push(push_atom(ast, Leaf::Name(vname.to_string())));
        } else {
            // `(vname payload…)`.
            let mut vlist = vec![push_atom(ast, Leaf::Name(vname.to_string()))];
            for p in *payloads {
                vlist.push(push_atom(ast, Leaf::Name(p.to_string())));
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
    let head = push_atom(ast, Leaf::Str("record".to_string()));
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
            let ih = push_atom(ast, Leaf::Name("intrinsic".to_string()));
            let who = push_atom(ast, Leaf::Name("sum-ctor".to_string()));
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
        let k = push_atom(ast, Leaf::Name(variant.name.clone()));
        children.push(push_list(ast, vec![k, ctor]));
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
        let ek = push_atom(ast, Leaf::Name("expect".to_string()));
        children.push(push_list(ast, vec![ek, expect_op]));
    }

    (push_list(ast, children), ctors)
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
        let ih = push_atom(ast, Leaf::Name("intrinsic".to_string()));
        let who = push_atom(ast, Leaf::Name("String".to_string()));
        push_list(ast, vec![ih, who])
    };
    let arrow2 = push_atom(ast, Leaf::Name("->".to_string()));
    let inner = push_list(ast, vec![arrow2, string, ret]); // (-> String <payload0>)
    let sum = sum_applied(ast, decl); // (Sum params) or the bare typeval
    let arrow1 = push_atom(ast, Leaf::Name("->".to_string()));
    let body = push_list(ast, vec![arrow1, sum, inner]); // (-> (Sum params) (-> String payload0))
    if decl.params.is_empty() {
        return body;
    }
    let fn_head = push_atom(ast, Leaf::Name("fn".to_string()));
    let param_atoms: Vec<StructId> = decl
        .params
        .iter()
        .map(|p| push_atom(ast, Leaf::Name(p.clone())))
        .collect();
    let params_list = push_list(ast, param_atoms);
    push_list(ast, vec![fn_head, params_list, body])
}

/// The `expect` operation record: `(record ((meta t) TYPE-SCHEME) ((meta apply) (intrinsic sum-expect)))`
/// — the same operator-record shape as a variant constructor, but its `(meta apply)` is the `sum-expect`
/// intrinsic (`Prim::SumExpect`). Projecting `(. Option expect)` gives this record; applying it dispatches
/// through the ordinary `(meta apply)` path to the unwrap-or-trap lowering.
fn expect_op_record(ast: &mut Arenas, type_scheme: StructId) -> StructId {
    let head = push_atom(ast, Leaf::Str("record".to_string()));
    let t_field = meta_field(ast, "t", type_scheme);
    let builder = {
        let ih = push_atom(ast, Leaf::Name("intrinsic".to_string()));
        let who = push_atom(ast, Leaf::Name("sum-expect".to_string()));
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
    // The record PRIMITIVE head is the STRING `"record"` (the NAME `record` is a shadowable alias).
    let head = push_atom(ast, Leaf::Str("record".to_string()));
    let ctor_ty = ctor_type_scheme(ast, decl, variant);
    let t_field = meta_field(ast, "t", ctor_ty);
    // `(meta apply)` = the shared sum-new intrinsic.
    let builder = {
        let ih = push_atom(ast, Leaf::Name("intrinsic".to_string()));
        let who = push_atom(ast, Leaf::Name("sum-new".to_string()));
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
    let fn_head = push_atom(ast, Leaf::Name("fn".to_string()));
    let param_atoms: Vec<StructId> = decl
        .params
        .iter()
        .map(|p| push_atom(ast, Leaf::Name(p.clone())))
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
        let arrow = push_atom(ast, Leaf::Name("->".to_string()));
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
                && n == decl.name
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
    let name = push_atom(ast, Leaf::Name(decl.name.clone()));
    let mut items = vec![name];
    for p in &decl.params {
        items.push(push_atom(ast, Leaf::Name(p.clone())));
    }
    push_list(ast, items)
}

/// The sum's type-value as an arena node: `(typeval (Sum NAME <decl>))`. The dual of
/// `resolve::decode_ty`'s `Sum` arm and `eval::encode_ty`'s `Sum` arm — the declaration occurrence is
/// the identity (an integer literal in the wire form), the name is for rendering.
fn sum_typeval(ast: &mut Arenas, decl: &TypeDecl) -> StructId {
    let tv_head = push_atom(ast, Leaf::Name("typeval".to_string()));
    let sum_head = push_atom(ast, Leaf::Name("Sum".to_string()));
    let nm = push_atom(ast, Leaf::Name(decl.name.clone()));
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
