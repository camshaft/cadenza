//! The `@param` sidecar — annotation-driven runtime-parameter codegen (DESIGN-runtime-parameter-host-
//! effect.md). The operator's model: a function/value marked `@param(widget: …, …) name : Type` is a
//! RUNTIME INPUT the host supplies; a build-time SIDECAR scans every `@param` site, GENERATES a single
//! strongly-typed effect `Param` with one accessor op per param (`Param.width : Length`, …), and the host
//! binds each accessor at run time. This file owns the SCAN + GENERATE half (v-metaprogramming); v-effects
//! owns the generated effect's runtime MECHANISM (each `(-> Unit R)` op lowers to a `HostCall` the host
//! binds by name); v-syntax owns the `@param` annotation PARSE.
//!
//! CANONICAL SHAPE (v-syntax, confirmed on trunk): `@param(widget: slider, …) width : Type` parses to
//!   `(: (@ (param (: widget slider) …) width) Type)`
//! — the OUTER colon carries the explicit type, its inner is the `@`-annotation over the param NAME, and
//! the `(param …)` application's tail is the config kv pairs (each a `(: key value)`).
//!
//! B-INVARIANT (concierge + v-effects): `@param` MUST carry an explicit type. An untyped `@param(…) name`
//! parses to a bare `(@ (param …) name)` with NO wrapping colon — nothing to generate an accessor from —
//! and is rejected here rather than silently dropped (the generated op's result type IS the annotation
//! type, so an un-typed param has no accessor type).
//!
//! ORDERING (my analysis, accepted): the guest USES a param via `(Param.width)`, which is unbound until
//! this pass generates `Param`. So the generate runs BEFORE resolve, reading each param's EXPLICIT
//! annotation type (not a resolved program), and splices the generated `(effect Param …)` into the module
//! so the ordinary compile sees it exactly like a hand-written effect. This is the FIRST BRICK: a single
//! scalar param generates one `(op name (-> Unit Type))`; the widget MANIFEST + Quantity ABI are later.

use crate::ast::{Arenas, Leaf, Struct, StructId};
use crate::prelude::{push_atom, push_list};

/// A scanned `@param` site: the accessor NAME and its declared TYPE node (the outer colon's type child,
/// spliced verbatim into the generated `(-> Unit <Type>)`). The widget/range metadata is read but not yet
/// emitted (the manifest is a later brick); this brick generates only the typed effect interface.
struct ParamSite {
    /// The parameter's name occurrence (an `Atom(Name)`), reused as the generated op's member name.
    name: StructId,
    /// The declared type occurrence (the outer colon's second child), reused as the op result type.
    ty: StructId,
}

/// A malformed `@param` — an `@param(…)` annotation with NO wrapping `(: … Type)` (missing the required
/// explicit type). Carries the annotation node for `collect_faults` to reject (require-explicit-type).
/// (Returned so the load path can surface it; the first brick records it for a diagnostic.)
#[derive(Default)]
pub struct ParamFaults {
    /// `@param` annotation occurrences that lack an explicit type wrapper (B-invariant violations).
    pub untyped: Vec<StructId>,
}

/// Scan every `@param` site and GENERATE the `Param` effect, appending it as a module member. Returns the
/// faults (untyped `@param` sites) for the load path to reject. The scan matches the confirmed shape
/// `(: (@ (param <kv>) name) Type)`; an `@param` NOT wrapped by an outer colon is an untyped fault.
pub fn generate(ast: &mut Arenas) -> ParamFaults {
    // Only ORIGINAL nodes can be a source `@param` site; the generate APPENDS nodes, so bound the scan.
    let original_len = ast.structure.len() as u32;
    let mut sites: Vec<ParamSite> = Vec::new();
    let mut faults = ParamFaults::default();

    for i in 0..original_len {
        let id = StructId(i);
        // A well-formed site is a colon node `(: <inner> <Type>)` whose `<inner>` is a `@param`
        // annotation over the param name. Read the colon's two children.
        if let Some(&[inner, ty]) = ast.as_form(id, ":")
            && let Some(name) = param_annotation_name(ast, inner)
        {
            sites.push(ParamSite { name, ty });
        }
    }

    // The UNTYPED-fault scan: a `(@ (param …) name)` NOT wrapped by an outer `(: … Type)` is a
    // require-explicit-type violation (B-invariant) — the accessor would have no result type to generate.
    for i in 0..original_len {
        let id = StructId(i);
        // A bare `@param` annotation: `(@ (param …) <inner>)`. If it is NOT the inner of some typing
        // colon (i.e. not part of a matched site), it is untyped.
        if is_param_annotation(ast, id) && !any_colon_types(ast, id, original_len) {
            faults.untyped.push(id);
        }
    }

    if sites.is_empty() {
        // No `@param` sites: generate nothing (an empty project needs no `Param` effect). A later brick
        // may emit an empty effect + manifest for tooling uniformity; the first brick simply no-ops.
        return faults;
    }

    // GENERATE `(effect Param (op <name> (-> Unit <Type>)) …)` — one op per site, result-typed by the
    // annotation. The op member name reuses the param's name occurrence; the result type reuses the
    // declared type occurrence (both original nodes, so their spans + resolution carry). This is exactly
    // the shape a hand-written effect parses to, so v-effects' host-bind path consumes it unchanged.
    let effect = build_param_effect(ast, &sites);
    append_module_member(ast, effect);
    faults
}

// (No `Copied2` helper — the scan reads the colon tail via a direct `&[inner, ty]` slice match.)

/// If `inner` is a `@param` annotation `(@ (param <kv>…) <name>)`, return the param NAME occurrence (the
/// annotation's second child). `None` if it is not a `@param` annotation or has no name.
fn param_annotation_name(ast: &Arenas, inner: StructId) -> Option<StructId> {
    let tail = ast.as_form(inner, "@")?;
    let (&app, &name) = (tail.first()?, tail.get(1)?);
    // The annotation's name position must be the application `(param …)`.
    ast.as_form(app, "param")?;
    Some(name)
}

/// Whether `id` is a `@param` annotation node `(@ (param …) _)`.
fn is_param_annotation(ast: &Arenas, id: StructId) -> bool {
    ast.as_form(id, "@")
        .and_then(|tail| tail.first().copied())
        .is_some_and(|app| ast.as_form(app, "param").is_some())
}

/// Whether some `(: <id> <Type>)` colon node (in `0..original_len`) types the annotation `ann` — i.e.
/// `ann` is the inner of a typing colon. Used to detect an UNtyped `@param` (no such colon).
fn any_colon_types(ast: &Arenas, ann: StructId, original_len: u32) -> bool {
    (0..original_len).any(|i| {
        let cid = StructId(i);
        ast.as_form(cid, ":")
            .and_then(|t| t.first().copied())
            .is_some_and(|inner| inner == ann)
    })
}

/// Build `(effect Param (op <name> (-> Unit <Type>)) …)` from the scanned sites and return its node id.
fn build_param_effect(ast: &mut Arenas, sites: &[ParamSite]) -> StructId {
    let effect_head = push_atom(ast, Leaf::Name("effect".to_string()));
    let param_name = push_atom(ast, Leaf::Name("Param".to_string()));
    let mut children = vec![effect_head, param_name];
    for site in sites {
        // `(op <name> (-> Unit <Type>))` — nullary host-delegated accessor, result-typed by the param.
        let op_head = push_atom(ast, Leaf::Name("op".to_string()));
        let arrow_head = push_atom(ast, Leaf::Name("->".to_string()));
        let unit_ty = push_atom(ast, Leaf::Name("Unit".to_string()));
        let arrow = push_list(ast, vec![arrow_head, unit_ty, site.ty]);
        let op = push_list(ast, vec![op_head, site.name, arrow]);
        children.push(op);
    }
    push_list(ast, children)
}

/// Append `member` as a top-level member of the `(module NAME …)` root (or a `(do …)` root). The module's
/// structure is `(module NAME form1 …)`; append `member` after the existing members so the generated
/// effect is visible to the ordinary compile exactly like a hand-written declaration.
fn append_module_member(ast: &mut Arenas, member: StructId) {
    let root = ast.root;
    if let Struct::List(items) = ast.get(root) {
        let mut new_items = items.clone();
        // Insert the effect right after the module NAME (position 1) so it precedes uses of `Param` —
        // ordering among top-level members does not matter for resolution (module members bind mutually),
        // but keeping it early is tidy. Fall back to push if the module is malformed (no name slot).
        let insert_at = if ast.as_form(root, "module").is_some() && new_items.len() >= 2 {
            2
        } else {
            new_items.len()
        };
        new_items.insert(insert_at, member);
        ast.structure[root.0 as usize] = Struct::List(new_items);
    }
}
