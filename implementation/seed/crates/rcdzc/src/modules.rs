//! Nested-module synthesis — the program-driven twin of `prelude::install` and `sums`/`effects`, for a
//! `(module NAME def…)` declaration reachable in a `do`-block (`core-semantics.md` §A Module Groups
//! Definitions Under A Name). It appends a RECORD to the arena whose fields are the module's exported
//! definitions, and records the record on each [`ModuleDecl::synth`] — so the module NAME resolves to a
//! `Ref` to that record and `(. NAME field)` is ORDINARY member access, exactly as a sum's variants or an
//! effect's operations are reached. Nothing about a module is privileged by name.
//!
//= spec/capabilities/core-semantics.md#a-module-evaluates-to-a-record-of-its-exports
//# Evaluating a module MUST produce a record whose fields are the names its definitions export bound to their values.
//!
//= spec/capabilities/core-semantics.md#a-module-evaluates-to-a-record-of-its-exports
//# Each definition a module exports MUST register its name and value as a field of the module's record.
//!
//= spec/capabilities/core-semantics.md#a-module-evaluates-to-a-record-of-its-exports
//# A definition a module does not export MUST NOT register a field of the module's record, so that the record's fields are exactly the module's visible surface (modules-and-namespaces.md §Visibility Is Explicit).
//!
//= spec/capabilities/core-semantics.md#a-module-evaluates-to-a-record-of-its-exports
//# A module's exported definition MUST be reachable by member access on the module's record.
//!
//! A field's VALUE mirrors [`crate::resolve::do_def_binds`] (the do-local `def` binder): a VALUE
//! declaration `(def x V)` or a nullary `(def (x) V)` binds its body `V`; a FUNCTION declaration `(def
//! (f p…) BODY)` binds the lambda `(fn (p…) BODY)` (a fresh arena node, applied by the ordinary path). So
//! `(. m x)` projects the value and `((. m f) a)` applies the lambda — the same shapes the do-local `def`
//! scope already realizes, now grouped under a record.

use crate::ast::{Arenas, CompoundCtor, Leaf, Struct, StructId};
use crate::db::{Def, ModuleDecl};
use crate::fxhash::{FxHashMap, FxHashSet};
use crate::prelude::{push_atom, push_list};

/// Synthesize each module declaration's record, recording it on `decl.synth`. Runs during `Db::load`
/// AFTER the scan (it reads the declarations) and BEFORE the parent index (which must index the
/// synthesized nodes so a name inside a module member resolves by the ordinary scope walk).
pub fn synthesize(ast: &mut Arenas, decls: &mut [ModuleDecl]) {
    // Records are built INNER-FIRST: a MODULE-IN-MODULE member embeds the inner module's already-built
    // record, so the inner must exist when the outer is built. `db::collect_module_decl` registers an
    // outer module BEFORE its inner members, so iterating `decls` in REVERSE is inner-before-outer. Each
    // built record is recorded in `synth_by_occ` (keyed by declaration occurrence) for an enclosing module
    // to look up when it embeds a nested module member.
    let mut synth_by_occ: FxHashMap<StructId, StructId> = FxHashMap::default();
    for decl in decls.iter_mut().rev() {
        let rec = module_record(ast, decl.occ, &synth_by_occ);
        decl.synth = Some(rec);
        synth_by_occ.insert(decl.occ, rec);
    }
}

/// Build one module's record `(record (name <field-value>)…)` from its `(module NAME member…)`
/// declaration occurrence. Each `(def …)` member becomes a `(field-name <value>)` field: a value/nullary
/// def's body, or a function def's synthesized `(fn (params) body)` lambda. A NESTED `(module inner …)`
/// member becomes an `(inner <inner-record>)` field whose value is the inner module's already-built
/// record (looked up in `synth_by_occ`) — nesting the record, so a member-access chain projects it. A
/// non-def / non-module member (a nested `(type …)`, `(effect …)`, `(doc …)`) is skipped — a legitimate
/// non-export (correctly absent from the record; projecting it is the closed-record CDZ0201).
fn module_record(
    ast: &mut Arenas,
    module_form: StructId,
    synth_by_occ: &FxHashMap<StructId, StructId>,
) -> StructId {
    // The record head is the NATIVE ctor-LEAF (`Leaf::Ctor(Record)`, M2/M3 — recognized by kind, not head
    // text), so a compiler-synthesized record resolves structurally to `Resolved::Record` independent of any
    // user binding of the shadowable prelude alias `record`. The ctor-leaf is unshadowable, recognized by
    // kind via `compound_ctor_leaf`; the M3 reader-flip removed the legacy `"record"` STRING head.
    let head = push_atom(ast, Leaf::Ctor(CompoundCtor::Record));
    let mut children = vec![head];
    // `(module NAME def…)` — the members are everything after NAME (index 0 of the tail).
    let members: Vec<StructId> = ast
        .as_form(module_form, "module")
        .and_then(|tail| tail.get(1..))
        .map(<[StructId]>::to_vec)
        .unwrap_or_default();
    // VISIBILITY IS EXPLICIT (`modules-and-namespaces.md` §Visibility Is Explicit): when a module carries
    // an `(export a b …)` clause, its export record contains ONLY the listed names — a definition NOT
    // named is PRIVATE (absent from the record, so `(. m private)` is the closed-record CDZ0201, and a
    // cross-module import cannot reach it). The `(export …)` clause IS the explicit rule the spec demands;
    // it is what the ML surface `export { a, b }` compiles to. A module with NO export clause is the
    // export-EVERYTHING default (`export_set` is `None` → no filter) — every corpus module today relies on
    // it, and it matches the spec-body line "each definition MUST register its name as a field". A PRIVATE
    // member is still MUTUALLY VISIBLE to its siblings inside the module (`resolve::module_sibling_binds`
    // scans the members, not this record), so a private helper stays internally callable — only its
    // OUTWARD reachability through the record is withheld.
    //= spec/capabilities/modules-and-namespaces.md#visibility-is-explicit
    //# A definition that is not made visible MUST NOT be importable by another module.
    let export_set = module_export_set(ast, &members);
    let visible = |field_name: &str| {
        export_set
            .as_ref()
            .is_none_or(|set| set.contains(field_name))
    };
    for &member in &members {
        // A NESTED module member — a field `(inner <inner-record>)`. The inner record is built first (see
        // `synthesize`), so `synth_by_occ` carries it; an inner that FAILED to register (an unmodeled
        // member) has no entry, so it contributes no field and `(. outer inner)` is a closed-record
        // CDZ0201 rather than a miscompile. A nested module named by no `(export …)` clause is private,
        // exactly as a private def.
        if let Some(inner_name) = module_member_name(ast, member)
            && let Some(&inner_rec) = synth_by_occ.get(&member)
        {
            if !visible(&inner_name) {
                continue;
            }
            let k = push_atom(ast, Leaf::Name(inner_name.into()));
            children.push({
                let eq = push_atom(ast, Leaf::Name("=".into()));
                push_list(ast, vec![eq, k, inner_rec])
            });
        } else if let Some(name) = def_member_name(ast, member) {
            // A `(def …)` member — include its field ONLY if visible (exported, or no clause). `def_field`
            // is called after the visibility check so a private member appends nothing.
            if !visible(&name) {
                continue;
            }
            if let Some(field) = def_field(ast, member) {
                children.push(field);
            }
        }
    }
    // The module's MANIFEST as a `(meta capabilities)` metadata field — the union of the effects its
    // members DELEGATE to the host via `(host (E…) …)` (`capabilities-and-effects.md` §The Program Manifest
    // Is The Union Of Its Entrypoints' Delegations; the delegation, not the declaration, is the grant). The
    // capabilities live in the `meta` namespace, DISTINCT from the export namespace, so they never collide
    // with an export and are reached by `(. m (meta capabilities))` — never as a plain field (a declared
    // effect `log` is not itself an export; projecting `(. m log)` is the closed-record CDZ0201). Only
    // added when non-empty: an empty `(list)` has no determined element type, and a module that delegates
    // nothing carries no capability metadata to observe. Built with the NATIVE `Leaf::Ctor(List)` head (like
    // the record above) so it resolves structurally, independent of any user binding of the `list` alias.
    //= spec/capabilities/core-semantics.md#a-module-carries-its-manifest-and-entry-as-metadata
    //# A module MUST carry the capabilities it declares as metadata separate from its exported fields, so that a declared capability is not itself an export.
    //= spec/capabilities/core-semantics.md#a-module-carries-its-manifest-and-entry-as-metadata
    //# A module's metadata MUST be reachable by a metadata key distinct from every export name, so that metadata access cannot collide with an export.
    let caps = module_capabilities(ast, &members);
    if !caps.is_empty() {
        let list_head = push_atom(ast, Leaf::Ctor(CompoundCtor::List));
        let mut list_children = vec![list_head];
        for name in caps {
            list_children.push(push_atom(ast, Leaf::Str(name.into())));
        }
        let list_val = push_list(ast, list_children);
        // The key `(meta capabilities)` — a `meta`-namespaced symbol, read by `resolve::read_key`.
        let meta_head = push_atom(ast, Leaf::Name("meta".into()));
        let caps_name = push_atom(ast, Leaf::Name("capabilities".into()));
        let meta_key = push_list(ast, vec![meta_head, caps_name]);
        children.push({
            let eq = push_atom(ast, Leaf::Name("=".into()));
            push_list(ast, vec![eq, meta_key, list_val])
        });
    }
    push_list(ast, children)
}

/// The module's MANIFEST — the ordered, deduplicated list of effect NAMES its members delegate to the host
/// via `(host (E…) …)`. A purely SYNTACTIC scan of the members' def bodies (synthesis runs pre-resolution):
/// each `(host (<name>…) <body>)` contributes its bare effect-list names, and the walk descends every
/// child so a nested/guarded delegation is still found. Order is first-seen; duplicates (the same effect
/// delegated in two entrypoints, or twice in one) collapse to one entry — the manifest is a set rendered
/// as a stable list.
fn module_capabilities(ast: &Arenas, members: &[StructId]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for &member in members {
        // Only a `(def …)` member has a body that can delegate — a nested module's delegations are its OWN
        // manifest, not the outer's (the outer reaches it via member access, not by absorbing its effects).
        if let Some(tail) = ast.as_form(member, "def")
            && let Some(&body) = tail.get(1)
        {
            collect_host_names(ast, body, 0, &mut out);
        }
    }
    out
}

/// Collect (append, first-seen, deduped) into `out` every bare effect NAME in a `(host (name…) body)`
/// form's effect list, walking the whole subtree at `node`. Structural + depth-bounded.
fn collect_host_names(ast: &Arenas, node: StructId, depth: u32, out: &mut Vec<String>) {
    if depth > 128 {
        return;
    }
    if let Some(tail) = ast.as_form(node, "host")
        && let Some(&effects_occ) = tail.first()
        && let Struct::List(effs) = ast.get(effects_occ)
    {
        for &e in effs {
            if let Some(name) = ast.as_name(e)
                && !out.iter().any(|n| n == name)
            {
                out.push(name.to_string());
            }
        }
    }
    if let Struct::List(children) = ast.get(node) {
        for &c in children.clone().iter() {
            collect_host_names(ast, c, depth + 1, out);
        }
    }
}

/// The NAME of a `(module NAME …)` member, if `member` is a module form with a bare-name head — the field
/// key an enclosing module registers for a nested module. `None` for a non-module member (a `(def …)` /
/// `(type …)` / …), so the caller falls through to the ordinary `def_field`.
fn module_member_name(ast: &Arenas, member: StructId) -> Option<String> {
    let tail = ast.as_form(member, "module")?;
    ast.as_name(*tail.first()?).map(str::to_string)
}

/// The FIELD NAME a `(def …)` member registers — the name `def_field` would key its field on, WITHOUT
/// building the field (no arena mutation). A bare-name value `(def x V)` names `x`; a signature `(def
/// (f p…) BODY)` (function or nullary) names `f`. `None` for a non-def / malformed member. Used to test a
/// member's visibility against the module's `(export …)` set before deciding to build its field.
fn def_member_name(ast: &Arenas, member: StructId) -> Option<String> {
    let tail = ast.as_form(member, "def")?;
    let sig = *tail.first()?;
    if let Some(name) = ast.as_name(sig) {
        return Some(name.to_string()); // bare-name value def `(def x V)`
    }
    let Struct::List(children) = ast.get(sig) else {
        return None;
    };
    children
        .first()
        .and_then(|&c| ast.as_name(c))
        .map(str::to_string)
}

/// The set of names a module's `(export a b …)` clauses make visible, or `None` if the module carries NO
/// export clause. `None` means the export-EVERYTHING default (no filtering); `Some(set)` — even an empty
/// one, `(export)` — means ONLY the listed names are visible (`modules-and-namespaces.md` §Visibility Is
/// Explicit). Unions every `(export …)` clause the module carries (a module may split its exports across
/// clauses, as the ML surface's per-name `export { a }` lines do); a duplicate export across clauses is a
/// separate well-formedness concern (the record's fixed-field-set check), not this set's job.
fn module_export_set(ast: &Arenas, members: &[StructId]) -> Option<FxHashSet<String>> {
    let mut set: Option<FxHashSet<String>> = None;
    for &member in members {
        if let Some(tail) = ast.as_form(member, "export") {
            let entry = set.get_or_insert_with(FxHashSet::default);
            for &name_occ in tail {
                if let Some(name) = ast.as_name(name_occ) {
                    entry.insert(name.to_string());
                }
            }
        }
    }
    set
}

/// A `(field-name <value>)` field for a `(def SIG BODY)` module member, or `None` for a non-def / malformed
/// member. Mirrors `resolve::do_def_binds`'s value/function split: a bare-name `(def x V)` or nullary `(def
/// (x) V)` field value is the body `V`; a `(def (f p…) BODY)` field value is a fresh `(fn (p…) BODY)`.
fn def_field(ast: &mut Arenas, member: StructId) -> Option<StructId> {
    let tail = ast.as_form(member, "def")?;
    let sig = *tail.first()?;
    let body = *tail.get(1)?;
    // Bare-name value declaration `(def x V)` — field `x` → its value `V`.
    if let Some(name) = ast.as_name(sig).map(str::to_string) {
        let k = push_atom(ast, Leaf::Name(name.into()));
        return Some({
            let eqh = push_atom(ast, Leaf::Name("=".into()));
            push_list(ast, vec![eqh, k, body])
        });
    }
    // List signature `(NAME param…)` — clone the children out before mutating the arena.
    let Struct::List(children) = ast.get(sig) else {
        return None;
    };
    let children = children.clone();
    let name = children.first().and_then(|&c| ast.as_name(c))?.to_string();
    let params: Vec<StructId> = children[1..].to_vec();
    let k = push_atom(ast, Leaf::Name(name.into()));
    let fn_head = push_atom(ast, Leaf::Name("fn".into()));
    if params.is_empty() {
        // Nullary FUNCTION `(def (answer) V)` — a `Unit → T` export INVOKED by applying it to the unit value
        // `((. m answer) unit)` (core-semantics.md §A Nullary Function's Argument Type Is Unit; the module
        // cases write `((. m answer) unit)`). Distinct from a bare-name VALUE `(def v V)` (handled above,
        // whose field IS the value, projected `(. m v)` with no application). Field value is the lambda
        // `(fn ((: _$u Unit)) V)` over ONE ignored param — a fresh `_`-prefixed name that never collides
        // with a user binder (unused-binding-suppressed) — so `((. m answer) unit)` β-reduces to `V` by the
        // ordinary application path, and the body (which references no param) is unchanged. The param is
        // ANNOTATED `Unit` (not bare): a bare param would get a FRESH TYPE VARIABLE from HM, typing the
        // export as `∀a. a → T` and silently accepting a NON-unit argument (`((. m answer) 5)` → V, an
        // accept-ill-formed type hole); annotating it `Unit` makes a non-unit argument fail CDZ0203, exactly
        // as a written `(def (f (: u Unit)) …)` does — the behavior the nullary-arg-is-Unit rule requires.
        let unit_param = push_atom(ast, Leaf::Name("_$u".into()));
        let colon = push_atom(ast, Leaf::Name(":".into()));
        let unit_ty = push_atom(ast, Leaf::Name("Unit".into()));
        let annotated = push_list(ast, vec![colon, unit_param, unit_ty]);
        let params_list = push_list(ast, vec![annotated]);
        let lambda = push_list(ast, vec![fn_head, params_list, body]);
        return Some({
            let eqh = push_atom(ast, Leaf::Name("=".into()));
            push_list(ast, vec![eqh, k, lambda])
        });
    }
    // Function `(def (f p…) BODY)` — field value is the lambda `(fn (p…) BODY)`. The params are the RAW
    // signature occurrences (bare `a` or annotated `(: a T)`), exactly the shape a `Resolved::Lambda` / a
    // top-level def's params carry, so the ordinary application path β-reduces it.
    let params_list = push_list(ast, params);
    let lambda = push_list(ast, vec![fn_head, params_list, body]);
    Some({
        let eqh = push_atom(ast, Leaf::Name("=".into()));
        push_list(ast, vec![eqh, k, lambda])
    })
}

/// Register each module member FUNCTION (a `(def (f p…) body)` with parameters) — recursively, into
/// nested modules — as an INTERNAL [`Def`] (`Def::internal`), so a RECURSIVE call to it lowers to a
/// standalone `Core::Call` instead of declining ("needs runtime specialization"). Runs during `Db::load`
/// AFTER `synthesize` (it reads the same `(module …)` declarations) and after accum/binding-params
/// (which transform the TOP-LEVEL defs; a module member compiles as an ordinary recursive `Core::Call`,
/// growing the stack exactly like a non-tail top-level recursive fn — the accumulator/loop transforms are
/// a later optimization for module members).
///
/// The internal def reuses the member's ORIGINAL signature params + body occurrences — the SAME ones the
/// synth field lambda `(fn (p…) body)` carries — so `def_by_body` maps the body back to this index (what
/// `lower::callee_def_index` needs), `def_scheme` types it from those params, and a body param reference
/// (resolved via the synth `(fn …)` scope) keys on the same occurrence. It is deliberately kept OUT of
/// `def_name_index` (its NAME resolves by lexical scope — `resolve::module_sibling_binds` — never by a
/// global name lookup; the no-keys-outside-the-prelude rule) and is not an export / unused-warning target.
///
/// A NULLARY member `(def (answer) v)` or a value member `(def v V)` is NOT registered: it has no
/// recursive-call lowering need (a nullary `()→T` folds; a value is projected), and registering a nullary
/// body as a def would make it an unused-def-warning candidate. Only a member with ≥1 parameter registers.
pub fn register_callable(ast: &Arenas, decls: &[ModuleDecl], defs: &mut Vec<Def>) {
    for decl in decls {
        let members: Vec<StructId> = ast
            .as_form(decl.occ, "module")
            .and_then(|tail| tail.get(1..))
            .map(<[StructId]>::to_vec)
            .unwrap_or_default();
        for member in members {
            register_fn_def(ast, member, defs);
        }
    }
}

/// Register every DO-LOCAL FUNCTION declaration in the program as an internal callable def — the do-block
/// analogue of [`register_callable`]. A do-local `(def (fac n) …)` is in scope in its own body (self-
/// recursion) and a sibling's (mutual recursion) via `resolve::do_local_binds`, but a recursive call still
/// needs a `db.defs` index to lower to a `Core::Call` — this gives it one, exactly as a module member gets.
/// Walks the WHOLE user arena for `(do …)` blocks (a do-block may nest anywhere — inside a def body, a
/// module member, another do), registering each block's `(def (f p…) BODY)` FUNCTION forms. A value/nullary
/// do-local def is skipped (it folds/projects, no recursive-call lowering need). Registering globally by
/// body is sound: the do-local name only RESOLVES inside its block (`do_local_binds`), so a body reached
/// only through that resolution keys `def_by_body` to this def; a reference elsewhere never names it.
pub fn register_do_local_callables(ast: &Arenas, defs: &mut Vec<Def>) {
    // A do-form def whose BODY is ALREADY a registered def is a TOP-LEVEL (name-resolvable) def, not a
    // genuine do-local — the program root `(do …)`'s defs, AND in a linked multi-file package each LIBRARY
    // FILE's root `(do …)` defs (a `(module "lib" (do (def (parse …) …)))` clause), are top-level defs the
    // package scan already registered. Re-registering one as an INTERNAL def would duplicate it under the
    // same body (a `def_by_body` clobber) and shadow the real, name-resolvable, cross-file-linked def —
    // breaking import resolution. Skip any def already present; register ONLY a genuinely do-LOCAL one.
    let existing: crate::fxhash::FxHashSet<StructId> = defs.iter().filter_map(|d| d.body).collect();
    // Scan every structure occurrence for a `(do …)` block, registering its direct `(def …)` FUNCTION
    // children. A block nested inside another is reached because the scan visits every node, not by
    // descending — so no recursion is needed, and each block's forms are its DIRECT children only (a def's
    // body is a separate `(do …)` node, visited on its own).
    for i in 0..ast.structure.len() {
        let node = StructId(i as u32);
        let Some(forms) = ast.as_form(node, "do") else {
            continue;
        };
        for &form in forms {
            // Skip a def whose body is already a top-level def (root / library-file scope) — see above.
            if let Some(body) = ast.as_form(form, "def").and_then(|t| t.get(1).copied())
                && existing.contains(&body)
            {
                continue;
            }
            register_fn_def(ast, form, defs);
        }
    }
}

/// Register a single `(def (f p…) body)` form as an internal callable def, if it is a FUNCTION with
/// parameters. A value/nullary/non-def form registers nothing. Shared by module-member registration
/// ([`register_callable`]) and do-local registration ([`register_do_local_callables`]).
fn register_fn_def(ast: &Arenas, member: StructId, defs: &mut Vec<Def>) {
    let Some(tail) = ast.as_form(member, "def") else {
        return;
    };
    let (Some(&sig), Some(&body)) = (tail.first(), tail.get(1)) else {
        return;
    };
    // A LIST signature `(NAME p…)` with ≥1 param — a bare-name value def or a nullary `(x)` is skipped.
    let Struct::List(children) = ast.get(sig) else {
        return;
    };
    if children.len() < 2 {
        return; // nullary `(x)` — no recursive-call lowering need
    }
    let Some(name) = children.first().and_then(|&c| ast.as_name(c)) else {
        return;
    };
    let params: Vec<StructId> = children[1..].to_vec();
    defs.push(Def {
        name: name.to_string(),
        sig_occ: sig,
        params,
        body: Some(body),
        internal: true,
    });
}
