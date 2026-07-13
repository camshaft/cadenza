//! Nested-module synthesis — the program-driven twin of `prelude::install` and `sums`/`effects`, for a
//! `(module NAME def…)` declaration reachable in a `do`-block (`core-semantics.md` §A Module Groups
//! Definitions Under A Name). It appends a RECORD to the arena whose fields are the module's exported
//! definitions, and records the record on each [`ModuleDecl::synth`] — so the module NAME resolves to a
//! `Ref` to that record and `(. NAME field)` is ORDINARY member access, exactly as a sum's variants or an
//! effect's operations are reached. Nothing about a module is privileged by name.
//!
//! A field's VALUE mirrors [`crate::resolve::do_def_binds`] (the do-local `def` binder): a VALUE
//! declaration `(def x V)` or a nullary `(def (x) V)` binds its body `V`; a FUNCTION declaration `(def
//! (f p…) BODY)` binds the lambda `(fn (p…) BODY)` (a fresh arena node, applied by the ordinary path). So
//! `(. m x)` projects the value and `((. m f) a)` applies the lambda — the same shapes the do-local `def`
//! scope already realizes, now grouped under a record.

use crate::ast::{Arenas, Leaf, Struct, StructId};
use crate::db::ModuleDecl;
use crate::prelude::{push_atom, push_list};

/// Synthesize each module declaration's record, recording it on `decl.synth`. Runs during `Db::load`
/// AFTER the scan (it reads the declarations) and BEFORE the parent index (which must index the
/// synthesized nodes so a name inside a module member resolves by the ordinary scope walk).
pub fn synthesize(ast: &mut Arenas, decls: &mut [ModuleDecl]) {
    for decl in decls.iter_mut() {
        decl.synth = Some(module_record(ast, decl.occ));
    }
}

/// Build one module's record `(record (name <field-value>)…)` from its `(module NAME def…)` declaration
/// occurrence. Each `(def …)` member becomes a `(field-name <value>)` field: a value/nullary def's body,
/// or a function def's synthesized `(fn (params) body)` lambda. A non-`def` member (a nested `(type …)`,
/// `(module …)`, a `(delegate …)`) is skipped here — this increment realizes the value/function export
/// surface; the metadata/capability channels are later increments (their cases decline meanwhile).
fn module_record(ast: &mut Arenas, module_form: StructId) -> StructId {
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
        if let Some(field) = def_field(ast, member) {
            children.push(field);
        }
    }
    push_list(ast, children)
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
        // Nullary FUNCTION `(def (answer) V)` — a `() → T` export INVOKED by applying it to the unit value
        // `((. m answer) unit)` (core-semantics.md §A Nullary Function's Argument Type Is Unit; the module
        // cases write `((. m answer) unit)`). Distinct from a bare-name VALUE `(def v V)` (handled above,
        // whose field IS the value, projected `(. m v)` with no application). Field value is the lambda
        // `(fn (_$u) V)` over ONE ignored unit param — a fresh `_`-prefixed name that never collides with
        // a user binder (unused-binding-suppressed) — so `((. m answer) unit)` β-reduces to `V` by the
        // ordinary application path, and the body (which references no param) is unchanged.
        let unit_param = push_atom(ast, Leaf::Name("_$u".to_string()));
        let params_list = push_list(ast, vec![unit_param]);
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
