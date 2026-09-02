//! The sidecar request list — the program that DRIVES a compilation.
//!
//! `compile(inputs, targets)` today takes a fixed `targets: &[Target]` — a flat "materialize these
//! output columns" list with no branching and no way to ask a fact. This module generalizes that one
//! parameter into a **request list**: an ordered `Vec<Request>` where each request either MATERIALIZES
//! an output column (`Emit` — today's `Target`) or READS a fact column (`Query` — a node's type, the
//! occurrences that use a name). The list crosses the tool boundary as one more KINDED INPUT ARTIFACT
//! (`kind == "sidecar"`), so it adds no new invocation surface: it rides the same `ast_bytes →
//! result_bytes` ABI every toolchain entry already speaks (`DESIGN-sidecar-api.md`).
//!
//! **Why this is not new analysis.** Every `Query` is a read of a column the compiler already fills
//! (or a trivial derivation over one): `TypeOf` reads the solved-type column (`infer::type_of`),
//! `UsesOf` is the transpose of the resolution column (`resolve::resolved_of`). The oracle contract
//! already obliges every such fact to be TOTAL, DETERMINISTIC, and equal to what a full compile
//! determines — so this surface EXPOSES an obligation the compiler carries: an agent learns a static
//! fact by asking, and the answer is read from the SAME columns a compile reads (never a separate
//! analyzer that could disagree with the compiler or the executable semantics).
//!
//= spec/capabilities/tooling-and-lsp.md#an-agent-queries-the-compiler-for-any-static-fact
//# An agent MUST be able to query the compiler for any static fact about a program — the type of any node, a name's resolution, the inferred manifest/effect row, the solved constraints — and the answer MUST be total, deterministic, and equal to what a full compile determines, so that an agent learns a static fact by asking rather than by instrumenting the program.
//!
//= spec/capabilities/tooling-and-lsp.md#tooling-shares-the-compiler-and-the-semantics
//# A type, definition, or diagnostic the tooling reports MUST agree with the compiler and the executable semantics rather than be computed by a separate implementation.
//!
//! This IS the agent-authoring STRUCTURAL READ interface: a documented request vocabulary reads a
//! program's canonical representation WITHOUT textual patching, addressing a node by its `StructId` — a
//! deterministic function of the canonical representation — and reading a fact column directly rather than
//! re-parsing unrelated code, so a query is a deterministic function of the canonical representation and
//! an agent can target/re-target reproducibly. (The structural REWRITE half — the `Rewrite` request — is
//! a later rung, so the well-formed-or-report edit obligation is not yet realized.)
//= spec/capabilities/agent-authoring.md#a-structural-interface-exists
//# The language MUST expose a documented interface to read a program's canonical representation without textual patching.
//= spec/capabilities/agent-authoring.md#a-structural-interface-exists
//# A structural query or edit MUST operate without re-parsing code unrelated to its target.
//= spec/capabilities/agent-authoring.md#structural-addressing-is-deterministic
//# The address by which the structural interface identifies a node MUST be a deterministic function of the canonical representation.
//= spec/capabilities/agent-authoring.md#structural-addressing-is-deterministic
//# A structural query MUST return a result that is a deterministic function of the canonical representation, so that an agent can target and re-target edits reproducibly.
//!
//! (`query-engine.md` §A Batch Compile And A Point Query Are The Same Reads Of The Same Columns.)
//!
//! **Why no effects, no mutable state.** A fact is a pure read, so a sidecar that branches on one —
//! "emit debug only if the module declares an effect", "rewrite only where the type is `T`" — is
//! ordinary control flow over a value, needing neither effects nor a mutable place (which would break
//! the model's no-cache / incremental-equals-batch invariants). This module is the DATA form of that
//! program: an inert list a driver hands in. The branching program that PRODUCES the list is a later
//! rung (it needs the self-host); the list it returns is what crosses here.
//!
//! **The wire form** is the BINARY AST (operator ruling 2026-08-26: "the sidecar absolutely needs to use
//! the binary AST" — the one bespoke sidecar format, retired). The request list is a cdzast value — an
//! `Ast.List` of per-request forms — encoded/decoded through the SAME `crate::codec` a program's AST uses
//! (rcdzc's copy; a delegating `cdz` builds the byte-identical tree via cadenza-syntax's copy —
//! copy-don't-depend, no shared crate). Each form: `(emit <target>)` | `(emit-tests…)` |
//! `(query <selector> <args>…)` — a NAME-arg query carries an `Ast.Str`, a NODE-arg query an `Ast.Int`.
//! Total decode (the codec's discipline): a malformed tree / unknown head / bad operand is a DECLINE (a
//! diagnostic), never a panic or a silent drop — reject-don't-miscompile at the tool edge.

use crate::ast::{Builder, CompoundCtor, Leaf, Struct, StructId};
use crate::db::Db;
use crate::resolved::Resolved;

// The result-artifact KIND vocabulary now lives in the shared `cadenza-compile-abi` crate (the
// compile-boundary wire `cdz` and `rcdzc` agree on — moved alongside the `Request`/`Query`
// vocabulary + codec). Re-exported here so `crate::sidecar::KIND_*` / `rcdzc::sidecar::KIND_*`
// stay byte-stable and the `run_query` producer below keeps resolving each answer's kind.
pub use cadenza_compile_abi::sidecar::{
    KIND_CLOSURE_HASH, KIND_DIAGNOSTICS, KIND_DOC, KIND_EXPORT_TYPES, KIND_EXPORTS,
    KIND_FUNC_LAYOUT, KIND_HIGHLIGHT, KIND_INSTANTIATIONS, KIND_PARAM_MANIFEST, KIND_RESOLVE,
    KIND_SCOPE, KIND_SHRED_MANIFEST, KIND_SIDECAR, KIND_SYMBOLS, KIND_TEST_LIST, KIND_TYPE_AT,
    KIND_TYPE_INFO, KIND_USES,
};

// The binary-AST query-answer codecs (per-KIND `*_wire` modules in `cadenza-compile-abi`) re-exported
// so a consumer that depends only on `rcdzc` (e.g. `cdz-wasm`) can decode via `rcdzc::sidecar::…`
// without a direct `cadenza-compile-abi` dep — the same copy the producer above encodes with.
pub use cadenza_compile_abi::{decode_highlight, encode_highlight};
// The `Query::Exports` (`KIND_EXPORTS`) decode, re-exported for the same reason: `cdz-wasm`'s
// `export_types` decodes the binary exports payload via `rcdzc::sidecar::decode_exports` (then renders
// each export's display type-name from the decoded structure), without a direct `cadenza-compile-abi` dep.
pub use cadenza_compile_abi::exports_wire::decode_exports;
// The `Query::TypeAt` (`KIND_TYPE_AT`) decode + its `TypeAt` hover-verdict enum — re-exported for the
// SAME reason as `decode_exports`/`decode_highlight`: the browser consumer `cdz-wasm` (`type_at`, the
// guide editor's live hover) decodes this now-BINARY wire via `rcdzc::sidecar::…` without a direct
// `cadenza-compile-abi` dep. The producer above emits binary AST (`type_at_wire`), so a
// `String::from_utf8` consumer is a bug — the consumer decodes the verdict + renders it (mirrors the
// `cdz` LSP `render_type_at` + the `decode_exports` template).
pub use cadenza_compile_abi::TypeAt;
pub use cadenza_compile_abi::type_at_wire::decode_type_at;
// The `Query::ResolveOf` (`KIND_RESOLVE`) decode — the go-to-definition target node id (or none), now a
// BINARY-AST value (`resolve_wire`, flipped in #6152). Re-exported so cdz-wasm's `define_at` reads the
// node id via `rcdzc::sidecar::decode_resolve` instead of `String::from_utf8` + `parse::<u32>()`.
pub use cadenza_compile_abi::resolve_wire::decode_resolve;
// The `Query::Instantiations` (`KIND_INSTANTIATIONS`) decode + its report/`Instance` structs — a
// BINARY-AST value (`instantiations_wire`). Re-exported so cdz-wasm's `disposition` reads the report via
// `rcdzc::sidecar::decode_instantiations` instead of `String::from_utf8` + TAB-splitting the binary.
pub use cadenza_compile_abi::instantiations_wire::{
    Instance as InstantiationsInstance, Instantiations, decode as decode_instantiations,
};

// The `Request` (materialize an output column) + `Query` (read a fact column) enums that make up one
// sidecar request list now live in the shared `cadenza-compile-abi` crate — the compile-boundary
// contract types `cdz` and `rcdzc` agree on. Re-exported here so `crate::sidecar::Request`/`::Query`
// and `rcdzc::Request`/`rcdzc::Query` (lib.rs) stay byte-stable and every consumer path keeps
// resolving. The encode/decode CODEC below + the `run_query` IMPLEMENTATION (over a live `Db`) stay.
pub use cadenza_compile_abi::{Query, Request};

// The `Request`/`Query` encode/decode CODEC now lives in the shared `cadenza-compile-abi` crate (moved
// alongside the request/query vocabulary — the compile-boundary wire `cdz` and `rcdzc` agree on). Re-
// exported here so `crate::sidecar::encode`/`decode` and `rcdzc::encode`/`decode` stay byte-stable and
// every caller keeps resolving. `run_query` + `diagnostics_wire` (below) stay in `rcdzc`; `run_query`
// builds its result AST values with the small `atom_str` helper kept here (the codec has its own copy).
pub use cadenza_compile_abi::{decode, encode};

/// An `Ast.Str` leaf atom — a tiny `Builder` helper the query engine (`run_query`) uses to build a
/// query result's AST value. The request/query codec keeps its own copy in `cadenza-compile-abi`; this
/// serves the STAYING query-result construction, so neither crate reaches across the boundary for it.
fn atom_str(b: &mut Builder, s: &str) -> StructId {
    b.atom_leaf(Leaf::Str(s.into()))
}

/// Run one `Query` against the loaded `Db`, producing its answer as UTF-8 text bytes. This is the
/// query half of the engine: a read of a fact column, TOTAL over every input (a name that names
/// nothing yields a defined "no such …" line, never an error) — the oracle contract. Pure with respect
/// to the artifact channel: a query NEVER denies the component, and reading a fact does not change what
/// any other request produces.
///
/// Every arm is TOTAL — a name/node with no answer (an unbound name, a node past the program, a poison)
/// yields a defined "unknown"/empty result, so a query over an incomplete or malformed program returns a
/// partial answer rather than failing opaquely, and never crashes the caller's session:
//= spec/capabilities/tooling-and-lsp.md#queries-over-incomplete-source-are-total
//# A tooling query over source that does not fully parse MUST return a defined partial result rather than fail opaquely.
//= spec/capabilities/tooling-and-lsp.md#queries-over-incomplete-source-are-total
//# A tooling query MUST NOT crash the editor session on malformed source.
/// Serialize the well-formedness fault set as the `KIND_DIAGNOSTICS` wire: one fault per line, 8 TAB-
/// separated columns — `severity  code  node-id  fix-kind  fix-node  fix-replacement  fix-verified
/// message` (see the `Query::Diagnostics` arm for the column contract; `message` is last so it stays a
/// free-text remainder). SHARED by that sidecar arm AND `rcdzc::cli`'s `--emit-diagnostics` side-artifact
/// flag, so the two serializations never drift. `crate::diagnostics(db)` is the ungated "as-you-type" fault
/// set (`collect_faults`, no export/layout gating), so this is well-defined even on an error/decline compile.
pub fn diagnostics_wire(diags: &[crate::Diagnostic]) -> Vec<u8> {
    // The diagnostics wire is now the canonical BINARY AST (operator seq-254/284: binary AST everywhere,
    // no bespoke tab text) — `cadenza_compile_abi::encode_diagnostics` encodes the FULL `Diagnostic`
    // (severity/code/node/message + the optional fix INCLUDING its `label`), the inverse of every
    // consumer's `decode_diagnostics`. Supersedes the former bespoke 8-TAB-column text (which dropped
    // `label`, collapsed message/replacement newlines, and spelled `InsertInto` as `insert`).
    // `crate::Diagnostic` IS `cadenza_compile_abi::Diagnostic` (re-export), so this is a drop-in.
    cadenza_compile_abi::encode_diagnostics(diags)
}

pub fn run_query(db: &mut Db, query: &Query) -> QueryResult {
    match query {
        Query::TypeOf { name } => {
            // The `cdz type NAME` verdict as a tagged binary-AST sum (`cadenza_compile_abi::type_info_wire`),
            // NOT a rendered string (operator 307: full type AST; the consumer renders + decides exit code
            // from the STRUCTURE, no `is_no_such_definition` string-match):
            //   • Found(ty-payload) — the def's GENERALIZED scheme type as the structured payload
            //     (`encode_ty_payload`); the consumer renders it via `render_ty_scheme` (distinct quantified
            //     vars get stable letters `a`,`b`,… so a generic sig's TIE STRUCTURE is visible), preserving
            //     the old `render_scheme` display.
            //   • Unknown — the def exists but its type could not be solved (an ambiguous unannotated param);
            //     a DEFINED "unknown", not an error (the query is total).
            //   • NoDef(msg) — the name names no definition; the total verdict message, with the same closed-
            //     set "did you mean?" (`did_you_mean` over the program's own def names) the compiler's
            //     unbound-name sites carry. The consumer prints it + exits FAILURE (a typo isn't a type).
            let info = match db.def_by_name(name) {
                Some(def) => match crate::infer::def_scheme(db, def) {
                    Some(scheme) => {
                        let ty_root = crate::eval::encode_ty_payload(db, &scheme.ty);
                        cadenza_compile_abi::TypeInfo::Found(extract_subtree(&db.ast, ty_root))
                    }
                    None => cadenza_compile_abi::TypeInfo::Unknown,
                },
                None => {
                    let names: Vec<&str> = db.defs.iter().map(|d| d.name.as_str()).collect();
                    let hint = crate::diag::suggest::did_you_mean(name, names, 3);
                    cadenza_compile_abi::TypeInfo::NoDef(format!(
                        "no such definition `{name}`{hint}"
                    ))
                }
            };
            QueryResult {
                kind: KIND_TYPE_INFO,
                name: name.clone(),
                bytes: cadenza_compile_abi::encode_type_info(&info),
            }
        }
        Query::UsesOf { name } => {
            // The reference node ids, ascending — the deterministic order the columns model requires
            // (a query answer is a function of the program, not of traversal order). The wire is
            // canonical BINARY AST (`uses_wire`, operator P0 seq-284/307-308: no bespoke TAB/newline
            // text), a root list of `Ast.Int` node-id leaves the `cdz` consumers decode via the shared
            // codec — never a string split.
            let ids: Vec<u32> = uses_of(db, name).into_iter().map(|id| id.0).collect();
            QueryResult {
                kind: KIND_USES,
                name: name.clone(),
                bytes: cadenza_compile_abi::encode_uses(&ids),
            }
        }
        Query::TypeAt { node } => {
            let id = crate::ast::StructId(*node);
            // A HOVER answer as a tagged binary-AST verdict (`cadenza_compile_abi::type_at_wire`), NOT a
            // rendered string (operator 307): a grammar keyword names itself (`Keyword`), a definition
            // carries its signature payload (`Def`), a genuinely-typed node its type payload (`Ty`), an
            // untypeable/poison node `Unknown`. The FULL structured Ty payload crosses; the consumer
            // renders the display (`name : <type>` / `<type>` / `keyword <kw>` / `unknown`).
            let verdict = hover_verdict(db, id);
            QueryResult {
                kind: KIND_TYPE_AT,
                name: node.to_string(),
                bytes: cadenza_compile_abi::encode_type_at(&verdict),
            }
        }
        Query::Diagnostics => {
            // Every well-formedness fault, WITHOUT gating on export/layout — the "diagnostics as you
            // type" set (`compile::diagnostics`, which runs `collect_faults` alone). One fault per line,
            // TAB-separated: `severity  code  node-id  fix-kind  fix-node  fix-replacement  fix-verified
            //   message`.
            //   - `code` is the `CDZ####` or `-` for an uncoded decline;
            //   - `node-id` is the anchor (`-` if unanchored) the consumer maps to a span;
            //   - the FOUR fix columns carry the structural repair
            //     (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix): the edit
            //     KIND (`replace` a node's spelling, or `insert` child forms into the node), the target
            //     node, the surface payload, and `verified`/`heuristic` — all `-` when no fix is proposed;
            //   - `message` is LAST so it stays a free-text remainder (a consumer splits on the first seven
            //     tabs). A rendered replacement is tab-free (names / s-expressions), so the fix columns
            //     never collide with the message split.
            QueryResult {
                kind: KIND_DIAGNOSTICS,
                name: "diagnostics".to_string(),
                bytes: diagnostics_wire(&crate::diagnostics(db)),
            }
        }
        Query::ResolveOf { node } => {
            let id = crate::ast::StructId(*node);
            // The defining occurrence a reference resolves to: `Ref { value }` (a nullary def body, a
            // let initializer, a parameter binder, a sum/prelude binding) or `Lambda { body }` (a def
            // with parameters). Anything else — a literal, a prim, an unbound name, a non-user id — is
            // NOT a navigable reference: the empty result (total). The consumer maps the id to a span.
            //
            // When the resolved-to node is a top-level DEFINITION's body, jump to the def's NAME
            // occurrence, not its body/value: go-to-definition should land on `helper` (so an editor
            // highlights the name), not the `(+ x 1)` body a `Lambda`/`Ref` resolves to. Both a nullary
            // def (`Ref` → the body value) and a function def (`Lambda` → the lambda body) resolve to a
            // def body, so `def_index_by_body` maps either back to the def, and its sig's first child is
            // the name. A non-def target (a `let` initializer, a parameter binder) has no such owner —
            // keep the resolved occurrence itself, which IS the binding site to jump to. (Same
            // name-occurrence convention the `Exports` query uses.)
            let name_occ_of_def_body = |db: &Db, target: crate::ast::StructId| {
                db.def_index_by_body(target)
                    .map(|di| db.defs[di].sig_occ)
                    .and_then(|sig| match db.ast.get(sig) {
                        Struct::List(kids) => kids.first().copied(),
                        _ => None,
                    })
                    .unwrap_or(target)
            };
            let mut resolved: Option<u32> = None;
            if db.is_user_node(id) {
                let target = match crate::resolve::resolved_of(db, id) {
                    Resolved::Ref { value } => Some(value),
                    Resolved::Lambda { body, .. } => Some(body),
                    _ => None,
                };
                if let Some(target) = target {
                    resolved = Some(name_occ_of_def_body(db, target).0);
                }
            }
            // The defining occurrence's node id, or none — canonical BINARY AST (`resolve_wire`,
            // operator P0 seq-284/307-308: no bespoke text), a root list of zero/one `Ast.Int` the `cdz`
            // consumers decode via the shared codec, never a string parse.
            QueryResult {
                kind: KIND_RESOLVE,
                name: node.to_string(),
                bytes: cadenza_compile_abi::encode_resolve(resolved),
            }
        }
        Query::ScopeAt { node } => {
            // Every binding visible at the node, INNERMOST-first, as ONE canonical binary-AST value
            // `(scope (binding <name> <ty-payload> <binder-node>)…)` via the shared
            // `cadenza_compile_abi::scope_wire` codec — each binder's FULL structured type payload
            // (`encode_ty_payload` of `type_of`), NOT a `render_name` string (operator 307: full type AST;
            // the consumer renders a display name from the decoded structure). `type_of` is total, so the
            // ty payload + binder node are always present.
            let binders = scope_at(db, crate::ast::StructId(*node));
            let mut bindings: Vec<cadenza_compile_abi::ScopeBinding> =
                Vec::with_capacity(binders.len());
            for (name, binder) in binders {
                let ty = crate::infer::type_of(db, binder);
                let ty_root = crate::eval::encode_ty_payload(db, &ty);
                let ty_arena = extract_subtree(&db.ast, ty_root);
                bindings.push(cadenza_compile_abi::ScopeBinding {
                    name,
                    ty: ty_arena,
                    node: binder.0,
                });
            }
            QueryResult {
                kind: KIND_SCOPE,
                name: node.to_string(),
                bytes: cadenza_compile_abi::encode_scope(&bindings),
            }
        }
        Query::Exports => {
            // The module's interface: each `(export NAME)` clause paired with the named def's FULL
            // structured type payload (its generalized signature, via `def_scheme` → `encode_ty_payload`)
            // and the def's NAME occurrence (sig's first child, for go-to). Carried as ONE canonical
            // binary-AST value `(exports (export <name> <ty-opt> <node-opt>)…)` via the shared
            // `cadenza_compile_abi::exports_wire` codec — NO `render_name` string on the wire (operator
            // 307: full type AST; the consumer renders a display name from the decoded structure). An
            // export naming no def carries both optionals absent; an unsolved scheme carries `ty` absent.
            // Deterministic (export order).
            let exports: Vec<(String, Option<usize>)> =
                db.exports.iter().map(|e| (e.name.clone(), e.def)).collect();
            let mut entries: Vec<cadenza_compile_abi::ExportEntry> =
                Vec::with_capacity(exports.len());
            for (name, def) in exports {
                let (ty, node) = match def {
                    Some(d) => {
                        // The generalized scheme type → structured payload sub-AST, extracted into a
                        // standalone arena; `None` when the def's type could not be solved.
                        let ty = crate::infer::def_scheme(db, d).map(|scheme| {
                            let root = crate::eval::encode_ty_payload(db, &scheme.ty);
                            extract_subtree(&db.ast, root)
                        });
                        // The def's NAME occurrence (the sig's first child), for go-to; fall back to the
                        // sig occurrence itself if the signature is malformed.
                        let sig = db.defs[d].sig_occ;
                        let name_node = match db.ast.get(sig) {
                            Struct::List(kids) => kids.first().copied().unwrap_or(sig),
                            _ => sig,
                        };
                        (ty, Some(name_node.0))
                    }
                    // An export naming no definition — both optionals absent (reported as "unknown" by
                    // the consumer, which is where any diagnostic surfaces).
                    None => (None, None),
                };
                entries.push(cadenza_compile_abi::ExportEntry { name, ty, node });
            }
            QueryResult {
                kind: KIND_EXPORTS,
                name: "exports".to_string(),
                bytes: cadenza_compile_abi::encode_exports(&entries),
            }
        }
        Query::ExportedTypes => {
            // The STRUCTURED companion of `Exports` (cadenza-docs I2, architecture `(C)`): each exported
            // item's resolved type as a cdzast sub-AST, so `cdz doc` grafts a queryable `(ty …)` rather than
            // a printed string. Covers ALL THREE exported item kinds (v-syntax's ask): a value DEF (its
            // GENERALIZED `def_scheme.ty` — polymorphic `(Var N)`, the right DOC shape), a TYPE decl, and an
            // EFFECT decl (each resolved to its type-value via `typeval_of`). An export is resolved by trying
            // the value-def first (the `e.def` slot); when that is absent — a type/effect export carries
            // `def: None` — fall back to the type-decl then effect-decl lookup BY NAME. An export whose type
            // does not resolve in ANY kind is OMITTED (graceful-degrade: the doc-item's `(ty …)` is optional).
            // Deterministic (export order). Wire: ONE canonical binary-AST value
            // `(export-types (export-type <name> <ty-payload>)…)` (see `cadenza_compile_abi::export_types_wire`),
            // the resolved type sub-AST grafted directly — no bespoke outer framing.
            let exports: Vec<(String, Option<usize>)> =
                db.exports.iter().map(|e| (e.name.clone(), e.def)).collect();
            let mut records: Vec<(String, crate::ast::Arenas)> = Vec::new();
            for (name, def) in exports {
                // Resolve the export to a cdzast type sub-AST ROOT (in `db.ast`), trying each kind:
                //   • value DEF  → the generalized `def_scheme.ty` (polymorphic `(Var N)`) via
                //     `encode_ty_payload` → a `(-> …)`/scalar payload.
                //   • TYPE decl  → `typeval_of` on its occ → `encode_ty_payload` → a `(Sum …)`/`(Record …)`.
                //   • EFFECT decl → a GROUPED op-signature payload `(effect (op <name> <arrow>)…)` — an
                //     effect has NO single `Ty` (it is N ops), so build the shape directly from its `ops`,
                //     each `op.ty` arrow via `typeval_of`→`encode_ty_payload`. `effect_op_types_node` returns
                //     `None` when NO op resolves (omit the whole `(ty …)`, per the doc-model's degrade rule).
                // `None` from all kinds → omit this export.
                let ty_root = def
                    .and_then(|d| crate::infer::def_scheme(db, d).map(|s| s.ty))
                    .or_else(|| {
                        db.type_decl_by_name(&name)
                            .and_then(|occ| crate::eval::typeval_of(db, occ))
                    })
                    .map(|ty| crate::eval::encode_ty_payload(db, &ty))
                    .or_else(|| effect_op_types_node(db, &name));
                let Some(ty_root) = ty_root else { continue };
                // EXTRACT the sub-AST into a standalone arena rooted at the type (a `db.ast` subtree
                // StructId cannot travel alone); the wire encoder grafts each such arena's root subtree
                // into the ONE shared `(export-types …)` value — a `(-> …)`/`(Sum …)`/`(effect …)` root alike.
                let ty_arena = extract_subtree(&db.ast, ty_root);
                records.push((name, ty_arena));
            }
            // ONE canonical binary-AST value: `(export-types (export-type <name> <ty-payload>)…)` — the
            // bespoke `u32_le count | per-record name_len/name/ty_len/ty_bytes` OUTER framing is retired
            // (operator 307: full type AST, no bespoke envelope). The consumer decodes once with the
            // shared `cadenza-compile-abi` codec; no byte-length reader, no nested artifacts.
            QueryResult {
                kind: KIND_EXPORT_TYPES,
                name: "export-types".to_string(),
                bytes: cadenza_compile_abi::encode_export_types(&records),
            }
        }
        Query::TestList => {
            // The `@test`-marked defs the runner would run, for `cdz test --list` (binary, no rcdzc link in
            // `cdz`). Answered as a CADENZA-AST VALUE (`codec::encode`d), NOT a bespoke blob — the operator
            // directive is cadenza-ast-binary as the universal tooling exchange format, so `cdz --list`
            // FORWARDS these bytes verbatim (no re-encode) and every consumer (v-test-shred/v-nix) decodes
            // with the ONE shared codec. Shape: `(test-list (test <name> <is-property> <file>)…)` — a
            // `test-list`-headed form, one `(test …)` child per test, POSITIONAL: name (`Str`), is-property
            // (`Bool`), file (`Str`). Enumerate `db.test_defs()` (already package-linkage-scoped by the linked
            // closure the db was loaded from — the same walk the runner + test-manifest use). is-property =
            // `!params.is_empty() || name.ends_with("-gen")` (canonical `compile_tests`, incl the `-gen`
            // `Test.gen` wrapper). Deterministic (`test_defs` order); a fileless test emits an empty `file`
            // Str (never omitted — its name is the drift-guard identity).
            let mut b = Builder::new();
            let mut tests: Vec<StructId> = Vec::new();
            for def in db.test_defs() {
                let d = &db.defs[def];
                let is_property = !d.params.is_empty() || d.name.ends_with("-gen");
                let file = db
                    .file_of(d.sig_occ)
                    .and_then(|fi| db.file_path(fi))
                    .unwrap_or("")
                    .to_string();
                let head = b.name("test");
                let name = atom_str(&mut b, &d.name);
                let isprop = b.atom_leaf(crate::ast::Leaf::Bool(is_property));
                let file_node = atom_str(&mut b, &file);
                tests.push(b.list(vec![head, name, isprop, file_node]));
            }
            let list_head = b.name("test-list");
            let mut children = Vec::with_capacity(tests.len() + 1);
            children.push(list_head);
            children.extend(tests);
            let root = b.list(children);
            QueryResult {
                kind: KIND_TEST_LIST,
                name: "test-list".to_string(),
                bytes: crate::codec::encode(&b.finish(root)),
            }
        }
        Query::Highlight => {
            // Every USER LEAF classified by the role it plays — one `node-id<TAB>kind` line, ascending
            // id order. A leaf is what an editor paints (a name atom, a literal); a list form is not
            // itself a token (its head/children are), so only leaves are emitted. Classification reads
            // the resolved column + the meta channels a value already carries (see `classify_highlight`).
            let node_count = db.ast.structure.len();
            let mut tokens: Vec<(u32, &str)> = Vec::new();
            for i in 0..node_count {
                let id = StructId(i as u32);
                if !db.is_user_node(id) {
                    continue;
                }
                if let Some(kind) = classify_highlight(db, id) {
                    tokens.push((id.0, kind.as_str()));
                }
            }
            // Each classified leaf as a `(node-id, kind)` pair — canonical BINARY AST (`highlight_wire`,
            // operator P0 seq-284/307-308: no bespoke TAB/newline text), a root list of `(Int Str)` forms
            // the `cdz`/`cdz-wasm` consumers decode via the shared codec, never a string split.
            QueryResult {
                kind: KIND_HIGHLIGHT,
                name: "highlight".to_string(),
                bytes: cadenza_compile_abi::encode_highlight(&tokens),
            }
        }
        Query::DocOf { name } => {
            // The documentation of a NAME, from the ordered fallback (user def → built-in `(meta doc)` →
            // grammar keyword). Each step reads a column the compiler already fills; none matches the name
            // against a hardcoded key (the built-in step resolves the name to its prelude record, THEN
            // reads a channel — the same generic path member access takes).
            //
            // TOTAL, but with TWO distinct "no answer" verdicts so a consumer can tell a TYPO from an
            // undocumented-but-real name (a `cdz doc` then maps the "no such definition" variant to a
            // non-zero exit): a name that refers to SOMETHING (a user def / built-in / grammar keyword)
            // but carries no doc → "no documentation for `X`"; a name that refers to NOTHING → "no such
            // definition `X`". A cdz-side pre-check can't draw this line (it also documents built-ins that
            // aren't in the file's `Symbols`), but the query knows the `Db` + prelude + keyword tables.
            // A STRUCTURED tagged answer (`doc_wire::DocAnswer`), NOT prose the consumer must string-match
            // (operator P0 seq-284/307-308: no discriminator carried in text). The user-facing wording
            // (`no documentation for `X``, `no such definition `X` — did you mean `Y`?`) lives on the
            // consumer, which holds the queried name; here we carry only the outcome + the optional
            // suggestion (over the DOCUMENTABLE names the query knows — user defs + built-in prelude
            // bindings, via `suggest::nearest`'s edit-distance + 1-char/empty guards; `None` when nothing
            // is close enough).
            let answer = match doc_of_name(db, name) {
                Some(doc) => cadenza_compile_abi::DocAnswer::Doc(doc),
                None if name_is_known(db, name) => cadenza_compile_abi::DocAnswer::Undocumented,
                None => cadenza_compile_abi::DocAnswer::NoSuchDef {
                    suggestion: nearest_known_name(db, name),
                },
            };
            QueryResult {
                kind: KIND_DOC,
                name: name.clone(),
                bytes: cadenza_compile_abi::encode_doc(&answer),
            }
        }
        Query::DocAt { node } => {
            // The documentation of the definition the node belongs to or references — the "hover doc". A
            // node inside a def's OWN body/signature, or a REFERENCE resolving to a def, maps to that def's
            // signature occurrence; its `(doc "…")` text is read from the doc column. TOTAL: a node that
            // does not reach a documented definition yields the empty result (no line).
            let id = StructId(*node);
            // Structured (`doc_wire::DocAnswer`): the doc text when the node reaches a documented
            // definition, else `Undocumented` (the total "no answer"). The consumer renders the message.
            let answer = match doc_at_node(db, id) {
                Some(doc) => cadenza_compile_abi::DocAnswer::Doc(doc),
                None => cadenza_compile_abi::DocAnswer::Undocumented,
            };
            QueryResult {
                kind: KIND_DOC,
                name: node.to_string(),
                bytes: cadenza_compile_abi::encode_doc(&answer),
            }
        }
        Query::Instantiations { name } => {
            // Every concrete monomorphization of the named generic / ad-hoc-polymorphic definition. Force
            // monomorphization over the whole program first (`force_monomorphize` lowers every def body,
            // firing `type_specialize` at each recursive-generic / type-valued / `const` call) so
            // `db.instantiations` is complete — a query is TOTAL and must not depend on what a query-only
            // run would otherwise lower. Then filter the record to the instances whose SOURCE definition is
            // the named one (mapping each record's `orig_body` back to its def via `def_index_by_body`).
            crate::layout::force_monomorphize(db);
            let report = instantiations_value(db, name);
            QueryResult {
                kind: KIND_INSTANTIATIONS,
                name: name.clone(),
                bytes: cadenza_compile_abi::instantiations_wire::encode(&report),
            }
        }
        Query::Symbols => {
            // The DOCUMENT OUTLINE: every top-level declaration classified by kind, one
            // `(name, kind, name-node-id)` record. Reads the declaration columns `scan_top_level` filled
            // — no new pass. Records grouped (defs → types → effects → modules), declaration order within
            // each, so the answer is a deterministic function of the program. INTERNAL defs (the do-local /
            // module-member callables `modules::register_callable` registers) are not top-level source
            // declarations, so they are skipped. The wire is canonical BINARY AST (`symbols_wire`,
            // operator P0 seq-284/307-308: no bespoke TAB/newline text), a root list of `(Str Str Int)`
            // forms the `cdz` consumers decode via the shared codec, never a string split.
            let entries = symbols_entries(db);
            let borrowed: Vec<(&str, &str, u32)> = entries
                .iter()
                .map(|(name, kind, node)| (name.as_str(), *kind, *node))
                .collect();
            QueryResult {
                kind: KIND_SYMBOLS,
                name: "symbols".to_string(),
                bytes: cadenza_compile_abi::encode_symbols(&borrowed),
            }
        }
        Query::ParamManifest => {
            let sites = param_manifest_value(db);
            QueryResult {
                kind: KIND_PARAM_MANIFEST,
                name: "param-manifest".to_string(),
                bytes: cadenza_compile_abi::param_manifest_wire::encode(&sites),
            }
        }
        Query::FuncLayout => {
            let fl = func_layout_value(db);
            QueryResult {
                kind: KIND_FUNC_LAYOUT,
                name: "func-layout".to_string(),
                bytes: cadenza_compile_abi::func_layout_wire::encode(&fl),
            }
        }
        Query::ClosureHash => {
            let hash = closure_hash_value(db);
            QueryResult {
                kind: KIND_CLOSURE_HASH,
                name: "closure-hash".to_string(),
                bytes: cadenza_compile_abi::encode_closure_hash(hash),
            }
        }
    }
}

/// Copy the subtree of `src` rooted at `root` into a fresh standalone [`Arenas`] (rooted at the copied
/// subtree), so it can be serialized on its own via [`codec::encode`]. `codec::encode` serializes a WHOLE
/// arena keyed by its `root`, and a `db.ast` StructId cannot travel the tool boundary — so the
/// `ExportedTypes` fact extracts each type sub-AST into its own arena, encodes it, and frames the bytes.
/// Recurses the structure; leaves are re-interned in the fresh `Builder` (its dedup is harmless — a
/// distinct occurrence is not required for a serialized type payload, unlike the value-form template).
/// Build the GROUPED op-signature type sub-AST for an EFFECT export (cadenza-docs I2, reviewer #2619
/// follow-up): `(effect (op <name> <arrow-sub-AST>)…)` in `db.ast`, rooted at the `(effect …)` node.
/// An effect has NO single `Ty` (it is a capability with N operations, each its own arrow), so — unlike a
/// def/type which resolves to one `Ty` — its structured `(ty …)` is the LIST of its operations' signatures.
/// Each `op`'s arrow (`OpDecl::ty`, a `(-> Param Result)` occurrence) is reduced via `typeval_of` and
/// re-encoded via `encode_ty_payload`, so the op arrows are the SAME structured sub-ASTs a def/type carries
/// (structured-is-truth). Head-names are the ordinary List heads `"effect"` and `"op"` (keywords-are-data,
/// no new arena variant) — the exact spellings v-syntax's doc-model + renderer key off. GRACEFUL-DEGRADE
/// per op: an op whose arrow does not resolve (`typeval_of` = `None`, or a malformed `(op NAME)` with no
/// type) is OMITTED from the list; if NO op resolves, return `None` so the caller omits the whole `(ty …)`
/// (a bare `(effect)` with no ops says nothing — the doc-model's "omit when nothing resolved" rule). Only
/// fires for a name that is an effect declaration; a non-effect name returns `None`.
fn effect_op_types_node(db: &mut Db, name: &str) -> Option<StructId> {
    // Recover the effect declaration's operations (name + arrow occ), CLONED out so the `&mut db` encode
    // calls below don't hold a borrow of the decl. `effect_decl_by_name` gives the synth record; map it to
    // the `EffectDecl` (which carries `ops`).
    let synth = db.effect_decl_by_name(name)?;
    let ops: Vec<(String, Option<StructId>)> = db
        .effect_decl_by_synth(synth)?
        .ops
        .iter()
        .map(|op| (op.name.clone(), op.ty))
        .collect();
    let mut op_nodes: Vec<StructId> = Vec::new();
    for (op_name, op_ty) in ops {
        // The op's arrow → a `Ty` → an `encode_ty_payload` sub-AST. Omit an op with no resolvable arrow.
        let Some(ty) = op_ty.and_then(|t| crate::eval::typeval_of(db, t)) else {
            continue;
        };
        let arrow = crate::eval::encode_ty_payload(db, &ty);
        let op_head = db.push_name("op");
        let nm = db.push_name(&op_name);
        op_nodes.push(db.push_list(vec![op_head, nm, arrow]));
    }
    if op_nodes.is_empty() {
        return None; // no op resolved — omit the whole (ty …) rather than an empty (effect)
    }
    let eff_head = db.push_name("effect");
    let mut children = vec![eff_head];
    children.extend(op_nodes);
    Some(db.push_list(children))
}

pub(crate) fn extract_subtree(src: &crate::ast::Arenas, root: StructId) -> crate::ast::Arenas {
    fn copy(src: &crate::ast::Arenas, id: StructId, b: &mut crate::ast::Builder) -> StructId {
        match src.get(id) {
            Struct::Atom(lid) => {
                let leaf = src.leaf(*lid).clone();
                b.atom_leaf(leaf)
            }
            Struct::List(children) => {
                let children = children.clone();
                let copied: Vec<StructId> = children.iter().map(|&c| copy(src, c, b)).collect();
                b.list(copied)
            }
        }
    }
    let mut b = crate::ast::Builder::new();
    let new_root = copy(src, root, &mut b);
    b.finish(new_root)
}

/// The `FuncLayout` read: force monomorphization then lay out the boundary, and report each reachable
/// definition's absolute wasm FUNCTION INDEX + a content-hash of its `(def …)` AST subtree, in func-index
/// order. Preceded by a `defs-begin<TAB><import_base><TAB>-` marker so a consumer knows where the def
/// region starts (runtime-op imports occupy `0..import_base`). FORCES monomorphization + runs
/// `layout::compute` like `Instantiations`, so the set + order equal a real emit's. The content-hash is a
/// stable structural hash of the def's AST subtree (see [`def_content_hash`]) — a def shared byte-identically
/// across two builds hashes the SAME, which is what the compile-reuse witness diffs (same func-index AND
/// same content-hash per shared def) and the Option-A cache-key basis. Roots on `(export …)`; a program
/// with NO export falls back to the `@test`-rooted layout (`compute_tests`) so a pure-`@test` file — the
/// witness's target — lays out the same func-index set `cdz test` emits. TOTAL: only when BOTH decline (no
/// export AND no `@test`) is the result empty; an empty program yields just the marker.
fn func_layout_value(db: &mut Db) -> cadenza_compile_abi::func_layout_wire::FuncLayout {
    use cadenza_compile_abi::func_layout_wire::{FuncLayout, FuncLayoutRow};
    // Force monomorphization so the reachable set is complete + stable regardless of what a query-only run
    // would otherwise lower (mirrors `Instantiations`); then lay out the boundary to get the func-index
    // order. Both layouts declining (no export AND no `@test`, see the fallback below) yields a DECLINE
    // (`laid_out: false`) — the query is total.
    crate::layout::force_monomorphize(db);
    // Root on `(export …)` when the program has any (a normal library/export build); otherwise fall back to
    // the `@test`-rooted layout (`compute_tests`), which lays out the SAME func-index set `cdz test` emits.
    // A pure-`@test` file (no export) is exactly the compile-reuse witness's target (sread-eval-fns / -ho),
    // so without this fallback the query would decline on the files it most needs to observe. Both
    // declining (no export AND no `@test`) yields a DECLINE — the query stays total.
    let layout = match crate::layout::compute(db) {
        Ok(l) => l,
        Err(_) => match crate::layout::compute_tests(db) {
            Ok(l) => l,
            Err(_) => {
                return FuncLayout {
                    import_base: 0,
                    laid_out: false,
                    rows: Vec::new(),
                };
            }
        },
    };
    // `layout.order` is the defined-function emission sequence; def at position `k` is wasm func
    // `import_base + k` (== `layout.abs(def)`). Emit in that order so the rows are func-index-ascending.
    let rows: Vec<FuncLayoutRow> = layout
        .order
        .iter()
        .map(|&def| FuncLayoutRow {
            name: db.defs[def].name.clone(),
            hash: def_content_hash(db, def),
            idx: layout.abs(def),
        })
        .collect();
    FuncLayout {
        import_base: layout.import_base,
        laid_out: true,
        rows,
    }
}

/// The Option-C shared-closure CONTENT-HASH for the program's `@test` dir (the [`Query::ClosureHash`] read) —
/// LAYOUT-ONLY, no provider emit. Force monomorphization, lay out the `@test` boundary (`compute_tests`),
/// bucket the `@test` defs by file, compute the UNION cross-edge set over the filed buckets
/// ([`crate::layout::cross_component_edges_union`]), and fold each edge's content-hash into one `u64`
/// ([`closure_content_hash`]) — the SAME canonical value the `EmitTestsComposed` miss-path `closure-hash`
/// sidecar emits, so a provider-cache's decision key, persist key, and validation all agree. A build with no
/// `@test` layout, or no cross-edge (single-file / nothing shared), folds the EMPTY set → a defined stable
/// "no closure" hash. TOTAL: never errors (an un-layoutable program yields the empty-set hash). Returns the
/// raw `u64` fold; the caller encodes it as the canonical binary-AST `KIND_CLOSURE_HASH` wire.
fn closure_hash_value(db: &mut Db) -> u64 {
    use std::collections::BTreeMap;
    crate::layout::force_monomorphize(db);
    let layout = match crate::layout::compute_tests(db) {
        Ok(l) => l,
        Err(_) => return closure_content_hash(db, &[]),
    };
    // Bucket `@test` defs by file (as `EmitTestsComposed` does); the union cross-edge set is over the FILED
    // buckets (a `None`-file / unfiled test contributes no cross-edge — it has no shared closure to import).
    let mut files: Vec<usize> = Vec::new();
    let mut seen: BTreeMap<usize, ()> = BTreeMap::new();
    for def in db.test_defs() {
        if let Some(fi) = db.file_of(db.defs[def].sig_occ)
            && seen.insert(fi, ()).is_none()
        {
            files.push(fi);
        }
    }
    let union_edges = crate::layout::cross_component_edges_union(db, &layout, &files);
    closure_content_hash(db, &union_edges)
}

/// A stable structural content-hash of definition `def`'s `(def …)` AST subtree — the identity a
/// compile-reuse cache keys on and the witness diffs. Hashes the def's signature + body subtrees by walking
/// their arena structure (node shape + leaf bytes) into an [`FxHasher`], so two byte-identical source defs
/// (even at different global `StructId`s, since link splices files at an offset) hash the SAME. Independent
/// of the def's global id and of surrounding files — only its own subtree content.
///
/// WARNING: RUN-STABILITY: every `Name` leaf in the hashed subtrees is desuffixed to its
/// [`crate::layout::source_boundary_name`] (the name before any `$acc`/`#mono{N}` transform suffix) by
/// `hash_subtree` — both the def's OWN `#mono{N}` head name AND every body reference to a sibling
/// specialization. A monomorphized specialization mints its name as `{base}#mono{db.defs.len()}`
/// (`lower.rs`), and `db.defs.len()` at mint time is RUN-VARYING (specialization order differs run to run),
/// so hashing the raw names gave the SAME specialization a DIFFERENT hash each run → its shared-closure
/// provider-cache key ([`closure_content_hash`]) never hit → the closure re-emitted (full lower) on every
/// `--warm-only`, the gate's stubborn ~256s. The `#mono{N}` counter carries NO semantic identity (two
/// DISTINCT specializations of one base still hash differently — the BODY subtree bakes in their concrete
/// type args), so stripping the suffix makes identical specializations hash identically across runs without
/// colliding distinct ones. Both sides of the two-sided provider-cache key fold THIS function, so they stay
/// in agreement by construction. See `def_content_hash_is_invariant_to_the_monomorph_mint_counter`.
pub(crate) fn def_content_hash(db: &Db, def: usize) -> u64 {
    use std::hash::Hasher;
    let mut h = crate::fxhash::FxHasher::default();
    // Hashes the sig + body subtrees via `hash_subtree`, which desuffixes every `Name` leaf (the def's own
    // `#mono{N}` head name AND every body reference to a sibling specialization) to its run-stable boundary
    // name — so the run-varying mint counter never enters the hash. See the run-stability note above.
    hash_subtree(db, db.defs[def].sig_occ, &mut h);
    if let Some(body) = db.defs[def].body {
        hash_subtree(db, body, &mut h);
    }
    h.finish()
}

/// The CLOSURE content-hash for a set of shared cross-edge def indices — a single `u64` folding each def's
/// [`def_content_hash`] over the SORTED-by-hash set, so it is a pure function of WHICH defs (by content),
/// independent of their arena position or the order they were discovered. This is the Option-C provider-cache
/// KEY: the shared closure a composed `cdz test <dir>` provider exports is identified by this hash, so a
/// runner caches the emitted provider component keyed on it and skips re-emitting the closure on a HIT
/// (`Request::EmitTestsConsumerOnly`). The `EmitTestsComposed` driver emits it as a `closure-hash` sidecar on
/// the MISS path (recompute-free persist); a runner's own hash (folding the SAME `def_content_hash` — the
/// FuncLayout col-2 value — over its cross-edge set) must match, so the two sides key identically (a drift-
/// guard). SORTED so the fold is order-independent. Returns the raw `u64` fold; the emit paths encode it as
/// the canonical binary-AST `KIND_CLOSURE_HASH` wire (operator P0 seq-284: binary AST everywhere — no bespoke
/// hex-string byte form), and a consumer that wants the historical `{:016x}` cache-key filename formats the
/// decoded `u64` itself.
pub(crate) fn closure_content_hash(db: &Db, edges: &[usize]) -> u64 {
    use std::hash::Hasher;
    let mut per_def: Vec<u64> = edges.iter().map(|&d| def_content_hash(db, d)).collect();
    per_def.sort_unstable();
    let mut h = crate::fxhash::FxHasher::default();
    for hv in per_def {
        h.write_u64(hv);
    }
    h.finish()
}

/// Feed the AST subtree rooted at `id` into `h` — the node's structural KIND then, recursively, its
/// children (a list) or its leaf bytes (a name/literal). A pure function of the subtree's shape + leaf
/// content, so it is identical for two byte-identical source subtrees regardless of their arena position.
fn hash_subtree(db: &Db, id: StructId, h: &mut crate::fxhash::FxHasher) {
    use crate::ast::Struct;
    use std::hash::{Hash, Hasher};
    match db.ast.get(id) {
        Struct::List(kids) => {
            h.write_u8(0); // tag: list
            h.write_u32(kids.len() as u32);
            // A `(typeval …)` payload encodes a `Ty::Sum`/`Ty::Nominal` as `(Sum NAME <decl> arg…)` /
            // `(Nominal NAME <decl> (args…) <inner>)` where `<decl>` (child index 2) is the declaration
            // OCCURRENCE — a raw arena `StructId` rendered as an Int leaf (`encode_ty`, eval.rs). That id is
            // RUN-VARYING for a monomorphized/synth type (its arena position shifts with mint order), and a
            // mono def body bakes in such type-exprs, so hashing the decl int reintroduces run-instability
            // (the residual the Name-desuffix alone couldn't reach — it's an Int, not a Name). The decl id
            // carries NO CONTENT identity for a closure-cache key: the NAME (index 1, desuffixed) + the ARGS
            // + the INNER already identify the type; the arena occurrence is position, not content. So SKIP
            // the decl int for these two shapes. Two structurally-identical Sum/Nominal type-exprs differing
            // only in arena decl.0 then hash the same across runs, without colliding distinct types (name +
            // args + inner still distinguish them). Any other list hashes every child as before.
            let head = db.ast.head_name(id);
            let skip_decl = matches!(head, Some("Sum") | Some("Nominal")) && kids.len() >= 3;
            for (i, &k) in kids.iter().enumerate() {
                if skip_decl && i == 2 {
                    continue; // the run-varying arena decl-id — excluded from the CONTENT hash
                }
                hash_subtree(db, k, h);
            }
        }
        Struct::Atom(leaf) => {
            h.write_u8(1); // tag: atom
            // The `Leaf` (Int/Float/Str/Name/Char/…) IS the atom's content identity; it derives `Hash`, so
            // hash it directly — two byte-identical source atoms produce the same leaf, hence the same
            // contribution regardless of arena position. EXCEPT a `Name` leaf: hash its SUFFIX-STRIPPED form
            // (`source_boundary_name`), because a monomorphized def's body REFERENCES its sibling
            // specializations by their `{base}#mono{N}` names, and that `N` (= `db.defs.len()` at mint) is
            // RUN-VARYING — so hashing the raw referenced name reintroduces the run-instability that
            // desuffixing the def's OWN name (in `def_content_hash`) removes. Stripping the suffix on every
            // Name leaf makes a body call to `f#mono5` hash identically to one to `f#mono3` — the reference's
            // SEMANTIC identity (the base callee) is run-stable; the mint counter is not. A non-transformed
            // name (no `$`/`#`) is unchanged, so an ordinary program hashes exactly as before.
            match db.ast.leaf(*leaf) {
                crate::ast::Leaf::Name(n) => {
                    h.write_u8(2); // sub-tag: name (distinct from other leaves)
                    crate::layout::source_boundary_name(n).hash(h);
                }
                other => other.hash(h),
            }
        }
    }
}

/// The `ParamManifest` read: one [`ParamSite`] per `@param` site. Scans the sites read-only
/// (`param_sidecar::scan_manifest` over `db.ast`), then, for each, resolves the DECLARED TYPE and carries it
/// as a FULL structured `Ty` sub-AST (`eval::encode_ty_payload`) — NOT a `Ty::render_name` string (operator
/// P0 seq-254/284: "I want the full type ast!"); the consumer renders it back to a display string via
/// `cadenza_syntax::render_ty::render_ty`. The value fields (range elements, options list, default
/// value) ride as ARENA NODE IDS (absent when the kv is absent), which the consumer renders via the
/// shared-`StructId` arena; `widget` is the bare atom or absent; `name_node` is the param name occurrence
/// the consumer maps to `file:line:col`. Order is scan order (ascending site id), a deterministic function
/// of the program. TOTAL: no sites → empty vec.
///
/// [`ParamSite`]: cadenza_compile_abi::param_manifest_wire::ParamSite
fn param_manifest_value(db: &mut Db) -> Vec<cadenza_compile_abi::param_manifest_wire::ParamSite> {
    use cadenza_compile_abi::param_manifest_wire::ParamSite;
    // Scan first (an immutable borrow of the arena), collecting the records, THEN reduce+encode each type (a
    // mutable borrow of `db` for `typeval_of`/`type_of`/`encode_ty_payload`) — the two borrows must not
    // overlap. `scan_manifest` returns OWNED records, so its borrow ends before the loop.
    let records = crate::param_sidecar::scan_manifest(&db.ast);
    let mut sites = Vec::with_capacity(records.len());
    for rec in records {
        // `rec.ty` is the annotation's TYPE-EXPRESSION node (the `Type` in `(: … Type)`). REDUCE it to the
        // declared type VALUE (`eval::typeval_of` — the same evaluator the annotation path uses, so the
        // manifest's type equals what the type checker asserts), NOT `type_of` (which would give the node's
        // KIND, `Type`); a node that does not reduce falls back to `type_of` so the field stays definite.
        // Then carry the FULL structured Ty as a sub-AST (`encode_ty_payload`), extracted into a standalone
        // arena (as `ExportedTypes`/`result_types` do), so no render-name string rides the wire.
        let ty_val = crate::eval::typeval_of(db, rec.ty)
            .unwrap_or_else(|| crate::infer::type_of(db, rec.ty));
        let ty_node = crate::eval::encode_ty_payload(db, &ty_val);
        let ty = extract_subtree(&db.ast, ty_node);
        sites.push(ParamSite {
            name: rec.name,
            widget: rec.widget,
            ty,
            range: rec.range.map(|(lo, hi)| (lo.0, hi.0)),
            options: rec.options.map(|s| s.0),
            default: rec.default.map(|s| s.0),
            name_node: rec.name_node.0,
        });
    }
    sites
}

/// A top-level declaration's OUTLINE KIND — the fixed closed vocabulary a `Symbols` line reports, the LSP
/// `SymbolKind` analogue keyed to what Cadenza declares. A `def` splits into `value` (a nullary binding
/// `(def answer 42)`) vs `function` (a def WITH parameters `(def (f x) …)`) — the same signature-shape
/// split `scan_top_level` makes — so an outline distinguishes a constant from a function the way an editor
/// paints them differently.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SymbolKind {
    /// A nullary value definition — `(def answer 42)`.
    Value,
    /// A function definition (a def with ≥1 parameter) — `(def (f x) …)`.
    Function,
    /// A sum-type declaration — `(type Option (Some a) None)`.
    Type,
    /// An effect declaration — `(effect E (op emit (-> Int64 Unit)))`.
    Effect,
    /// A module (namespace) declaration — `(module m …)`.
    Module,
}

impl SymbolKind {
    /// Every `SymbolKind` in declaration order — the complete symbols-query vocabulary, so a consumer
    /// can iterate the whole set (build an outline-icon legend, assert a 1:1 wire mapping) without
    /// hand-listing the variants. Mirrors `HighlightKind::ALL`; a variant added to the enum but not here
    /// is caught by the exhaustive-match completeness test.
    pub const ALL: &'static [SymbolKind] = &[
        SymbolKind::Value,
        SymbolKind::Function,
        SymbolKind::Type,
        SymbolKind::Effect,
        SymbolKind::Module,
    ];

    /// The wire spelling — a short stable kebab-case token, the same style as `HighlightKind::as_str`.
    pub fn as_str(self) -> &'static str {
        match self {
            SymbolKind::Value => "value",
            SymbolKind::Function => "function",
            SymbolKind::Type => "type",
            SymbolKind::Effect => "effect",
            SymbolKind::Module => "module",
        }
    }
}

/// The `Symbols` read: every top-level declaration as `name<TAB>kind<TAB>name-node-id`, columns grouped
/// (defs → types → effects → modules) and in declaration order within each — a deterministic function of
/// the program. The NAME occurrence is the node a consumer jumps to; for a def it is the signature's first
/// child (a function's `(NAME param…)` head) or the bare value-def name, matching `Exports`. Skips INTERNAL
/// defs (do-local / module-member callables) — they are not source-level top-level declarations. Total: an
/// empty program yields the empty string.
fn symbols_entries(db: &Db) -> Vec<(String, &'static str, u32)> {
    let mut entries: Vec<(String, &'static str, u32)> = Vec::new();
    let mut emit = |name: &str, kind: SymbolKind, node: StructId| {
        entries.push((name.to_string(), kind.as_str(), node.0));
    };
    for def in &db.defs {
        // A synthesized callable (a recursive do-local / module member) is not a top-level source
        // declaration an outline names — skip it, exactly as the unused-def / export candidacy paths do.
        // Likewise a def whose signature is not a USER node (a prelude / accum-synthesized / β-copied def)
        // has no source span, so it is not part of the outline (the `Highlight`/diagnostic-edge filter).
        if def.internal || !db.is_user_node(def.sig_occ) {
            continue;
        }
        // A def's NAME occurrence: the sig's first child when the signature is a `(NAME param…)` list (a
        // function), else the bare-name signature itself (a value def) — the same node `Exports` reports.
        let name_node = match db.ast.get(def.sig_occ) {
            Struct::List(kids) => kids.first().copied().unwrap_or(def.sig_occ),
            _ => def.sig_occ,
        };
        let kind = if def.params.is_empty() {
            SymbolKind::Value
        } else {
            SymbolKind::Function
        };
        emit(&def.name, kind, name_node);
    }
    for td in &db.type_decls {
        // A type/effect/module declaration's occurrence is its `(type …)`/`(effect …)`/`(module …)` FORM
        // (nominal identity); the NAME node is the form's first tail element (the name atom). Fall back to
        // the form itself if malformed — the name still resolves, only the jump target is coarser.
        //
        // The prelude injects its OWN sums (`Option`/`Result`/`Ordering`/`Ast`/…) into `type_decls` as
        // synthesized declarations with occurrences PAST the source program (no span) — an outline is the
        // USER'S declarations, so filter those exactly as `Highlight` filters synthesized/prelude leaves
        // (`is_user_node`). Effects/modules are user-only today, but gate them the same way for uniformity.
        if !db.is_user_node(td.occ) {
            continue;
        }
        emit(
            &td.name,
            SymbolKind::Type,
            decl_name_node(db, td.occ, "type"),
        );
    }
    for ed in &db.effect_decls {
        if !db.is_user_node(ed.occ) {
            continue;
        }
        emit(
            &ed.name,
            SymbolKind::Effect,
            decl_name_node(db, ed.occ, "effect"),
        );
    }
    for md in &db.modules {
        if !db.is_user_node(md.occ) {
            continue;
        }
        emit(
            &md.name,
            SymbolKind::Module,
            decl_name_node(db, md.occ, "module"),
        );
    }
    entries
}

/// The NAME occurrence of a `(head NAME …)` declaration FORM at `occ` — the first tail element (the name
/// atom), for a go-to jump. Falls back to the form occurrence itself if the form is malformed (no name),
/// so the answer stays total.
fn decl_name_node(db: &Db, occ: StructId, head: &str) -> StructId {
    db.ast
        .as_form(occ, head)
        .and_then(|tail| tail.first().copied())
        .unwrap_or(occ)
}

/// The `Instantiations` read: the definition named `name`'s DISPOSITION plus, if it is specialized, every
/// concrete instantiation. Two line kinds, each TAB-separated with a leading KIND tag:
///   - `disp<TAB>def-name-node-id<TAB>disposition` — the def's fate: `specialized` / `inlined` /
///     `emitted` / `unreferenced` (a `+`-joined set when more than one applies — e.g. a def both inlined
///     at one site and called at another). Exactly ONE `disp` line per matching def.
///   - `inst<TAB>spec-name<TAB>def-name-node-id<TAB>arg;arg;…` — one per DISTINCT specialization (only for
///     the `specialized` disposition), the instantiation payload (unchanged from before the disposition
///     extension — the tag is the sole addition).
///
/// Reads `db.instantiations` (specializations), `db.inlined` (β-reduced call sites), `db.called`
/// (`Core::Call` callees) and `db.exports`, all forced complete by the caller (`force_monomorphize`).
/// Deterministic: instances are in synthesis order (a function of the source). Total: an UNKNOWN name has
/// no def → the empty string (no `disp` line); a known def always gets exactly one `disp` line.
fn instantiations_value(
    db: &mut Db,
    name: &str,
) -> cadenza_compile_abi::instantiations_wire::Instantiations {
    use cadenza_compile_abi::instantiations_wire::{Instance, Instantiations};
    // Resolve the name to its source def. An unknown name → `known: false` (total). A def name that ALSO
    // has specializations resolves to the ORIGINAL (a specialization's synthetic `#mono` name never equals
    // a user name), so `def_by_name` is the right lookup.
    let Some(def_idx) = db.def_by_name(name) else {
        return Instantiations {
            known: false,
            name_node: None,
            dispositions: Vec::new(),
            instances: Vec::new(),
        };
    };

    // The disposition set: which fate(s) this def met over the whole program. Each is a read of a column
    // monomorphization/lowering already fills.
    //   - `specialized` — it has ≥1 instantiation record (a recursive-generic / type-valued / `const`-dict
    //     copy, `db.instantiations`). Its instances are listed below. This SUBSUMES "emitted": the source
    //     generic is realized AS its monomorphic copies, so `emitted` is not also reported.
    //   - `transformed→COPY` — the ACCUMULATOR transform rewrote it (`db.transformed`): its own body folds
    //     to a seed call while the emitted loop lives under the copy's name. Reported as `transformed→f$acc`
    //     rather than the literal `inlined` (which the seed wrapper technically is) — the truer answer to
    //     "is there a function realizing f's recursion?".
    //   - `inlined` — β-reduced into a call site (`db.inlined`) and NOT accumulator-transformed (a
    //     transformed def's seed-wrapper inline is reported as `transformed→…`, not doubled here).
    //   - `emitted` — a `Core::Call` names it under its OWN name (`db.called`), or it is an EXPORT (emitted
    //     as a boundary function even if nothing internal calls it) — and it is neither specialized nor
    //     transformed (those describe HOW it is emitted more precisely).
    //   - `unreferenced` — none of the above: never called, inlined, specialized, transformed, or exported.
    // A def may hold more than one (e.g. inlined at one call yet emitted for a first-class reference) —
    // report the set joined by `+`, in this fixed order (a function of the source).
    let has_instances = db
        .instantiations
        .iter()
        .any(|inst| db.def_index_by_body(inst.orig_body) == Some(def_idx));
    let transformed_to: Option<String> = db
        .transformed
        .get(&def_idx)
        .map(|&copy| db.defs[copy].name.clone());
    let is_inlined = db.inlined.contains(&def_idx);
    let is_exported = db.exports.iter().any(|e| e.def == Some(def_idx));
    let is_called = db.called.contains(&def_idx) || is_exported;
    let mut dispositions: Vec<String> = Vec::new();
    if has_instances {
        dispositions.push("specialized".to_string());
    }
    if let Some(copy) = &transformed_to {
        dispositions.push(format!("transformed→{copy}"));
    }
    // A transformed def's own body inlines the seed wrapper — don't ALSO report that as `inlined`
    // (`transformed→…` already describes it). A genuinely inlined non-transformed def reports `inlined`.
    if is_inlined && transformed_to.is_none() {
        dispositions.push("inlined".to_string());
    }
    // `emitted` only when no more-specific "how it is emitted" tag (specialized / transformed) applies.
    if is_called && !has_instances && transformed_to.is_none() {
        dispositions.push("emitted".to_string());
    }
    if dispositions.is_empty() {
        dispositions.push("unreferenced".to_string());
    }

    // The source def's NAME occurrence (its signature's first child), so a consumer can jump to the
    // definition; `None` (the old `-`) if the signature is malformed. Same name-occurrence convention as
    // `Exports`.
    let sig = db.defs[def_idx].sig_occ;
    let name_node = match db.ast.get(sig) {
        Struct::List(kids) => kids.first().map(|n| n.0),
        _ => None,
    };

    // The instantiation payload — one entry per DISTINCT specialization of this def, in synthesis order.
    // Snapshot first (the immutable `db.instantiations` borrow cannot coexist with the `def_index_by_body`
    // reads), then keep those whose `orig_body` maps back to this def.
    let records: Vec<(StructId, usize, Vec<String>)> = db
        .instantiations
        .iter()
        .map(|inst| (inst.orig_body, inst.spec_index, inst.args.clone()))
        .collect();
    let mut instances: Vec<Instance> = Vec::new();
    for (orig_body, spec_index, args) in records {
        if db.def_index_by_body(orig_body) != Some(def_idx) {
            continue;
        }
        instances.push(Instance {
            spec_name: db.defs[spec_index].name.clone(),
            args,
        });
    }
    Instantiations {
        known: true,
        name_node,
        dispositions,
        instances,
    }
}

/// The documentation of a NAME — the `DocOf` read, an ordered fallback returning the FIRST source that
/// documents it, or `None` if none does (the query then reports a defined "no documentation" line).
///  1. a USER definition's `(doc "…")` text, keyed by the def's signature occurrence (`db.doc_of_def`);
///  2. else a BUILT-IN binding's `(meta doc)` channel — resolve the name to its prelude record, then read
///     the channel generically (`eval::project_meta`), never matching the name against a key;
///  3. else a GRAMMAR KEYWORD's doc (a small table, the doc analogue of `resolve::GRAMMAR`).
fn doc_of_name(db: &mut Db, name: &str) -> Option<String> {
    // 1. A user definition's captured doc.
    if let Some(idx) = db.def_by_name(name) {
        let sig = db.defs[idx].sig_occ;
        if let Some(doc) = db.doc_of_def(sig) {
            return Some(doc.to_string());
        }
    }
    // 2. A built-in binding's `(meta doc)` channel — read off its prelude record, generically.
    if let Some(&rec) = db.prelude.get(name)
        && let Some(doc_node) = crate::eval::project_meta(db, rec, "doc")
        && let Some(text) = db.ast.as_str(doc_node)
    {
        return Some(text.to_string());
    }
    // 2b. A DOTTED MEMBER `Prefix.member` (`Map.insert`) — the common editor case (hovering a member
    // call). The plain-name steps above miss it: the prelude holds `Map` (the module record), not
    // `Map.insert`. Resolve the PREFIX to its record (a built-in prelude module, or a user def bound to
    // a record), then read the MEMBER's `(meta doc)` channel through the SAME generic member path a
    // `(. Map insert)` access takes (`member_value` → `project_meta`), never a hardcoded key. Split on
    // the LAST `.` so a nested prefix (were one to exist) still names its record.
    if let Some(dot) = name.rfind('.') {
        let (prefix, member) = (&name[..dot], &name[dot + 1..]);
        if !prefix.is_empty() && !member.is_empty() {
            let rec = db
                .prelude
                .get(prefix)
                .copied()
                .or_else(|| db.def_by_name(prefix).and_then(|i| db.defs[i].body));
            if let Some(rec) = rec
                && let crate::eval::Member::Field(v) =
                    crate::eval::member_value(db, rec, &crate::resolved::Symbol::plain(member))
                && let Some(doc_node) = crate::eval::project_meta(db, v, "doc")
                && let Some(text) = db.ast.as_str(doc_node)
            {
                return Some(text.to_string());
            }
        }
    }
    // 3. A grammar keyword's doc.
    grammar_keyword_doc(name).map(str::to_string)
}

/// Whether `name` refers to ANYTHING documentable — a user definition, a built-in prelude binding, or a
/// grammar keyword — regardless of whether it actually carries doc text. The SAME resolution ladder
/// `doc_of_name` walks, minus the doc-text read, so `DocOf` can tell a real-but-undocumented name from a
/// typo and give the two cases different "no answer" verdicts. No hardcoded key match (the built-in step
/// tests the prelude record's existence — the generic path a name resolution takes).
fn name_is_known(db: &mut Db, name: &str) -> bool {
    if db.def_by_name(name).is_some()
        || db.prelude.contains_key(name)
        || grammar_keyword_doc(name).is_some()
    {
        return true;
    }
    // A DOTTED MEMBER `Prefix.member` that RESOLVES (mirrors `doc_of_name`'s step 2b) — a real member
    // with no `(meta doc)` is "undocumented", not "no such definition". Resolve the prefix record + test
    // the member field exists (no doc-text read).
    if let Some(dot) = name.rfind('.') {
        let (prefix, member) = (&name[..dot], &name[dot + 1..]);
        if !prefix.is_empty() && !member.is_empty() {
            let rec = db
                .prelude
                .get(prefix)
                .copied()
                .or_else(|| db.def_by_name(prefix).and_then(|i| db.defs[i].body));
            if let Some(rec) = rec {
                return matches!(
                    crate::eval::member_value(db, rec, &crate::resolved::Symbol::plain(member)),
                    crate::eval::Member::Field(_)
                );
            }
        }
    }
    false
}

/// The closest DOCUMENTABLE name to `name` — a user def or a built-in prelude binding — for the
/// "did you mean?" on `DocOf`'s "no such definition" verdict, via the shared `suggest::nearest` (its
/// edit-distance cutoff + 1-char/empty guards decide whether anything is close enough). `None` when no
/// candidate is a confident near-miss (a genuinely novel typo). Grammar keywords are omitted — someone
/// asking `cdz doc <typo>` almost always meant a definition, not a keyword.
fn nearest_known_name(db: &Db, name: &str) -> Option<String> {
    let candidates = db
        .defs
        .iter()
        .map(|d| d.name.as_str())
        .chain(db.prelude.keys().map(String::as_str));
    crate::diag::suggest::nearest(name, candidates)
}

/// The documentation of the definition the node at `id` belongs to or references — the `DocAt` read.
/// Maps the node to a definition by two routes, then reads that def's captured `(doc "…")` text:
///  - the node is a REFERENCE resolving to a def (`resolve::resolved_of` → the def body → its def index),
///    so hovering a USE shows the definition's doc; OR
///  - the node is a def HEADER — its `(def …)` form, its signature list, or its NAME atom
///    (`def_index_by_ident`, the same header→def map `TypeAt`'s hover uses), so hovering the DEFINITION
///    shows its own doc.
///
/// Either route yields a `db.defs` index; its `sig_occ` keys the doc column. `None` if the node reaches
/// no documented definition (the empty hover).
fn doc_at_node(db: &mut Db, id: StructId) -> Option<String> {
    // A reference resolves to the defining occurrence — a nullary def's body (`Ref`) or a function def's
    // lambda body (`Lambda`). Either is a def body `def_index_by_body` maps to its def. This is the SAME
    // body→def path `ResolveOf` walks to find a def's name occurrence, reused here to find its doc.
    let via_ref = match crate::resolve::resolved_of(db, id) {
        Resolved::Ref { value } => Some(value),
        Resolved::Lambda { body, .. } => Some(body),
        _ => None,
    };
    let def_idx = via_ref
        .and_then(|target| db.def_index_by_body(target))
        // Not a reference into a body — the node may BE a def's header (its form/signature/name), the
        // "hover on the definition itself" case, resolved by the header→def map.
        .or_else(|| db.def_index_by_ident(id));
    let di = def_idx?;
    let sig = db.defs[di].sig_occ;
    db.doc_of_def(sig).map(str::to_string)
}

/// The documentation of a GRAMMAR KEYWORD — the final `DocOf` fallback. A keyword (`if`/`let`/`match`/…)
/// is NOT a binding in the prelude map (it is dispatched structurally by `resolve`, see `resolve::GRAMMAR`),
/// so its documentation cannot be a `(meta doc)` channel on a record; it lives here, the doc analogue of
/// the hardcoded keyword set. `None` for a non-keyword (the query then reports "no documentation"). This
/// is a fixed catalog of the language's OWN forms — a presentation surface, not name-dispatched meaning.
fn grammar_keyword_doc(name: &str) -> Option<&'static str> {
    Some(match name {
        "let" => {
            "Bind one or more names over a body: (let ((x e) …) body). Bindings are sequential — a later initializer sees the earlier bindings."
        }
        "if" => {
            "Conditional: (if cond then else). The condition must be Bool; both branches must have the same type."
        }
        "match" => {
            "Deconstruct a value against patterns: (match e (pat body) …). Must be exhaustive — every case of the scrutinee's type is covered."
        }
        "do" => {
            "A sequence of declarations and a final result expression: (do decl… result). Introduces a scope for do-local defs."
        }
        "fn" => {
            "An anonymous function: (fn (param…) body). Its type is the arrow from the parameter types to the body's type."
        }
        "def" => {
            "Define a name: (def (name param…) body) for a function, or (def name value) for a value. May carry a leading (doc \"…\")."
        }
        "export" => "Expose a definition as part of the module's interface: (export name).",
        "module" => {
            "A named collection of definitions: (module name item…). Members are reached by member access."
        }
        "and" => "Short-circuiting boolean AND: (and a b). Evaluates b only if a is true.",
        "or" => "Short-circuiting boolean OR: (or a b). Evaluates b only if a is false.",
        "not" => "Boolean negation: (not a).",
        "quote" => {
            "Suppress evaluation, yielding the operand's AST rather than its value: (quote form)."
        }
        "quasiquote" => {
            "A selective-evaluation template (`): literal structure with (unquote …) / (unquote-splicing …) escapes filled by evaluation."
        }
        "|>" => "Pipeline: (|> L R) threads L as the first argument of the application R.",
        "." => "Member access: (. record key) projects a field or operation from a record/module.",
        ":" => "Type annotation: (: expr Type) asserts and checks expr against Type.",
        "handle" => {
            "Install an effect handler over a body: (handle body (op arm…) …). Handles the effect operations performed within."
        }
        "resume" => {
            "Inside a handler arm, hand a value back to the point that performed the operation: (resume v)."
        }
        "host" => {
            "Delegate an effect operation to the host boundary rather than an in-program handler."
        }
        _ => return None,
    })
}

/// The semantic role of a token, for syntax highlighting — a fixed, closed vocabulary an editor maps to
/// colours. Each kind names WHAT a token means (read off the compiler's columns), not how it is spelled,
/// so a name is coloured by its role: a type distinctly from a value, a constructor from a call, a local
/// from an unbound typo. Deliberately COARSER than the full `Resolved`/`Prim` space — an editor needs a
/// handful of colours, not one per primitive — so many resolved forms fold into one kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HighlightKind {
    /// A grammar keyword atom in syntactic HEAD/annotation position (`if`/`let`/`match`/`def`/`fn`/`:`/…).
    Keyword,
    /// A name denoting a TYPE — a ground type or a type constructor (`Int64`, `String`, `List`, `Option`,
    /// `Map`) — in either value position (a module operand `List.len`) or type position (`(: x Option)`).
    Type,
    /// A SUM VARIANT CONSTRUCTOR (`Some`/`None`/`Ok`/`Err`, a user `(type …)` variant) — a value that
    /// builds a sum, distinguished from an ordinary call by its `(meta variant)` channel.
    Constructor,
    /// A name denoting a CALLABLE VALUE — a top-level function, a prelude operation (`List.len`, `+`), a
    /// user function parameter that is applied. The "this is a function" colour.
    Function,
    /// A FUNCTION PARAMETER binder or a reference to one (a `fn`/`def` formal).
    Param,
    /// A LOCAL VALUE binding or a reference to one — a `let` binding, a match-arm binder, or a reference
    /// to a nullary top-level value def. The ordinary "a bound value" colour.
    Variable,
    /// An EFFECT name or one of its operations (`Ask`, `Ask.ask`) — the effect-system colour.
    Effect,
    /// A LABEL taken as data, never resolved to a value: a record field name, a member-access key, a
    /// symbol literal's content. Coloured like a property/attribute.
    Label,
    /// A numeric literal (int or float).
    Number,
    /// A string literal.
    Str,
    /// A char literal (`#\a`).
    Char,
    /// A byte-string literal (`b"…"`).
    Bytes,
    /// A symbol literal (`#"sym"`).
    Symbol,
    /// A boolean or unit literal (`true`/`false`/`unit`).
    Literal,
    /// A name that FAILS TO RESOLVE — an unbound reference (a typo). The "this is wrong" colour, the one
    /// classification a lexical tokenizer can never make.
    Unbound,
}

impl HighlightKind {
    /// Every `HighlightKind` variant, in declaration order — the canonical vocabulary the `Highlight`
    /// query can emit. A downstream consumer (e.g. the `cdz lsp` server's semantic-token legend) can
    /// iterate this to prove it handles the WHOLE vocabulary, so adding a variant forces a decision there
    /// rather than silently rendering the new kind unclassified. Keep in sync with the enum + `as_str`
    /// (the `highlight_kind_all_is_the_complete_1to1_wire_vocabulary` test pins that this list is complete + 1:1).
    pub const ALL: &'static [HighlightKind] = &[
        HighlightKind::Keyword,
        HighlightKind::Type,
        HighlightKind::Constructor,
        HighlightKind::Function,
        HighlightKind::Param,
        HighlightKind::Variable,
        HighlightKind::Effect,
        HighlightKind::Label,
        HighlightKind::Number,
        HighlightKind::Str,
        HighlightKind::Char,
        HighlightKind::Bytes,
        HighlightKind::Symbol,
        HighlightKind::Literal,
        HighlightKind::Unbound,
    ];

    /// The wire spelling — a short stable token an editor keys its theme on. Kebab-case, single word.
    pub fn as_str(self) -> &'static str {
        match self {
            HighlightKind::Keyword => "keyword",
            HighlightKind::Type => "type",
            HighlightKind::Constructor => "constructor",
            HighlightKind::Function => "function",
            HighlightKind::Param => "param",
            HighlightKind::Variable => "variable",
            HighlightKind::Effect => "effect",
            HighlightKind::Label => "label",
            HighlightKind::Number => "number",
            HighlightKind::Str => "string",
            HighlightKind::Char => "char",
            HighlightKind::Bytes => "bytes",
            HighlightKind::Symbol => "symbol",
            HighlightKind::Literal => "literal",
            HighlightKind::Unbound => "unbound",
        }
    }
}

/// Classify the LEAF at `id` into its highlight role, or `None` to leave it unclassified (an editor
/// falls back to its lexical colour). Reads the SAME columns a compile reads — no new analysis, no name
/// table:
///  - a LITERAL leaf is coloured by its resolved constant kind (`Int`→number, `Str`→string, …);
///  - a NAME in a LABEL position (a member key, a record field, an annotation-nothing) is a `Label`
///    (it is data, never resolved to a value — the same positions `resolve` treats as symbols);
///  - a NAME in HEAD position that is a grammar keyword is a `Keyword`;
///  - any other NAME is resolved (`resolve::resolved_of`) and classified by what it denotes, reading the
///    meta channels a value carries: `(meta variant)` → a `Constructor`, `(meta t)`/a type prim → a
///    `Type`, an effect binding → an `Effect`, `(meta apply)` value op / a function ref → a `Function`,
///    a parameter → `Param`, any other binding → `Variable`, a `Poison(Unbound)` → `Unbound`.
///
/// Only NAME and literal leaves classify; a list form is not itself a token (its children are), so it
/// returns `None`. Total — every path yields a defined kind or `None`, never panics.
fn classify_highlight(db: &mut Db, id: StructId) -> Option<HighlightKind> {
    // Only leaves are painted. A list form's own id is not a token.
    let leaf = match db.ast.get(id) {
        Struct::Atom(l) => *l,
        Struct::List(_) => return None,
    };
    // A literal leaf: colour by the constant kind, read off the leaf directly (no resolution needed).
    match db.ast.leaf(leaf) {
        crate::ast::Leaf::Int { .. }
        | crate::ast::Leaf::Float(_)
        | crate::ast::Leaf::FloatNan
        | crate::ast::Leaf::FloatInf { .. }
        // A type-suffixed numeric literal (`100N`/`0.5R`) is a number for highlighting. (The codec
        // decodes it to a plain Int/Float for the compiler, so it only survives on the syntax side.)
        | crate::ast::Leaf::Suffixed { .. } => {
            return Some(HighlightKind::Number);
        }
        crate::ast::Leaf::Str(_) => return Some(HighlightKind::Str),
        crate::ast::Leaf::Char(_) | crate::ast::Leaf::BadChar(_) => {
            return Some(HighlightKind::Char);
        }
        crate::ast::Leaf::Bytes(_) => return Some(HighlightKind::Bytes),
        crate::ast::Leaf::Sym(_) => return Some(HighlightKind::Symbol),
        crate::ast::Leaf::Bool(_) => return Some(HighlightKind::Literal),
        // A `BadEscape` marker rides inside a string literal — colour it as a string.
        crate::ast::Leaf::BadEscape(_) => return Some(HighlightKind::Str),
        // A native-compound-data CTOR-HEAD leaf (a `#list`/`#record`/… collection head, the `=` field-pair,
        // the `.` member) is a reserved structural constructor head — paint it as a keyword.
        crate::ast::Leaf::Ctor(_) | crate::ast::Leaf::FieldPair | crate::ast::Leaf::Member | crate::ast::Leaf::Rational => {
            return Some(HighlightKind::Keyword);
        }
        // A name leaf — classified below by what it denotes.
        crate::ast::Leaf::Name(_) => {}
    }
    let name = db.ast.as_name(id)?;

    // `unit` is a nullary literal name (the unit value), coloured like `true`/`false`.
    if name == "unit" {
        return Some(HighlightKind::Literal);
    }

    // A LABEL position — a name taken as data, never resolved to a value. These are exactly the positions
    // `resolve` reads as a `Symbol` rather than looking up: a member-access KEY `(. operand key)` and a
    // record FIELD name `(field value)` inside a `("record" …)`. Colouring them as values would resolve a
    // field/key name in scope and mis-flag it (or paint it as an unbound typo).
    if is_label_position(db, id) {
        return Some(HighlightKind::Label);
    }

    // An EFFECT OPERATION NAME — the second child of an `(op <name> <type>)` signature inside an
    // `(effect …)` declaration. It DECLARES an operation of the effect (it never resolves to a value on
    // its own — the resolver reads the effect record, not the bare op name), so classify it structurally
    // as an `Effect` rather than letting it fall through to a spurious `Unbound`.
    if is_effect_op_name(db, id) {
        return Some(HighlightKind::Effect);
    }

    // A GRAMMAR KEYWORD in head/annotation position (`if`/`let`/`def`/`:`/`and`/…, plus the declaration
    // heads `effect`/`op`/`type` and the desugar-orphaned control heads `handle`/`host`/`resume`). Only
    // in a syntactic position — a bare value that happens to share the spelling is not a keyword.
    if is_keyword_position(db, id) {
        return Some(HighlightKind::Keyword);
    }

    // An ANNOTATION SIGIL or NAME — the `@` head and the annotation name of a well-formed `(@ NAME (def …))`
    // (`@test`, `@exhaustive`, `@inline-never`, …). `db::strip_annotations` UNWRAPS the wrapper node in
    // place (the def adopts the inner children) and ORPHANS the original `@`/name occurrences to
    // `parent_of == None` — so a leaf walk still visits them (they keep source spans) but they resolve to
    // nothing (`@` is no grammar head, no value op; the name denotes no binding) → a spurious `Unbound`.
    // Paint them `Keyword` (annotations are keyword-like syntax) so a `@test` reads as a decorator, not a
    // wall of error-red or a generic data symbol. Sound because ONLY `strip_annotations` orphans a `@` or a
    // non-`handle` name to a MISSING parent: an annotation on a NON-def is a CDZ0201 (not rewritten, so its
    // `@` KEEPS its parent → falls through to red with its real squiggle); quoted data retains a `Some`
    // parent (the chain stops at the quoted list, not the leaf); `desugar_handles` orphans only `handle`.
    if is_annotation_token(db, id) {
        return Some(HighlightKind::Keyword);
    }

    // The SIGIL or HEAD of a LIVE `@param` annotation `(@ (param …) name)` — the call-style annotation
    // `strip_annotations` does NOT unwrap (it wraps a PARAM binder inside a `(: … Type)`, not a `(def …)`,
    // so the def-only unwrap never fires and the whole form stays in the arena, root-reachable — unlike the
    // orphaned def-annotation case above). Its `@` and `param` head resolve to nothing → a spurious
    // `Unbound`. Paint them `Keyword` (a decorator). The config payload (`widget`/`slider`) is handled at
    // the unbound step below, not here. (Keyed on the recognized `(param …)` head so a malformed
    // `(@ foo 42)` CDZ0201 is NOT matched and keeps its real red squiggle.)
    if is_param_annotation_keyword(db, id) {
        return Some(HighlightKind::Keyword);
    }

    // A BINDING DECLARATION site — a `let` binder name, or a bare match-arm binder pattern. These are
    // where a name is DECLARED, not referenced: resolving one looks it up before it binds and reports a
    // spurious `Unbound`. So classify by the binding role (a local `variable`) without resolving. (A
    // PARAMETER declaration is handled by resolution — it resolves to `Resolved::Param`; a DEF name
    // resolves to its own body, colouring as function/variable — so only let/match binders need this.)
    if is_local_binder_decl(db, id) {
        return Some(HighlightKind::Variable);
    }

    // Any other name: resolve it and classify by what it denotes, reading the meta channels the value
    // carries. `resolved_of` is the SAME producer a compile reads, so a token's colour equals its meaning.
    let kind = classify_resolved_name(db, id);

    // QUOTED-DATA STOPGAP. A name that resolves to `Unbound` but is DETACHED from the program root is
    // inert quoted data, not a live typo: `reify_quotes` overwrites a `(quasiquote …)` node with its
    // synthetic Ast-construction and leaves the ORIGINAL quoted children dangling (they keep their source
    // spans — so the highlighter still paints them — but their ancestor chain no longer reaches
    // `db.ast.root`). Painting such a leaf `Unbound` (error-RED) is wrong: the program is well-formed
    // (`check` reports no error). Reclassify a detached would-be-unbound leaf as `Symbol` (the "name taken
    // as data" colour) so a quasiquote template reads as data, not a wall of fake errors. Narrow by design:
    // only a would-be-`Unbound` leaf that is unreachable from root is touched — a live unbound typo IS
    // reachable, so real errors still colour red. (Precise provenance is v-metaprogramming's follow-up:
    // a `quoted-data` marker on these leaves; this reachability heuristic is the interim, and tightens to
    // reading that marker once it lands.)
    if kind == HighlightKind::Unbound && !reaches_root(db, id) {
        return Some(HighlightKind::Symbol);
    }

    // A `@param` CONFIG value — a would-be-`Unbound` leaf inside the `(param <kv>…)` application of a live
    // `@param` annotation (`widget`/`slider` in `@param(widget: slider) width`). The config kv pairs are
    // inert metadata the sidecar reads structurally (a widget descriptor, not a value scope), so a bare
    // config name resolves to nothing → a spurious `Unbound` on a well-formed program. Soften it to
    // `Symbol` (data), like the quoted-data case. Only a would-be-`Unbound` leaf is touched — a config
    // position that DID resolve (a future live reference) keeps its real colour, and the `@`/`param` head +
    // the annotation TARGET (the param name/type) are handled elsewhere, so this reaches only the payload.
    if kind == HighlightKind::Unbound && within_param_annotation_config(db, id) {
        return Some(HighlightKind::Symbol);
    }
    Some(kind)
}

/// Whether the leaf at `id` is the `@` sigil or the `param` HEAD of a LIVE `@param` annotation
/// `(@ (param …) name)` — the syntax of the call-style annotation, painted `Keyword` (a decorator). Keyed
/// on the RECOGNIZED shape (an `@` form whose first child is a `(param …)` application), matching
/// `param_sidecar`'s own `(@ (param …) _)` detector, so an unrelated / malformed `(@ …)` is not caught
/// (its `@` keeps falling through to its real `Unbound` red).
///  - the `@` sigil: `id` is the HEAD of a `(@ (param …) name)` form;
///  - the `param` head: `id` is the HEAD of the `(param …)` application that is the `@`'s first child.
fn is_param_annotation_keyword(db: &Db, id: StructId) -> bool {
    let Some(parent) = db.parent_of(id) else {
        return false;
    };
    // `id` is the `@` sigil: `id` is the HEAD of its parent `(@ APP name)`, and the annotation's first
    // tail element `APP` is a `(param …)` application. (`as_form(parent, "@")` returns the tail AFTER the
    // `@` head, so `tail.first()` is `APP`, not `id`; `id` being the head is checked structurally.)
    if db.ast.as_name(id) == Some("@")
        && matches!(db.ast.get(parent), Struct::List(kids) if kids.first() == Some(&id))
        && let Some(tail) = db.ast.as_form(parent, "@")
        && tail
            .first()
            .is_some_and(|&app| db.ast.as_form(app, "param").is_some())
    {
        return true;
    }
    // `id` is the `param` head: `id` is the HEAD of its parent `(param …)`, and that parent is in turn the
    // FIRST child (name position) of an enclosing `(@ (param …) name)` — the `@param` shape.
    if db.ast.head_name(parent) == Some("param")
        && matches!(db.ast.get(parent), Struct::List(kids) if kids.first() == Some(&id))
        && let Some(grand) = db.parent_of(parent)
        && let Some(gtail) = db.ast.as_form(grand, "@")
        && gtail.first() == Some(&parent)
    {
        return true;
    }
    false
}

/// Whether the leaf at `id` sits inside the `(param <kv>…)` application of a live `@param` annotation
/// `(@ (param …) name)` — a CONFIG-payload position (a `widget`/`slider` kv leaf), inert metadata softened
/// to `Symbol`. Walks up from `id`: some ancestor must be a `(param …)` application that is the FIRST
/// child (name position) of an enclosing `(@ …)` form. Excludes the annotation TARGET (the param name is
/// the `@`'s SECOND child, never under the `(param …)` subtree) so it still resolves normally.
fn within_param_annotation_config(db: &Db, id: StructId) -> bool {
    let mut cursor = db.parent_of(id);
    while let Some(node) = cursor {
        if db.ast.head_name(node) == Some("param")
            && let Some(grand) = db.parent_of(node)
            && let Some(gtail) = db.ast.as_form(grand, "@")
            && gtail.first() == Some(&node)
        {
            return true;
        }
        cursor = db.parent_of(node);
    }
    false
}

/// Whether the node at `id` is connected to the arena root by the parent chain. A node whose walk hits
/// `None` before reaching `db.ast.root` is DETACHED — e.g. the original quoted children `reify_quotes`
/// orphans when it overwrites a quasiquote node with its synthetic copy. Bounded by the arena depth.
fn reaches_root(db: &Db, id: StructId) -> bool {
    if id == db.ast.root {
        return true;
    }
    let mut cursor = db.parent_of(id);
    while let Some(node) = cursor {
        if node == db.ast.root {
            return true;
        }
        cursor = db.parent_of(node);
    }
    false
}

/// Whether the name at `id` is in a LABEL position — a member-access KEY (`(. operand KEY)`, second
/// child of a `.` form) or a RECORD FIELD name (first child of a `(field value)` pair inside a
/// `("record" …)`). These are the positions `resolve` reads as a `Symbol` (data), not a value lookup.
fn is_label_position(db: &Db, id: StructId) -> bool {
    let Some(parent) = db.parent_of(id) else {
        return false;
    };
    // Member key: `(. operand KEY)` — `id` is the third child (index 2) of a `.`-headed form.
    if let Some(tail) = db.ast.as_form(parent, ".")
        && tail.len() == 2
        && tail[1] == id
    {
        return true;
    }
    // Record field name: `id` is a record field entry's LABEL, and the entry's enclosing form is a record
    // (the native `#record` ctor-leaf head OR the `("record" …)` alias — `compound_form_of` reads both).
    // Two entry spellings: the legacy 2-element POSITIONAL `(field value)` (label = child[0], read as the
    // field label by `read_record_fields`) and the native FieldPair `(= field value)` (label = the key).
    // A `#map`'s FieldPair KEY is a VALUE, not a label — restricting the grandparent to Record excludes it.
    if let Some(grand) = db.parent_of(parent)
        && db
            .ast
            .compound_form_of(grand, CompoundCtor::Record)
            .is_some()
    {
        // Native `(= field value)` FieldPair (or the name-head `=` alias) — the KEY is the field label.
        let fieldpair_label = db
            .ast
            .field_pair_parts(parent)
            .or_else(|| db.ast.field_pair(parent))
            .map(|(k, _)| k);
        if fieldpair_label == Some(id) {
            return true;
        }
        // Legacy 2-element positional `(field value)` — the first child is the field label.
        if let Struct::List(pair) = db.ast.get(parent)
            && pair.len() == 2
            && pair[0] == id
        {
            return true;
        }
    }
    false
}

/// Whether the name at `id` is a GRAMMAR KEYWORD used in a syntactic position — the head (first child)
/// of its enclosing list, or the annotation colon. Only a name in head position is the keyword; the same
/// spelling as a bare value is not.
///
/// The keyword set is the resolver's `GRAMMAR` heads PLUS the DECLARATION-form heads (`effect`/`op`/
/// `type`) — forms the resolver dispatches structurally by head-name but that are NOT in `GRAMMAR` (they
/// are recognized as declarations elsewhere), so their head token would otherwise fall through to
/// resolution and paint as a spurious `Unbound`. These are exactly the keyword heads the hover
/// classifier (`grammar_keyword`) already lists — keeping the two token classifiers in agreement.
///
/// The head-position check tolerates a MISSING parent: `effects::desugar_handles` re-spells a
/// `(handle …)` head to `handle-internal` IN PLACE, which ORPHANS the original `handle` atom (it stays
/// in the arena — so the leaf walk still visits it — but is no longer any list's child). A control-flow
/// keyword atom with no parent is therefore that orphaned head; it is still the keyword it spells.
fn is_keyword_position(db: &Db, id: StructId) -> bool {
    let Some(name) = db.ast.as_name(id) else {
        return false;
    };
    if !is_highlight_keyword(name) {
        return false;
    }
    match db.parent_of(id) {
        Some(parent) => {
            matches!(db.ast.get(parent), Struct::List(kids) if kids.first() == Some(&id))
        }
        // No parent: a desugar-orphaned control head (see the doc comment). `desugar_handles` is the only
        // pass that orphans a head atom, and only for `handle`; treat any parentless keyword atom as its
        // (former) head position rather than resolving it to `Unbound`.
        None => true,
    }
}

/// The keyword-head names the highlighter paints as `Keyword` — the resolver's structural `GRAMMAR`
/// heads plus the declaration-form heads `effect`/`op`/`type` (declarations the resolver dispatches by
/// head-name but keeps out of `GRAMMAR`). One source of truth for [`is_keyword_position`].
fn is_highlight_keyword(name: &str) -> bool {
    crate::resolve::is_grammar_head(name) || matches!(name, "effect" | "op" | "type")
}

/// Whether the leaf at `id` is an ANNOTATION TOKEN — the `@` sigil or the NAME of a well-formed
/// `(@ NAME (def …))` annotation, LEFT ORPHANED (`parent_of == None`) by `db::strip_annotations`'s
/// in-place unwrap. Two shapes qualify, distinguished by spelling:
///  - the `@` head itself: `@` has NO other role in the language (not a `GRAMMAR` head, not a value op),
///    and only `strip_annotations` orphans a `@` occurrence — so a PARENTLESS `@` is unambiguously an
///    unwrapped annotation sigil. (An annotation on a NON-def is a CDZ0201 that is NOT rewritten, so its
///    `@` keeps its parent and is excluded here — its red squiggle is correct.)
///  - an annotation NAME in [`crate::db::KNOWN_ANNOTATIONS`] (`test`/`exhaustive`/`inline-never`/…): a
///    parentless leaf spelled as one of these is the name half of the same unwrapped wrapper. Restricting
///    to the known catalog keeps it sound — the only other pass that orphans a NAME leaf is
///    `desugar_handles`, which orphans only `handle` (not a known annotation), and quoted data keeps a
///    `Some` parent (its chain stops at the quoted list, never `None` at the leaf itself).
///
/// Painting these `Keyword` makes a `@test`/`@exhaustive` render as a decorator rather than a generic data
/// `Symbol` (the incidental colour they get today from the `reaches_root` unbound-stopgap).
fn is_annotation_token(db: &Db, id: StructId) -> bool {
    // Both annotation-token shapes are orphaned to a MISSING parent; a leaf that still has a parent is a
    // live occurrence (a value ref, or the `@` of a malformed CDZ0201 annotation) — never one of these.
    if db.parent_of(id).is_some() {
        return false;
    }
    match db.ast.as_name(id) {
        Some("@") => true,
        Some(name) => crate::db::KNOWN_ANNOTATIONS.contains(&name),
        None => false,
    }
}

/// Whether the name at `id` is an EFFECT OPERATION NAME — the second child of an `(op <name> <type>)`
/// operation signature (the shape an `(effect …)` declaration's operations take). Such a name declares
/// an operation of the effect; it resolves to nothing on its own (the effect RECORD is what resolves),
/// so a leaf walk would paint it `Unbound` — classify it as an `Effect` by position instead.
fn is_effect_op_name(db: &Db, id: StructId) -> bool {
    let Some(parent) = db.parent_of(id) else {
        return false;
    };
    matches!(db.ast.as_form(parent, "op"), Some(tail) if tail.first() == Some(&id))
}

/// Whether the name at `id` is a LOCAL BINDER DECLARATION — a name that DECLARES a binding rather than
/// references a value. Resolving one looks the name up before it binds and reports a spurious `Unbound`,
/// so these are classified structurally (as `Variable`) without resolving. Covers exactly the binder
/// positions `resolve`/`binder_in` recognize but that a whole-program leaf walk would otherwise resolve
/// as value references:
///  - a `let` binding NAME `(name init)` (first child of a pair in a let's bindings-list);
///  - a bare match-arm binder pattern `(name body)` (the pattern binds the whole scrutinee);
///  - a VARIANT-pattern payload binder `(Ctor v …)` (a NON-head name in an arm pattern — the ctor head
///    itself resolves to the constructor, so it is excluded);
///  - a `(module NAME …)` declaration name (a top-level module name binds no value — it is the wrapper).
///
/// A def/type/effect declaration name is NOT here — each resolves meaningfully (a def to its body/lambda,
/// a type/effect to its synth record), so those classify correctly through resolution.
fn is_local_binder_decl(db: &Db, id: StructId) -> bool {
    let Some(parent) = db.parent_of(id) else {
        return false;
    };
    // A `(module NAME …)` declaration name — the first element after `module`.
    if db
        .ast
        .as_form(parent, "module")
        .and_then(|t| t.first().copied())
        == Some(id)
    {
        return true;
    }
    // A `let` binding pair `(name init)`: `id` is its first child, and the pair sits in a let's
    // bindings-list (the same shape `collect_let_bindings` reads).
    if let Struct::List(kv) = db.ast.get(parent)
        && kv.len() == 2
        && kv[0] == id
        && let Some(bindings_list) = db.parent_of(parent)
        && let_of_bindings_list(db, bindings_list)
    {
        return true;
    }
    // A match ARM `(pattern body)` whose pattern is the bare binder `id` (not the wildcard `_`, not a
    // ctor pattern — those are lists). The parent must be a `(match …)` arm, not the scrutinee.
    if let Struct::List(arm) = db.ast.get(parent)
        && arm.len() == 2
        && arm[0] == id
        && db.ast.as_name(id) != Some("_")
        && is_match_arm(db, parent)
    {
        return true;
    }
    // A VARIANT-pattern payload binder `(Ctor v …)`: `id` is a NON-head element of a list that is itself
    // an arm's PATTERN (pb[0]). The constructor head is `parent`'s first child (excluded here — it
    // resolves to the variant ctor). Mirrors `is_list_pattern_element_occurrence`, generalized past the
    // `(list …)` head to any constructor pattern.
    if let Struct::List(kids) = db.ast.get(parent)
        && kids.first() != Some(&id)
        && kids.contains(&id)
        && let Some(arm) = db.parent_of(parent)
        && let Struct::List(pb) = db.ast.get(arm)
        && pb.len() == 2
        && pb[0] == parent
        && is_match_arm(db, arm)
    {
        return true;
    }
    false
}

/// Whether `arm` is a MATCH ARM — a two-element `(pattern body)` list whose parent is a `(match
/// scrutinee arm…)` and which is NOT the scrutinee (the first tail element). The shared "is this a match
/// arm" check `is_local_binder_decl` uses for both the bare-binder and the payload-binder patterns.
fn is_match_arm(db: &Db, arm: StructId) -> bool {
    let Some(matchf) = db.parent_of(arm) else {
        return false;
    };
    match db.ast.as_form(matchf, "match") {
        Some(mtail) => mtail.first().copied() != Some(arm) && mtail.contains(&arm),
        None => false,
    }
}

/// Classify a resolved NAME (already known not to be a label or keyword) by what it denotes. Reads the
/// resolved column and the meta channels a value carries — the role lives in the data, so there is no
/// name→role table here. `Unbound` is the one classification a lexical pass cannot make.
fn classify_resolved_name(db: &mut Db, id: StructId) -> HighlightKind {
    match crate::resolve::resolved_of(db, id) {
        // A formal parameter binder / reference.
        Resolved::Param { .. } => HighlightKind::Param,
        // A direct primitive (an `(intrinsic …)` value, or a prim reached bare) — a type former colours
        // as a type, everything else is a callable operation.
        Resolved::Prim(p) => {
            if p.denotes_type() {
                HighlightKind::Type
            } else {
                HighlightKind::Function
            }
        }
        // A reference to a bound value. WHAT it is bound to decides the colour — read the target's role
        // off its own resolved form + meta channels, the same demand-driven reads a compile makes.
        Resolved::Ref { value } => classify_ref_target(db, id, value),
        // A name that heads a def WITH parameters resolves to a Lambda — a function.
        Resolved::Lambda { .. } => HighlightKind::Function,
        // An unbound name (a typo) — the classification only the compiler can make.
        Resolved::Poison(reject) => {
            if reject.code == Some(crate::diag::Code::Unbound) {
                HighlightKind::Unbound
            } else {
                // A malformed/declined name is not necessarily unbound — leave it to the lexical fallback
                // by classifying it as a plain variable (it still names something value-shaped).
                HighlightKind::Variable
            }
        }
        // Any other resolved form at a NAME leaf (a member/apply/etc. would be a list, not a leaf) — treat
        // as a plain value binding.
        _ => HighlightKind::Variable,
    }
}

/// Classify a `Ref { value }` at name occurrence `id` by the ROLE of its target `value`. The target is a
/// def body / let init / sum record / effect record / prelude module — read its role off the resolved
/// column + meta channels:
///  - a value carrying `(meta variant)` is a CONSTRUCTOR (`Some`/`None`/a user variant);
///  - a value that reduces to a TYPE (`(meta t)` / a type prim) is a TYPE (`Int64`, `Option`, `List`);
///  - a value that IS an effect declaration is an EFFECT;
///  - a value carrying `(meta apply)` a non-type prim, or a function def, is a FUNCTION (`List`'s
///    operations, `+`, a top-level fn);
///  - anything else is a plain VARIABLE (a nullary value def, a `let`/match binding).
///
/// The order matters: a variant record ALSO carries `(meta t)` (its constructor type), so the variant
/// check precedes the type check — exactly the precedence `typeval_of` uses (`(meta variant)` shadows
/// `(meta t)`).
fn classify_ref_target(db: &mut Db, id: StructId, value: StructId) -> HighlightKind {
    // (0) A reference to a PARAMETER — `binder_in` resolves a param reference to a `Ref` at the param's
    // binder occurrence, which itself resolves to `Resolved::Param`. Colour the reference like the binder
    // (both `param`), so a param and its uses share one colour.
    if matches!(
        crate::resolve::resolved_of(db, value),
        Resolved::Param { .. }
    ) {
        return HighlightKind::Param;
    }
    // (1) A CONSTRUCTOR — the value carries a `(meta variant)` discriminant. Checked first: a variant
    // record also has a `(meta t)`, so this must precede the type check (the `typeval_of` precedence).
    if crate::eval::variant_disc_of(db, value).is_some() {
        return HighlightKind::Constructor;
    }
    // (2) An EFFECT name — the reference resolves to an effect declaration's synth record. Recognized by
    // the reverse index (`effect_decl_by_synth`), NOT a name match. An effect OPERATION (`Ask.ask`) is a
    // member access (a list, not this leaf), so this catches the bare effect name; the operation leaf is
    // the member KEY, already a `Label` — acceptable (the effect name carries the colour).
    if db.effect_decl_by_synth(value).is_some() {
        return HighlightKind::Effect;
    }
    // (3) A TYPE — the value reduces to a type-value (a ground type or an applied type constructor). Reads
    // `(meta t)` via `typeval_of` (which already excludes variant records). A bare type constructor whose
    // `(meta apply)` is a type former also counts.
    if crate::eval::typeval_of(db, value).is_some() {
        return HighlightKind::Type;
    }
    if let Some(p) = crate::eval::meta_apply_of(db, value) {
        return if p.denotes_type() {
            HighlightKind::Type
        } else {
            // (4) A CALLABLE operation — `(meta apply)` a value prim (`+`, `List.len`, a module op).
            HighlightKind::Function
        };
    }
    // (5) A function def (with parameters) reached through a `Ref` to its body — the body's owning def has
    // parameters. (A nullary value def has none → a plain variable.)
    if let Some(d) = db.def_index_by_body(value)
        && !db.defs[d].params.is_empty()
    {
        return HighlightKind::Function;
    }
    // (6) Anything else — a nullary value def, a `let`/match binding, a parameter's bound value.
    let _ = id;
    HighlightKind::Variable
}

/// The bindings visible at `node`, innermost-first, deduplicated by name (a shadowed name keeps the
/// nearest enclosing binder). Walks parents (the same `parent_of` substrate `resolve::lookup_scope`
/// uses), and at each enclosing form collects the binders it introduces AS SEEN from the child we
/// ascended through:
///  - a `let` ascended from its BODY → all its bindings; ascended from a bindings-list PAIR → the
///    bindings strictly before that pair (sequential scope: an initializer sees earlier bindings);
///  - a `fn`/`def` ascended from its BODY → its parameters (`db.scope_binders_of`);
///  - a match ARM ascended from its body, whose pattern is a bare binder → that binder (binds the
///    scrutinee for the arm).
///
/// Returns `(name, binder-name-occurrence)` pairs; the caller types each binder. A non-user or unknown
/// node yields the empty scope (total).
fn scope_at(db: &Db, node: StructId) -> Vec<(String, StructId)> {
    let mut out: Vec<(String, StructId)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    if !db.is_user_node(node) {
        return out;
    }
    // Ascend parent by parent; `from` is the child we came up through (its position in a `let`
    // bindings-list decides which bindings are visible).
    let mut from = node;
    let mut cursor = db.parent_of(node);
    while let Some(form) = cursor {
        let mut add = |name: &str, occ: StructId, out: &mut Vec<(String, StructId)>| {
            // Innermost wins: only the first sighting of a name (nearest enclosing binder) is kept.
            if seen.insert(name.to_string()) {
                out.push((name.to_string(), occ));
            }
        };
        match db.ast.head_name(form) {
            // `let` ascended from the BODY → every binding; ascended from a pair (handled below via
            // the bindings-list form) is a different shape.
            Some("let") => {
                if let Some(tail) = db.ast.as_form(form, "let") {
                    let bindings_occ = tail.first().copied();
                    let body_occ = tail.get(1).copied();
                    if Some(from) == body_occ
                        && let Some(bindings_occ) = bindings_occ
                    {
                        collect_let_bindings(db, bindings_occ, None, &mut |n, occ| {
                            add(n, occ, &mut out)
                        });
                    }
                }
            }
            // `fn`/`def` ascended from the BODY → its parameters (the precomputed binder index).
            Some("fn") | Some("def") => {
                let tail = db
                    .ast
                    .as_form(form, "fn")
                    .or_else(|| db.ast.as_form(form, "def"));
                let body_occ = tail.and_then(|t| t.get(1).copied());
                if Some(from) == body_occ {
                    // Sorted for determinism (the index is a hash map).
                    let mut params: Vec<(String, StructId)> = db
                        .scope_binders_of(form)
                        .map(|(n, occ)| (n.to_string(), occ))
                        .collect();
                    params.sort();
                    for (n, occ) in params {
                        add(&n, occ, &mut out);
                    }
                }
            }
            _ => {
                // A let's BINDINGS-LIST (a headless list of pairs) ascended from a pair → the bindings
                // strictly before that pair are visible (an initializer sees the earlier ones).
                if let_of_bindings_list(db, form) {
                    collect_let_bindings(db, form, Some(from), &mut |n, occ| add(n, occ, &mut out));
                }
                // A match ARM `(pattern body)` ascended from `body`, pattern a bare binder → it binds.
                if let Some((n, occ)) = match_arm_binder(db, form, from) {
                    add(&n, occ, &mut out);
                }
            }
        }
        from = form;
        cursor = db.parent_of(form);
    }
    out
}

/// Collect a let bindings-list's `(name init)` pairs as `(name, name-occurrence)`, calling `add` for
/// each. When `stop_before` is `Some(pair)`, only the pairs STRICTLY BEFORE it (sequential scope);
/// `None` = all. Later-bound names naturally shadow earlier ones because `add`'s caller keeps the FIRST
/// sighting — so we emit in REVERSE (last binding first) to make the innermost/last win.
fn collect_let_bindings(
    db: &Db,
    bindings_occ: StructId,
    stop_before: Option<StructId>,
    add: &mut dyn FnMut(&str, StructId),
) {
    let Struct::List(pairs) = db.ast.get(bindings_occ) else {
        return;
    };
    let end = match stop_before {
        None => pairs.len(),
        Some(from) => db.child_ix_of(from).min(pairs.len()),
    };
    for &pair in pairs[..end].iter().rev() {
        if let Struct::List(kv) = db.ast.get(pair)
            && kv.len() == 2
            && let Some(name) = db.ast.as_name(kv[0])
        {
            // The binder's TYPE is its initializer's type — key the binding on the init occurrence
            // (what a `Resolved::Ref` to this name points at), so `type_of` gives the bound value's type.
            add(name, kv[1]);
        }
    }
}

/// Whether `form` is a `let`'s bindings-list (its parent is a `let` and `form` is that let's first
/// tail element). Mirrors `resolve::let_of_bindings_list` but returns a bool.
fn let_of_bindings_list(db: &Db, form: StructId) -> bool {
    db.parent_of(form)
        .and_then(|p| {
            db.ast
                .as_form(p, "let")
                .map(|t| t.first().copied() == Some(form))
        })
        .unwrap_or(false)
}

/// If `form` is a match ARM `(pattern body)`, ascended from `body`, whose `pattern` is a bare BINDER
/// name (not a literal, not the wildcard `_`), return that binder `(name, name-occurrence)`. The binder
/// binds the scrutinee for the arm; its type is the scrutinee's, so key on the scrutinee occurrence.
fn match_arm_binder(db: &Db, form: StructId, from: StructId) -> Option<(String, StructId)> {
    let Struct::List(arm) = db.ast.get(form) else {
        return None;
    };
    if arm.len() != 2 || Some(from) != arm.get(1).copied() {
        return None;
    }
    let pattern = arm[0];
    let name = db.ast.as_name(pattern)?;
    if name == "_" {
        return None;
    }
    // The parent must be a `(match …)` for this to be a match arm (not any 2-element list).
    let parent = db.parent_of(form)?;
    let scrutinee = db.ast.as_form(parent, "match")?.first().copied()?;
    // The binder's value is the scrutinee — type/flow through it — so key the binding there.
    Some((name.to_string(), scrutinee))
}

/// The HOVER verdict for a node as a structured [`cadenza_compile_abi::TypeAt`] (the consumer renders the
/// display text). In priority order:
///  1. a non-user node (past the program, or a synthesized one with no source) → [`TypeAt::Unknown`];
///  2. a DEFINITION the node identifies (its `(def …)` form, its signature list, or its NAME atom) →
///     [`TypeAt::Def`] carrying its name + signature scheme payload (or `None` when unsolved) — so hovering
///     a def shows what it defines (rendered `name : <type>`), not its body's type alone;
///  3. a GRAMMAR KEYWORD atom (`def`/`export`/`module`/`if`/`let`/`match`/`fn`/`:`/`and`/`or`/`not`/`.`) →
///     [`TypeAt::Keyword`] — syntax, not an expression, so it has no type;
///  4. otherwise the node's solved type, cleaned at the Ty LEVEL (see [`hover_ty_verdict`]): the fallback
///     `Ty::Any` → [`TypeAt::Unknown`], a leaked prelude-operator RECORD → its callable field, else the
///     structured type payload as [`TypeAt::Ty`].
fn hover_verdict(db: &mut Db, id: StructId) -> cadenza_compile_abi::TypeAt {
    use cadenza_compile_abi::TypeAt;
    if !db.is_user_node(id) {
        return TypeAt::Unknown;
    }
    // (2) A definition the node identifies → its signature scheme payload (or None when unsolved).
    if let Some(def) = def_identified_by(db, id) {
        let name = db.defs[def].name.clone();
        let ty = crate::infer::def_scheme(db, def).map(|scheme| {
            let root = crate::eval::encode_ty_payload(db, &scheme.ty);
            extract_subtree(&db.ast, root)
        });
        return TypeAt::Def { name, ty };
    }
    // (3) A grammar keyword atom.
    if let Some(kw) = grammar_keyword(db, id) {
        return TypeAt::Keyword(kw.to_string());
    }
    // (4) The solved type. (4b) An UN-ANNOTATED def-parameter binder types as `Ty::Any` here — a
    // non-recursive def's param is never solved standalone (it inlines at each call). But the body's uses
    // DO constrain it (`(+ x 1)` pins `x` to `Int64`). Recover that INFERRED type (`query_param_ty`, which
    // normalizes the binder occurrence AND a body USE of it, and returns `None` for a non-param / genuinely
    // generic node) so the hover shows the type the author did NOT write, rather than "unknown".
    let mut ty = crate::infer::type_of(db, id);
    if matches!(ty, crate::ty::Ty::Any)
        && let Some(t) = crate::infer::query_param_ty(db, id)
    {
        ty = t;
    }
    hover_ty_verdict(db, &ty)
}

/// Map a hover node's solved `Ty` to the [`cadenza_compile_abi::TypeAt`] type verdict, cleaning two poor
/// forms at the Ty level (what the old string `clean_hover_type` did on the render): a bare [`Ty::Any`]
/// (untypeable/poison) → [`TypeAt::Unknown`]; a leaked prelude-operator RECORD (a `Ty::Record` carrying a
/// callable `t` field — the value an operator NAME like `+` resolves to) → its `t` field's callable scheme,
/// so hover surfaces the arrow, not the record. Any other type → the structured payload as [`TypeAt::Ty`].
fn hover_ty_verdict(db: &mut Db, ty: &crate::ty::Ty) -> cadenza_compile_abi::TypeAt {
    use cadenza_compile_abi::TypeAt;
    if matches!(ty, crate::ty::Ty::Any) {
        return TypeAt::Unknown;
    }
    // A prelude operator name resolves to a record `{apply, t: <callable scheme>}`; surface the callable
    // `t` field so hover reads as the operator's arrow, not the internal record (Ty-level analogue of the
    // old `clean_hover_type` `(record (apply …) (t …))` string-unwrap).
    if let crate::ty::Ty::Record(fields) = ty
        && let Some((_, callable)) = fields.iter().find(|(sym, _)| &*sym.name == "t")
    {
        let root = crate::eval::encode_ty_payload(db, callable);
        return TypeAt::Ty(extract_subtree(&db.ast, root));
    }
    let root = crate::eval::encode_ty_payload(db, ty);
    TypeAt::Ty(extract_subtree(&db.ast, root))
}

/// The definition a node IDENTIFIES, if any: the def's `(def …)` form, its signature list
/// (`(NAME param…)`), or its NAME atom (the signature's first child). Returns the `db.defs` index.
/// Used so hovering any part of a definition's "header" shows the def's signature rather than the
/// body's type (the form) or `Any` (the `def` keyword handled separately). O(1) via the header→index
/// map (`Db::def_index_by_ident`) — was a linear `defs.iter().enumerate()` scan per query, O(N²) under
/// `cdz query --where`'s per-match `TypeAt`.
fn def_identified_by(db: &mut Db, id: StructId) -> Option<usize> {
    db.def_index_by_ident(id)
}

/// The grammar keyword this atom spells, if it is one — the syntactic heads that are NOT expressions
/// (they head a form; they have no type). Only recognized in HEAD position (the first child of a list)
/// or as the annotation colon, so a user value that happens to share the spelling is not mislabeled.
fn grammar_keyword(db: &Db, id: StructId) -> Option<&'static str> {
    const KEYWORDS: &[&str] = &[
        "module", "def", "export", "import", "if", "let", "match", "fn", ":", "and", "or", "not",
        ".", "type", "effect", "handle", "do",
    ];
    let name = db.ast.as_name(id)?;
    let kw = KEYWORDS.iter().find(|k| **k == name)?;
    // Confirm it's in a syntactic position: the head (first child) of its parent list. (A bare atom
    // elsewhere with the same spelling is a value/label, not the keyword.)
    let parent = db.parent_of(id)?;
    if let Struct::List(kids) = db.ast.get(parent)
        && kids.first() == Some(&id)
    {
        return Some(kw);
    }
    None
}

/// The occurrences that RESOLVE to the top-level definition or sum type named `name`, in ascending id
/// order — the transpose of the resolution column. Walks every USER node, resolves it, and keeps those
/// whose `Ref`/`Lambda` points at the named target's defining occurrence. The defining occurrence
/// itself (a def's body / a type's synth record) is a definition, not a use, so it is excluded.
///
/// Total and deterministic: it reads `resolve::resolved_of` (the same producer a compile reads), so an
/// answer equals what a full compile determines (`query-engine.md` §A Batch Compile And A Point Query
/// Are The Same Reads Of The Same Columns).
fn uses_of(db: &mut Db, name: &str) -> Vec<StructId> {
    // The definition's identifying occurrence(s): a nullary def denotes its body; a def with params
    // denotes a lambda whose body is that occurrence; a sum type denotes its synthesized record. A
    // reference to `name` resolves to a `Ref { value }` / `Lambda { body }` pointing at one of these.
    let mut targets: Vec<StructId> = Vec::new();
    if let Some(def) = db.def_by_name(name)
        && let Some(body) = db.defs[def].body
    {
        targets.push(body);
    }
    if let Some(synth) = db.type_decl_by_name(name) {
        targets.push(synth);
    }
    if targets.is_empty() {
        return Vec::new();
    }

    // DECLARATION-SITE name occurrences are not uses. A def's signature name (`helper` in `(def
    // (helper) …)`) resolves to a `Ref` to its own body — but it is where the name is DECLARED, not a
    // reference to it — so exclude every def's signature-name head. (The body/synth target occurrences
    // are already excluded below.) A HASH SET, not a `Vec`: the per-node membership test below runs once
    // per node, so an O(defs) linear `Vec::contains` made the whole walk O(nodes × defs) = O(N²) for a
    // program with N defs (`cdz uses` over a wide module: ~84% self in this walk); the set is O(1).
    let mut decl_sites: crate::fxhash::FxHashSet<StructId> = crate::fxhash::FxHashSet::default();
    for def in &db.defs {
        if let crate::ast::Struct::List(children) = db.ast.get(def.sig_occ)
            && let Some(&name_occ) = children.first()
        {
            decl_sites.insert(name_occ);
        }
    }

    // Walk every user node in id order; a reference is a node that resolves to a Ref/Lambda at a
    // target occurrence. (Non-name nodes resolve to something else and are skipped.) The target
    // occurrences and declaration sites themselves are not uses.
    let node_count = db.ast.structure.len();
    let mut uses = Vec::new();
    for i in 0..node_count {
        let id = StructId(i as u32);
        if !db.is_user_node(id) || targets.contains(&id) || decl_sites.contains(&id) {
            continue;
        }
        let hit = match crate::resolve::resolved_of(db, id) {
            Resolved::Ref { value } => targets.contains(&value),
            Resolved::Lambda { body, .. } => targets.contains(&body),
            _ => false,
        };
        if hit {
            uses.push(id);
        }
    }
    uses
}

/// The answer to one `Query`, ready to become an output artifact: the artifact kind, a name (the
/// queried symbol, for the artifact's name), and the rendered bytes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct QueryResult {
    pub kind: &'static str,
    pub name: String,
    pub bytes: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Target;

    #[test]
    fn request_list_round_trips() {
        // EXHAUSTIVE over every Request/Query variant — a missing variant would let a broken encode/
        // decode arm (a copy-paste tag mismatch, a dropped operand) slip through. Emit targets:
        // Wasm/WasmDebug/Dwarf/Tests/Rust/RustAsync. Queries: all 13 (name-keyed, node-keyed, nullary).
        let requests = vec![
            Request::Emit(Target::Wasm),
            Request::Emit(Target::WasmDebug),
            Request::Emit(Target::Dwarf),
            Request::EmitTests,
            Request::Emit(Target::Rust),
            Request::Emit(Target::RustAsync),
            Request::Query(Query::TypeOf {
                name: "main".into(),
            }),
            Request::Query(Query::UsesOf {
                name: "helper".into(),
            }),
            Request::Query(Query::TypeAt { node: 42 }),
            Request::Query(Query::Diagnostics),
            Request::Query(Query::ResolveOf { node: 7 }),
            Request::Query(Query::ScopeAt { node: 9 }),
            Request::Query(Query::Exports),
            Request::Query(Query::Highlight),
            Request::Query(Query::DocOf {
                name: "helper".into(),
            }),
            Request::Query(Query::DocAt { node: 11 }),
            Request::Query(Query::Instantiations {
                name: "fold-n".into(),
            }),
            Request::Query(Query::Symbols),
            Request::Query(Query::ParamManifest),
        ];
        assert_eq!(decode(&encode(&requests)), Some(requests));
    }

    #[test]
    fn every_request_variant_round_trips_individually() {
        // Round-trip each variant ALONE too, so a variant that only survives when neighboured by others
        // (a length/offset bug masked by the next request's bytes) is still caught. An exhaustive `match`
        // in the builder below forces this list to extend when a Request/Query variant is added.
        let each: Vec<Request> = vec![
            Request::Emit(Target::Wasm),
            Request::Emit(Target::WasmDebug),
            Request::Emit(Target::Dwarf),
            Request::EmitTests,
            Request::Emit(Target::Rust),
            Request::Emit(Target::RustAsync),
            Request::Query(Query::TypeOf { name: "n".into() }),
            Request::Query(Query::UsesOf { name: "n".into() }),
            Request::Query(Query::TypeAt { node: 1 }),
            Request::Query(Query::Diagnostics),
            Request::Query(Query::ResolveOf { node: 1 }),
            Request::Query(Query::ScopeAt { node: 1 }),
            Request::Query(Query::Exports),
            Request::Query(Query::Highlight),
            Request::Query(Query::DocOf { name: "n".into() }),
            Request::Query(Query::DocAt { node: 1 }),
            Request::Query(Query::Instantiations { name: "n".into() }),
            Request::Query(Query::Symbols),
            Request::Query(Query::ParamManifest),
            Request::Query(Query::FuncLayout),
            Request::Query(Query::ClosureHash),
            Request::Query(Query::ExportedTypes),
            Request::EmitTestsPerFile,
            Request::EmitTestsComposed,
            Request::EmitTestsConsumerOnly,
            Request::EmitTestsShred,
            Request::EmitTestsShredStandalone,
            Request::EmitTestsShredTwoStage,
        ];
        // Exhaustiveness guard: adding a Request/Query variant fails to compile until listed above AND here.
        for r in &each {
            match r {
                Request::Emit(_)
                | Request::EmitTests
                | Request::EmitTestsPerFile
                | Request::EmitTestsComposed
                | Request::EmitTestsConsumerOnly
                | Request::EmitTestsShred
                | Request::EmitTestsShredStandalone
                | Request::EmitTestsShredTwoStage
                | Request::Query(
                    Query::TypeOf { .. }
                    | Query::UsesOf { .. }
                    | Query::TypeAt { .. }
                    | Query::Diagnostics
                    | Query::ResolveOf { .. }
                    | Query::ScopeAt { .. }
                    | Query::Exports
                    | Query::Highlight
                    | Query::DocOf { .. }
                    | Query::DocAt { .. }
                    | Query::Instantiations { .. }
                    | Query::Symbols
                    | Query::ParamManifest
                    | Query::FuncLayout
                    | Query::ClosureHash
                    | Query::ExportedTypes
                    | Query::TestList,
                ) => {}
            }
        }
        for r in each {
            let one = vec![r.clone()];
            assert_eq!(
                decode(&encode(&one)),
                Some(one),
                "variant did not round-trip alone: {r:?}"
            );
        }
    }

    #[test]
    fn empty_list_round_trips() {
        assert_eq!(decode(&encode(&[])), Some(vec![]));
    }

    #[test]
    fn truncated_is_none_not_panic() {
        // A truncated codec artifact (half a valid request tree) declines rather than panicking.
        let mut bytes = encode(&[Request::Emit(Target::Wasm)]);
        bytes.truncate(bytes.len() / 2);
        assert_eq!(decode(&bytes), None);
    }

    #[test]
    fn garbage_bytes_are_none() {
        // Bytes that are not a cdzast codec artifact at all — a decline, not a panic.
        assert_eq!(decode(&[0xff, 0xff, 0xff, 0xff]), None);
    }

    #[test]
    fn trailing_garbage_is_none() {
        // The cdzast codec consumes the WHOLE input or refuses (no trailing bytes), so an extra byte past
        // the encoded request tree is a malformed artifact and decode declines.
        let mut bytes = encode(&[Request::Emit(Target::Wasm)]);
        bytes.push(0x00);
        assert_eq!(decode(&bytes), None);
    }

    #[test]
    fn unknown_request_head_is_none() {
        // A well-formed cdzast tree whose request form has an UNKNOWN head declines (never panics).
        let mut b = Builder::new();
        let head = b.name("frobnicate");
        let form = b.list(vec![head]);
        let root = b.list(vec![form]);
        assert_eq!(decode(&crate::codec::encode(&b.finish(root))), None);
    }

    #[test]
    fn unknown_query_selector_is_none() {
        // `(query bogus)` — a known `query` head with an UNKNOWN selector declines.
        let mut b = Builder::new();
        let q = b.name("query");
        let sel = b.name("bogus");
        let form = b.list(vec![q, sel]);
        let root = b.list(vec![form]);
        assert_eq!(decode(&crate::codec::encode(&b.finish(root))), None);
    }

    #[test]
    fn wrong_operand_type_is_none() {
        // `(query type-at "x")` — a node-id operand given as an `Ast.Str` (not an `Ast.Int`) declines.
        let mut b = Builder::new();
        let q = b.name("query");
        let sel = b.name("type-at");
        let bad = atom_str(&mut b, "x");
        let form = b.list(vec![q, sel, bad]);
        let root = b.list(vec![form]);
        assert_eq!(decode(&crate::codec::encode(&b.finish(root))), None);
    }

    #[test]
    fn extract_subtree_encodes_a_standalone_arena_that_codec_round_trips() {
        // The `ExportedTypes` fact extracts each type sub-AST out of `db.ast` into a standalone arena and
        // `codec::encode`s it (the consumer's byte-identical codec decodes it back). Prove `extract_subtree`
        // + the encode/decode round-trip on a representative structured type shape `(-> (List Int64) Bool)`
        // built directly (no compile harness needed): the decoded arena must be structurally identical.
        use crate::ast::{Builder, Leaf, Struct};
        // Build a bigger source arena with the target type as a SUBTREE (not the root), so extraction from
        // an interior root is exercised. Source: `(wrapper <the-type>)`.
        let mut src = Builder::new();
        let arrow = src.name("->");
        let list = src.name("List");
        let int_name = src.name("Int");
        let width = src.atom_leaf(Leaf::Int {
            value: crate::ast::IntValue::from_i64(64),
            radix: crate::ast::Radix::Dec,
        });
        let int_ty = src.list(vec![int_name, width]);
        let list_int = src.list(vec![list, int_ty]);
        let bool_ty = src.name("Bool");
        let ty_root = src.list(vec![arrow, list_int, bool_ty]);
        let wrapper_head = src.name("wrapper");
        let src_root = src.list(vec![wrapper_head, ty_root]);
        let src_arena = src.finish(src_root);

        // Extract the TYPE subtree (ty_root, an interior node), encode, decode.
        let extracted = extract_subtree(&src_arena, ty_root);
        let bytes = crate::codec::encode(&extracted);
        let decoded = crate::codec::decode(&bytes).expect("decode the extracted type artifact");

        // The decoded root is `(-> (List (Int 64)) Bool)` — check the head + arity structurally.
        match decoded.get(decoded.root) {
            Struct::List(kids) => {
                assert_eq!(kids.len(), 3, "arrow has head + 2 operands");
                assert_eq!(decoded.as_name(kids[0]), Some("->"));
                // operand 0 is `(List (Int 64))`
                match decoded.get(kids[1]) {
                    Struct::List(inner) => {
                        assert_eq!(decoded.as_name(inner[0]), Some("List"));
                    }
                    _ => panic!("operand 0 should be a `(List …)` list"),
                }
                // operand 1 is `Bool`
                assert_eq!(decoded.as_name(kids[2]), Some("Bool"));
            }
            _ => panic!("decoded root should be the `(-> …)` list"),
        }
    }

    /// Parse the KIND_EXPORT_TYPES value into `name → decoded-ty-arena` (the consumer's parse) — the wire
    /// is now ONE canonical binary-AST value `(export-types (export-type <name> <ty-payload>)…)`, decoded
    /// via the shared `cadenza_compile_abi` codec (the bespoke u32_le framing is retired).
    fn parse_export_types(bytes: &[u8]) -> Vec<(String, crate::ast::Arenas)> {
        cadenza_compile_abi::decode_export_types(bytes)
    }

    #[test]
    fn test_list_enumerates_tests_with_property_flag() {
        // `Query::TestList` answers the `@test` defs the runner would run, as a CADENZA-AST VALUE
        // `(test-list (test <name-Str> <is-property-Bool> <file-Str>)…)` (codec-encoded — the operator
        // exchange format; `cdz --list` forwards it verbatim). is_property = `!params.is_empty() ||
        // name.ends_with("-gen")`. A nullary test is a plain unit test (false); a PARAMETERIZED test is a
        // property test (true); a `-gen` wrapper is a property test by name-suffix (true) even nullary. A
        // non-`@test` def is omitted.
        let src = "(do \
            (@ test (def (t_unit) unit)) \
            (@ test (def (t_prop (: n Int64)) unit)) \
            (@ test (def (t_shrink-gen) unit)) \
            (def (helper) unit) \
            (export helper))";
        let mut db = crate::db::Db::load(crate::testkit::parse(src));
        let result = run_query(&mut db, &Query::TestList);
        assert_eq!(result.kind, KIND_TEST_LIST);
        // Decode the cadenza-ast value with the shared codec + walk `(test-list (test name isprop file)…)`.
        let arena = crate::codec::decode(&result.bytes).expect("decode test-list value");
        let Struct::List(kids) = arena.get(arena.root) else {
            panic!("test-list root should be a `(test-list …)` list");
        };
        assert_eq!(
            arena.as_name(kids[0]),
            Some("test-list"),
            "root head is `test-list`"
        );
        let mut by_name: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
        for &rec in &kids[1..] {
            let Struct::List(f) = arena.get(rec) else {
                panic!("each test is a `(test …)` list");
            };
            assert_eq!(arena.as_name(f[0]), Some("test"), "record head is `test`");
            let name = arena.as_str(f[1]).expect("name is a Str leaf").to_string();
            let is_property = arena.as_bool(f[2]).expect("is-property is a Bool leaf");
            by_name.insert(name, is_property);
        }
        assert_eq!(
            by_name.len(),
            3,
            "three @test defs (helper is not a test): {by_name:?}"
        );
        assert_eq!(
            by_name.get("t_unit"),
            Some(&false),
            "a nullary test is a plain unit test (not a property test)"
        );
        assert_eq!(
            by_name.get("t_prop"),
            Some(&true),
            "a parameterized test IS a property test"
        );
        assert_eq!(
            by_name.get("t_shrink-gen"),
            Some(&true),
            "a `-gen` wrapper is a property test by name-suffix (even nullary)"
        );
        assert!(
            !by_name.contains_key("helper"),
            "a non-`@test` def is not listed"
        );
    }

    #[test]
    fn doc_of_a_dotted_prelude_member_resolves_not_no_such_definition() {
        // ITEM C (breaker /tmp/mr4.sexp): `cdz doc Map.insert` must RESOLVE the dotted member — read its
        // per-member doc channel through the generic member path (`member_value` → `(meta doc)`) — not
        // report "no such definition". A real member with no per-member doc is `Undocumented`; a BOGUS
        // member (`Map.nope`) stays `NoSuchDef`. Regression witness for the dotted-member doc-resolution
        // gap: before, `doc_of_name` only looked the WHOLE name up as a def / prelude record / keyword, so
        // a dotted member fell through to `NoSuchDef` even though the module (`cdz doc Map`) resolves.
        use cadenza_compile_abi::DocAnswer;
        let src = "(do (def (main) (Map.insert Map.empty 1 2)) (export main))";
        let mut db = crate::db::Db::load(crate::testkit::parse(src));
        let member = cadenza_compile_abi::decode_doc(
            &run_query(
                &mut db,
                &Query::DocOf {
                    name: "Map.insert".into(),
                },
            )
            .bytes,
        );
        assert!(
            !matches!(member, DocAnswer::NoSuchDef { .. }),
            "`Map.insert` is a REAL prelude member (resolves to Undocumented or Doc), not a typo: {member:?}"
        );
        let bogus = cadenza_compile_abi::decode_doc(
            &run_query(
                &mut db,
                &Query::DocOf {
                    name: "Map.nope".into(),
                },
            )
            .bytes,
        );
        assert!(
            matches!(bogus, DocAnswer::NoSuchDef { .. }),
            "`Map.nope` is a bogus member → NoSuchDef: {bogus:?}"
        );
    }

    #[test]
    fn exported_types_covers_def_type_and_effect_kinds() {
        // The reviewer #2619 follow-up: ExportedTypes must emit a structured type for ALL THREE export
        // kinds. A def → a `(-> …)`; a type → a `(Sum …)`; an EFFECT → the grouped
        // `(effect (op <name> <arrow>)…)` shape (the kind that was silently omitted before this fix).
        let src = "(do \
            (effect Log (op emit (-> String Unit))) \
            (type Color (Red) (Green)) \
            (def (inc (: n Int64)) n) \
            (export Log) (export Color) (export inc))";
        let mut db = crate::db::Db::load(crate::testkit::parse(src));
        let result = run_query(&mut db, &Query::ExportedTypes);
        assert_eq!(result.kind, KIND_EXPORT_TYPES);
        let records = parse_export_types(&result.bytes);
        let by_name: std::collections::HashMap<&str, &crate::ast::Arenas> =
            records.iter().map(|(n, a)| (n.as_str(), a)).collect();

        // DEF `inc : (-> Int64 Int64)` → a `(-> …)` root.
        let inc = by_name.get("inc").expect("inc export has a (ty …)");
        match inc.get(inc.root) {
            Struct::List(kids) => assert_eq!(inc.as_name(kids[0]), Some("->")),
            _ => panic!("inc type root should be a `(-> …)` list"),
        }

        // TYPE `Color` → a `(Sum …)` root.
        let color = by_name.get("Color").expect("Color export has a (ty …)");
        match color.get(color.root) {
            Struct::List(kids) => assert_eq!(color.as_name(kids[0]), Some("Sum")),
            _ => panic!("Color type root should be a `(Sum …)` list"),
        }

        // EFFECT `Log` → the grouped `(effect (op emit (-> String Unit)))` root — the #2619 fix.
        let log = by_name
            .get("Log")
            .expect("Log effect export has a (ty …) — the #2619 effect fix");
        match log.get(log.root) {
            Struct::List(kids) => {
                assert_eq!(
                    log.as_name(kids[0]),
                    Some("effect"),
                    "effect payload roots at `effect`"
                );
                assert!(kids.len() >= 2, "effect has at least one `(op …)`");
                match log.get(kids[1]) {
                    Struct::List(op) => {
                        assert_eq!(log.as_name(op[0]), Some("op"));
                        assert_eq!(log.as_name(op[1]), Some("emit"));
                        match log.get(op[2]) {
                            Struct::List(arr) => assert_eq!(log.as_name(arr[0]), Some("->")),
                            _ => panic!("op arrow should be a `(-> …)`"),
                        }
                    }
                    _ => panic!("effect child should be an `(op …)` list"),
                }
            }
            _ => panic!("Log type root should be an `(effect …)` list"),
        }
    }
}
