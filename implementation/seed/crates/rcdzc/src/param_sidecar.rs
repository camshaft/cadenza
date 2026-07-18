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
//! B-INVARIANT (concierge + v-effects): `@param` MUST carry an explicit type — the generated op's result
//! type IS the annotation type, so an un-typed param has no accessor type. This is enforced UPSTREAM, not
//! by this pass: an untyped `@param(…) name` parses to a bare `(@ (param …) name)` (an annotation over a
//! plain name, not a def and not a `(: … Type)` binder), which `strip_annotations` already REJECTS as
//! CDZ0201 "annotation wraps no definition" before this pass runs. So an untyped `@param` never reaches a
//! generate — this scan simply matches only the well-typed `(: (@ (param …) name) Type)` shape, and the
//! untyped case is already a coded reject (verified). A dedicated `@param`-specific untyped diagnostic (a
//! clearer message than the generic wraps-no-definition) is a possible later polish, not a correctness gap.
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
    /// How this param's declared type crosses the host boundary — a plain scalar, or a heap Rational-family
    /// value that has no host boundary form and desugars to a num/den scalar pair (v-effects #13).
    kind: ParamKind,
}

/// The host-boundary shape a `@param`'s declared type demands.
enum ParamKind {
    /// A scalar (or unit) type — crosses the boundary directly as one `(op <name> (-> Unit <Type>))`.
    Scalar,
    /// A heap `Rational` — no host boundary form, so it desugars to two scalar `Int64` num/den accessors the
    /// guest recombines with `Rational.of` (#13 B1). The use `(Param.<name>)` becomes `(Rational.of
    /// (Param.<name>-num) (Param.<name>-den))`.
    Rational,
    /// A `(Qty Rational <unit>)` — a Rational-MAGNITUDE quantity (a `@param … : Length`). The magnitude
    /// crosses as the same num/den pair; the guest recombines with `Rational.of` and re-attaches the unit
    /// GUEST-SIDE via `Qty.of(…, <unit>)` — the unit is a compile-time value erased at the boundary (#13 B2).
    /// Carries the `<unit>` node from the annotation (the third element of `(Qty Rational <unit>)`).
    QtyRational { unit: StructId },
}

impl ParamSite {
    /// Whether this param desugars to a num/den scalar pair (both the bare `Rational` and the `(Qty Rational
    /// <unit>)` cases share the num/den accessor generation; they differ only in the use-site recombination).
    fn is_rational_family(&self) -> bool {
        matches!(
            self.kind,
            ParamKind::Rational | ParamKind::QtyRational { .. }
        )
    }
}

/// Classify a `@param`'s declared type node into its host-boundary [`ParamKind`]: a bare `Rational`, a `(Qty
/// Rational <unit>)` (Rational-magnitude quantity — the `Length` shape), or an ordinary scalar. Only an
/// EXACT `(Qty Rational <unit>)` form is a QtyRational; a `Qty` of a non-Rational magnitude (e.g. `(Qty Int64
/// …)`) is scalar-inner and rides the ordinary scalar path, so it is left `Scalar` here.
fn classify_kind(ast: &Arenas, ty: StructId) -> ParamKind {
    if ast.as_name(ty) == Some("Rational") {
        return ParamKind::Rational;
    }
    // `(Qty <magnitude> <unit>)` — a Rational magnitude is the only heap case that needs the num/den desugar.
    if let Some([magnitude, unit]) = ast.as_form(ty, "Qty")
        && ast.as_name(*magnitude) == Some("Rational")
    {
        return ParamKind::QtyRational { unit: *unit };
    }
    ParamKind::Scalar
}

/// Scan every well-typed `@param` site and GENERATE the `Param` effect, appending it as a module member.
/// The scan matches only the confirmed shape `(: (@ (param <kv>) name) Type)` — an untyped `@param` (a
/// bare `(@ (param …) name)` with no `(: … Type)` wrapper) is REJECTED UPSTREAM by `strip_annotations`
/// (CDZ0201 "wraps no definition") before this pass runs, so it never reaches a generate; this pass need
/// not (and does not) re-detect it. No `@param` sites → no `Param` effect (an empty program needs none).
pub fn generate(ast: &mut Arenas) {
    // Only ORIGINAL nodes can be a source `@param` site; the generate APPENDS nodes, so bound the scan.
    let original_len = ast.structure.len() as u32;
    let mut sites: Vec<ParamSite> = Vec::new();

    for i in 0..original_len {
        let id = StructId(i);
        // A well-formed site is a colon node `(: <inner> <Type>)` whose `<inner>` is a `@param`
        // annotation over the param name. Read the colon's two children.
        if let Some(&[inner, ty]) = ast.as_form(id, ":")
            && let Some(name) = param_annotation_name(ast, inner)
        {
            let kind = classify_kind(ast, ty);
            sites.push(ParamSite { name, ty, kind });
        }
    }

    if sites.is_empty() {
        // No `@param` sites: generate nothing (an empty project needs no `Param` effect). A later brick
        // may emit an empty effect + manifest for tooling uniformity; the first brick simply no-ops.
        return;
    }

    // A heap `Rational` param has no host boundary form, so REWRITE each `(Param.<name>)` use into
    // `(Rational.of (Param.<name>-num) (Param.<name>-den))` BEFORE generating the effect — the effect
    // then declares the two scalar num/den ops the rewritten uses call (v-effects #13). Do the rewrite
    // first so it only ever touches ORIGINAL use sites, not the generated effect's own op declarations.
    rewrite_rational_uses(ast, &sites);

    // GENERATE `(effect Param (op <name> (-> Unit <Type>)) …)` — one op per site, result-typed by the
    // annotation. The op member name reuses the param's name occurrence; the result type reuses the
    // declared type occurrence (both original nodes, so their spans + resolution carry). This is exactly
    // the shape a hand-written effect parses to, so v-effects' host-bind path consumes it unchanged.
    let effect = build_param_effect(ast, &sites);
    append_module_member(ast, effect);
}

/// If `inner` is a `@param` annotation `(@ (param <kv>…) <name>)`, return the param NAME occurrence (the
/// annotation's second child). `None` if it is not a `@param` annotation or has no name.
fn param_annotation_name(ast: &Arenas, inner: StructId) -> Option<StructId> {
    Some(param_annotation_parts(ast, inner)?.1)
}

/// If `inner` is a `@param` annotation `(@ (param <kv>…) <name>)`, return `(param-app-node, name-node)` —
/// the `(param …)` application (whose tail is the config kv pairs) and the annotated param NAME. `None`
/// otherwise. The `_name`-only [`param_annotation_name`] wraps this for the generate path.
fn param_annotation_parts(ast: &Arenas, inner: StructId) -> Option<(StructId, StructId)> {
    let tail = ast.as_form(inner, "@")?;
    let (&app, &name) = (tail.first()?, tail.get(1)?);
    // The annotation's name position must be the application `(param …)`.
    ast.as_form(app, "param")?;
    Some((app, name))
}

// ── The WIDGET MANIFEST scan (v-metaprogramming's half; v-cdz-tooling plumbs the Query + `cdz
// param-manifest` CLI over these records — DESIGN-runtime-parameter-host-effect.md 2nd output). ──

/// One `@param` site's MANIFEST record — what the host reads to render a control. NODE-IDS for the type +
/// config values (not rendered strings): v-cdz-tooling's query handler renders the type via the Db type
/// column (`Ty::render_name`) and the config values via its JSON builder, and maps `name_node` to
/// `file:line:col` via the span table it holds (query-engine.md: the compiler emits node IDENTITY, the
/// front-end owns spans + rendering). `widget` is a bare name atom, read directly to a `String`.
pub struct ParamRecord {
    /// The param name (the accessor member name / manifest key / host-bind key).
    pub name: String,
    /// The declared TYPE node (the outer colon's type child) — render via the type column.
    pub ty: StructId,
    /// The `(: widget <name>)` config value as a `String` (e.g. `"slider"`); `None` if no widget kv.
    pub widget: Option<String>,
    /// The `(: range [<lo> <hi>])` element nodes `(lo, hi)`; `None` if no range kv.
    pub range: Option<(StructId, StructId)>,
    /// The `(: options [<v>…])` list node; `None` if no options kv.
    pub options: Option<StructId>,
    /// The `(: default <val>)` value node; `None` if no default kv.
    pub default: Option<StructId>,
    /// The param NAME occurrence — map to `file:line:col` via the front-end span table.
    pub name_node: StructId,
}

/// Scan every well-typed `@param` site `(: (@ (param <kv>) name) Type)` into a manifest record. READ-ONLY
/// over the arena (does NOT mutate / generate — that is [`generate`]). One record per site, config kv read
/// off the `(param …)` application's tail (`(: key value)` pairs). This is the SCAN half of the widget
/// manifest; v-cdz-tooling's `Query::ParamManifest` + `cdz param-manifest` renders these to JSON.
pub fn scan_manifest(ast: &Arenas) -> Vec<ParamRecord> {
    let mut records = Vec::new();
    for i in 0..ast.structure.len() as u32 {
        let id = StructId(i);
        let Some(&[inner, ty]) = ast.as_form(id, ":") else {
            continue;
        };
        let Some((app, name_node)) = param_annotation_parts(ast, inner) else {
            continue;
        };
        let Some(name) = ast.as_name(name_node).map(str::to_string) else {
            continue;
        };
        // The config kv pairs are the `(param …)` application's tail, each a `(: key value)` node.
        records.push(ParamRecord {
            name,
            ty,
            widget: config_name(ast, app, "widget"),
            range: config_range(ast, app),
            options: config_value(ast, app, "options"),
            default: config_value(ast, app, "default"),
            name_node,
        });
    }
    records
}

/// The VALUE node of a `(: <key> <value>)` config kv in the `(param …)` application `app`'s tail, if
/// present. The tail is the arguments after the `param` head; each is a `(: key value)` colon node.
fn config_value(ast: &Arenas, app: StructId, key: &str) -> Option<StructId> {
    let tail = ast.as_form(app, "param")?;
    for &kv in tail {
        if let Some(&[k, v]) = ast.as_form(kv, ":")
            && ast.as_name(k) == Some(key)
        {
            return Some(v);
        }
    }
    None
}

/// A config kv's value as a bare NAME string (e.g. `(: widget slider)` → `"slider"`); `None` if the key
/// is absent or its value is not a bare name.
fn config_name(ast: &Arenas, app: StructId, key: &str) -> Option<String> {
    let v = config_value(ast, app, key)?;
    ast.as_name(v).map(str::to_string)
}

/// The `(lo, hi)` element nodes of a `(: range [<lo> <hi>])` config kv, if present and a 2-element list.
/// The value is the `list`-headed 2-element form v-syntax's `[lo hi]` parses to.
fn config_range(ast: &Arenas, app: StructId) -> Option<(StructId, StructId)> {
    let v = config_value(ast, app, "range")?;
    // `[lo hi]` parses to a `(list lo hi)` form; take its two elements.
    match ast.as_form(v, "list")? {
        [lo, hi] => Some((*lo, *hi)),
        _ => None,
    }
}

/// Build `(effect Param (op <name> (-> Unit <Type>)) …)` from the scanned sites and return its node id.
fn build_param_effect(ast: &mut Arenas, sites: &[ParamSite]) -> StructId {
    let effect_head = push_atom(ast, Leaf::Name("effect".to_string()));
    let param_name = push_atom(ast, Leaf::Name("Param".to_string()));
    let mut children = vec![effect_head, param_name];
    for site in sites {
        if site.is_rational_family() {
            // A heap `Rational` (bare, or the magnitude of a `(Qty Rational <unit>)`) has no host boundary
            // form (v-effects #13): declare TWO scalar `Int64` accessors `<name>-num`/`<name>-den`. The guest
            // recombines them via `Rational.of` (and, for a Qty, re-attaches the unit) — see
            // `rewrite_rational_uses`, which rewrote each `(Param.<name>)` use to call this pair.
            let name = ast
                .as_name(site.name)
                .expect("a rational-family param site has a name atom")
                .to_string();
            for suffix in ["-num", "-den"] {
                let op_name = push_atom(ast, Leaf::Name(format!("{name}{suffix}")));
                children.push(build_scalar_op(ast, op_name, "Int64"));
            }
        } else {
            // A scalar (or unit) type crosses the host boundary directly: one `(op <name> (-> Unit <Type>))`
            // accessor, result-typed by the annotation (reusing the declared-type occurrence's node).
            children.push(build_op(ast, site.name, site.ty));
        }
    }
    push_list(ast, children)
}

/// Build `(op <name> (-> Unit <ty-node>))` — a nullary host-delegated accessor whose result type reuses the
/// existing `ty` occurrence (so its span + resolution carry). Used for a scalar param's single accessor.
fn build_op(ast: &mut Arenas, name: StructId, ty: StructId) -> StructId {
    let op_head = push_atom(ast, Leaf::Name("op".to_string()));
    let arrow = build_arrow(ast, ty);
    push_list(ast, vec![op_head, name, arrow])
}

/// Build `(op <name> (-> Unit <TyName>))` with a FRESH result-type name atom (e.g. `"Int64"`). Used for the
/// synthesized num/den accessors, whose `Int64` result is not an existing occurrence in the source.
fn build_scalar_op(ast: &mut Arenas, name: StructId, ty_name: &str) -> StructId {
    let ty = push_atom(ast, Leaf::Name(ty_name.to_string()));
    build_op(ast, name, ty)
}

/// Build `(-> Unit <ty>)` — the nullary accessor's function type (no argument but `Unit`, result `ty`).
fn build_arrow(ast: &mut Arenas, ty: StructId) -> StructId {
    let arrow_head = push_atom(ast, Leaf::Name("->".to_string()));
    let unit_ty = push_atom(ast, Leaf::Name("Unit".to_string()));
    push_list(ast, vec![arrow_head, unit_ty, ty])
}

/// Rewrite every `(Param.<name>)` USE of a rational-family param into its guest-side recombination of the two
/// scalar num/den host accessors the effect declares (v-effects #13). A bare `Rational` param becomes
/// `(Rational.of (Param.<name>-num) (Param.<name>-den))`; a `(Qty Rational <unit>)` param wraps that in
/// `(Qty.of … <unit>)`, re-attaching the unit guest-side (#13 B2). A `(Param.<name>)` use parses to a nullary
/// call of a member access — `((. Param <name>))`, an outer `List` whose sole child is `(. Param <name>)`. We
/// overwrite that outer list in place with the recombination so the ordinary compile sees the reconstructed
/// value. Only ORIGINAL nodes are scanned (the rewrite runs before the effect is appended), and only the
/// exact rational-family param names match, so a scalar `(Param.width)` or any unrelated access is untouched.
fn rewrite_rational_uses(ast: &mut Arenas, sites: &[ParamSite]) {
    // Map each rational-family param NAME to its unit node (`Some` for a `(Qty Rational <unit>)`, `None` for a
    // bare `Rational`) — the use-site recombination needs the unit to re-attach it via `Qty.of`.
    let targets: Vec<(String, Option<StructId>)> = sites
        .iter()
        .filter_map(|s| {
            let name = ast.as_name(s.name)?.to_string();
            match s.kind {
                ParamKind::Rational => Some((name, None)),
                ParamKind::QtyRational { unit } => Some((name, Some(unit))),
                ParamKind::Scalar => None,
            }
        })
        .collect();
    if targets.is_empty() {
        return;
    }

    let original_len = ast.structure.len() as u32;
    for i in 0..original_len {
        let id = StructId(i);
        // Match `(Param.<name>)` = `((. Param <name>))`: an outer list with exactly one child that is the
        // member-access `(. Param <name>)`. Read the accessed param name; skip unless it is rational-family.
        let Some(name) = param_member_use_name(ast, id) else {
            continue;
        };
        let Some(&(_, unit)) = targets.iter().find(|(n, _)| n == &name) else {
            continue;
        };
        // `(Rational.of (Param.<name>-num) (Param.<name>-den))`, wrapped in `(Qty.of … <unit>)` for a Qty.
        let recombined = build_rational_recombine(ast, &name);
        let value = match unit {
            Some(unit) => build_qty_of(ast, recombined, unit),
            None => recombined,
        };
        ast.structure[id.0 as usize] = ast.get(value).clone();
    }
}

/// Build `(Qty.of <magnitude> <unit>)` = `((. Qty of) <magnitude> <unit>)` — re-attach the unit to a
/// recombined Rational magnitude guest-side (#13 B2). `unit` is the annotation's unit node, reused in place.
fn build_qty_of(ast: &mut Arenas, magnitude: StructId, unit: StructId) -> StructId {
    let of = build_member_access(ast, "Qty", "of");
    push_list(ast, vec![of, magnitude, unit])
}

/// If `id` is a `(Param.<name>)` use — the nullary-call `((. Param <name>))` shape — the accessed param NAME.
/// `None` otherwise. The use is an outer `List` of one element, that element the member access `(. Param x)`.
fn param_member_use_name(ast: &Arenas, id: StructId) -> Option<String> {
    let Struct::List(items) = ast.get(id) else {
        return None;
    };
    let [access] = items[..] else {
        return None;
    };
    // The single element must be `(. Param <name>)`: a member-access form headed `.` over `Param`.
    let tail = ast.as_form(access, ".")?;
    let (&recv, &member) = (tail.first()?, tail.get(1)?);
    if ast.as_name(recv) != Some("Param") {
        return None;
    }
    ast.as_name(member).map(str::to_string)
}

/// Build `(Rational.of (Param.<name>-num) (Param.<name>-den))` = `((. Rational of) ((. Param <name>-num))
/// ((. Param <name>-den)))` — the guest recombination for a rational param's num/den accessor pair.
fn build_rational_recombine(ast: &mut Arenas, name: &str) -> StructId {
    let of = build_member_access(ast, "Rational", "of");
    let num = build_param_accessor_call(ast, &format!("{name}-num"));
    let den = build_param_accessor_call(ast, &format!("{name}-den"));
    push_list(ast, vec![of, num, den])
}

/// Build the nullary accessor CALL `(Param.<member>)` = `((. Param <member>))`.
fn build_param_accessor_call(ast: &mut Arenas, member: &str) -> StructId {
    let access = build_member_access(ast, "Param", member);
    push_list(ast, vec![access])
}

/// Build a member-access form `(. <recv> <member>)`.
fn build_member_access(ast: &mut Arenas, recv: &str, member: &str) -> StructId {
    let dot = push_atom(ast, Leaf::Name(".".to_string()));
    let recv_atom = push_atom(ast, Leaf::Name(recv.to_string()));
    let member_atom = push_atom(ast, Leaf::Name(member.to_string()));
    push_list(ast, vec![dot, recv_atom, member_atom])
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
