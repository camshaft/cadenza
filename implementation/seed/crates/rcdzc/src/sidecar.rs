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
//! (`tooling-and-lsp.md` §An Agent Queries The Compiler For Any Static Fact) already obliges every
//! such fact to be TOTAL, DETERMINISTIC, and equal to what a full compile determines — so this surface
//! EXPOSES an obligation the compiler carries, it does not compute a second, possibly-disagreeing
//! answer (`query-engine.md` §A Batch Compile And A Point Query Are The Same Reads Of The Same
//! Columns).
//!
//! **Why no effects, no mutable state.** A fact is a pure read, so a sidecar that branches on one —
//! "emit debug only if the module declares an effect", "rewrite only where the type is `T`" — is
//! ordinary control flow over a value, needing neither effects nor a mutable place (which would break
//! the model's no-cache / incremental-equals-batch invariants). This module is the DATA form of that
//! program: an inert list a driver hands in. The branching program that PRODUCES the list is a later
//! rung (it needs the self-host); the list it returns is what crosses here.
//!
//! **The wire form** (hand-rolled leb128, total decode — the same discipline as `codec`, so it ports
//! to the Cadenza self-host and never panics on untrusted bytes): a `VarU64` request count, then each
//! request as a one-byte TAG followed by its operands. A malformed request list is a DECLINE (a
//! diagnostic), never a panic or a silent drop — reject-don't-miscompile at the tool edge.

use crate::ast::{Struct, StructId};
use crate::db::Db;
use crate::leb128::{self, Reader};
use crate::resolved::Resolved;

/// The kinded input artifact carrying the request list.
pub const KIND_SIDECAR: &str = "sidecar";

/// The output artifact kind for a `TypeOf` query result — the rendered type of the queried node(s).
pub const KIND_TYPE_INFO: &str = "type-info";

/// The output artifact kind for a `UsesOf` query result — the node indices that reference a name.
pub const KIND_USES: &str = "uses";

/// The output artifact kind for a `TypeAt` query result — the rendered type of a specific node.
pub const KIND_TYPE_AT: &str = "type-at";

/// The output artifact kind for a `Diagnostics` query result — the program's well-formedness faults.
pub const KIND_DIAGNOSTICS: &str = "diagnostics";

/// The output artifact kind for a `ResolveOf` query result — the defining occurrence's node id.
pub const KIND_RESOLVE: &str = "resolve";

/// The output artifact kind for a `ScopeAt` query result — the bindings visible at a node.
pub const KIND_SCOPE: &str = "scope";

/// The output artifact kind for an `Exports` query result — the module's exported names + types.
pub const KIND_EXPORTS: &str = "exports";

/// One request in a sidecar's list. Either MATERIALIZE an output column (`Emit`) or READ a fact column
/// (`Query`). `Rewrite` (the validated-transaction arm of `DESIGN-sidecar-api.md`) is a later rung and
/// not modeled here yet.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Request {
    /// Materialize a backend artifact — the generalization of a `Target`. Carried as the target's own
    /// enum so the emit path is literally today's, reached through the request list instead of the
    /// fixed `targets` slice.
    Emit(crate::backend::Target),
    /// Read a fact column.
    Query(Query),
}

/// A read of a fact column — the query half of the request vocabulary. Each arm names the column it
/// reads; the answer is total (a node with no answer yields a defined "unknown", never a crash).
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Query {
    /// The solved type of a top-level definition, BY NAME — a read of the type column
    /// (`infer::def_scheme` / `infer::type_of`). Answered as the definition's rendered type
    /// (`Ty::render_name`), the same canonical text a value's annotation carries. A name that names no
    /// definition yields a defined "no such definition" result rather than an error.
    TypeOf { name: String },
    /// Every occurrence that RESOLVES to the named top-level definition or sum type — the transpose of
    /// the resolution column. Answered as the list of referencing node indices (`StructId.0`), which a
    /// consumer holding the span table maps to source regions (`query-engine.md` §Provenance Is
    /// Recovered By Back-Reference). The defining occurrence itself is not a use.
    UsesOf { name: String },
    /// The solved type of a SPECIFIC NODE, by its `StructId` — a read of the type column
    /// (`infer::type_of` → `Ty::render_name`). This is the node-granular companion of `TypeOf` (which
    /// is by definition name): given a node id, answer its rendered type. It is the query behind a
    /// "type at cursor" hover — the CONSUMER (which holds the span table) resolves a source OFFSET to
    /// the innermost node id and asks this, so the compiler stays span-free (`query-engine.md`
    /// §Provenance Is Recovered By Back-Reference: the compiler emits/consumes node IDENTITY, the
    /// front-end owns spans). Total: a node id past the program, or one with no meaningful type, yields
    /// a defined "unknown" answer rather than an error.
    TypeAt { node: u32 },
    /// The DEFINING OCCURRENCE a reference node resolves to, by its `StructId` — a read of the
    /// resolution column (`resolve::resolved_of`), the go-to-definition counterpart of `UsesOf` (which
    /// is the reverse index). A name reference resolves to `Ref { value }` (a nullary def's body, a
    /// `let` initializer, a parameter binder, a sum/prelude binding) or `Lambda { body }` (a def WITH
    /// parameters — the function case); the answer is that target occurrence's node id, which the
    /// consumer maps to a source range. Node-id-keyed and span-free, like `TypeAt`: the consumer
    /// resolves a cursor OFFSET to the reference node and asks this. Total: a node that is not a
    /// navigable reference (a literal, a prim, an unbound name) yields the EMPTY result (no line).
    ResolveOf { node: u32 },
    /// EVERY well-formedness fault in the program — a read of the fault set (`compile::diagnostics`),
    /// WITHOUT requiring the program to export anything or emit. This is the "diagnostics as you type"
    /// primitive: the same faults `compile` reports (type mismatch, unbound name, duplicate def/field,
    /// non-linear binder, …), but not gated on `layout`/export, so a mid-edit buffer that declares no
    /// export still gets its diagnostics. Answered as one fault per line, TAB-separated: `severity  code
    ///   node-id  fix-node  fix-replacement  fix-verified  message` (node-id-keyed like `UsesOf` — the
    /// consumer maps each node to a source range; the three fix columns carry the structural repair and
    /// are `-` when absent). Total: a clean program yields the empty result.
    Diagnostics,
    /// The BINDINGS VISIBLE at a node, by its `StructId` — "variable scope tracking" (the operator's
    /// original motivating example). Walks the lexical scope (`db.parent_of`, the same substrate
    /// `resolve::lookup_scope` uses) collecting every binder in scope: a `let`'s bindings visible from
    /// the point (earlier bindings from a later initializer; all from the body), an enclosing `fn`/`def`'s
    /// parameters, a match-arm binder. Answered as one binding per line, `name<TAB>type<TAB>binder-node-id`
    /// (the type read from the binder's `infer::type_of`; node-id-keyed like the others). INNERMOST wins:
    /// a shadowed name appears once, bound to the nearest enclosing binder. Total: at the top level (no
    /// enclosing binder) the result is empty. This is the query an editor's autocomplete / scope panel
    /// rides on.
    ScopeAt { node: u32 },
    /// The MODULE's EXPORTS — each exported name paired with its type, one per line as
    /// `name<TAB>type<TAB>def-name-node-id`. Reads `db.exports` (the `(export …)` clauses) and each
    /// export's def scheme (`infer::def_scheme` → the signature). The node id is the exported def's NAME
    /// occurrence, so a consumer can jump to it. Node-id-keyed like the others; TOTAL — a module with no
    /// exports yields the empty result. This is the "module interface at a glance" query.
    Exports,
}

/// The one-byte request tags. Stable across versions (additive — a new request is a new tag).
mod tag {
    pub const EMIT_WASM: u8 = 0x00;
    pub const EMIT_RUST: u8 = 0x01;
    pub const EMIT_RUST_ASYNC: u8 = 0x02;
    /// Emit a WebAssembly component carrying EMBEDDED debug info (Mode E of
    /// `DESIGN-debug-info-rcdzc.md`) — a `Target::WasmDebug`. The additive tag that enables debug output
    /// as a sidecar request, not a `--debug` flag: a driver puts this in the request list (and supplies
    /// the `spans` input for the DWARF increments) rather than passing a build flag. Its artifact is a
    /// `component`, decorated and strippable (§5). (Mode S — a separate `dwarf` file — is `EMIT_DWARF`.)
    pub const EMIT_WASM_DEBUG: u8 = 0x03;
    /// Emit a DETACHED DWARF sidecar (Mode S of `DESIGN-debug-info-rcdzc.md` §9.2) — a `Target::Dwarf`,
    /// a separate `kind == "dwarf"` artifact carrying only the `.debug_*` sections. The second
    /// enablement mode; like `EMIT_WASM_DEBUG` it needs the `spans` input. A run can request both (a
    /// lean component + its detached DWARF) or either alone.
    pub const EMIT_DWARF: u8 = 0x04;
    pub const QUERY_TYPE_OF: u8 = 0x10;
    pub const QUERY_USES_OF: u8 = 0x11;
    pub const QUERY_TYPE_AT: u8 = 0x12;
    pub const QUERY_DIAGNOSTICS: u8 = 0x13;
    pub const QUERY_RESOLVE_OF: u8 = 0x14;
    pub const QUERY_SCOPE_AT: u8 = 0x15;
    pub const QUERY_EXPORTS: u8 = 0x16;
}

/// Decode a request list from its wire bytes. Total: a truncated or malformed list yields `None` (the
/// caller turns that into a decline diagnostic), never a panic. Mirrors `codec::decode`'s discipline
/// exactly so the format ports to the self-host.
pub fn decode(bytes: &[u8]) -> Option<Vec<Request>> {
    let mut r = Reader::new(bytes);
    let count = r.read_var_len()?;
    let mut requests = Vec::with_capacity(count.min(1024));
    for _ in 0..count {
        requests.push(decode_one(&mut r)?);
    }
    // A trailing garbage byte is a malformed list — reject rather than silently accept a prefix.
    if !r.at_end() {
        return None;
    }
    Some(requests)
}

/// Decode one request: a tag byte, then its operands.
fn decode_one(r: &mut Reader) -> Option<Request> {
    match r.byte()? {
        tag::EMIT_WASM => Some(Request::Emit(crate::backend::Target::Wasm)),
        tag::EMIT_WASM_DEBUG => Some(Request::Emit(crate::backend::Target::WasmDebug)),
        tag::EMIT_DWARF => Some(Request::Emit(crate::backend::Target::Dwarf)),
        tag::EMIT_RUST => Some(Request::Emit(crate::backend::Target::Rust)),
        tag::EMIT_RUST_ASYNC => Some(Request::Emit(crate::backend::Target::RustAsync)),
        tag::QUERY_TYPE_OF => Some(Request::Query(Query::TypeOf {
            name: read_string(r)?,
        })),
        tag::QUERY_USES_OF => Some(Request::Query(Query::UsesOf {
            name: read_string(r)?,
        })),
        tag::QUERY_TYPE_AT => Some(Request::Query(Query::TypeAt {
            node: u32::try_from(r.read_varu64()?).ok()?,
        })),
        tag::QUERY_DIAGNOSTICS => Some(Request::Query(Query::Diagnostics)),
        tag::QUERY_RESOLVE_OF => Some(Request::Query(Query::ResolveOf {
            node: u32::try_from(r.read_varu64()?).ok()?,
        })),
        tag::QUERY_SCOPE_AT => Some(Request::Query(Query::ScopeAt {
            node: u32::try_from(r.read_varu64()?).ok()?,
        })),
        tag::QUERY_EXPORTS => Some(Request::Query(Query::Exports)),
        _ => None,
    }
}

/// Encode a request list to its wire bytes — the counterpart to `decode`, used by a driver (and the
/// tests) to build a sidecar input. `decode(encode(rs)) == rs`.
pub fn encode(requests: &[Request]) -> Vec<u8> {
    let mut out = Vec::new();
    leb128::write_u64(&mut out, requests.len() as u64);
    for req in requests {
        encode_one(&mut out, req);
    }
    out
}

fn encode_one(out: &mut Vec<u8>, req: &Request) {
    match req {
        Request::Emit(crate::backend::Target::Wasm) => out.push(tag::EMIT_WASM),
        Request::Emit(crate::backend::Target::WasmDebug) => out.push(tag::EMIT_WASM_DEBUG),
        Request::Emit(crate::backend::Target::Dwarf) => out.push(tag::EMIT_DWARF),
        Request::Emit(crate::backend::Target::Rust) => out.push(tag::EMIT_RUST),
        Request::Emit(crate::backend::Target::RustAsync) => out.push(tag::EMIT_RUST_ASYNC),
        Request::Query(Query::TypeOf { name }) => {
            out.push(tag::QUERY_TYPE_OF);
            write_string(out, name);
        }
        Request::Query(Query::UsesOf { name }) => {
            out.push(tag::QUERY_USES_OF);
            write_string(out, name);
        }
        Request::Query(Query::TypeAt { node }) => {
            out.push(tag::QUERY_TYPE_AT);
            leb128::write_u64(out, *node as u64);
        }
        Request::Query(Query::Diagnostics) => out.push(tag::QUERY_DIAGNOSTICS),
        Request::Query(Query::ResolveOf { node }) => {
            out.push(tag::QUERY_RESOLVE_OF);
            leb128::write_u64(out, *node as u64);
        }
        Request::Query(Query::ScopeAt { node }) => {
            out.push(tag::QUERY_SCOPE_AT);
            leb128::write_u64(out, *node as u64);
        }
        Request::Query(Query::Exports) => out.push(tag::QUERY_EXPORTS),
    }
}

/// A length-prefixed UTF-8 string: a `VarU64` byte length, then the bytes. Total decode.
fn read_string(r: &mut Reader) -> Option<String> {
    let len = r.read_var_len()?;
    let bytes = r.take(len)?;
    String::from_utf8(bytes.to_vec()).ok()
}

fn write_string(out: &mut Vec<u8>, s: &str) {
    leb128::write_u64(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

/// Run one `Query` against the loaded `Db`, producing its answer as UTF-8 text bytes. This is the
/// query half of the engine: a read of a fact column, TOTAL over every input (a name that names
/// nothing yields a defined "no such …" line, never an error) — the oracle contract. Pure with respect
/// to the artifact channel: a query NEVER denies the component, and reading a fact does not change what
/// any other request produces.
pub fn run_query(db: &mut Db, query: &Query) -> QueryResult {
    match query {
        Query::TypeOf { name } => {
            let text = match db.def_by_name(name) {
                Some(def) => match crate::infer::def_scheme(db, def) {
                    // The definition's rendered type — the same canonical text a value's annotation
                    // carries (`Ty::render_name`). A demand-driven read of the type column.
                    Some(scheme) => scheme.ty.render_name(),
                    // A def whose type could not be solved (an ambiguous unannotated parameter) — a
                    // DEFINED "unknown", not an error: the query is total.
                    None => "unknown".to_string(),
                },
                None => format!("no such definition `{name}`"),
            };
            QueryResult {
                kind: KIND_TYPE_INFO,
                name: name.clone(),
                bytes: text.into_bytes(),
            }
        }
        Query::UsesOf { name } => {
            let uses = uses_of(db, name);
            // One node index per line, in ascending id order — the deterministic order the columns
            // model requires (a query answer is a function of the program, not of traversal order).
            let mut text = String::new();
            for id in uses {
                text.push_str(&id.0.to_string());
                text.push('\n');
            }
            QueryResult {
                kind: KIND_USES,
                name: name.clone(),
                bytes: text.into_bytes(),
            }
        }
        Query::TypeAt { node } => {
            let id = crate::ast::StructId(*node);
            // A HOVER answer, not just a raw type: a grammar keyword names itself, a definition shows
            // its signature (`name : type`), an untypeable/poison node says "unknown" — never the bare
            // fallback `Any` or a leaked internal operator record. A genuinely-typed node renders its
            // type (a function name already renders as its arrow `(-> A B)`).
            let text = hover_text(db, id);
            QueryResult {
                kind: KIND_TYPE_AT,
                name: node.to_string(),
                bytes: text.into_bytes(),
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
            let mut text = String::new();
            for d in crate::diagnostics(db) {
                let severity = match d.severity {
                    crate::Severity::Error => "error",
                    crate::Severity::Warning => "warning",
                };
                let code = d.code.as_deref().unwrap_or("-");
                let node = d.node.map_or_else(|| "-".to_string(), |n| n.to_string());
                let (fix_kind, fix_node, fix_repl, fix_verified) = match &d.fix {
                    Some(f) => (
                        match f.kind {
                            crate::FixKind::Replace => "replace",
                            crate::FixKind::InsertInto => "insert",
                        }
                        .to_string(),
                        f.node.to_string(),
                        f.replacement.replace(['\n', '\t'], " "),
                        if f.verified { "verified" } else { "heuristic" }.to_string(),
                    ),
                    None => (
                        "-".to_string(),
                        "-".to_string(),
                        "-".to_string(),
                        "-".to_string(),
                    ),
                };
                // Newlines in a message would break the one-line-per-fault framing — collapse them.
                let message = d.message.replace('\n', " ");
                text.push_str(&format!(
                    "{severity}\t{code}\t{node}\t{fix_kind}\t{fix_node}\t{fix_repl}\t{fix_verified}\t{message}\n"
                ));
            }
            QueryResult {
                kind: KIND_DIAGNOSTICS,
                name: "diagnostics".to_string(),
                bytes: text.into_bytes(),
            }
        }
        Query::ResolveOf { node } => {
            let id = crate::ast::StructId(*node);
            // The defining occurrence a reference resolves to: `Ref { value }` (a nullary def body, a
            // let initializer, a parameter binder, a sum/prelude binding) or `Lambda { body }` (a def
            // with parameters). Anything else — a literal, a prim, an unbound name, a non-user id — is
            // NOT a navigable reference: the empty result (total). The consumer maps the id to a span.
            let mut text = String::new();
            if db.is_user_node(id) {
                match crate::resolve::resolved_of(db, id) {
                    Resolved::Ref { value } => text = format!("{}\n", value.0),
                    Resolved::Lambda { body, .. } => text = format!("{}\n", body.0),
                    _ => {}
                }
            }
            QueryResult {
                kind: KIND_RESOLVE,
                name: node.to_string(),
                bytes: text.into_bytes(),
            }
        }
        Query::ScopeAt { node } => {
            // Every binding visible at the node, INNERMOST-first, one per line as
            // `name<TAB>type<TAB>binder-node-id`. Collected by the scope walk, then each binder typed.
            let binders = scope_at(db, crate::ast::StructId(*node));
            let mut text = String::new();
            for (name, binder) in binders {
                let ty = crate::infer::type_of(db, binder).render_name();
                text.push_str(&format!("{name}\t{ty}\t{}\n", binder.0));
            }
            QueryResult {
                kind: KIND_SCOPE,
                name: node.to_string(),
                bytes: text.into_bytes(),
            }
        }
        Query::Exports => {
            // The module's interface: each `(export NAME)` clause paired with the named def's type
            // (its signature, via `def_scheme`), one per line `name<TAB>type<TAB>def-name-node-id`.
            // The node id is the exported def's NAME occurrence (sig's first child) so a consumer can
            // jump to it; `-` when the export names no definition. Deterministic (export order).
            let exports: Vec<(String, Option<usize>, StructId)> = db
                .exports
                .iter()
                .map(|e| (e.name.clone(), e.def, e.occ))
                .collect();
            let mut text = String::new();
            for (name, def, occ) in exports {
                let (ty, name_node) = match def {
                    Some(d) => {
                        let ty = match crate::infer::def_scheme(db, d) {
                            Some(scheme) => scheme.ty.render_name(),
                            None => "unknown".to_string(),
                        };
                        // The def's NAME occurrence (the sig's first child), for go-to; fall back to the
                        // export clause occurrence if the sig is malformed.
                        let sig = db.defs[d].sig_occ;
                        let name_node = match db.ast.get(sig) {
                            Struct::List(kids) => kids.first().copied().unwrap_or(sig),
                            _ => sig,
                        };
                        (ty, name_node.0.to_string())
                    }
                    // An export naming no definition — a diagnostic elsewhere; here, report it as unknown.
                    None => ("unknown".to_string(), "-".to_string()),
                };
                let _ = occ;
                text.push_str(&format!("{name}\t{ty}\t{name_node}\n"));
            }
            QueryResult {
                kind: KIND_EXPORTS,
                name: "exports".to_string(),
                bytes: text.into_bytes(),
            }
        }
    }
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

/// The HOVER text for a node — a presentation answer, not a raw type. In priority order:
///  1. a non-user node (past the program, or a synthesized one with no source) → `unknown`;
///  2. a DEFINITION the node identifies (its `(def …)` form, its signature list, or its NAME atom) →
///     the def's SIGNATURE, `name : <type>` (a function reads as `name : (-> A B)`, a value as
///     `name : T`) — so hovering a def shows what it defines, not its body's type alone;
///  3. a GRAMMAR KEYWORD atom (`def`/`export`/`module`/`if`/`let`/`match`/`fn`/`:`/`and`/`or`/`not`/`.`)
///     → `keyword <kw>` — these are syntax, not expressions, so they have no type;
///  4. otherwise the node's solved type (`type_of` → `render_name`), with the two poor renderings
///     cleaned up: the fallback `Any` (an untypeable/poison node) → `unknown`, and a leaked operator
///     RECORD (`(record (apply …) (t …))`, a prelude operator value) → its callable `(t …)` field.
fn hover_text(db: &mut Db, id: StructId) -> String {
    if !db.is_user_node(id) {
        return "unknown".to_string();
    }
    // (2) A definition the node identifies → its signature.
    if let Some(def) = def_identified_by(db, id) {
        let name = db.defs[def].name.clone();
        let sig = match crate::infer::def_scheme(db, def) {
            Some(scheme) => scheme.ty.render_name(),
            None => "unknown".to_string(),
        };
        return format!("{name} : {sig}");
    }
    // (3) A grammar keyword atom.
    if let Some(kw) = grammar_keyword(db, id) {
        return format!("keyword {kw}");
    }
    // (4) The solved type, cleaned up.
    let ty = crate::infer::type_of(db, id).render_name();
    clean_hover_type(&ty)
}

/// The definition a node IDENTIFIES, if any: the def's `(def …)` form, its signature list
/// (`(NAME param…)`), or its NAME atom (the signature's first child). Returns the `db.defs` index.
/// Used so hovering any part of a definition's "header" shows the def's signature rather than the
/// body's type (the form) or `Any` (the `def` keyword handled separately).
fn def_identified_by(db: &Db, id: StructId) -> Option<usize> {
    for (i, d) in db.defs.iter().enumerate() {
        // The def FORM `(def sig body)` — its parent-less shape is `(def …)`; match by its signature's
        // parent (the form) or the signature/name directly.
        let name_occ = if let Struct::List(kids) = db.ast.get(d.sig_occ) {
            kids.first().copied()
        } else {
            None
        };
        // The `(def …)` form is the parent of the signature occurrence.
        let def_form = db.parent_of(d.sig_occ);
        if id == d.sig_occ || Some(id) == name_occ || Some(id) == def_form {
            return Some(i);
        }
    }
    None
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

/// Clean up a rendered type for hover: the untypeable fallback `Any` reads as `unknown`, and a leaked
/// prelude-operator RECORD (`(record (apply …) (t T))` — the value an operator name resolves to) reads
/// as its callable `(t …)` field, the operator's actual scheme. Any other type passes through.
fn clean_hover_type(ty: &str) -> String {
    if ty == "Any" {
        return "unknown".to_string();
    }
    // An operator record leaks as `(record (apply …) (t <scheme>))` — surface the `(t …)` payload.
    if let Some(rest) = ty.strip_prefix("(record ")
        && let Some(t_at) = rest.find("(t ")
    {
        // Extract the balanced parenthesized group after `(t `.
        let after = &rest[t_at + 3..];
        if let Some(scheme) = balanced_prefix(after) {
            return scheme.to_string();
        }
    }
    ty.to_string()
}

/// The balanced-parenthesis prefix of `s` starting mid-group: read until the paren depth (which starts
/// at 1, accounting for the enclosing `(t …)` we're inside) returns to 0. Returns the inner text.
fn balanced_prefix(s: &str) -> Option<&str> {
    let mut depth = 1i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&s[..i]);
                }
            }
            _ => {}
        }
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
    // are already excluded below.)
    let mut decl_sites: Vec<StructId> = Vec::new();
    for def in &db.defs {
        if let crate::ast::Struct::List(children) = db.ast.get(def.sig_occ)
            && let Some(&name_occ) = children.first()
        {
            decl_sites.push(name_occ);
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
        let requests = vec![
            Request::Emit(Target::Wasm),
            Request::Query(Query::TypeOf {
                name: "main".into(),
            }),
            Request::Emit(Target::Rust),
            Request::Emit(Target::RustAsync),
            Request::Query(Query::UsesOf {
                name: "helper".into(),
            }),
            Request::Query(Query::TypeAt { node: 42 }),
            Request::Query(Query::Diagnostics),
            Request::Query(Query::ResolveOf { node: 7 }),
            Request::Query(Query::ScopeAt { node: 9 }),
            Request::Query(Query::Exports),
        ];
        assert_eq!(decode(&encode(&requests)), Some(requests));
    }

    #[test]
    fn empty_list_round_trips() {
        assert_eq!(decode(&encode(&[])), Some(vec![]));
    }

    #[test]
    fn truncated_is_none_not_panic() {
        // A count of 2 but only one request present.
        let mut bytes = encode(&[Request::Emit(Target::Wasm)]);
        bytes[0] = 2; // claim two requests
        assert_eq!(decode(&bytes), None);
    }

    #[test]
    fn unknown_tag_is_none() {
        // count=1, tag=0xff (unrecognized) — a decline, not a panic.
        assert_eq!(decode(&[0x01, 0xff]), None);
    }

    #[test]
    fn trailing_garbage_is_none() {
        let mut bytes = encode(&[Request::Emit(Target::Wasm)]);
        bytes.push(0x00); // an extra byte past the declared list
        assert_eq!(decode(&bytes), None);
    }

    #[test]
    fn truncated_string_is_none() {
        // count=1, QUERY_TYPE_OF, string len=5, but no string bytes.
        assert_eq!(decode(&[0x01, tag::QUERY_TYPE_OF, 0x05]), None);
    }
}
