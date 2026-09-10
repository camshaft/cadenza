//! Top-level AST scanning + declaration collection + default-numeric-literal classification/marking +
//! type normalization helpers — the free functions that build a `Db`'s scan-time indexes
//! (`scan_top_level`, `scan_type_decl`/`scan_effect_decl`, the `collect_*`/`mark_*` literal &
//! overflow-mode passes, `top_items`, `normalize_embedded_sums`, `subst_template_vars`, ...). Extracted
//! verbatim from `db.rs` to keep it under `xtask_support::MAX_SOURCE_BYTES` (512 KiB); pure code move,
//! behavior-neutral. `use super::*` brings the parent `db` module's items (Db, Arenas, StructId, the
//! decl structs, ...) into scope. Private fns are `pub(super)` so the parent's `use scan::*;` re-imports
//! them (call sites in db.rs unchanged); the three cross-module `pub(crate)` fns keep that visibility and
//! are re-exported by the parent so `crate::db::<fn>` paths (sums.rs, ...) still resolve.
use super::*;

/// The one cheap top-level scan: gather the definitions, export requests, and `(type …)` declarations
/// from the top form only, without entering any body. Recognizes `(module NAME item…)`, a bare
/// `(do item…)`, or a lone item.
pub(super) fn scan_top_level(ast: &Arenas) -> TopScan {
    let mut defs: Vec<Def> = Vec::new();
    let mut exports: Vec<Export> = Vec::new();
    let mut types: Vec<TypeDecl> = Vec::new();
    let mut effects: Vec<EffectDecl> = Vec::new();
    let mut modules: Vec<ModuleDecl> = Vec::new();

    for item in top_items(ast) {
        if let Some(tail) = ast.as_form(item, "def") {
            // Two def SHAPES share the top-level scan (the same two `resolve::do_def_binds` reads inside a
            // `do`): a FUNCTION/nullary def `(def (NAME param…) BODY)` whose signature is a LIST, and a
            // VALUE def `(def NAME VALUE)` whose signature is a bare NAME atom (`(def answer 42)` — the
            // name-plus-value form a module/top-level uses for a constant binding). A value def has NO
            // parameters; its `VALUE` is the body. Distinguished by whether the first element is a list
            // (a signature) or an atom (a value-def name).
            let (name, params) = match tail.first().map(|&s| (s, ast.get(s))) {
                Some((_, Struct::List(children))) if !children.is_empty() => {
                    let name = ast.as_name(children[0]).unwrap_or("").to_string();
                    (name, children[1..].to_vec())
                }
                // `(def NAME VALUE)` — a bare-name VALUE definition (no parameters).
                Some((sig, Struct::Atom(_))) => {
                    (ast.as_name(sig).unwrap_or("").to_string(), Vec::new())
                }
                _ => (String::new(), Vec::new()),
            };
            let sig_occ = tail.first().copied().unwrap_or(item);
            let body = tail.get(1).copied();
            defs.push(Def {
                name,
                sig_occ,
                params,
                body,
                internal: false,
            });
        } else if let Some(tail) = ast.as_form(item, "export") {
            // An `(export a b …)` clause exports EVERY name in its tail — the multi-name surface the ML
            // reader writes `export { a, b, … }` and the printer round-trips (per `is_export_shape`). One
            // `Export` per name, each sharing the clause `occ` but carrying its OWN `name_occ`. A non-name
            // element (a malformed `(export a 5)`) is caught by the well-formedness pass; skip it here so
            // the well-formed names still register (matching how `scan_type_decl`/`scan_effect_decl` scan
            // per element). Reading only `tail.first()` — the prior behavior — SILENTLY dropped every name
            // past the first, so a valid `(export main helper)` published only `main`.
            for &s in tail.iter() {
                // A CONSTRUCTOR-EXPORT element — the list `(. T A)` / `(. T *)` (`as_name` is `None`, so
                // it was already skipped) OR the wildcard ATOM `T.*` (`as_name` is `Some("T.*")`, so it
                // WOULD be pushed as a bare export naming no definition → a misleading CDZ0101). Route both
                // to `ctor_export_elements`' semantic validation, not the bare-export path.
                if is_ctor_export_shape(ast, s) {
                    continue;
                }
                if let Some(name) = ast.as_name(s) {
                    exports.push(Export {
                        name: name.to_string(),
                        def: None,
                        occ: item,
                        name_occ: s,
                    });
                }
            }
        } else if ast.as_form(item, "type").is_some()
            && let Some(decl) = scan_type_decl(ast, item)
        {
            types.push(decl);
        } else if ast.as_form(item, "effect").is_some()
            && let Some(decl) = scan_effect_decl(ast, item)
        {
            effects.push(decl);
        }
    }

    // NESTED type/effect declarations — a `(type …)` / `(effect …)` written inside a `do` block that is
    // NOT a top-level item (`(def (main) (do (type Color …) …))`, a local sum). `top_items` sees only the
    // root's direct children, so descend the def bodies + any nested `do` blocks to gather these too.
    // Their identity is nominal (the declaration occurrence), so a synthesized nested sum resolves through
    // the same occurrence-keyed `type_decls` / `def_by_name` paths as a top-level one — a nested type is
    // brought into scope program-wide (well-formed local declarations do not collide by name, so global
    // visibility is a safe over-approximation of "scoped to its `do`"). A do-local `(def …)` is bound
    // lazily by `resolve`'s do-case; a nested `(type …)` needs the sum RECORD synthesized here, which is
    // why the declaration must be gathered at load rather than resolved on demand.
    let top: std::collections::HashSet<StructId> = top_items(ast).into_iter().collect();
    for &body in defs.iter().filter_map(|d| d.body.as_ref()) {
        collect_nested_decls(ast, body, &top, &mut types, &mut effects, &mut modules);
    }
    // A `(module …)` that is a TOP-LEVEL ITEM — an element of a top-level `(do …)` sequence root, `(do
    // (module m …) (def (main) …) (export main))`. The main scan loop above handles only def/export/type/
    // effect items (no `module` branch), and `collect_nested_decls` SKIPS a `top`-set form (it treats it as
    // "already scanned"), so such a module was registered by NEITHER path — its name stayed unbound and its
    // members escaped type-checking (a bare `(module m …)` and a def-body-nested one both register + check).
    // Register each top-level module item here via the shared `collect_module_decl` (which also descends its
    // members for deeper nesting), matching the do-local / bare-module paths. A module reached as a def
    // body's `(do …)` element is already handled by the `collect_nested_decls` descent above.
    for &item in &top {
        if ast.as_form(item, "module").is_some() {
            collect_module_decl(ast, item, &top, &mut types, &mut effects, &mut modules);
        }
    }

    // A DECLARED type name is NOT an implicit type parameter, even lowercase. `collect_type_params`
    // captures every free lowercase payload name as a tyvar (the `a` in `(type Box (W a))`), following
    // the "types are Capitalized, lowercase is a type variable" convention. But a type is a VALUE, so a
    // declared type name — of ANY case — referenced in a field is a reference to that type, not a fresh
    // variable: `(type mylist (Nil) (Cons Int64 mylist))` self-references `mylist`, and `(type wrap (W
    // num))` over a declared `(type num …)` references `num`. Without this the reference re-lexed as a
    // tyvar, the sum silently became generic, and its variants failed to resolve (a confusing CDZ0203
    // far from the cause). Now that ALL type names are gathered (top-level + nested), drop any param that
    // names a declared type — so a lowercase self/cross type reference resolves to the type (step 3 of
    // `resolve_name`) instead of being captured. A genuine tyvar (`a`, matching no declaration) is kept.
    // (Runs here, after the full gather, because a forward/self reference needs the whole type-name set.)
    let declared_type_names: std::collections::HashSet<String> =
        types.iter().map(|t| t.name.clone()).collect();
    for t in &mut types {
        t.params.retain(|p| !declared_type_names.contains(p));
    }

    // Resolve each export's target index by name against the gathered defs (a signature read, not a
    // body read).
    // Resolve each export to the def it names. A per-export `defs.iter().position(|d| d.name == …)`
    // is an O(defs) STRING-comparison scan, so N exports over N defs is O(N²) memcmp (the dominant
    // cost of loading a many-export program). Build a `name → first def index` map once, then each
    // export resolves in O(1). First-wins matches `position`'s first-match (a duplicate def name keeps
    // the earlier def, which the well-formedness pass reports separately).
    let mut def_of_name: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (i, d) in defs.iter().enumerate() {
        def_of_name.entry(d.name.as_str()).or_insert(i);
    }
    for e in &mut exports {
        e.def = def_of_name.get(e.name.as_str()).copied();
    }

    (defs, exports, types, effects, modules)
}

/// Scan an `(effect NAME (op f (-> A B)) …)` declaration at `item` into an [`EffectDecl`] — the effect
/// name and each operation (its name occurrence + type occurrence). `synth` is left `None` (filled by
/// `effects::synthesize`). Returns `None` if `item` is not an `(effect …)` form. The effect analogue of
/// `scan_type_decl`. A malformed operation clause (not `(op NAME TYPE)`) is skipped; the name may be
/// empty (malformed `(effect)`), in which case it binds nothing.
pub(crate) fn scan_effect_decl(ast: &Arenas, item: StructId) -> Option<EffectDecl> {
    let tail = ast.as_form(item, "effect")?;
    let name = tail
        .first()
        .and_then(|&s| ast.as_name(s))
        .unwrap_or("")
        .to_string();
    let mut ops = Vec::new();
    for &clause in tail.iter().skip(1) {
        // Each operation is `(op NAME TYPE)`. The op keyword names the clause; the first tail element is
        // the operation name, the second its type expression `(-> Param Result)`.
        let Some(op_tail) = ast.as_form(clause, "op") else {
            continue;
        };
        let Some(&name_occ) = op_tail.first() else {
            continue;
        };
        let op_name = ast.as_name(name_occ).unwrap_or("").to_string();
        // The OPTIONAL 3rd op-child is the `@resource`-marker sibling `(resource <idx>)` (v-syntax
        // 2026-08-13): the 0-based param position designated the SEC-F1 resource. Absent for a target-free
        // op. Read the index off the `resource` form's Int atom (rcdzc's `as_int` → `usize`); a malformed
        // sibling (no int) leaves `resource = None`, so a garbled marker degrades to target-free rather
        // than mis-indexing.
        let resource = op_tail
            .get(2)
            .and_then(|&s| ast.as_form(s, "resource"))
            .and_then(|res_tail| res_tail.first().copied())
            .and_then(|idx_occ| ast.as_int(idx_occ))
            .and_then(|iv| iv.to_i64())
            .and_then(|n| usize::try_from(n).ok());
        ops.push(OpDecl {
            name: op_name,
            name_occ,
            ty: op_tail.get(1).copied(),
            resource,
        });
    }
    Some(EffectDecl {
        name,
        occ: item,
        ops,
        synth: None,
    })
}

/// Scan a `(type NAME variant…)` declaration at `item` into a [`TypeDecl`] — the name, each variant
/// (its name occurrence + payload type occurrences), and the implicit type parameters (free lowercase
/// payload names, first-appearance order). `synth` is left `None` (filled by `sums::synthesize`).
/// Returns `None` if `item` is not a `(type …)` form. Shared by the top-level scan and the built-in
/// prelude-sum synthesis, so a prelude `Option`/`Result` declaration scans exactly like a user one.
pub(crate) fn scan_type_decl(ast: &Arenas, item: StructId) -> Option<TypeDecl> {
    let tail = ast.as_form(item, "type")?;
    // The type NAME is the first tail element in TWO spellings: a BARE atom `(type Box (Mk a))` — the name
    // is the atom — OR a PARENTHESIZED head `(type (Box a b…) …)` — a `(Name params…)` list whose HEAD atom
    // is the name and whose tail are the declared type parameters. Both are canonical in the corpus (`(type
    // Box (Box a))` and `(type (Box a) (Full a) (Nil unit))`). Without the list-head case, `as_name` on the
    // `(Box a)` list returned `None` → the name defaulted to `""`, so the type was registered under the
    // empty string: it worked in VALUE position (resolved by its VARIANT names) but was UNRESOLVABLE by
    // NAME in a TYPE-EXPRESSION position — `(: b (Box Int64))` / `(: b Box)` / a `(Wrap (Box Int64))`
    // payload reported CDZ0101 "unknown type `Box`", while the built-in `(Option Int64)` (prelude) worked.
    let head = tail.first().copied();
    // The type NAME via the shared decoder (bare atom `(type Box …)` OR parenthesized `(type (Box a) …)`
    // head), so every raw `(type …)`-tail name-reader agrees (see `Arenas::type_decl_head_name`).
    let name = head
        .and_then(|s| ast.type_decl_head_name(s))
        .unwrap_or("")
        .to_string();
    // Explicit HEAD params from a parenthesized `(Name params…)` head — the lowercase tail atoms, in
    // first-appearance order, DE-DUPED (a `(type (Box a a) …)` names `a` once — mirrors `collect_type_params`
    // below, so `decl.params.len()` is the true arity, not an overcount). `unit` is the value/type, never a
    // param. Empty for a bare-atom head. Payload-implied params are appended (also de-duped) below.
    let head_params: Vec<String> = match head.map(|s| ast.get(s)) {
        Some(Struct::List(kids)) => {
            let mut ps: Vec<String> = Vec::new();
            for &p in kids.iter().skip(1) {
                if let Some(n) = ast.as_name(p)
                    && n.starts_with(|c: char| c.is_ascii_lowercase())
                    && n != "unit"
                    && !ps.iter().any(|q| q == n)
                {
                    ps.push(n.to_string());
                }
            }
            ps
        }
        _ => Vec::new(),
    };
    // An OPEN sum ends in a trailing `.. r` row-variable marker (`type-system.md §204/§208`): `(type T
    // (Known Int64) .. r)`. The `..` token already lexes (list-rest); here it marks the sum OPEN and `r`
    // (a lowercase name) is the row variable. Detect it as the two FINAL elements of the `(type …)` tail
    // and STRIP them so they are not mistaken for two nullary variants. Closed stays the default — a tail
    // with no trailing `.. name` yields `open_tail: None` (every existing corpus sum is unchanged). The
    // row variable's name is NOT registered as a type parameter (it stands for the open variant set, not a
    // payload-position generic), so `collect_type_params` below never sees it (it scans only payloads).
    let mut items: Vec<StructId> = tail.iter().skip(1).copied().collect();
    let open_tail = {
        let n = items.len();
        // An open row ends in a rest naming a lowercase ROW VARIABLE — the flat `.. rowvar` (two trailing
        // items) or the wrapped `(.. rowvar)` node (one). `rest_marker` recognizes both; `trailing_start ==
        // n` requires it to be the FINAL element (a row var binds the open tail), and `truncate(k)` drops
        // from the marker index (flat: `..`+rowvar = 2; wrapped: the `(.. rowvar)` node = 1).
        if let Some((k, rowvar_occ, trailing_start)) = ast.rest_marker(&items)
            && trailing_start == n
            && ast
                .as_name(rowvar_occ)
                .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_lowercase()))
        {
            let rowvar = ast.as_name(rowvar_occ).unwrap().to_string();
            items.truncate(k);
            Some(rowvar)
        } else {
            None
        }
    };
    let mut variants = Vec::new();
    for &v in &items {
        // A leading `(doc "…")` metadata form (the ML reader attaches a `///` doc comment on a type as a
        // `(doc …)` form after the type NAME — `parser.rs` `type_expr`) is NOT a variant: skip it, exactly
        // as `strip_def_docs` drops a leading doc on a `(def …)`. Without this, `///`-documented type
        // declarations mis-parse the doc as a bogus `doc` variant (CDZ0201 "declared more than once").
        if ast.as_form(v, "doc").is_some() {
            continue;
        }
        let (name_occ, payloads) = match ast.get(v) {
            // A bare nullary variant name — no payloads.
            Struct::Atom(_) => (v, Vec::new()),
            // `(vname payload…)` — the variant name is the list head; the rest are payload type
            // occurrences in declaration order.
            Struct::List(children) => match children.first() {
                Some(&head) => (head, children.iter().skip(1).copied().collect()),
                None => continue,
            },
        };
        if let Some(vname) = ast.as_name(name_occ) {
            variants.push(Variant {
                name: vname.to_string(),
                name_occ,
                payloads,
                ctor: None,
            });
            // (`ctor` is filled by `sums::synthesize` once the record is built.)
        }
    }
    // Collect the type parameters. EXPLICIT head params (`(type (Box a) …)` → `["a"]`) come first, in the
    // order declared; then the IMPLICIT payload params — a free LOWERCASE name in any variant payload, in
    // first-appearance order (`(type Option (Some a) None)` → `["a"]`; a Capitalized name is a type). A
    // param already named in the head is not re-added (a head-declared `a` mentioned again in a payload is
    // one param). This keeps a HEAD-ONLY (phantom) param and de-dups the common case where the head param
    // also appears in a payload.
    let mut params: Vec<String> = head_params;
    for variant in &variants {
        for &p in &variant.payloads {
            collect_type_params(ast, p, &mut params);
        }
    }
    Some(TypeDecl {
        name,
        occ: item,
        params,
        variants,
        open_tail,
        synth: None,
        // A scanned decl (user OR prelude) declares no associated members here; the built-in `Ast`'s are
        // attached in `sums::prelude_decls` (prelude-defined), consumed generically by `sum_record`.
        associated: Vec::new(),
        // Set by `collect_module_decl` for a module-MEMBER type; a top-level / `do`-nested scan leaves None.
        module_scope: None,
    })
}

/// Collect the IMPLICIT type parameters mentioned in a payload type expression at `occ`, appending each
/// new one to `params` in first-appearance order. A parameter is a free LOWERCASE name (types are
/// Capitalized — `Int64`, `Bool`, `Option` — so a lowercase name in type position is a type variable,
/// the same convention the prelude's operator type-lambdas use with `a`). Descends a type application
/// `(Option a)` / `(Tuple a b)` into its arguments. A duplicate is not re-added (a `HashSet`-like
/// linear check keeps the small param list ordered by first appearance).
///
/// WARNING: A `(Record (field Type)…)` payload's field NAME is a LABEL, not a type expression — a lowercase
/// field name (`(Record (v Int64))`) must NOT be mistaken for a type parameter, or the sum spuriously
/// becomes generic over `v` and its ctor arrow breaks (the payload reads as an unresolvable variable, so
/// the variant looks nullary). So a `(Record …)` form descends only into each field pair's TYPE (the
/// second element), skipping the name — the same asymmetry `reduce_ctor`/`decode_ty` apply to a record
/// field pair. Every OTHER form (`Tuple`/`List`/`Option`/`->`) descends all children uniformly.
pub(super) fn collect_type_params(ast: &Arenas, occ: StructId, params: &mut Vec<String>) {
    match ast.get(occ) {
        Struct::Atom(_) => {
            if let Some(n) = ast.as_name(occ)
                && n.starts_with(|c: char| c.is_ascii_lowercase())
                // `unit` is the prelude UNIT value/type (the empty product), NOT a type PARAMETER. Without
                // this, a variant payload `(None unit)` / `(Nil unit)` harvested `unit` as a spurious type
                // param, so `(type (Box a) (Full a) (Nil unit))` read as 2-parameter `[a, unit]` → `(Full 1)`
                // typed `Sum{Box, args:[Int64, <free Var>]}` (the unfilled phantom); the stray free Var left
                // the sum non-Eq/non-Ord → a Set/Map of it DECLINED on the rust backend (wasm erased the open
                // arg + tolerated it). `unit` in a type position instead reduces to `Ty::Unit` (a concrete
                // type — see `typeval_of`'s unit arm), so the pervasive nullary-variant idiom `(None unit)`
                // (prelude.rs §"the pervasive nullary-variant idiom"; guide PatternMatching) keeps resolving
                // unchanged — it is NOT a param. The bare-atom analogue of the lowercase compound-type
                // ALIASES (`tuple`/`record`/`list`/`map`) skipped in the List arms below.
                && n != "unit"
                && !params.iter().any(|p| p == n)
            {
                params.push(n.to_string());
            }
        }
        // A `(Record (field Type)…)` type: a field NAME is a label, never a type parameter. Descend only
        // into each field pair's TYPE element, skipping the name (which may be lowercase — `(Record (v
        // Int64))` — and would otherwise be collected as a spurious param). Matches BOTH the capital
        // `Record` and the lowercase `record` ALIAS — the lowercase compound-type aliases (`tuple`/
        // `record`/`list`/`map`) are recognized type constructors in a payload-type position exactly as
        // they are in an annotation position (`(: e (record …))`), so their head is a TYPE CTOR, not a
        // type parameter. Without matching `record` here, a `(type P (Mk (record (x Int64))))` harvested
        // the lowercase head `record` (and each field label) as spurious params, making `P` bogus-generic
        // → the `Mk` ctor scheme mis-reduced and read as NULLARY (CDZ0201/0203 on construction).
        Struct::List(children)
            if matches!(
                children.first().and_then(|&h| ast.as_name(h)),
                Some("Record") | Some("record")
            ) =>
        {
            for &pair in children.iter().skip(1) {
                if let Struct::List(items) = ast.get(pair) {
                    // The field TYPE occurrence — the name is a label, skipped. A field is either a
                    // 2-element `(name Type)` pair (s-expr `(Record (v a))`) OR a 3-element `(: name Type)`
                    // annotation triple (the ML surface `{v: a}` lowering). Without the triple case, a
                    // param mentioned in an ML-surfaced record field (`(type Box (Box (Record (: v a))))`)
                    // was NOT collected → `decl.params` empty → no ctor type-lambda → the `a` in the ctor
                    // arrow stayed free → `typeval_of` gave None → the ctor read NULLARY (CDZ0201 at
                    // construction). The `collect_type_params` companion of the `(: name type)` decode the
                    // RecordCtor reducers already accept.
                    match items.as_slice() {
                        [_name, ty] => collect_type_params(ast, *ty, params),
                        [colon, _name, ty] if ast.as_name(*colon) == Some(":") => {
                            collect_type_params(ast, *ty, params);
                        }
                        _ => {}
                    }
                }
            }
        }
        // A lowercase compound-type ALIAS application whose args are all TYPES — `(tuple T…)`, `(list T)`,
        // `(map K V)`. The head (`tuple`/`list`/`map`) is a recognized type constructor (a prelude alias),
        // NOT a type parameter, so it must be SKIPPED — the capital spellings (`Tuple`/`List`/`Map`) are
        // already skipped by the lowercase filter in the `Atom` arm, but the lowercase alias head would
        // otherwise be harvested. Descend into the type arguments only (they may hold a real param, e.g.
        // `(tuple a Int64)`). Without this a `(type P (Mk (tuple Int64 Int64)))` harvested `tuple` as a
        // spurious param → `P` bogus-generic → `Mk` read as NULLARY (CDZ0201 on construction). (`record`
        // is handled above with its label-skipping; a lowercase `set` alias does not exist in the reader.)
        Struct::List(children)
            if matches!(
                children.first().and_then(|&h| ast.as_name(h)),
                Some("tuple") | Some("list") | Some("map")
            ) =>
        {
            for &arg in children.iter().skip(1) {
                collect_type_params(ast, arg, params);
            }
        }
        // A `(Qty T u)` quantity type: the FIRST argument is the inner numeric TYPE, but the SECOND is a
        // compile-time UNIT expression (`(Unit.base #"meter")`, `(Unit.* …)`) whose leaf names are UNIT
        // BASES, not type variables — `eval::QtyCtor` reads it via `unit_of` and `resolve::decode_ty`'s
        // "Qty" arm decodes it as a `Unit`, never as a type. Descending into it uniformly would harvest a
        // lowercase unit-builder name (`base` in `Unit.base`) as a spurious type parameter, so a
        // `(type Holder (H (Qty Rational (Unit.base #"meter"))))` becomes generic `Holder(base)` and a
        // bare `Holder` stops resolving (CDZ0203). Descend ONLY into the inner-type argument; skip the
        // unit. Same asymmetry `decode_ty`/`push_payload_type_positions` apply. (A malformed-arity `Qty`
        // falls through to the uniform descent below — a well-formedness fault reported elsewhere.)
        Struct::List(children)
            if children.first().and_then(|&h| ast.as_name(h)) == Some("Qty")
                && children.len() == 3 =>
        {
            collect_type_params(ast, children[1], params);
        }
        // A type application `(Head arg…)` — descend into every child (the head of a nested application
        // may itself be a name, but a Capitalized head is a type constructor, not a parameter; the
        // lowercase filter above handles that). This reaches a parameter nested in `(Option a)`.
        Struct::List(children) => {
            for &c in children {
                collect_type_params(ast, c, params);
            }
        }
    }
}

/// The top-level item occurrences: the tail of `(module NAME …)` (past the name), the tail of
/// `(do …)`, or the root as a single item.
/// Gather `(type …)` / `(effect …)` declarations nested inside a `do` block reachable from `body` —
/// the local-declaration companion of `scan_top_level`'s top-item loop. A declaration only appears as a
/// FORM of a `do` block, so descend `do` blocks (and the bodies of any do-local `(def …)`), collecting
/// each `(type …)`/`(effect …)` form. `top` is the set of top-level items already scanned (skip them so a
/// top-level type in a `(do …)` root is not counted twice). Bounded to declaration-bearing structure
/// (`do` forms + def bodies), not arbitrary expressions, so a future quoted `(type …)` datum is not
/// mistaken for a declaration.
pub(super) fn collect_nested_decls(
    ast: &Arenas,
    body: StructId,
    top: &std::collections::HashSet<StructId>,
    types: &mut Vec<TypeDecl>,
    effects: &mut Vec<EffectDecl>,
    modules: &mut Vec<ModuleDecl>,
) {
    let Some(forms) = ast.as_form(body, "do") else {
        return;
    };
    for &form in forms {
        if top.contains(&form) {
            continue; // a top-level item (the root `do`'s own child) — already scanned
        }
        if ast.as_form(form, "type").is_some() {
            if let Some(decl) = scan_type_decl(ast, form) {
                types.push(decl);
            }
        } else if ast.as_form(form, "effect").is_some() {
            if let Some(decl) = scan_effect_decl(ast, form) {
                effects.push(decl);
            }
        } else if ast.as_form(form, "module").is_some() {
            // A do-local `(module NAME member…)` — register it and descend its members (including any
            // NESTED `(module …)`) via the shared `collect_module_decl`, which handles arbitrarily deep
            // module nesting.
            collect_module_decl(ast, form, top, types, effects, modules);
        } else if let Some(def_tail) = ast.as_form(form, "def") {
            // A do-local `(def sig body)` whose body may itself be a `(do …)` carrying declarations.
            if let Some(&def_body) = def_tail.get(1) {
                collect_nested_decls(ast, def_body, top, types, effects, modules);
            }
        } else {
            // Any other form may itself be (or contain) a nested `(do …)` — descend it directly.
            collect_nested_decls(ast, form, top, types, effects, modules);
        }
    }
}

/// Register a single `(module NAME member…)` DECLARATION and descend its members for further nested
/// declarations — the recursive core shared by `collect_nested_decls`'s do-local module branch and its
/// own recursion for a MODULE-IN-MODULE member. A `(def …)` member becomes an export FIELD; a nested
/// `(module inner …)` member is itself registered (so `inner` is a field of the outer's record — a
/// nested record) and recursed into, for arbitrarily deep module nesting. An `(effect …)`/`(op …)`/
/// `(type …)`/`(doc …)` member is a legitimate NON-export (correctly ABSENT from the record, so
/// projecting it is the closed-record CDZ0201 the corpus wants). A member the compiler does NOT model as
/// either a field or a benign non-export — a `(pragma …)` (a validation OBLIGATION not yet built) —
/// blocks registration: the module NAME stays unbound and the program DECLINES rather than silently
/// dropping the obligation (decline-don't-miscompile). The modeled set is closed; anything outside it
/// (today just `pragma`) blocks. The `all_modeled` guard is per-module, so an unmodeled member in the
/// INNER module blocks only the inner's registration, not the outer's — matching a top-level module's
/// independence.
pub(super) fn collect_module_decl(
    ast: &Arenas,
    form: StructId,
    top: &std::collections::HashSet<StructId>,
    types: &mut Vec<TypeDecl>,
    effects: &mut Vec<EffectDecl>,
    modules: &mut Vec<ModuleDecl>,
) {
    let Some(mod_tail) = ast.as_form(form, "module") else {
        return;
    };
    let members = mod_tail.get(1..).unwrap_or(&[]).to_vec();
    // A `(pragma default-integer <T>)` member is MODELED — its VALIDATION is built (CDZ0601/0602/0303,
    // `compile::collect_faults`) and its EFFECT (a bare literal in this module defaults to `<T>`) is
    // realized via `ModuleDecl::default_int` + the load-time literal map, so it is not an unmodeled
    // obligation blocking registration. A `(pragma …)` with any OTHER key is still unmodeled (blocks) —
    // the modeled set is exactly the keys whose meaning the compiler realizes.
    // A `(pragma <key> …)` member is MODELED for a key whose EFFECT the compiler realizes: `default-integer`
    // (a bare literal defaults to `<T>`), `default-fraction` (a bare numeric literal grounds to the exact
    // rational `<T>`), and `default-float` (a bare decimal literal defaults to the float type `<T>`). Each
    // is realized via a `ModuleDecl` field + a load-time literal map, so they don't block registration; any
    // OTHER pragma key stays unmodeled (blocks).
    let is_modeled_pragma = |member: StructId| {
        ast.as_form(member, "pragma").is_some_and(|t| {
            matches!(
                t.first().and_then(|&k| ast.as_name(k)),
                Some("default-integer" | "default-fraction" | "default-float" | "overflow")
            )
        })
    };
    let modeled = |member: StructId| {
        matches!(
            ast.head_name(member),
            // `export` is modeled: a `(module m … (export a b))` member NAMES the module's visible fields
            // (`modules-and-namespaces.md` §Visibility Is Explicit; the ML surface `export { a, b }` emits
            // it). `modules::module_record` reads it to filter the record; here it just must not block
            // registration. Its VALIDATION (a duplicate export, an export of an undefined name) rides the
            // ordinary well-formedness pass, exactly as a top-level `(export …)`'s does.
            Some("def" | "effect" | "op" | "type" | "module" | "doc" | "export")
        ) || is_modeled_pragma(member)
    };
    let all_modeled = members.iter().all(|&m| modeled(m));
    // The type-expression (2nd operand) of a well-formed `(pragma <key> <T>)` member for `key`. A malformed
    // pragma (wrong arity) is caught by the CDZ0602 validation; here take the type occ when present.
    let pragma_ty = |key: &str| {
        members.iter().find_map(|&m| {
            let t = ast.as_form(m, "pragma")?;
            (t.first().and_then(|&k| ast.as_name(k)) == Some(key))
                .then(|| t.get(1).copied())
                .flatten()
        })
    };
    let default_int = pragma_ty("default-integer");
    let default_fraction = pragma_ty("default-fraction");
    let default_float = pragma_ty("default-float");
    // The overflow policy — parsed from the whole `(pragma overflow (signed …) (unsigned …))` member (its
    // operands are sub-forms, not a single type-expr, so `pragma_ty` does not apply).
    let overflow = members
        .iter()
        .find(|&&m| {
            ast.as_form(m, "pragma")
                .and_then(|t| t.first().and_then(|&k| ast.as_name(k)))
                == Some("overflow")
        })
        .and_then(|&m| parse_overflow_spec(ast, m));
    if all_modeled
        && let Some(&name) = mod_tail.first()
        && let Some(name_str) = ast.as_name(name)
    {
        modules.push(ModuleDecl {
            name: name_str.to_string(),
            occ: form,
            synth: None,
            default_int,
            default_fraction,
            default_float,
            overflow,
        });
    }
    for &member in &members {
        if ast.as_form(member, "module").is_some() {
            // A MODULE-IN-MODULE member — register + descend it (a nested record field of this module).
            collect_module_decl(ast, member, top, types, effects, modules);
        } else if ast.as_form(member, "type").is_some() {
            // A module-MEMBER `(type Sh …)` — collect it so it is SYNTHESIZED (record built by
            // `sums::synthesize`) and TAG it with its enclosing module (`module_scope = form`). The tag keeps
            // it OUT of the flat global name maps (name-index loop skips it), so it resolves ONLY
            // MODULE-SCOPED via `Db::module_scoped_type` / `module_scoped_variant_ctor(_qualified)` (the
            // inline-module analogue of `file_scoped_*`, consulted first at the type/ctor resolver sites): a
            // sibling sees `Sh` / `Sh.Box` / bare `Box`, but the name does NOT leak to the global bare scope
            // and does NOT collide with another module's same-named member type. Without collection a member
            // type was never registered (this loop descended only nested modules + def bodies) → the #7946 gap.
            if let Some(mut decl) = scan_type_decl(ast, member) {
                decl.module_scope = Some(form);
                types.push(decl);
            }
        } else if let Some(def_tail) = ast.as_form(member, "def")
            && let Some(&def_body) = def_tail.get(1)
        {
            // A `(def …)` member whose body may itself carry a `(do …)` with declarations.
            collect_nested_decls(ast, def_body, top, types, effects, modules);
        }
    }
}

/// Record every bare integer-LITERAL node in the DEF-BODY subtrees of a `(pragma default-integer <T>)`
/// module `mod_form` → the pragma's `<T>` occurrence, into `out`. DEFINITION-SITE scoped: it descends
/// only the module's OWN `(def …)` member bodies (a literal in a NESTED `(module inner …)` member is
/// governed by `inner`'s OWN pragma, not this one — so nested modules are NOT descended here; each is
/// walked when `collect_default_int_literals` is called for it). Keyed by the ORIGINAL literal node, so a
/// later β-copy that reparents the literal (moving it out of the module structurally) still finds its
/// default via this map — the parent-walk a naive lookup would use is unusable post-copy.
///
/// `(pragma default-integer <T>)` is a MEANING-CHANGING directive — it changes the type its module's bare
/// literals take — and it is read HERE from the module's CANONICAL AST (`mod_form`, the pragma node in the
/// binary AST), never from a compilation option outside the form. So a module's meaning is determined by
/// its canonical form alone.
//= spec/capabilities/modules-and-namespaces.md#a-meaning-changing-directive-is-part-of-the-canonical-form
//# A module directive that changes the meaning of the module's definitions MUST be carried in the module's canonical form, so that the module's meaning is determined by its canonical form alone and does not depend on a compilation option outside it.
pub(super) fn collect_default_int_literals(
    ast: &Arenas,
    mod_form: StructId,
    ty_expr: StructId,
    out: &mut crate::fxhash::FxHashMap<StructId, StructId>,
) {
    let Some(mod_tail) = ast.as_form(mod_form, "module") else {
        return;
    };
    for &member in mod_tail.get(1..).unwrap_or(&[]) {
        // Only a `(def …)` member's BODY carries value literals to default; a nested `(module …)` has its
        // own scope (skip), and `pragma`/`type`/`effect`/`doc` members carry no value literals.
        if let Some(def_tail) = ast.as_form(member, "def")
            && let Some(&def_body) = def_tail.get(1)
        {
            mark_int_literals(ast, def_body, ty_expr, out);
        }
    }
}

/// Mark `node` and every descendant into `out` — the subtree marker behind [`Db::type_expr_nodes`]. A
/// type expression `(List Ast)` / `(-> A B)` and everything inside it is a type, never a value construct,
/// so the whole subtree is off-limits to the construct-position variant shadow. Bounded by the subtree
/// size; a node visited twice (overlapping roots cannot occur, but defensively) is idempotent.
pub(super) fn mark_subtree(
    ast: &Arenas,
    node: StructId,
    out: &mut crate::fxhash::FxHashSet<StructId>,
) {
    if !out.insert(node) {
        return;
    }
    if let Struct::List(children) = ast.get(node) {
        for &c in children {
            mark_subtree(ast, c, out);
        }
    }
}

/// Mark every integer-literal node reachable from `node` (recursively) with `ty_expr` in `out`, WITHOUT
/// descending into a nested `(module …)` (its own scope). A literal already recorded is left as-is (the
/// nearest enclosing pragma wins — the outer walk visits an inner module separately with its own type).
pub(super) fn mark_int_literals(
    ast: &Arenas,
    node: StructId,
    ty_expr: StructId,
    out: &mut crate::fxhash::FxHashMap<StructId, StructId>,
) {
    if ast.as_int(node).is_some() {
        out.entry(node).or_insert(ty_expr);
        return;
    }
    // A nested module carries its OWN default (or none) — do not leak this module's default into it.
    if ast.as_form(node, "module").is_some() {
        return;
    }
    if let Struct::List(children) = ast.get(node) {
        for &c in children {
            mark_int_literals(ast, c, ty_expr, out);
        }
    }
}

/// The `trap`/`wrap` mode named by a `(signed <mode>)` / `(unsigned <mode>)` sub-form's argument `node`, or
/// `None` if it is not one of the two mode names (a malformed mode is reported as CDZ0602 by the pragma
/// validation pass; here an unrecognized name simply yields no policy for that signedness).
pub(super) fn parse_overflow_mode(ast: &Arenas, node: StructId) -> Option<OverflowMode> {
    match ast.as_name(node)? {
        "trap" => Some(OverflowMode::Trap),
        "wrap" => Some(OverflowMode::Wrap),
        _ => None,
    }
}

/// Parse a `(pragma overflow (signed <mode>) (unsigned <mode>))` form into an [`OverflowSpec`]. Either
/// sub-form may be absent (that signedness stays `None` → falls through to the next precedence level). Order
/// is irrelevant; each sub-form is matched by its head (`signed`/`unsigned`). Returns `None` only when the
/// form carries NEITHER a well-formed `signed` nor `unsigned` sub-form (nothing to record).
pub(super) fn parse_overflow_spec(ast: &Arenas, pragma_form: StructId) -> Option<OverflowSpec> {
    let ptail = ast.as_form(pragma_form, "pragma")?;
    // Skip the `overflow` key; the rest are the `(signed …)` / `(unsigned …)` sub-forms.
    let mut spec = OverflowSpec::default();
    for &sub in ptail.get(1..).unwrap_or(&[]) {
        if let Some(t) = ast.as_form(sub, "signed") {
            spec.signed = t.first().and_then(|&m| parse_overflow_mode(ast, m));
        } else if let Some(t) = ast.as_form(sub, "unsigned") {
            spec.unsigned = t.first().and_then(|&m| parse_overflow_mode(ast, m));
        }
    }
    (spec.signed.is_some() || spec.unsigned.is_some()).then_some(spec)
}

/// Record every unqualified `+`/`-`/`*` arithmetic node in the DEF-BODY subtrees of a `(pragma overflow …)`
/// module `mod_form` → the module's `spec`, into `out`. The overflow twin of [`collect_default_int_literals`]
/// — DEFINITION-SITE scoped (descends only the module's OWN `(def …)` bodies; a nested `(module inner …)`
/// keeps its own policy), keyed by the ORIGINAL operator node so a later β-copy that reparents the op still
/// finds its policy.
pub(super) fn collect_overflow_modes(
    ast: &Arenas,
    mod_form: StructId,
    spec: OverflowSpec,
    out: &mut crate::fxhash::FxHashMap<StructId, OverflowSpec>,
) {
    let Some(mod_tail) = ast.as_form(mod_form, "module") else {
        return;
    };
    for &member in mod_tail.get(1..).unwrap_or(&[]) {
        if let Some(def_tail) = ast.as_form(member, "def")
            && let Some(&def_body) = def_tail.get(1)
        {
            mark_overflow_nodes(ast, def_body, spec, out);
        }
    }
}

/// Mark every unqualified `+`/`-`/`*` arithmetic node reachable from `node` (recursively) with `spec` in
/// `out`, WITHOUT descending into a nested `(module …)` (its own scope). A node already recorded is left
/// as-is (the nearest enclosing pragma wins — the outer walk visits an inner module separately). Only the
/// bare `+`/`-`/`*` head is matched: a named `Int64.wrapping-add` / `(. Int64 checked-add)` form is a
/// different head and is IMMUNE (it carries its own overflow contract).
pub(super) fn mark_overflow_nodes(
    ast: &Arenas,
    node: StructId,
    spec: OverflowSpec,
    out: &mut crate::fxhash::FxHashMap<StructId, OverflowSpec>,
) {
    // A nested module carries its OWN policy (or none) — do not leak this module's into it.
    if ast.as_form(node, "module").is_some() {
        return;
    }
    if let Struct::List(children) = ast.get(node) {
        // A `(+ a b)` / `(- a b)` / `(* a b)` whose HEAD is the bare operator name is a governed arithmetic
        // op. (`as_name` on the head is `None` for a dotted `(. Int64 wrapping-add)` head — never matched.)
        if let Some(&head) = children.first()
            && matches!(ast.as_name(head), Some("+" | "-" | "*"))
        {
            out.entry(node).or_insert(spec);
        }
        for &c in children {
            mark_overflow_nodes(ast, c, spec, out);
        }
    }
}

/// Mark each bare, un-suffixed integer literal written as the DIRECT argument of a constructor whose
/// corresponding DECLARED payload type is `BigInt`, so `infer` grounds it to `Ty::BigInt` instead of the
/// Int64 default (the operator-approved contextual-grounding: an integer literal grounds to BigInt
/// losslessly, and an explicit context takes precedence over the declared default — a grounding, not a
/// promotion). Keyed by the literal's ORIGINAL occurrence (β-copy-robust, like the default-literal maps).
///
/// SUFFIX DISCIPLINE FALLS OUT OF THE AST SHAPE: a suffixed `42N` reader-desugars to `(: 42 BigInt)` (an
/// annotation node), and a computed/already-typed argument is a `List`/`Annot` node — NONE is a bare
/// `Leaf::Int`. So matching a DIRECT integer-literal argument marks EXACTLY the bare, un-suffixed,
/// uncomputed case; a `42N`, a `(: 42 Int64)`, or a `(+ 1 1)` in a BigInt payload position is never marked
/// and still declines if it mismatches (the operator's load-bearing guard, enforced structurally).
pub(crate) fn collect_bigint_ctor_arg_literals(
    ast: &Arenas,
    type_decls: &[TypeDecl],
    out: &mut crate::fxhash::FxHashSet<StructId>,
) {
    // A constructor NAME → its declared payload type-expression occurrences (declaration order). A variant
    // name is the map key; a dotted head `(. Sum Variant)` matches on its `Variant` segment, a bare head on
    // the name directly. FIRST-wins on a name shared across sums (matches the resolver's own first-wins);
    // an ambiguous same-named ctor with a DIFFERENT payload type is rare and, if wrong, only misses/adds a
    // grounding the type-checker still validates (a mismatch still declines) — never unsound.
    let mut ctor_payloads: crate::fxhash::FxHashMap<&str, &[StructId]> =
        crate::fxhash::FxHashMap::default();
    for decl in type_decls {
        for v in &decl.variants {
            if !v.payloads.is_empty() {
                ctor_payloads.entry(&v.name).or_insert(&v.payloads);
            }
        }
    }
    if ctor_payloads.is_empty() {
        return; // no payload-carrying constructors → nothing to mark
    }
    // Walk EVERY form in the arena; a `(head arg…)` whose head names a known constructor marks each bare
    // integer-literal arg whose matching declared payload type is `BigInt`.
    for ix in 0..ast.structure.len() {
        let id = <StructId as crate::arena::Index>::from_ix(ix);
        let Struct::List(children) = ast.get(id) else {
            continue;
        };
        let Some((&head, args)) = children.split_first() else {
            continue;
        };
        // The constructor's variant name: a bare head `(W 42)` reads via `as_name`; a dotted head
        // `(Ast.Int 42)` is `(. Ast Int)` — take the KEY segment (2nd element of the `.` form).
        let ctor_name = ast.as_name(head).or_else(|| {
            ast.as_form(head, ".")
                .and_then(|t| t.get(1).copied())
                .and_then(|k| ast.as_name(k))
        });
        let Some(name) = ctor_name else { continue };
        let Some(payloads) = ctor_payloads.get(name) else {
            continue;
        };
        for (arg, &payload_ty) in args.iter().zip(payloads.iter()) {
            // ONLY a DIRECT bare integer literal (not an annotation, not a computed expression) whose
            // declared payload type is the bare name `BigInt`.
            if ast.as_int(*arg).is_some() && ast.as_name(payload_ty) == Some("BigInt") {
                out.insert(*arg);
            }
        }
    }
    // SLICE 2 — ANNOTATED COLLECTION ELEMENTS: `(: (list 1 2 3) (List BigInt))` grounds each bare element
    // literal to BigInt (the collection-element analogue of the ctor-payload case — v-metaprogramming's
    // hand-written `(Ast.Int N)` inside a `(list …)`). Walk every annotation `(: <value> <ty-expr>)` and
    // mark the bare integer literals the type-expr expects to be `BigInt`.
    for ix in 0..ast.structure.len() {
        let id = <StructId as crate::arena::Index>::from_ix(ix);
        if let Some(tail) = ast.as_form(id, ":")
            && let [value, ty_expr] = tail
        {
            mark_bigint_expected_literals(ast, *value, *ty_expr, out);
        }
    }
}

/// Mark each bare integer literal in `value` whose expected type (from the annotation type-expr `ty_expr`)
/// is `BigInt`. Descends the two positions where an integer literal sits under a known type-expr shape:
/// a direct `(: 42 BigInt)` (redundant with the annotation path, but harmless), and a
/// `(: (list …) (List BigInt))` whose ELEMENT type is BigInt — each bare-literal element grounds. Only a
/// DIRECT bare `Leaf::Int` is marked (a suffixed/computed/nested-annotated element is a different node and
/// keeps its own type — the same bare-only discipline the ctor-payload walk uses).
pub(super) fn mark_bigint_expected_literals(
    ast: &Arenas,
    value: StructId,
    ty_expr: StructId,
    out: &mut crate::fxhash::FxHashSet<StructId>,
) {
    // A bare `BigInt` type-expr over a bare integer literal → ground it.
    if ast.as_name(ty_expr) == Some("BigInt") {
        if ast.as_int(value).is_some() {
            out.insert(value);
        }
        return;
    }
    // A `(List BigInt)` type-expr over a list literal: ground each bare-literal element. The TYPE ctor head
    // is always the NAME `List` — the reader emits a type constructor as a name-head application `(List …)`,
    // NEVER a string-head `("List" …)` (that has no reader/ML surface; `from_spelling` is lowercase-VALUE
    // only), so `as_form` (name-head) is the sole live reader-produced arm (verified with v-syntax, M3). The
    // VALUE side (the annotated list literal) is read via `compound_form_of` below, which accepts both the
    // native `#list(…)` ctor-leaf head and the `(list …)`/`("list" …)` aliases.
    if let Some(list_ty_tail) = ast.as_form(ty_expr, "List")
        && let Some(&elem_ty) = list_ty_tail.first()
        && ast.as_name(elem_ty) == Some("BigInt")
        && let Some(elems) = ast.compound_form_of(value, CompoundCtor::List)
    {
        for &e in elems {
            if ast.as_int(e).is_some() {
                out.insert(e);
            }
        }
    }
}

/// The `default-fraction` analogue of [`collect_default_int_literals`]: record every bare NUMERIC literal
/// (integer OR decimal) in the DEF-BODY subtrees of a `(pragma default-fraction <T>)` module → the
/// pragma's `<T>` occurrence. DEFINITION-SITE scoped (own `(def …)` bodies only; a nested module has its
/// own scope). Keyed by the ORIGINAL literal node (β-copy-robust).
pub(super) fn collect_default_fraction_literals(
    ast: &Arenas,
    mod_form: StructId,
    ty_expr: StructId,
    out: &mut crate::fxhash::FxHashMap<StructId, StructId>,
) {
    let Some(mod_tail) = ast.as_form(mod_form, "module") else {
        return;
    };
    for &member in mod_tail.get(1..).unwrap_or(&[]) {
        if let Some(def_tail) = ast.as_form(member, "def")
            && let Some(&def_body) = def_tail.get(1)
        {
            mark_numeric_literals(ast, def_body, ty_expr, out);
        }
    }
}

/// Mark every NUMERIC-literal node (integer via `as_int` OR decimal via `as_float`) reachable from `node`
/// (recursively) with `ty_expr`, WITHOUT descending into a nested `(module …)`. The fraction analogue of
/// [`mark_int_literals`] — an exact default applies to a decimal `0.5` (grounding to `1/2`) as well as an
/// integer, so both leaf kinds are marked. A literal already recorded is left as-is.
pub(super) fn mark_numeric_literals(
    ast: &Arenas,
    node: StructId,
    ty_expr: StructId,
    out: &mut crate::fxhash::FxHashMap<StructId, StructId>,
) {
    if ast.as_int(node).is_some() || ast.as_float(node).is_some() {
        out.entry(node).or_insert(ty_expr);
        return;
    }
    if ast.as_form(node, "module").is_some() {
        return;
    }
    if let Struct::List(children) = ast.get(node) {
        for &c in children {
            mark_numeric_literals(ast, c, ty_expr, out);
        }
    }
}

/// The `default-float` analogue of [`collect_default_int_literals`]: record every bare DECIMAL literal in
/// the DEF-BODY subtrees of a `(pragma default-float <T>)` module → the pragma's `<T>` occurrence.
/// DEFINITION-SITE scoped (own `(def …)` bodies only; a nested module has its own scope). Keyed by the
/// ORIGINAL literal node (β-copy-robust). Like the integer twin, this is a MEANING-CHANGING directive read
/// from the module's CANONICAL AST alone, so a module's meaning does not depend on a compilation option.
pub(super) fn collect_default_float_literals(
    ast: &Arenas,
    mod_form: StructId,
    ty_expr: StructId,
    out: &mut crate::fxhash::FxHashMap<StructId, StructId>,
) {
    let Some(mod_tail) = ast.as_form(mod_form, "module") else {
        return;
    };
    for &member in mod_tail.get(1..).unwrap_or(&[]) {
        if let Some(def_tail) = ast.as_form(member, "def")
            && let Some(&def_body) = def_tail.get(1)
        {
            mark_float_literals(ast, def_body, ty_expr, out);
        }
    }
}

/// Mark every DECIMAL-literal node (via `as_float`) reachable from `node` (recursively) with `ty_expr`,
/// WITHOUT descending into a nested `(module …)`. The float analogue of [`mark_int_literals`] — a default
/// float width governs how a WRITTEN-DECIMAL literal grounds (`3.14` → `Float32`), not how an integer
/// literal does (an integer keeps its integer default), so ONLY float leaves are marked. A literal already
/// recorded is left as-is.
pub(super) fn mark_float_literals(
    ast: &Arenas,
    node: StructId,
    ty_expr: StructId,
    out: &mut crate::fxhash::FxHashMap<StructId, StructId>,
) {
    if ast.as_float(node).is_some() {
        out.entry(node).or_insert(ty_expr);
        return;
    }
    if ast.as_form(node, "module").is_some() {
        return;
    }
    if let Struct::List(children) = ast.get(node) {
        for &c in children {
            mark_float_literals(ast, c, ty_expr, out);
        }
    }
}

/// Scan EVERY `(Unit.define #"name" base-unit-expr num den)` form in the arena — a TOP-LEVEL declaration
/// OR an INLINE one in a `Qty.of` unit position — into `(name, base-unit-occurrence, scale-num, scale-den)`
/// entries, the user family-declaration surface. (Walking the whole arena, not just top-level items, is
/// what feeds an inline define the SAME name→conversion uniqueness table `check_unit_defines` consults, so
/// an inline redeclaration of a built-in ratio is rejected CDZ0502 like the top-level form — see the body.)
/// `Unit.define` reads as the member-access head `(. Unit define)` applied to four args: a symbol name,
/// a base-unit expression (reduced later by `eval::unit_of`), and an integer `num`/`den` scaling the
/// base. A malformed form (wrong arity, non-symbol name, non-integer scale) is skipped here — it
/// surfaces as an ordinary fault when the node is checked, not silently. The name's TEXT comes from the
/// `Leaf::Sym`; a non-symbol first arg is not a unit definition.
pub(super) fn scan_unit_defines(ast: &Arenas) -> Vec<(String, StructId, i128, i128)> {
    let mut out = Vec::new();
    // Scan EVERY arena node, not just top-level items: a `(Unit.define #"name" base num den)` is a
    // name→conversion declaration wherever it appears — a TOP-LEVEL statement OR an INLINE unit value in a
    // `Qty.of` unit position (`(Qty.of a (Unit.define #"foot" (Unit.base #"meter") 2 1))`). Both must feed
    // the SAME uniqueness table `check_unit_defines` consults: else an inline define silently redefines a
    // built-in unit's ratio (foot=2m) or uses one name at two ratios in one expression — the exact
    // silent-wrong-physics the CDZ0502 "A Named Unit's Conversion Is Unique" rule exists to prevent
    // (`units-of-measure.md`). Arena index order is deterministic (a node's children have higher indices
    // than… — actually the reader assigns ids in construction order), giving `check_unit_defines`'s
    // "earlier declaration"/"first-wins" a stable order. A top-level define is just a node here, scanned
    // once (no double-count). The recognized-top-level-form set (`TOP_LEVEL_FORMS` / `scan_top_level`)
    // still handles a top-level `Unit.define` separately for the "unknown top-level" decline — this scan
    // only feeds the conversion table + the known-unit set + the value-reduction lookup.
    for i in 0..ast.structure.len() {
        let item = StructId(i as u32);
        if let Some(args) = unit_member_call(ast, item, "define")
            && args.len() == 4
            && let Some(name) = ast.as_sym(args[0])
            && let Some(num) = ast.as_int(args[2]).and_then(|v| v.to_i128())
            && let Some(den) = ast.as_int(args[3]).and_then(|v| v.to_i128())
        {
            out.push((name.to_string(), args[1], num, den));
        }
    }
    out
}

/// Scan the program's `(bind Effect "cadenza:pkg/iface")` top-level directives into the effect→peer-
/// contract default map (U2, the effects-unification of cross-component interop). Each binds an escaping
/// effect NAME to a peer interface STRING — the component-scope default route for that effect. A malformed
/// `(bind …)` (wrong arity, non-name effect, non-string interface) is skipped (a diagnostic elsewhere);
/// first-wins on a duplicate effect name. Empty for a program with no `(bind …)`.
pub(super) fn scan_effect_bindings(ast: &Arenas) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for item in top_items(ast) {
        if let Some(tail) = ast.as_form(item, "bind")
            && tail.len() == 2
            && let Some(effect) = ast.as_name(tail[0])
            && let Some(iface) = ast.as_str(tail[1])
        {
            out.entry(effect.to_string())
                .or_insert_with(|| iface.to_string());
        }
    }
    out
}

/// The argument list of a `((. Unit KEY) arg…)` member-access call at `item`, or `None` if `item` is not
/// that shape. Recognizes the reader's desugaring of `(Unit.KEY arg…)` — the head is a `(. Unit KEY)`
/// list. Used to scan `Unit.define` top-level forms (and to recognize them as modeled forms so they
/// don't decline as "unknown top-level").
pub(super) fn unit_member_call<'a>(
    ast: &'a Arenas,
    item: StructId,
    key: &str,
) -> Option<&'a [StructId]> {
    let Struct::List(items) = ast.get(item) else {
        return None;
    };
    let head = *items.first()?;
    // The head must be `(. Unit KEY)`.
    let dot = ast.as_form(head, ".")?;
    if dot.len() == 2 && ast.as_name(dot[0]) == Some("Unit") && ast.as_name(dot[1]) == Some(key) {
        Some(&items[1..])
    } else {
        None
    }
}

pub(super) fn top_items(ast: &Arenas) -> Vec<StructId> {
    let root = ast.root;
    if let Some(tail) = ast.as_form(root, "module") {
        return tail.get(1..).unwrap_or(&[]).to_vec();
    }
    if let Some(tail) = ast.as_form(root, "do") {
        return tail.to_vec();
    }
    vec![root]
}

/// The recognized TOP-LEVEL form heads — the constructs the scan turns into an index entry, PLUS
/// `module-doc` (a benign no-op — see below). A top-level item whose head is NOT one of these is a
/// construct the compiler does not model (e.g. `(pragma …)`), and the whole program DECLINES rather than
/// silently ignoring it and compiling the rest (decline-don't-miscompile — a program with an unmodeled
/// declaration is out of scope, not partially meaningful). Kept in sync with `scan_top_level`'s branches.
///
/// `module-doc` is a file/module-level doc-comment node — a `///` header before a non-documentable form
/// (e.g. an `import`), emitted by the reader so a file header round-trips as `///` rather than a `//`
/// comment. It DECLARES nothing (`scan_top_level` has no branch for it — it is simply skipped), so it
/// produces no index entry; it is listed here ONLY so `unknown_top_forms` recognizes it as legitimate
/// and does not reject it as an unmodeled declaration. It is inert at every later stage (a no-op form).
///
/// `world` is an in-source `(world …)` TARGET-WORLD declaration — the paren-form twin of an external
/// KIND_WIT_WORLD artifact. It follows the SAME recognize-register-nothing pattern as `module-doc`
/// (`scan_top_level` has no branch, so it registers no def/type/effect and never resolves as a value),
/// but it is NOT inert: `compile` reads it via `top_world_form` and codec-encodes its subtree into
/// `db.wit_world` (when no artifact overrides), so it drives emit-to-match without being a runtime decl.
pub(super) const TOP_LEVEL_FORMS: &[&str] = &[
    "def",
    "export",
    "type",
    "effect",
    "bind",
    "module-doc",
    "world",
];

/// The declaration/directive keywords a top-level `(head …)` form may legitimately lead with — the
/// closed candidate pool for a "did you mean?" when an unknown top-level head is a plausible TYPO of one
/// (`(exprot f)` → `export`, `(deff …)` → `def`). A superset of [`TOP_LEVEL_FORMS`]: it adds `module`
/// (a grammar head, not in the scan set) and `pragma` (a directive validated separately), because a user
/// mistyping any of these writes a top-level form, and pointing at the intended keyword is the fix. Kept
/// in one place so the suggestion pool cannot drift into naming a keyword the grammar would then reject.
pub const TOP_LEVEL_KEYWORDS: &[&str] = &[
    "def", "export", "type", "effect", "bind", "module", "pragma", "world",
];

/// Substitute a generic newtype's TEMPLATE `Ty` at a concrete instantiation: replace each `Ty::Var(i)`
/// (a declaration parameter's positional slot, planted by `infer::decode_payload_template`) with `args[i]`
/// — the type the sum was instantiated at. A `Var(i)` with `i` past `args` (a malformed/under-applied
/// instantiation) is left as-is (a free var the boundary rejects, not a panic). A template with no vars
/// (a monomorphic newtype) is cloned unchanged. Descends every structural `Ty` that carries inner types.
/// Rewrite an embedded `Ty::Sum` (or `Ty::Nominal`) whose `decl` is an erasable NEWTYPE into its
/// `normalize_sum` form, so a stored `newtype_inner` template agrees with what a query-time reader would
/// decode. Used ONCE at load to fix a template that referenced a newtype not-yet-keyed when it was decoded
/// (the cross-module decl-order case — 9939). Recurses into structural children and into a keyed decl's
/// type-ARGUMENTS, but NOT into a produced `Nominal`'s `inner`: `inner` is the derived one-level unfold
/// `normalize_sum` computes from that decl's OWN stored template (rewritten as its own map entry), so
/// descending it would re-expand a RECURSIVE newtype's `Sum` back-edge forever. Stopping at the node
/// yields exactly the finite shape query-time `normalize_sum` produces. Pure over `&Db` (a map lookup +
/// structural rebuild).
pub(super) fn normalize_embedded_sums(db: &Db, t: &crate::ty::Ty) -> crate::ty::Ty {
    use crate::ty::Ty;
    match t {
        // A keyed newtype reference (as a raw `Sum` baked by the load-order, or an already-erased
        // `Nominal`) → its canonical `normalize_sum` form, with its args normalized first. `normalize_sum`
        // re-derives `inner` from the decl's own (separately-rewritten) stored template; we do not walk it.
        Ty::Sum { decl, args } | Ty::Nominal { decl, args, .. }
            if db.newtype_inner.contains_key(decl) =>
        {
            let new_args: Vec<Ty> = args
                .iter()
                .map(|a| normalize_embedded_sums(db, a))
                .collect();
            db.normalize_sum(*decl, new_args)
        }
        // A non-newtype sum stays boxed; normalize its args (a generic sum's payload template).
        Ty::Sum { decl, args } => Ty::Sum {
            decl: *decl,
            args: args
                .iter()
                .map(|a| normalize_embedded_sums(db, a))
                .collect(),
        },
        Ty::Nominal { decl, args, inner } => Ty::Nominal {
            decl: *decl,
            args: args
                .iter()
                .map(|a| normalize_embedded_sums(db, a))
                .collect(),
            inner: inner.clone(),
        },
        Ty::Tuple(elems) => Ty::Tuple(
            elems
                .iter()
                .map(|e| normalize_embedded_sums(db, e))
                .collect(),
        ),
        Ty::List(e) => Ty::List(Box::new(normalize_embedded_sums(db, e))),
        Ty::Map(k, v) => Ty::Map(
            Box::new(normalize_embedded_sums(db, k)),
            Box::new(normalize_embedded_sums(db, v)),
        ),
        Ty::Set(e) => Ty::Set(Box::new(normalize_embedded_sums(db, e))),
        Ty::Fn(p, r) => Ty::Fn(
            Box::new(normalize_embedded_sums(db, p)),
            Box::new(normalize_embedded_sums(db, r)),
        ),
        Ty::Record(fields) => Ty::Record(std::rc::Rc::new(
            fields
                .iter()
                .map(|(k, t)| (k.clone(), normalize_embedded_sums(db, t)))
                .collect(),
        )),
        Ty::Qty { inner, unit } => Ty::Qty {
            inner: Box::new(normalize_embedded_sums(db, inner)),
            unit: unit.clone(),
        },
        other => other.clone(),
    }
}

pub(super) fn subst_template_vars(
    template: &crate::ty::Ty,
    args: &[crate::ty::Ty],
) -> crate::ty::Ty {
    #[cfg(test)]
    SUBST_TEMPLATE_VARS_VISITS.with(|c| c.set(c.get() + 1));
    use crate::ty::Ty;
    match template {
        Ty::Var(i) => args.get(*i as usize).cloned().unwrap_or(Ty::Var(*i)),
        Ty::Tuple(elems) => Ty::Tuple(elems.iter().map(|t| subst_template_vars(t, args)).collect()),
        Ty::List(elem) => Ty::List(Box::new(subst_template_vars(elem, args))),
        Ty::Map(k, v) => Ty::Map(
            Box::new(subst_template_vars(k, args)),
            Box::new(subst_template_vars(v, args)),
        ),
        Ty::Set(e) => Ty::Set(Box::new(subst_template_vars(e, args))),
        Ty::Fn(p, r) => Ty::Fn(
            Box::new(subst_template_vars(p, args)),
            Box::new(subst_template_vars(r, args)),
        ),
        Ty::Record(fields) => Ty::Record(std::rc::Rc::new(
            fields
                .iter()
                .map(|(k, t)| (k.clone(), subst_template_vars(t, args)))
                .collect(),
        )),
        // A scalar / sum / nominal / qty template leaf carries no param slot to fill (a template that
        // reached a `Ty::Sum` was rejected by the sum-free guard, so it never lands here) — clone as-is.
        other => other.clone(),
    }
}
