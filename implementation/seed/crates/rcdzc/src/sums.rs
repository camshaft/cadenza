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
        let record = sum_record(ast, decl);
        decl.synth = Some(record);
    }
}

/// Build one sum's record: `(record ((meta t) <sum-typeval>) (<variant> <ctor>)…)`. The `(meta t)` is
/// the sum's own type-value; each variant is a field to its constructor record. A GENERIC sum (with
/// type params) ALSO gets `(meta apply)` = the `sum-ctor` intrinsic + `(meta sum-decl)` = its
/// declaration occurrence, so `(Option Int64)` in type position APPLIES the sum constructor to build
/// `Ty::Sum { decl, args: [Int64] }` — the same "a type ctor's `(meta apply)` builds a type" model as
/// `Int`/`Tuple`. A monomorphic sum needs no `(meta apply)` (it is never applied in type position).
fn sum_record(ast: &mut Arenas, decl: &TypeDecl) -> StructId {
    let head = push_atom(ast, Leaf::Name("record".to_string()));
    let mut children = vec![head];

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
        let k = push_atom(ast, Leaf::Name(variant.name.clone()));
        children.push(push_list(ast, vec![k, ctor]));
    }
    push_list(ast, children)
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
    let head = push_atom(ast, Leaf::Name("record".to_string()));
    let ctor_ty = ctor_type(ast, decl, variant);
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

/// The constructor's type expression. A NULLARY variant (no payloads) constructs the sum directly —
/// its type is the sum type-value. A PAYLOAD variant `(Some Int64)` has type `(-> Int64 Sum)`; multiple
/// payloads curry (`(Cons a b) : (-> a (-> b Sum))`). Each payload TYPE occurrence is COPIED fresh (a
/// structural copy, so it re-resolves in the synthesized scope and does not steal the user occurrence's
/// single parent — the single-parent invariant the scope walk relies on), and the arrow ends in the sum
/// type-value.
fn ctor_type(ast: &mut Arenas, decl: &TypeDecl, variant: &Variant) -> StructId {
    // Innermost: the sum type-value the constructor produces.
    let mut ty = sum_typeval(ast, decl);
    // Wrap right-to-left in `(-> payload …)` so the arrow curries in declaration order.
    for &payload in variant.payloads.iter().rev() {
        let arrow = push_atom(ast, Leaf::Name("->".to_string()));
        let p = copy_subtree(ast, payload);
        ty = push_list(ast, vec![arrow, p, ty]);
    }
    ty
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
