//! `resolve : Ast → Hir` — name resolution and desugaring, module-wide.
//!
//! Mirrors `implementation/compiler/cdzc/20-resolve.cdz`. Collects the module's top-level `def`s into
//! an environment (function name → index), then resolves each function body: parameters and
//! `let`-locals resolve to `Local(id)` (lexical nearest-binding), a call `(f a b)` resolves `f` to
//! its function index, arithmetic/comparison/`if`/`let`/`Int64.max`-min as in Phase 1. A value-def
//! `(def k v)` desugars to a nullary function `k` (callers reference it as a nullary call). A name or
//! form it does not handle is `Hir::Error` (a clean decline) — never a guess. A `(doc "…")` leading
//! form in a body is skipped (documentation, not a runtime expression).
//!
//! Resolution happens HERE, once (compiler-pipeline.md §The Compiler Resolves Names Before It Selects
//! Instructions), not at emit.

use crate::diag::Code;
use crate::ir::{ArithOp, BitOp, CmpOp, Export, Hir, HirFunc, HirModule, Intrinsic, Reject, ShiftOp};
use crate::ty::{SumDef, SumRef, Ty, VariantDef};
use cdz_compiler::ast::Node;
use std::collections::HashMap;
use std::sync::Arc;

/// The prelude — a flat set of NAMED `Hir` nodes, nothing more. A sum type NAME binds to a compile-time
/// RECORD of its constructor values (so `(. Sign Pos)` is ORDINARY record projection to the `Ctor` — the
/// record folds away, zero sum-specific access code); an UNQUALIFIED sum ALSO binds each variant as a
/// BARE constructor name (`Some`→`Ctor{Option,0}`, `None`→`Ctor{Option,1}`, `Ok`, `Err`). Some/None are
/// not special names — they are ordinary named entries. `expect` is not here; it is a syntactic desugar
/// to a `match` that traps (a function that matches and traps — the prelude `expect` HIR function).
use crate::prelude::Prelude;

/// Collect the program's user `(type Name (V1 p | V2 | …))` declarations into `prelude`, binding each
/// type's constructors — the SAME machinery a prelude sum uses (a type name → a `Hir::Record` of
/// `Hir::Ctor` values, and each variant ALSO as a bare `Hir::Ctor` so a bare nullary `NNil` or a
/// qualified `(. Node NLit)` both resolve). A user sum is QUALIFIED (renders `Node.NLit`).
///
/// Built in TWO PHASES so a RECURSIVE payload (`Neg Expr`, `Cons (Tuple Int64 IntList)`) and a
/// MUTUALLY-recursive sibling resolve to the right `Arc<SumDef>`: phase 1 forward-declares every type's
/// Arc (variants unset); phase 2 parses each variant's payload type-expression against the full set of
/// declared Arcs (+ prelude sums + scalar builtins) and fills the variants. A duplicate variant name
/// WITHIN one type is CDZ0201 (a sum's variant names are a set — the fourth closed name-set).
///
/// Types are found by recursively scanning every non-quoted form (a `(type …)` may sit at the top level
/// or nested in a `do`/`let`/module body — declaration scoping), mirroring the old compiler's
/// `collect_sum_types`. Variant tags share a program-global binding here (last-writer-wins on a reused
/// name, so a program's `(type …)` overrides a prelude variant's arity — the reused-prelude-name case).
fn collect_user_types(forms: &[Node], prelude: &mut Prelude) -> Result<(), Reject> {
    // ── Phase 1: find every `(type Name (body))`, forward-declare its Arc, and stash its raw variant
    //    segments (tag + payload type-expr Nodes) for phase 2. The variant NAMES + count are known here
    //    (the tags), so the constructor bindings go into `prelude` NOW — a bare/qualified ctor and a
    //    RECURSIVE payload type-expression (`Neg Expr`) then resolve through the ONE map, and only the
    //    payload TYPES remain for phase 2 to fill on the shared `Arc<SumDef>`. ──
    let mut raw: Vec<(SumRef, Vec<(String, Vec<Node>)>)> = Vec::new();
    collect_type_forms(forms, &mut raw)?;
    for (sref, segments) in &raw {
        // Duplicate variant name WITHIN one type is CDZ0201 (a sum's variant names are a closed set).
        let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for (tag, _) in segments {
            if !seen.insert(tag.as_str()) {
                return Err(Reject::coded(
                    Code::TypeError,
                    format!(
                        "sum type `{}` declares variant `{tag}` more than once",
                        sref.name
                    ),
                ));
            }
        }
        // The type NAME → a record of its constructor values; each variant ALSO bound bare (a bare
        // nullary `NNil` / a qualified `(. Node NLit)` both resolve through these). Ctors carry the
        // shared `SumRef` whose variants phase 2 fills — the bindings stay valid after the fill.
        let fields: Vec<(String, Hir)> = segments
            .iter()
            .enumerate()
            .map(|(i, (tag, _))| {
                (
                    tag.clone(),
                    Hir::Ctor {
                        def: sref.clone(),
                        index: i,
                    },
                )
            })
            .collect();
        for (i, (tag, _)) in segments.iter().enumerate() {
            prelude.insert(
                tag.clone(),
                Hir::Ctor {
                    def: sref.clone(),
                    index: i,
                },
            );
        }
        prelude.insert(sref.name.clone(), Hir::Record(fields));
    }

    // ── Phase 2: parse each type's variant payload type-expressions against the ONE map (now carrying
    //    every user type's ctor record, so a recursive/mutually-recursive reference resolves) and fill
    //    the variants on the shared Arc. ──
    for (sref, segments) in &raw {
        let mut variants: Vec<VariantDef> = Vec::with_capacity(segments.len());
        for (tag, payload_nodes) in segments {
            // A single-token segment (no payload nodes) is a NULLARY variant (argument type Unit); a
            // segment with a payload token is unary, its payload type the parsed type-expression.
            let payload = match payload_nodes.as_slice() {
                [] => None,
                [one] => Some(parse_type_expr(one, prelude)?),
                // Multiple bare payload tokens `(V a b)` are not the surface (a multi-field payload is a
                // `(Tuple …)`); decline rather than guess.
                _ => return Err(Reject::decline(
                    "a variant with multiple bare payload tokens is a later phase (use a Tuple payload)",
                )),
            };
            variants.push(VariantDef {
                name: tag.clone(),
                payload,
            });
        }
        sref.0.set_variants(variants);
    }
    Ok(())
}

/// Recursively find `(type Name (body))` forms (skipping `quote`/`quasiquote` — a `(type …)` inside
/// quoted data is an AST value, not a declaration), forward-declaring each type's `Arc<SumDef>` and
/// recording its raw variant segments. The body `(V1 payload | V2 | …)` reads as ONE FLAT token list
/// with `|` a bare-name separator (`(Some a | None)` → `[Some, a, |, None]`); split on `|`, each
/// segment's HEAD is the tag and its remaining tokens are the payload type-expression.
fn collect_type_forms(
    forms: &[Node],
    raw: &mut Vec<(SumRef, Vec<(String, Vec<Node>)>)>,
) -> Result<(), Reject> {
    for form in forms {
        let items = match form {
            Node::List(items) => items,
            _ => continue,
        };
        let head = items.first().and_then(name_of);
        // Recurse into every non-quoted subtree (a nested `(do (type …) …)`).
        if !matches!(head, Some("quote") | Some("quasiquote")) {
            collect_type_forms(items, raw)?;
        }
        if head != Some("type") {
            continue;
        }
        let name = match items.get(1).and_then(name_of) {
            Some(n) => n.to_string(),
            None => return Err(Reject::decline("type declaration has no name")),
        };
        let body = match items.get(2) {
            Some(Node::List(b)) => b,
            _ => return Err(Reject::decline("type declaration has no variant body")),
        };
        // Forward-declare the Arc (variants unset). A user sum takes no type parameters in this slice
        // (the corpus user types are all monomorphic); a lowercase payload token is treated as a type
        // NAME by `parse_type_expr` (and declines if unbound), not as a parameter.
        let sref = SumRef(Arc::new(SumDef::forward(name.clone(), vec![], true)));
        // Split the flat body on the bare `|` into segments; head = tag, rest = payload type nodes.
        let mut segments: Vec<(String, Vec<Node>)> = Vec::new();
        for segment in body.split(|n| matches!(n, Node::Name(s) if s == "|")) {
            let tag = match segment.first() {
                Some(Node::Name(v)) => v.clone(),
                // A parenthesized variant `(V payload)` — take its head, payload = the rest of the list.
                Some(Node::List(v)) => match v.first().and_then(name_of) {
                    Some(h) => {
                        let mut seg = vec![(h.to_string(), v[1..].to_vec())];
                        segments.append(&mut seg);
                        continue;
                    }
                    None => return Err(Reject::decline("a variant has no tag")),
                },
                _ => return Err(Reject::decline("a variant segment has no tag")),
            };
            segments.push((tag, segment[1..].to_vec()));
        }
        raw.push((sref, segments));
    }
    Ok(())
}

/// Parse a payload TYPE-EXPRESSION node into a `Ty`. This is TYPE syntax (not value resolution): the
/// scalar keywords `Int64`/`Bool`/`String`/`Bytes`/`Unit` name leaf types, `(Tuple t…)`/`(List t)` are
/// the type constructors, and a bare name naming a SUM (user OR prelude — resolved by
/// `prelude::sum_ref` against the one map, which holds both the program's `(type …)` decls and the
/// built-in sums) is that `Ty::Sum` (recursion resolves here — the sum's own Arc is in the map).
/// An unknown name / unsupported shape DECLINES (never miscompiles).
fn parse_type_expr(node: &Node, prelude: &Prelude) -> Result<Ty, Reject> {
    match node {
        Node::List(items) if items.is_empty() => Ok(Ty::Unit),
        Node::Name(n) => match n.as_str() {
            "Int64" => Ok(Ty::Int),
            "Bool" => Ok(Ty::Bool),
            "String" => Ok(Ty::String),
            "Bytes" => Ok(Ty::Bytes),
            "Unit" => Ok(Ty::Unit),
            _ => match crate::prelude::sum_ref(prelude, n) {
                Some(def) => Ok(Ty::Sum { def, args: vec![] }),
                None => Err(Reject::decline(format!("unknown payload type `{n}`"))),
            },
        },
        Node::List(items) => {
            let head = items.first().and_then(name_of);
            match head {
                Some("Tuple") => {
                    let elems: Result<Vec<Ty>, Reject> = items[1..]
                        .iter()
                        .map(|e| parse_type_expr(e, prelude))
                        .collect();
                    Ok(Ty::Tuple(elems?))
                }
                Some("List") if items.len() == 2 => {
                    Ok(Ty::List(Box::new(parse_type_expr(&items[1], prelude)?)))
                }
                _ => Err(Reject::decline("unsupported payload type expression")),
            }
        }
        _ => Err(Reject::decline("a payload type is not a type expression")),
    }
}

/// A collected definition, before body resolution: its name, parameter names, body node, and whether
/// the source wrote it as a FUNCTION `(def (f …) …)` (vs a value `(def v …)`). The `is_fn` flag drives
/// the module-export view: a function export is a `FuncRef` value applied with `(f arg)`, while a value
/// export projects directly. A NULLARY function exported from a module is unit-wrapped (arity-1 taking
/// an ignored unit) so `((. m answer) unit)` applies it — see `gather_scope`.
#[derive(Clone)]
struct RawDef<'a> {
    name: &'a str,
    params: Vec<&'a str>,
    body: &'a Node,
    is_fn: bool,
}

/// A function collected for resolution. Either an ordinary `Source` def resolved against its
/// NAMESPACE (name → global function index), or a `Synthetic` nullary function whose body is
/// pre-built `Hir` — the module-record value-def, whose body is a `Hir::Record` of the module's
/// exports (`FuncRef`s and value exports). A top-level function's namespace is the top-level defs +
/// module names; a module function's namespace is its module siblings + the top-level defs + module
/// names — so a module ENCAPSULATES its internals (the parent cannot reach non-exported defs).
enum RawFunc<'a> {
    Source {
        def: RawDef<'a>,
        namespace: HashMap<&'a str, usize>,
    },
    Synthetic {
        name: String,
        body: Hir,
    },
    /// A reserved slot, filled once its scope's namespace is complete (reserve-then-fill lets a body
    /// reference any scope member regardless of definition order). Never survives `gather_scope`.
    Placeholder,
}

/// Resolve a whole program to an `HirModule`. A program is a top-level sequence of forms — the
/// implicit module — read as a single `(do <form>…)` node (the reader synthesizes the `do` wrapper
/// when a file/input has more than one top-level form, reusing `do`'s declaration-scoping: a `def`
/// is in scope for the forms that follow). The forms are `(def …)`, `(type …)`, and one or more
/// `(export name…)` naming the public items. Visibility is EXPLICIT — a definition is exposed iff an
/// `(export …)` names it, never by position or a well-known name (modules-and-namespaces.md
/// §Visibility Is Explicit).
pub fn resolve_program(node: &Node) -> Result<HirModule, Reject> {
    let forms = top_level_forms(node)?;

    // The ONE prelude map (`crate::prelude::all()` — every built-in sum constructor + built-in module,
    // cached, shared so a bare `Some`/`(. Option Some)`/`(. Bytes of)` all resolve to the same value).
    // CLONE it (an Arc-refcount bump — the `SumRef`s keep their identity) and layer the program's OWN
    // `(type …)` declarations on top, so `(T.A x)` / a bare nullary `NNil` / a `(match …)` resolve
    // through this ONE map exactly as a built-in does — resolve does nothing but look names up here.
    let mut prelude = crate::prelude::all().clone();
    collect_user_types(forms, &mut prelude)?;
    let prelude = prelude;

    // Gather every function (top-level and, recursively, every nested module's) into ONE flat list
    // `raw` with GLOBAL indices; a `(module m …)` hoists its functions and yields a synthetic
    // value-def binding `m` to a `(record …)` of its EXPORTS (so `(. m f)` reuses record projection,
    // and a module ENCAPSULATES its internals — the enclosing scope's namespace holds only the module
    // NAME, never the module's defs). `gather_scope` returns the bindings VISIBLE in a scope (its own
    // defs + module names), which become the enclosing namespace for that scope's functions.
    let mut g = Gather { raw: Vec::new() };
    // The top-level scope sees only what it defines; there is no enclosing namespace. It is NOT a
    // module scope (its nullary functions — the component entries — stay nullary, not unit-wrapped).
    let (top_ns, top_exports) = g.gather_scope(forms, &HashMap::new())?;

    // ── Exports (the component boundary) — ONLY top-level exports cross the wasm boundary; a nested
    // module's exports are in-program (its record's fields). ──
    if top_exports.is_empty() {
        return Err(Reject::decline(
            "program has no (export …) — nothing is public",
        ));
    }
    let mut exports = Vec::with_capacity(top_exports.len());
    for name in &top_exports {
        match top_ns.get(name) {
            Some(&func) => exports.push(Export {
                name: name.to_string(),
                func,
            }),
            None => {
                return Err(Reject::coded(
                    Code::UnboundName,
                    format!("export names an undefined item: {name}"),
                ))
            }
        }
    }

    // ── Resolve each gathered function body under ITS namespace + parameters. Ordered by global index
    // (the order they were pushed), so a body at position i is global function i. ──
    let mut funcs: Vec<Option<HirFunc>> = (0..g.raw.len()).map(|_| None).collect();
    for (idx, rf) in g.raw.iter().enumerate() {
        let hf = match rf {
            RawFunc::Source { def, namespace } => {
                let mut r = BodyResolver {
                    index: namespace,
                    next_local: 0,
                    prelude: &prelude,
                };
                let param_ids: Vec<u32> = def.params.iter().map(|_| r.fresh_local()).collect();
                let named: Vec<(&str, u32)> = def.params.iter().copied().zip(param_ids).collect();
                let body = resolve_with_params(&mut r, &named, def.body, &Scope::Empty);
                HirFunc {
                    name: def.name.to_string(),
                    arity: def.params.len(),
                    body,
                }
            }
            RawFunc::Synthetic { name, body } => HirFunc {
                name: name.clone(),
                arity: 0,
                body: body.clone(),
            },
            RawFunc::Placeholder => return Err(Reject::decline("internal: unfilled gather slot")),
        };
        funcs[idx] = Some(hf);
    }
    let funcs = funcs
        .into_iter()
        .map(|f| f.expect("every index filled"))
        .collect();

    Ok(HirModule { funcs, exports })
}

/// The gather accumulator: the flat, globally-indexed list of every function in the program (top-level
/// and every nested module's), each with the namespace it resolves against.
struct Gather<'a> {
    raw: Vec<RawFunc<'a>>,
}

impl<'a> Gather<'a> {
    /// Gather one SCOPE's forms (the top level, or a module body): hoist its `def`s and nested
    /// `module`s into `self.raw`, recursing into modules, and return `(namespace, exports)` — the
    /// scope's name→global-index map (its own defs + nested module NAMES, over the `enclosing`
    /// namespace) and its `(export …)` names. A def/module resolves names against this returned
    /// namespace; the enclosing scope sees only what this scope EXPORTS (via the record it becomes).
    fn gather_scope(
        &mut self,
        forms: &'a [Node],
        enclosing: &HashMap<&'a str, usize>,
    ) -> Result<(HashMap<&'a str, usize>, Vec<&'a str>), Reject> {
        // Collect this scope's defs, nested modules, and exports structurally.
        let mut defs: Vec<RawDef<'a>> = Vec::new();
        let mut submodules: Vec<ModuleForm<'a>> = Vec::new();
        let mut exports: Vec<&'a str> = Vec::new();
        for form in forms {
            if let Node::List(items) = form {
                match items.first().and_then(name_of) {
                    Some("export") => {
                        for n in &items[1..] {
                            match name_of(n) {
                                Some(name) => exports.push(name),
                                None => {
                                    return Err(Reject::decline("export names must be identifiers"))
                                }
                            }
                        }
                        continue;
                    }
                    Some("module") => {
                        submodules.push(collect_module(items)?);
                        continue;
                    }
                    _ => {}
                }
            }
            if let Some(d) = collect_def(form)? {
                defs.push(d);
            }
        }

        // ── Assign global indices for THIS scope's items, RESERVING a slot in `self.raw` per item so
        // a body (or a submodule) can reference any of them regardless of order. Layout per scope:
        // [its defs] [each submodule's functions (recursively)] [each submodule's record value-def].
        // A `def` slot is filled after the full scope namespace is known; a submodule's slots are
        // filled by the recursive `gather_scope`. ──
        //
        // The scope namespace = the enclosing one (so an inner scope reaches an outer name), with THIS
        // scope's own names layered on top (they SHADOW an enclosing binding — not a duplicate). A
        // repeat WITHIN this scope is the duplicate error; `local_names` tracks same-scope names to
        // tell a shadow (allowed) from a same-scope duplicate (CDZ0201).
        let mut ns = enclosing.clone();
        let mut local_names: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();

        // Defs: reserve a contiguous block of indices.
        let def_base = self.raw.len();
        for (i, d) in defs.iter().enumerate() {
            if !local_names.insert(d.name) {
                return Err(Reject::coded(
                    Code::TypeError,
                    format!("duplicate definition: {}", d.name),
                ));
            }
            ns.insert(d.name, def_base + i); // shadows any enclosing binding of the same name
        }
        // Reserve the def slots with a placeholder (filled once `ns` is complete).
        for _ in &defs {
            self.raw.push(RawFunc::Placeholder);
        }

        // Submodules: recurse (their functions append to `self.raw`), then reserve + bind the module
        // name to its record value-def slot. Recursion sees `ns`-so-far as its enclosing scope, so an
        // inner module can call an outer function or reach a sibling defined earlier.
        for m in &submodules {
            if !local_names.insert(m.name) {
                return Err(Reject::coded(
                    Code::TypeError,
                    format!("duplicate definition: {}", m.name),
                ));
            }
            let (sub_ns, _sub_exports) = self.gather_scope(m.forms, &ns)?;
            // The submodule's export fields (name → global index), CDZ0201 on a duplicate export.
            let mut fields: Vec<(String, Hir)> = Vec::new();
            let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
            for e in &m.exports {
                let &idx = sub_ns.get(*e).ok_or_else(|| {
                    Reject::coded(
                        Code::UnboundName,
                        format!("module `{}` exports an undefined item: {e}", m.name),
                    )
                })?;
                if !seen.insert(e) {
                    return Err(Reject::coded(
                        Code::TypeError,
                        format!("module `{}` exports `{e}` more than once", m.name),
                    ));
                }
                // A function export is a `FuncRef` value; a value export (or a submodule name) is a
                // nullary `Call` the fold reduces to the value/record.
                let field = if m.is_function_export(e) {
                    Hir::FuncRef(idx)
                } else {
                    Hir::Call {
                        func: idx,
                        args: Vec::new(),
                    }
                };
                fields.push((e.to_string(), field));
            }
            // Bind the module NAME in this scope to its record value-def, and push that synthetic func.
            let record_index = self.raw.len();
            ns.insert(m.name, record_index); // shadows an enclosing binding of the same name
            self.raw.push(RawFunc::Synthetic {
                name: m.name.to_string(),
                body: Hir::Record(fields),
            });
        }

        // Now `ns` is the complete scope namespace — fill each def's RawFunc with it.
        for (i, d) in defs.into_iter().enumerate() {
            self.raw[def_base + i] = RawFunc::Source {
                def: d,
                namespace: ns.clone(),
            };
        }

        Ok((ns, exports))
    }
}

/// A `(module name form…)` collected structurally: its name (borrowed from the source), its inner
/// forms (defs, submodules, exports), and its export names. Nesting is just a `(module …)` among the
/// forms — `gather_scope` recurses into `forms`.
struct ModuleForm<'a> {
    name: &'a str,
    forms: &'a [Node],
    exports: Vec<&'a str>,
}

impl ModuleForm<'_> {
    /// Whether the export named `e` is a FUNCTION def (vs a value def or a submodule name).
    fn is_function_export(&self, e: &str) -> bool {
        for form in self.forms {
            if let Node::List(items) = form {
                if items.first().and_then(name_of) == Some("def") {
                    // `(def (name p…) …)` is a function; `(def name v)` is a value.
                    if let Some(Node::List(sig)) = items.get(1) {
                        if sig.first().and_then(name_of) == Some(e) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }
}

/// Parse `(module name form…)` — the name, the inner forms, and any `(export …)` names among them.
fn collect_module(items: &[Node]) -> Result<ModuleForm<'_>, Reject> {
    let name = match items.get(1).and_then(name_of) {
        Some(n) => n,
        None => return Err(Reject::decline("module declaration has no name")),
    };
    let forms = &items[2..];
    let mut exports = Vec::new();
    for form in forms {
        if let Node::List(inner) = form {
            if inner.first().and_then(name_of) == Some("export") {
                for n in &inner[1..] {
                    match name_of(n) {
                        Some(name) => exports.push(name),
                        None => {
                            return Err(Reject::decline("module export names must be identifiers"))
                        }
                    }
                }
            }
        }
    }
    Ok(ModuleForm {
        name,
        forms,
        exports,
    })
}

/// The top-level forms of a program. The canonical shape is a single `(do <form>…)` node (the
/// reader's implicit-module wrapper); a lone top-level form (a one-definition program) may also
/// arrive unwrapped, so a non-`do` node is treated as a single-form program.
fn top_level_forms(node: &Node) -> Result<&[Node], Reject> {
    match node {
        Node::List(items) if items.first().and_then(name_of) == Some("do") => Ok(&items[1..]),
        // A single top-level form that is itself a list (e.g. one `(def …)`): treat as a 1-form
        // program. Borrow it as a slice via the enclosing node.
        Node::List(_) => Ok(std::slice::from_ref(node)),
        _ => Err(Reject::decline(
            "program is not a sequence of top-level forms",
        )),
    }
}

/// Chain a scope frame per parameter onto `parent`, then resolve `node` under the full chain.
fn resolve_with_params<'s>(
    r: &mut BodyResolver,
    params: &[(&'s str, u32)],
    node: &Node,
    parent: &Scope<'s>,
) -> Hir {
    match params.split_first() {
        None => r.expr(node, parent),
        Some(((name, id), rest)) => {
            let frame = Scope::Bind {
                name,
                id: *id,
                parent,
            };
            resolve_with_params(r, rest, node, &frame)
        }
    }
}

/// Collect a top-level form into a `RawDef` if it is a `def`; `None` for a non-def form.
fn collect_def(form: &Node) -> Result<Option<RawDef<'_>>, Reject> {
    let items = match form {
        Node::List(items) => items,
        _ => return Ok(None),
    };
    if items.first().and_then(name_of) != Some("def") {
        return Ok(None);
    }
    // `(def (name p…) body)` — a function; or `(def name value)` / `(def name (doc …) value)` — a value.
    match items.get(1) {
        // Function: `(def (name params…) body…)`.
        Some(Node::List(sig)) => {
            let name = match sig.first().and_then(name_of) {
                Some(n) => n,
                None => return Err(Reject::decline("function definition has no name")),
            };
            let mut params = Vec::new();
            for p in &sig[1..] {
                match name_of(p) {
                    Some(pn) => params.push(pn),
                    None => return Err(Reject::decline("function parameter is not a name")),
                }
            }
            let body = body_after_doc(&items[2..])
                .ok_or_else(|| Reject::decline("function definition has no body"))?;
            Ok(Some(RawDef {
                name,
                params,
                body,
                is_fn: true,
            }))
        }
        // Value: `(def name value)` or `(def name (doc …) value)`. Desugars to a nullary function.
        Some(Node::Name(name)) => {
            let body = body_after_doc(&items[2..])
                .ok_or_else(|| Reject::decline("value definition has no value expression"))?;
            Ok(Some(RawDef {
                name,
                params: Vec::new(),
                body,
                is_fn: false,
            }))
        }
        _ => Err(Reject::decline("malformed def")),
    }
}

/// The body expression among a def's trailing forms, skipping a leading `(doc "…")`. Returns the
/// single value form (the last non-doc form). Phase 2a expects exactly one value form after the doc.
fn body_after_doc<'a>(rest: &'a [Node]) -> Option<&'a Node> {
    let forms: Vec<&Node> = rest
        .iter()
        .filter(|f| !matches!(f, Node::List(d) if d.first().and_then(name_of) == Some("doc")))
        .collect();
    // Exactly one runtime form is the body (Phase 2a; a multi-form body is `do`, a later phase).
    match forms.as_slice() {
        [one] => Some(one),
        _ => None,
    }
}

/// A lexical scope frame chain (name → local id). Nearest binding wins.
enum Scope<'a> {
    Empty,
    Bind {
        name: &'a str,
        id: u32,
        parent: &'a Scope<'a>,
    },
}

impl<'a> Scope<'a> {
    fn lookup(&self, name: &str) -> Option<u32> {
        match self {
            Scope::Empty => None,
            Scope::Bind {
                name: n,
                id,
                parent,
            } => {
                if *n == name {
                    Some(*id)
                } else {
                    parent.lookup(name)
                }
            }
        }
    }
}

/// Resolves one function body under the module index, with a per-function local-id supply.
struct BodyResolver<'a> {
    index: &'a HashMap<&'a str, usize>,
    next_local: u32,
    /// The prelude sums — consulted (after locals/defs) to resolve a bare constructor name and a
    /// `(. Type Variant)` member access to `Hir::Ctor`.
    prelude: &'a Prelude,
}

impl<'a> BodyResolver<'a> {
    fn fresh_local(&mut self) -> u32 {
        let id = self.next_local;
        self.next_local += 1;
        id
    }

    fn expr(&mut self, node: &Node, scope: &Scope) -> Hir {
        match node {
            Node::Int(n) => Hir::Int(*n),
            Node::Bool(b) => Hir::Bool(*b),
            Node::Name(name) => {
                // `_` is the wildcard — meaningful only in pattern position, where a match's binders are
                // already in scope (so a genuine name resolves as a `Local`). A `_` reaching a VALUE
                // position is caught downstream (it never resolves to a value). This is the ONLY
                // pattern-specific arm; a pattern otherwise resolves exactly like an expression.
                if name == "_" {
                    return Hir::Wildcard;
                }
                // A local (param / let) first; then a nullary function (value-def) reference; then the
                // ONE prelude map — checked AFTER locals/defs so a program's own name shadows a built-in;
                // else unbound.
                if let Some(id) = scope.lookup(name) {
                    Hir::Local(id)
                } else if let Some(&func) = self.index.get(name.as_str()) {
                    Hir::Call {
                        func,
                        args: Vec::new(),
                    }
                } else if let Some(node) = self.prelude.get(name.as_str()) {
                    // A prelude entry — every named built-in is an ordinary value in the ONE map: `unit`,
                    // a bare constructor (`Some`/`None`/`Ok`/`Err` → a `Ctor`), a sum type name
                    // (`Option`/`Sign` → its constructor RECORD), or a built-in module
                    // (`Int64`/`Bytes`/`Map`/… → its operation RECORD). So `(. Sign Pos)` / `(. Int64
                    // wrapping-add)` are ordinary projection. NO built-in name is special-cased here; a
                    // new built-in is a `prelude` entry.
                    // ⚡ Layer 1 first-class types: a BARE scalar type name (`Int64`, `Bool`, `String`,
                    // `Bytes`, `Unit`) resolves to a `TypeVal` (typed as `Ty::Type`), not its operation
                    // record — so `(let ((t Int64)) t)` → `(: Int64 Type)`. BUT `(. Int64 wrapping-add)`
                    // still works: member access reads the record directly (below). This dual-role is the
                    // least-invasive L1 wiring.
                    // ⚡ Layer 2 first-class types: a BARE parametric type name (`List`/`Map`/`Set`/`Tuple`/
                    // `Option`/`Result`) resolves to its TYPE-BUILDER INTRINSIC (the head of `(List Int64)`,
                    // or a bare `List` value). This is a dual-role parallel to the scalars: member access
                    // `(. List push)` / `(. Option Some)` still projects the operation/ctor RECORD via the
                    // separate `member()` path (it never enters this arm), and applying `(List Int64)` folds
                    // the builder intrinsic to `TypeVal(Ty::List(Int))`.
                    match name.as_str() {
                        "Int64" => Hir::TypeVal(crate::ty::Ty::Int),
                        "Bool" => Hir::TypeVal(crate::ty::Ty::Bool),
                        "String" => Hir::TypeVal(crate::ty::Ty::String),
                        "Bytes" => Hir::TypeVal(crate::ty::Ty::Bytes),
                        "Unit" => Hir::TypeVal(crate::ty::Ty::Unit),
                        "List" => Hir::Intrinsic(Intrinsic::TypeList),
                        "Map" => Hir::Intrinsic(Intrinsic::TypeMap),
                        "Set" => Hir::Intrinsic(Intrinsic::TypeSet),
                        "Tuple" => Hir::Intrinsic(Intrinsic::TypeTuple),
                        "Option" => Hir::Intrinsic(Intrinsic::TypeOption),
                        "Result" => Hir::Intrinsic(Intrinsic::TypeResult),
                        _ => node.clone(),
                    }

                } else if looks_like_numeric_literal(name) {
                    // A digit-led token that reached resolution as a NAME is a numeric literal the
                    // reader could not parse — out of the Int64 range, or a malformed digit-separator
                    // shape. That is a malformed literal (CDZ0201), NOT an unbound name (CDZ0101):
                    // reporting `unbound name` for an all-digit token is the misleading diagnostic
                    // 01-literals.sexp forbids.
                    Hir::Error(Reject::coded(
                        Code::TypeError,
                        format!("malformed numeric literal: {name}"),
                    ))
                } else {
                    Hir::Error(Reject::coded(
                        Code::UnboundName,
                        format!("unbound name: {name}"),
                    ))
                }
            }
            // The empty tuple `()` IS the unit value (core-semantics.md §The Empty Tuple Is The Unit
            // Value) — reader gives it as an empty list.
            Node::List(items) if items.is_empty() => Hir::Unit,
            Node::List(items) => self.form(items, scope),
            Node::Float(_) => Hir::Error(Reject::decline("float literals are a later phase")),
            Node::Str(s) => Hir::Str(s.clone()),
        }
    }

    fn form(&mut self, items: &[Node], scope: &Scope) -> Hir {
        // A head that is itself a LIST is an APPLICATION of a function-VALUE expression to arguments:
        // `((. m f) x)` = apply the function projected from module `m` to `x`, and `((. Sign Pos) unit)`
        // = apply the constructor `Sign.Pos`. (A head that is a NAME bound to a function is the ordinary
        // `Call` case below.) The fold reduces `Apply(FuncRef(i), args)`/`Apply(Ctor, args)` to a
        // direct `Call`/`Mir::Sum`.
        if let Some(Node::List(_)) = items.first() {
            let func = self.expr(&items[0], scope);
            let args = items[1..].iter().map(|a| self.expr(a, scope)).collect();
            return Hir::Apply {
                func: Box::new(func),
                args,
            };
        }
        let head = items.first().and_then(name_of);
        match head {
            Some(".") if items.len() == 3 => self.member(items, scope),
            Some(op @ ("+" | "-" | "*")) if items.len() == 3 => {
                let a = self.expr(&items[1], scope);
                let b = self.expr(&items[2], scope);
                let op = match op {
                    "+" => ArithOp::Add,
                    "-" => ArithOp::Sub,
                    _ => ArithOp::Mul,
                };
                Hir::Arith(op, Box::new(a), Box::new(b))
            }
            Some(op @ ("&" | "|" | "^" | "/" | "%")) if items.len() == 3 => {
                let a = self.expr(&items[1], scope);
                let b = self.expr(&items[2], scope);
                let op = match op {
                    "&" => BitOp::And,
                    "|" => BitOp::Or,
                    "^" => BitOp::Xor,
                    "/" => BitOp::Div,
                    _ => BitOp::Rem,
                };
                Hir::Bit(op, Box::new(a), Box::new(b))
            }
            Some(op @ ("<<" | ">>")) if items.len() == 3 => {
                let a = self.expr(&items[1], scope);
                let b = self.expr(&items[2], scope);
                let op = if op == "<<" {
                    ShiftOp::Left
                } else {
                    ShiftOp::Right
                };
                Hir::Shift(op, Box::new(a), Box::new(b))
            }
            Some(op @ ("<" | ">" | "<=" | ">=" | "=")) if items.len() == 3 => {
                let a = self.expr(&items[1], scope);
                let b = self.expr(&items[2], scope);
                let op = match op {
                    "<" => CmpOp::Lt,
                    ">" => CmpOp::Gt,
                    "<=" => CmpOp::Le,
                    ">=" => CmpOp::Ge,
                    _ => CmpOp::Eq,
                };
                Hir::Cmp(op, Box::new(a), Box::new(b))
            }
            Some("if") if items.len() == 4 => {
                let c = self.expr(&items[1], scope);
                let t = self.expr(&items[2], scope);
                let e = self.expr(&items[3], scope);
                Hir::If(Box::new(c), Box::new(t), Box::new(e))
            }
            // `(: e T)` — annotate expression `e` with type `T`. Both are ordinary expressions; infer
            // checks `T : Type` and unifies its represented `Ty` with `e`'s type (mismatch → CDZ0203).
            Some(":") if items.len() == 3 => {
                let e = self.expr(&items[1], scope);
                let t = self.expr(&items[2], scope);
                Hir::Annot(Box::new(e), Box::new(t))
            }
            // `(const e)` — assert `e` fully compile-time-reduces. Infer types it, fold reduces it; the
            // erasure fence rejects it if not a fully-ground compile-time value.
            Some("const") if items.len() == 2 => {
                let e = self.expr(&items[1], scope);
                Hir::Const(Box::new(e))
            }
            // `(tuple e0 … en)` — a heap tuple of its element expressions. A LOCAL/param binding named
            // `tuple` SHADOWS this built-in constructor (name resolution is scope-FIRST): when shadowed,
            // fall through to the application path so `(tuple 3 4)` is `Apply(Local, [3,4])`.
            Some("tuple") if scope.lookup("tuple").is_none() => {
                let elems = items[1..].iter().map(|e| self.expr(e, scope)).collect();
                Hir::Tuple(elems)
            }
            // `(list e0 … en)` — a heap list of its element expressions (a homogeneous `List T`). The
            // empty `(list)` is the empty list. Parallel to `(tuple …)`, but its element type is a
            // single `T` unified across the elements (inference), not a per-position arity. A local/param
            // named `list` SHADOWS this built-in (scope-FIRST) — falls through to the application path.
            Some("list") if scope.lookup("list").is_none() => {
                let elems = items[1..].iter().map(|e| self.expr(e, scope)).collect();
                Hir::List(elems)
            }
            // `(record (k0 v0) … (kn vn))` — a heap record. Each entry MUST be a `(name value)` pair,
            // and the field names are a SET (each at most once) — a malformed entry or a duplicate
            // field name is CDZ0201 (core-semantics.md §A Record Has A Fixed Set Of Named Fields). A
            // local/param named `record` SHADOWS this built-in (scope-FIRST).
            Some("record") if scope.lookup("record").is_none() => self.record(items, scope),
            // `(map (k0 v0) … (kn vn))` — a heap map (the runtime CHAMP). Each entry is a `(key value)`
            // pair; unlike a record's field names, a map's KEYS are VALUE expressions (runtime data), so
            // `(map (a 1))` keys by the VALUE of `a`. The empty `(map)` is the empty map. Keys unify to
            // one `K`, values to one `V` (inference). Built via `map-insert` from empty. A local/param
            // named `map` SHADOWS this built-in (scope-FIRST).
            Some("map") if scope.lookup("map").is_none() => {
                let mut entries = Vec::new();
                for e in &items[1..] {
                    match e {
                        Node::List(kv) if kv.len() == 2 => {
                            entries.push((self.expr(&kv[0], scope), self.expr(&kv[1], scope)));
                        }
                        _ => return Hir::Error(Reject::decline("a map entry is not a (key value) pair")),
                    }
                }
                Hir::Map(entries)
            }
            // `(set e0 … en)` — a heap set (the runtime CHAMP set). Elements are VALUE expressions
            // unifying to one `E`; duplicates collapse at run time. The empty `(set)` is the empty set.
            // A local/param named `set` SHADOWS this built-in (scope-FIRST).
            Some("set") if scope.lookup("set").is_none() => {
                let elems = items[1..].iter().map(|e| self.expr(e, scope)).collect();
                Hir::Set(elems)
            }
            // `(match scrutinee (pattern body)…)` — dispatch on the scrutinee's shape.
            Some("match") if items.len() >= 2 => self.match_form(items, scope),
            // Boolean connectives SHORT-CIRCUIT (core-semantics.md §Boolean Connectives Short-Circuit):
            // `(and a b)` = `(if a b false)`, `(or a b)` = `(if a true b)`, `(not a)` = `(if a false
            // true)`. Desugared to `if` so the right operand is guarded exactly as an unselected
            // branch, and inference checks each operand as a Bool (the `if` condition + branch types
            // enforce it). No new IR node — the desugar reuses `Hir::If`.
            Some("and") if items.len() == 3 => {
                let a = self.expr(&items[1], scope);
                let b = self.expr(&items[2], scope);
                Hir::If(Box::new(a), Box::new(b), Box::new(Hir::Bool(false)))
            }
            Some("or") if items.len() == 3 => {
                let a = self.expr(&items[1], scope);
                let b = self.expr(&items[2], scope);
                Hir::If(Box::new(a), Box::new(Hir::Bool(true)), Box::new(b))
            }
            Some("not") if items.len() == 2 => {
                let a = self.expr(&items[1], scope);
                Hir::If(
                    Box::new(a),
                    Box::new(Hir::Bool(false)),
                    Box::new(Hir::Bool(true)),
                )
            }
            // A sequencing block `(do f1 f2 … fn)` yields its LAST form's value; a `(def name v)`
            // among the forms binds `name` for the forms that FOLLOW it (core-semantics.md §A
            // Declaration In A Sequencing Block Is Scoped To The Forms That Follow It). Resolved by
            // `do_seq`: a value-def prefix becomes a `let` scoping the rest; a pure prefix expression
            // is discarded (Phase 2a: only its value is dropped — side effects are a later phase).
            Some("do") if items.len() >= 2 => self.do_seq(&items[1..], scope),
            Some("let") if items.len() == 3 => self.let_form(&items[1], &items[2], scope),
            // `(fn (p…) body)` — a lambda (compile-time, transient value). Collect parameter names,
            // allocate fresh local ids, extend scope, resolve body. An immediately-applied lambda
            // `((fn (x) …) v)` already routed through the head-is-list Apply path above, so a lambda
            // resolves the same bound or immediately applied.
            Some("fn") if items.len() == 3 => self.lambda(&items[1], &items[2], scope),
            // A CALL: the head is a name bound to a module function (a local shadowing it would make
            // it a value, not a callee — Phase 2a has no first-class function values, so a local in
            // head position declines). Resolve each argument, reference the callee by index.
            Some(name) if self.index.contains_key(name) && scope.lookup(name).is_none() => {
                let func = self.index[name];
                let args = items[1..].iter().map(|a| self.expr(a, scope)).collect();
                Hir::Call { func, args }
            }
            // Applying a bare prelude entry: `(Some 5)`, `(None unit)`, `(Ok x)` — the head is a bare
            // constructor (not shadowed by a local/def). Resolve the head (→ a `Ctor`) and apply; infer
            // types the ctor as `Fn([payload], Sum)` so arity/payload errors are CDZ0201. (A qualified
            // `(Sign.Pos unit)` reads as `((. Sign Pos) unit)` and takes the head-is-list Apply path.)
            Some(name)
                if self.prelude.contains_key(name)
                    && scope.lookup(name).is_none()
                    && !self.index.contains_key(name) =>
            {
                let func = self.expr(&items[0], scope);
                let args = items[1..].iter().map(|a| self.expr(a, scope)).collect();
                Hir::Apply {
                    func: Box::new(func),
                    args,
                }
            }
            // Applying a LOCAL-bound function value: `(c unit)` where `c` was `let`-bound to a
            // constructor / function value (`(let ((c None)) (c unit))`). Resolve the head to its
            // `Local` and apply; the fold/infer reduce `Apply(<value>, args)` (a `Ctor`→`Mir::Sum`, a
            // `FuncRef`→`Call`). This is the first-class-function-value application path.
            Some(name) if scope.lookup(name).is_some() => {
                let func = self.expr(&items[0], scope);
                let args = items[1..].iter().map(|a| self.expr(a, scope)).collect();
                Hir::Apply {
                    func: Box::new(func),
                    args,
                }
            }
            _ => Hir::Error(Reject::decline("unsupported form (Phase 2a)")),
        }
    }

    /// Member access `(. value key)` — the ONE accessor (core-semantics.md §Member Access). The key's
    /// KIND selects the projection: an INTEGER key `(. t 0)` is POSITIONAL access (a tuple element —
    /// the unification of the old `tuple.N` accessor into ordinary member access); a NAME key `(. r f)`
    /// is a record field — including a PRELUDE MODULE's field (`(. Int64 max)`, `(. Int64 wrapping-add)`),
    /// which is ordinary record projection since a built-in module resolves to a record (no special
    /// case for `Int64.max`/`.min` — they are record fields like any other).
    fn member(&mut self, items: &[Node], scope: &Scope) -> Hir {
        // Positional access: an integer key projects a tuple element.
        if let Node::Int(n) = &items[2] {
            if *n < 0 {
                return Hir::Error(Reject::coded(Code::TypeError, "negative tuple index"));
            }
            return Hir::TupleProj(*n as usize, Box::new(self.expr(&items[1], scope)));
        }
        // A `(meta …)` key — the module-metadata channel (capabilities/entry). Realized with effects
        // (the corpus metadata cases are `(needs effects)`); a later phase, so DECLINE for now.
        if let Node::List(k) = &items[2] {
            if k.first().and_then(name_of) == Some("meta") {
                return Hir::Error(Reject::decline("module metadata access is a later phase"));
            }
        }
        // Named access: a record field `(. r f)`. The field name is the key; the operand is the record.
        let field = match name_of(&items[2]) {
            Some(f) => f,
            None => return Hir::Error(Reject::decline("member-access key is not a name or index")),
        };
        // Member access on a bare NAME that is neither a local nor a known function is a PRELUDE access
        // (a sum-type constructor, a built-in-module operation) or an unbound operand — resolved by ONE
        // lookup in the prelude map, no per-name special-case. If `obj` names a prelude RECORD (a sum's
        // constructor record OR a built-in module's operation record — both are just records in the one
        // map), a CARRIED field resolves STRAIGHT to its value: a `Hir::Ctor` for a sum variant
        // (`(. Node NLit)` = the same node as bare `NLit`, uniform for construction + match patterns),
        // or the field's `Hir::Intrinsic`/constant for a module op (`(. List push)` = a direct
        // intrinsic, so `lower` sees the op and threads solved types). A field the PARTIAL record does
        // NOT carry is a not-yet-realized member (a variant not declared, or a method like
        // `Option.expect`/`Ast.decode`) → DECLINE a later phase, never a wrong emit
        // (decline-don't-miscompile). An `obj` not in the prelude at all is an unbound operand.
        if let Node::Name(obj) = &items[1] {
            let is_local_or_def =
                scope.lookup(obj).is_some() || self.index.contains_key(obj.as_str());
            if !is_local_or_def {
                match self.prelude.get(obj.as_str()) {
                    Some(Hir::Record(fields)) => match fields.iter().find(|(n, _)| n == field) {
                        Some((_, v)) => return v.clone(),
                        None => {
                            return Hir::Error(Reject::decline(format!(
                                "`{obj}.{field}` is a built-in operation a later phase realizes"
                            )))
                        }
                    },
                    // A prelude entry that is not a record (e.g. `unit`) — fall through to projection,
                    // which infer rejects as member access on a non-record (CDZ0201).
                    Some(_) => {}
                    // Not in the prelude — an unbound operand / unrealized built-in module.
                    None => {
                        return Hir::Error(Reject::decline(format!(
                            "member access on `{obj}` (a built-in module or unbound operand) is a later phase"
                        )));
                    }
                }
            }
        }
        Hir::RecordProj(field.to_string(), Box::new(self.expr(&items[1], scope)))
    }

    /// Resolve `(match scrutinee (pattern body)…)` → `Hir::Match`. A pattern is resolved by
    /// `resolve_arm` exactly like an expression (heads → `Ctor`/`RecordProj`, `_` → `Wildcard`), with
    /// its binders introduced into scope like a `let`.
    fn match_form(&mut self, items: &[Node], scope: &Scope) -> Hir {
        let scrutinee = self.expr(&items[1], scope);
        let mut arms: Vec<(Hir, Hir)> = Vec::new();
        for arm in &items[2..] {
            let a = match arm {
                Node::List(a) if a.len() == 2 => a,
                _ => {
                    return Hir::Error(Reject::decline("a match arm is not a (pattern body) pair"))
                }
            };
            arms.push(self.resolve_arm(&a[0], &a[1], scope));
        }
        Hir::Match {
            scrutinee: Box::new(scrutinee),
            arms,
        }
    }

    /// Resolve one `(pattern, body)` arm: introduce the pattern's binders into the scope (like a `let`),
    /// then resolve the pattern AND the body under that extended scope with plain `expr`. Returns the
    /// resolved `(pattern-Hir, body-Hir)`.
    fn resolve_arm(&mut self, pat_node: &Node, body_node: &Node, scope: &Scope) -> (Hir, Hir) {
        let binders = self.pattern_binders(pat_node);
        let ids: Vec<u32> = binders.iter().map(|_| self.fresh_local()).collect();
        let named: Vec<(&str, u32)> = binders.iter().copied().zip(ids).collect();
        let pat = resolve_with_params(self, &named, pat_node, scope);
        let body = resolve_with_params(self, &named, body_node, scope);
        (pat, body)
    }

    /// A pattern's BINDER names — the fresh names it introduces. A bare `Name` is a binder UNLESS it is
    /// `_` (a wildcard) or a known CONSTRUCTOR (a bare nullary ctor like `None`, which the corpus writes
    /// as a pattern — the ONE lookup a pattern needs, answered by the single prelude map). A `(head
    /// sub…)` recurses into its sub-patterns (the head is a constructor / `tuple` / `(. T V)`, never a
    /// binder). This is the sole syntactic fact needed before a pattern resolves like an expression.
    fn pattern_binders<'n>(&self, node: &'n Node) -> Vec<&'n str> {
        let mut out = Vec::new();
        self.collect_binders(node, &mut out);
        out
    }

    fn collect_binders<'n>(&self, node: &'n Node, out: &mut Vec<&'n str>) {
        match node {
            // A bare leaf name is a binder unless it is `_` (wildcard) or a known constructor (a bare
            // nullary ctor like `None`, which the corpus writes as a pattern — the one lookup a pattern
            // needs, answered by the single prelude map).
            Node::Name(n)
                if n != "_" && !matches!(self.prelude.get(n.as_str()), Some(Hir::Ctor { .. })) =>
            {
                if !out.contains(&n.as_str()) {
                    out.push(n.as_str());
                }
            }
            // A `(head sub…)` pattern: recurse the SUB-patterns only — the head is never a binder. It is
            // a constructor (`Some`), the `tuple` keyword, or a qualified constructor `(. T V)` (which,
            // as `items[0]`, is skipped here — so `(Foo.Bar x)` = `((. Foo Bar) x)` collects only `x`,
            // nesting to any depth). No `.` special case: recurse `items[1..]`, never `items[0]`.
            Node::List(items) if !items.is_empty() => {
                for sub in &items[1..] {
                    self.collect_binders(sub, out);
                }
            }
            _ => {}
        }
    }

    /// Resolve a `(record (k0 v0) … (kn vn))` literal. Each entry must be a `(name value)` pair; the
    /// field names are a SET (a repeat, adjacent or not, is CDZ0201). Fields are kept in SOURCE order
    /// here — inference/lowering sort them into the canonical (name-sorted) slot order.
    fn record(&mut self, items: &[Node], scope: &Scope) -> Hir {
        let mut fields: Vec<(String, Hir)> = Vec::new();
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for entry in &items[1..] {
            // Each entry is a two-element list `(name value)`.
            let (name, value_node) = match entry {
                Node::List(kv) if kv.len() == 2 => match name_of(&kv[0]) {
                    Some(n) => (n.to_string(), &kv[1]),
                    None => {
                        return Hir::Error(Reject::coded(
                            Code::TypeError,
                            "a record field name is not an identifier",
                        ))
                    }
                },
                _ => {
                    return Hir::Error(Reject::coded(
                        Code::TypeError,
                        "a record entry is not a (name value) pair",
                    ))
                }
            };
            // The field names are a set: a duplicate (adjacent or not) is ill-typed.
            if !seen.insert(name.clone()) {
                return Hir::Error(Reject::coded(
                    Code::TypeError,
                    format!("record names the field `{name}` more than once"),
                ));
            }
            fields.push((name, self.expr(value_node, scope)));
        }
        Hir::Record(fields)
    }

    /// Resolve a `(do …)` sequence's forms. The block yields its LAST form; a `(def name v)` value
    /// declaration binds `name` for the forms that FOLLOW it (scoped like a `let`). A non-last form
    /// that is NOT a declaration is a pure prefix expression whose value is discarded (Phase 2a has no
    /// effects, so a discarded pure value is a no-op — a later phase threads effect order). A trailing
    /// `(def …)` (a declaration as the block's value) has no value → decline.
    fn do_seq(&mut self, forms: &[Node], scope: &Scope) -> Hir {
        match forms.split_first() {
            None => Hir::Error(Reject::decline("empty do block")),
            // The last form is the block's value.
            Some((last, [])) => {
                if is_value_def(last).is_some() {
                    return Hir::Error(Reject::decline(
                        "a do block's last form is a declaration (no value)",
                    ));
                }
                self.expr(last, scope)
            }
            Some((first, rest)) => {
                // A leading value-def `(def name v)` scopes `name` over the rest (a `let`).
                if let Some((name, value_node)) = is_value_def(first) {
                    let value = self.expr(value_node, scope);
                    let id = self.fresh_local();
                    let inner = Scope::Bind {
                        name,
                        id,
                        parent: scope,
                    };
                    let body = self.do_seq(rest, &inner);
                    return Hir::Let {
                        id,
                        value: Box::new(value),
                        body: Box::new(body),
                    };
                }
                // A pure prefix expression whose value is discarded — but it MUST still scope/type-
                // check (an unbound name in a discarded form is still rejected, 02-binding-and-control).
                // Bind it to a throwaway local so it flows through inference and its value is dropped;
                // the throwaway is never referenced, so the discard is a real drop. This keeps a
                // prefix `Hir::Error` from being silently swallowed.
                let prefix = self.expr(first, scope);
                let id = self.fresh_local();
                let body = self.do_seq(rest, scope);
                Hir::Let {
                    id,
                    value: Box::new(prefix),
                    body: Box::new(body),
                }
            }
        }
    }

    fn let_form(&mut self, binds: &Node, body: &Node, scope: &Scope) -> Hir {
        let binds = match binds {
            Node::List(b) => b,
            _ => return Hir::Error(Reject::decline("malformed let bindings")),
        };
        self.let_chain(binds, body, scope)
    }

    fn let_chain(&mut self, binds: &[Node], body: &Node, scope: &Scope) -> Hir {
        match binds.split_first() {
            None => self.expr(body, scope),
            Some((first, rest)) => {
                let (name, value_node) = match first {
                    Node::List(kv) if kv.len() == 2 => match name_of(&kv[0]) {
                        Some(n) => (n, &kv[1]),
                        None => {
                            return Hir::Error(Reject::decline("let binding name is not a name"))
                        }
                    },
                    _ => return Hir::Error(Reject::decline("malformed let binding")),
                };
                let value = self.expr(value_node, scope);
                let id = self.fresh_local();
                let inner = Scope::Bind {
                    name,
                    id,
                    parent: scope,
                };
                let body_hir = self.let_chain(rest, body, &inner);
                Hir::Let {
                    id,
                    value: Box::new(value),
                    body: Box::new(body_hir),
                }
            }
        }
    }

    /// `(fn (p…) body)` — resolve a lambda. Collect parameter names (a non-name declines), allocate a
    /// fresh local id per param, chain a `Scope::Bind` frame per param (reusing `resolve_with_params`),
    /// resolve body under the extended scope, return `Hir::Lambda`.
    fn lambda(&mut self, param_form: &Node, body: &Node, scope: &Scope) -> Hir {
        let param_names = match param_form {
            Node::List(plist) => plist,
            _ => return Hir::Error(Reject::decline("lambda parameter form is not a list")),
        };
        let mut params: Vec<(&str, u32)> = Vec::new();
        for p in param_names {
            match name_of(p) {
                Some(pn) => {
                    let id = self.fresh_local();
                    params.push((pn, id));
                }
                None => return Hir::Error(Reject::decline("lambda parameter is not a name")),
            }
        }
        let param_ids: Vec<u32> = params.iter().map(|(_, id)| *id).collect();
        let body_hir = resolve_with_params(self, &params, body, scope);
        Hir::Lambda {
            params: param_ids,
            body: Box::new(body_hir),
        }
    }
}

/// Is `form` a value declaration `(def name value)` (a NAME head, not a `(name params…)` function
/// signature)? Returns `(name, value-node)`. A `(def name (doc …) value)` is NOT matched here (a
/// doc'd value-def in a do block is a later nicety); a function def `(def (f …) …)` returns `None`.
fn is_value_def(form: &Node) -> Option<(&str, &Node)> {
    if let Node::List(items) = form {
        if items.first().and_then(name_of) == Some("def") && items.len() == 3 {
            if let Some(name) = items.get(1).and_then(name_of) {
                return Some((name, &items[2]));
            }
        }
    }
    None
}

/// Whether a token that reached resolution as a NAME is numeric IN SHAPE — a digit-led decimal, a
/// `0x`/`0b` radix literal, or a float-shaped token — so a token the reader left unparsed (out of
/// range, or a malformed digit-separator/float shape) is reported as a malformed literal (CDZ0201)
/// rather than an unbound name (CDZ0101). Mirrors the old compiler's `looks_like_numeric_literal`
/// (01-literals.sexp §the malformed-literal cases); the reader deliberately hands such a token
/// through as a `Node::Name` for the front end to diagnose here.
fn looks_like_numeric_literal(tok: &str) -> bool {
    let body = tok
        .strip_prefix('-')
        .or_else(|| tok.strip_prefix('+'))
        .unwrap_or(tok);
    // Radix-prefixed (`0x…`/`0b…`) — numeric in shape; if unparsed it is out of the Int64 range.
    if let Some(radix_body) = body.strip_prefix("0x") {
        return !radix_body.is_empty()
            && radix_body
                .chars()
                .all(|c| c.is_ascii_hexdigit() || c == '_');
    }
    if let Some(radix_body) = body.strip_prefix("0b") {
        return !radix_body.is_empty()
            && radix_body.chars().all(|c| c == '0' || c == '1' || c == '_');
    }
    // Float-shaped: digit-led, containing `.`/`e`/`E`, built only from the float character set.
    if body.chars().next().is_some_and(|c| c.is_ascii_digit())
        && (body.contains('.') || body.contains('e') || body.contains('E'))
        && body
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-' | '_'))
    {
        return true;
    }
    // Decimal-shaped: digit-led, digits and separators only.
    body.chars().next().is_some_and(|c| c.is_ascii_digit())
        && body.chars().all(|c| c.is_ascii_digit() || c == '_')
}

fn name_of(node: &Node) -> Option<&str> {
    match node {
        Node::Name(s) => Some(s.as_str()),
        _ => None,
    }
}
