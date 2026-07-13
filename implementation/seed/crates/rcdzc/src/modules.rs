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
//# Each definition in a module MUST register its name and value as a field of the module's record.
//!
//= spec/capabilities/core-semantics.md#a-module-evaluates-to-a-record-of-its-exports
//# A module's exported definition MUST be reachable by member access on the module's record.
//!
//! A field's VALUE mirrors [`crate::resolve::do_def_binds`] (the do-local `def` binder): a VALUE
//! declaration `(def x V)` or a nullary `(def (x) V)` binds its body `V`; a FUNCTION declaration `(def
//! (f p…) BODY)` binds the lambda `(fn (p…) BODY)` (a fresh arena node, applied by the ordinary path). So
//! `(. m x)` projects the value and `((. m f) a)` applies the lambda — the same shapes the do-local `def`
//! scope already realizes, now grouped under a record.

use crate::ast::{Arenas, Leaf, Struct, StructId};
use crate::db::ModuleDecl;
use crate::fxhash::FxHashMap;
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
    // The record PRIMITIVE head is the STRING `"record"` (the bare NAME `record` is a shadowable prelude
    // alias); a compiler-synthesized record uses the string head so it resolves structurally to
    // `Resolved::Record` independent of any user binding of `record`, exactly as `sums`/`effects` do.
    let head = push_atom(ast, Leaf::Str("record".to_string()));
    let mut children = vec![head];
    // `(module NAME def…)` — the members are everything after NAME (index 0 of the tail).
    let members: Vec<StructId> = ast
        .as_form(module_form, "module")
        .and_then(|tail| tail.get(1..))
        .map(<[StructId]>::to_vec)
        .unwrap_or_default();
    for member in members {
        // A NESTED module member — a field `(inner <inner-record>)`. The inner record is built first (see
        // `synthesize`), so `synth_by_occ` carries it; an inner that FAILED to register (an unmodeled
        // member) has no entry, so it contributes no field and `(. outer inner)` is a closed-record
        // CDZ0201 rather than a miscompile.
        if let Some(inner_name) = module_member_name(ast, member)
            && let Some(&inner_rec) = synth_by_occ.get(&member)
        {
            let k = push_atom(ast, Leaf::Name(inner_name));
            children.push(push_list(ast, vec![k, inner_rec]));
        } else if let Some(field) = def_field(ast, member) {
            children.push(field);
        }
    }
    push_list(ast, children)
}

/// The NAME of a `(module NAME …)` member, if `member` is a module form with a bare-name head — the field
/// key an enclosing module registers for a nested module. `None` for a non-module member (a `(def …)` /
/// `(type …)` / …), so the caller falls through to the ordinary `def_field`.
fn module_member_name(ast: &Arenas, member: StructId) -> Option<String> {
    let tail = ast.as_form(member, "module")?;
    ast.as_name(*tail.first()?).map(str::to_string)
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
        let k = push_atom(ast, Leaf::Name(name));
        return Some(push_list(ast, vec![k, body]));
    }
    // List signature `(NAME param…)` — clone the children out before mutating the arena.
    let Struct::List(children) = ast.get(sig) else {
        return None;
    };
    let children = children.clone();
    let name = children.first().and_then(|&c| ast.as_name(c))?.to_string();
    let params: Vec<StructId> = children[1..].to_vec();
    let k = push_atom(ast, Leaf::Name(name));
    let fn_head = push_atom(ast, Leaf::Name("fn".to_string()));
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
        let unit_param = push_atom(ast, Leaf::Name("_$u".to_string()));
        let colon = push_atom(ast, Leaf::Name(":".to_string()));
        let unit_ty = push_atom(ast, Leaf::Name("Unit".to_string()));
        let annotated = push_list(ast, vec![colon, unit_param, unit_ty]);
        let params_list = push_list(ast, vec![annotated]);
        let lambda = push_list(ast, vec![fn_head, params_list, body]);
        return Some(push_list(ast, vec![k, lambda]));
    }
    // Function `(def (f p…) BODY)` — field value is the lambda `(fn (p…) BODY)`. The params are the RAW
    // signature occurrences (bare `a` or annotated `(: a T)`), exactly the shape a `Resolved::Lambda` / a
    // top-level def's params carry, so the ordinary application path β-reduces it.
    let params_list = push_list(ast, params);
    let lambda = push_list(ast, vec![fn_head, params_list, body]);
    Some(push_list(ast, vec![k, lambda]))
}
