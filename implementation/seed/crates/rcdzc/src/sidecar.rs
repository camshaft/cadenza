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

use crate::ast::StructId;
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
    /// export still gets its diagnostics. Answered as one fault per line, `severity<TAB>code<TAB>node-id
    /// <TAB>message` (node-id-keyed like `UsesOf` — the consumer maps the node to a source range). Total:
    /// a clean program yields the empty result.
    Diagnostics,
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
            // Total: a node id past the program is not a user node — a defined "unknown", not a crash.
            // (The consumer resolves an offset to a node via the span table, which only holds user
            // nodes, so a well-formed request always names a user node; this guards a malformed one.)
            let text = if db.is_user_node(id) {
                crate::infer::type_of(db, id).render_name()
            } else {
                "unknown".to_string()
            };
            QueryResult {
                kind: KIND_TYPE_AT,
                name: node.to_string(),
                bytes: text.into_bytes(),
            }
        }
        Query::Diagnostics => {
            // Every well-formedness fault, WITHOUT gating on export/layout — the "diagnostics as you
            // type" set (`compile::diagnostics`, which runs `collect_faults` alone). One fault per
            // line: `severity<TAB>code<TAB>node-id<TAB>message`. `code` is the `CDZ####` or `-` for an
            // uncoded decline; `node-id` is the anchor (`-` if unanchored) the consumer maps to a span.
            let mut text = String::new();
            for d in crate::diagnostics(db) {
                let severity = match d.severity {
                    crate::Severity::Error => "error",
                    crate::Severity::Warning => "warning",
                };
                let code = d.code.as_deref().unwrap_or("-");
                let node = d.node.map_or_else(|| "-".to_string(), |n| n.to_string());
                // Newlines in a message would break the one-line-per-fault framing — collapse them.
                let message = d.message.replace('\n', " ");
                text.push_str(&format!("{severity}\t{code}\t{node}\t{message}\n"));
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
    }
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
