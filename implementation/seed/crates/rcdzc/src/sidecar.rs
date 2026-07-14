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

/// The output artifact kind for a `Highlight` query result — one `node-id  kind` line per classified
/// token (SEMANTIC SYNTAX HIGHLIGHTING), the LSP `semanticTokens` analogue.
pub const KIND_HIGHLIGHT: &str = "highlight";

/// The output artifact kind for a `DocOf`/`DocAt` query result — a definition's (or a built-in's)
/// documentation text, the "hover documentation" companion of `KIND_TYPE_AT`.
pub const KIND_DOC: &str = "doc";

/// The output artifact kind for an `Instantiations` query result — every concrete monomorphization of a
/// generic / ad-hoc-polymorphic definition (recursive-generic, type-valued-param, and `const`-dictionary
/// specializations), one per line.
pub const KIND_INSTANTIATIONS: &str = "instantiations";

/// One request in a sidecar's list. Either MATERIALIZE an output column (`Emit`) or READ a fact column
/// (`Query`). `Rewrite` (the validated-transaction arm of `DESIGN-sidecar-api.md`) is a later rung and
/// not modeled here yet.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Request {
    /// Materialize a backend artifact — the generalization of a `Target`. Carried as the target's own
    /// enum so the emit path is literally today's, reached through the request list instead of the
    /// fixed `targets` slice.
    Emit(crate::backend::Target),
    /// Materialize a UNIT-TEST wasm component — the artifact a `cdz test` build produces. It is the same
    /// wasm target as `Emit(Wasm)` EXCEPT the boundary is laid out from the program's `@test` NULLARY
    /// definitions (`layout::compute_tests`) instead of its `(export …)` clauses: every test crosses as a
    /// nullary entry the runner invokes (a failing test traps; a passing one returns unit). Kept a
    /// distinct request (not a `Target`) because it changes the EXPORT SOURCE — a layout concern — not the
    /// backend; the ordinary `(export …)` compile is untouched, so a normal build never carries a test's
    /// report-effect import.
    EmitTests,
    /// Read a fact column.
    Query(Query),
}

/// A read of a fact column — the query half of the request vocabulary. Each arm names the column it
/// reads; the answer is total (a node with no answer yields a defined "unknown", never a crash).
///
/// The `TypeOf`/`TypeAt` queries expose the types the compiler INFERRED in a machine-readable form: a
/// `cdz type`/`type-at` answers with the queried node's rendered type as an output artifact (data), not
/// human-formatted prose — so an agent reads an inferred type programmatically.
//= spec/capabilities/agent-authoring.md#every-compiler-output-is-machine-readable
//# The compiler MUST expose the types it inferred in a machine-readable form.
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
    /// SEMANTIC SYNTAX HIGHLIGHTING — every user node CLASSIFIED by the role it plays, so an editor can
    /// colour a name by what it MEANS (a type vs a constructor vs a local vs a call vs an unbound typo)
    /// rather than by its spelling. The LSP `semanticTokens` analogue, and the highlight companion of the
    /// other node-id-keyed queries: it reads the SAME columns a compile reads — the resolved column
    /// (`resolve::resolved_of`) and the meta channels a value already carries (`(meta variant)` marks a
    /// constructor, `(meta t)` a type, `(meta apply)` a callable) — so a token's colour equals what the
    /// compiler determines, never a second lexical guess. Answered as one `node-id<TAB>kind` line per
    /// classified LEAF (the atoms an editor paints; a list form is not itself painted — its children are),
    /// in ascending id order. `kind` is one of a fixed closed vocabulary (`HighlightKind`). Node-id-keyed
    /// and span-free like `TypeAt`/`UsesOf`: the consumer maps each id to a source range. TOTAL — a node
    /// with no meaningful classification is simply omitted (an editor leaves it to its lexical fallback).
    Highlight,
    /// The MODULE's EXPORTS — each exported name paired with its type, one per line as
    /// `name<TAB>type<TAB>def-name-node-id`. Reads `db.exports` (the `(export …)` clauses) and each
    /// export's def scheme (`infer::def_scheme` → the signature). The node id is the exported def's NAME
    /// occurrence, so a consumer can jump to it. Node-id-keyed like the others; TOTAL — a module with no
    /// exports yields the empty result. This is the "module interface at a glance" query.
    Exports,
    /// The DOCUMENTATION of a definition or built-in, BY NAME — the doc companion of `TypeOf`. Answered
    /// from an ordered fallback, all reads of columns the compiler already fills:
    ///   1. a user definition's `(doc "…")` text (`db.doc_of_def`, keyed by the def's signature — the
    ///      text `strip_def_docs` captured off the def body at load);
    ///   2. else a BUILT-IN binding's `(meta doc)` channel — a built-in module is just a record, so its
    ///      documentation is data on that record, read GENERICALLY (`eval::project_meta(rec, "doc")`),
    ///      never by matching the name (the no-keys-outside-the-prelude rule: the query resolves the name
    ///      to its prelude record, then reads a channel, exactly as member access does);
    ///   3. else a GRAMMAR KEYWORD's doc (`if`/`let`/`match`/… — not bindings in the prelude map, so their
    ///      text is a small table, the doc analogue of the hardcoded `resolve::GRAMMAR` set).
    ///
    /// TOTAL: a name that documents nothing yields a defined "no documentation for `name`" line.
    DocOf { name: String },
    /// The DOCUMENTATION of the definition the node at `node` belongs to or references, by its `StructId`
    /// — the doc companion of `TypeAt`/`ResolveOf`, the "documentation at cursor" hover. Resolves the node
    /// to a definition (its own def, or the def a reference resolves to via `resolve::resolved_of` +
    /// `def_index_by_body`), then reads that def's `(doc "…")` text (`db.doc_of_def`). Node-id-keyed and
    /// span-free like `TypeAt`: the consumer resolves a cursor OFFSET to the node and asks this. TOTAL: a
    /// node that is not (and does not reference) a documented definition yields the empty result.
    DocAt { node: u32 },
    /// The DISPOSITION of a definition BY NAME — what the compiler DID with it — plus, if it is
    /// specialized, every concrete instantiation. The reverse of "one source def emits one function":
    /// because generics do not cross the component boundary (`component-abi.md` §Generics Do Not Cross The
    /// Boundary) and the default is always-inline, one source def may become zero, one, or many emitted
    /// functions. This query reports which happened, reading columns lowering/monomorphization already fill:
    ///   - `specialized` — MONOMORPHIZED into one function per distinct instantiation
    ///     (`db.instantiations`, `lower::type_specialize`): a recursive generic at each concrete type, a
    ///     type-valued-parameter def at each type, or — the ad-hoc-polymorphism case — a `const` dictionary
    ///     inlined at each concrete dictionary. Its instances are listed.
    ///   - `transformed→COPY` — the ACCUMULATOR transform (`db.transformed`) rewrote a linear recursion into
    ///     a tail-recursive copy (a constant-stack loop); the source's body folds to a seed of the copy.
    ///   - `inlined` — β-reduced into each call site (`db.inlined`), no standalone function emitted (a
    ///     non-recursive def / generic — monomorphization by β-reduction).
    ///   - `emitted` — emitted as ONE standalone function under its own name and called (`db.called`) or
    ///     exported (a recursive def with a runtime argument, an `inline-never` def, a boundary entry).
    ///   - `unreferenced` — none of the above (dead code, or a template no use instantiated).
    ///
    /// It FORCES monomorphization over the whole program (`layout::force_monomorphize`) so the record is
    /// complete regardless of what a query-only run would otherwise lower. Answered as TAB-tagged lines: one
    /// `disp<TAB>def-name-node-id<TAB>disposition` (a `+`-joined set when more than one applies), then — for
    /// a `specialized` def — one `inst<TAB>spec-name<TAB>def-name-node-id<TAB>arg;arg;…` per instance (a kept
    /// runtime param `name: TYPE`, an erased compile-time param `const name = VALUE`). Total: an UNKNOWN
    /// name yields the empty result; a known def always gets exactly one `disp` line.
    Instantiations { name: String },
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
    ///
    /// Debug output is thus OPT-IN: absent this request (and `EMIT_DWARF`), a build emits an undecorated
    /// artifact, so a build includes or excludes the capability by whether it names a debug target.
    //= spec/capabilities/debug-information.md#this-capability-is-optional
    //# Debug-information emission MUST be an optional capability a build may include or exclude, in accordance with the build's declared defaults.
    pub const EMIT_WASM_DEBUG: u8 = 0x03;
    /// Emit a DETACHED DWARF sidecar (Mode S of `DESIGN-debug-info-rcdzc.md` §9.2) — a `Target::Dwarf`,
    /// a separate `kind == "dwarf"` artifact carrying only the `.debug_*` sections. The second
    /// enablement mode; like `EMIT_WASM_DEBUG` it needs the `spans` input. A run can request both (a
    /// lean component + its detached DWARF) or either alone.
    pub const EMIT_DWARF: u8 = 0x04;
    /// Emit a UNIT-TEST wasm component (`Request::EmitTests`) — the same wasm target as `EMIT_WASM` but
    /// with the boundary laid out from the program's `@test` NULLARY defs (`layout::compute_tests`)
    /// instead of its `(export …)` clauses. What a `cdz test` build requests.
    pub const EMIT_TESTS: u8 = 0x05;
    pub const QUERY_TYPE_OF: u8 = 0x10;
    pub const QUERY_USES_OF: u8 = 0x11;
    pub const QUERY_TYPE_AT: u8 = 0x12;
    pub const QUERY_DIAGNOSTICS: u8 = 0x13;
    pub const QUERY_RESOLVE_OF: u8 = 0x14;
    pub const QUERY_SCOPE_AT: u8 = 0x15;
    pub const QUERY_EXPORTS: u8 = 0x16;
    pub const QUERY_HIGHLIGHT: u8 = 0x17;
    pub const QUERY_DOC_OF: u8 = 0x18;
    pub const QUERY_DOC_AT: u8 = 0x19;
    pub const QUERY_INSTANTIATIONS: u8 = 0x1a;
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
        tag::EMIT_TESTS => Some(Request::EmitTests),
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
        tag::QUERY_HIGHLIGHT => Some(Request::Query(Query::Highlight)),
        tag::QUERY_DOC_OF => Some(Request::Query(Query::DocOf {
            name: read_string(r)?,
        })),
        tag::QUERY_DOC_AT => Some(Request::Query(Query::DocAt {
            node: u32::try_from(r.read_varu64()?).ok()?,
        })),
        tag::QUERY_INSTANTIATIONS => Some(Request::Query(Query::Instantiations {
            name: read_string(r)?,
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
        Request::EmitTests => out.push(tag::EMIT_TESTS),
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
        Request::Query(Query::Highlight) => out.push(tag::QUERY_HIGHLIGHT),
        Request::Query(Query::DocOf { name }) => {
            out.push(tag::QUERY_DOC_OF);
            write_string(out, name);
        }
        Request::Query(Query::DocAt { node }) => {
            out.push(tag::QUERY_DOC_AT);
            leb128::write_u64(out, *node as u64);
        }
        Request::Query(Query::Instantiations { name }) => {
            out.push(tag::QUERY_INSTANTIATIONS);
            write_string(out, name);
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
///
/// Every arm is TOTAL — a name/node with no answer (an unbound name, a node past the program, a poison)
/// yields a defined "unknown"/empty result, so a query over an incomplete or malformed program returns a
/// partial answer rather than failing opaquely, and never crashes the caller's session:
//= spec/capabilities/tooling-and-lsp.md#queries-over-incomplete-source-are-total
//# A tooling query over source that does not fully parse MUST return a defined partial result rather than fail opaquely.
//= spec/capabilities/tooling-and-lsp.md#queries-over-incomplete-source-are-total
//# A tooling query MUST NOT crash the editor session on malformed source.
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
                // A name that names no definition — but a near-miss for a REAL def name (`computee` for
                // `compute`) is almost always a typo, so name the nearest, the same closed-set "did you
                // mean?" the compiler's unbound-name / export / pragma sites carry. The candidate pool is
                // the program's own def names, so a suggestion can only ever point at a real definition.
                None => {
                    let names: Vec<&str> = db.defs.iter().map(|d| d.name.as_str()).collect();
                    let hint = crate::diag::suggest::did_you_mean(name, names, 3);
                    format!("no such definition `{name}`{hint}")
                }
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
                            crate::FixKind::Wrap => "wrap",
                            crate::FixKind::Delete => "delete",
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
            let mut text = String::new();
            if db.is_user_node(id) {
                let target = match crate::resolve::resolved_of(db, id) {
                    Resolved::Ref { value } => Some(value),
                    Resolved::Lambda { body, .. } => Some(body),
                    _ => None,
                };
                if let Some(target) = target {
                    text = format!("{}\n", name_occ_of_def_body(db, target).0);
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
        Query::Highlight => {
            // Every USER LEAF classified by the role it plays — one `node-id<TAB>kind` line, ascending
            // id order. A leaf is what an editor paints (a name atom, a literal); a list form is not
            // itself a token (its head/children are), so only leaves are emitted. Classification reads
            // the resolved column + the meta channels a value already carries (see `classify_highlight`).
            let node_count = db.ast.structure.len();
            let mut text = String::new();
            for i in 0..node_count {
                let id = StructId(i as u32);
                if !db.is_user_node(id) {
                    continue;
                }
                if let Some(kind) = classify_highlight(db, id) {
                    text.push_str(&format!("{}\t{}\n", id.0, kind.as_str()));
                }
            }
            QueryResult {
                kind: KIND_HIGHLIGHT,
                name: "highlight".to_string(),
                bytes: text.into_bytes(),
            }
        }
        Query::DocOf { name } => {
            // The documentation of a NAME, from the ordered fallback (user def → built-in `(meta doc)` →
            // grammar keyword). Each step reads a column the compiler already fills; none matches the name
            // against a hardcoded key (the built-in step resolves the name to its prelude record, THEN
            // reads a channel — the same generic path member access takes).
            let text =
                doc_of_name(db, name).unwrap_or_else(|| format!("no documentation for `{name}`"));
            QueryResult {
                kind: KIND_DOC,
                name: name.clone(),
                bytes: text.into_bytes(),
            }
        }
        Query::DocAt { node } => {
            // The documentation of the definition the node belongs to or references — the "hover doc". A
            // node inside a def's OWN body/signature, or a REFERENCE resolving to a def, maps to that def's
            // signature occurrence; its `(doc "…")` text is read from the doc column. TOTAL: a node that
            // does not reach a documented definition yields the empty result (no line).
            let id = StructId(*node);
            let text = doc_at_node(db, id).unwrap_or_default();
            QueryResult {
                kind: KIND_DOC,
                name: node.to_string(),
                bytes: text.into_bytes(),
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
            let text = instantiations_text(db, name);
            QueryResult {
                kind: KIND_INSTANTIATIONS,
                name: name.clone(),
                bytes: text.into_bytes(),
            }
        }
    }
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
fn instantiations_text(db: &mut Db, name: &str) -> String {
    // Resolve the name to its source def. An unknown name → empty (total). A def name that ALSO has
    // specializations resolves to the ORIGINAL (a specialization's synthetic `#mono` name never equals a
    // user name), so `def_by_name` is the right lookup.
    let Some(def_idx) = db.def_by_name(name) else {
        return String::new();
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
    // definition; `-` if the signature is malformed. Same name-occurrence convention as `Exports`.
    let sig = db.defs[def_idx].sig_occ;
    let name_node = match db.ast.get(sig) {
        Struct::List(kids) => kids
            .first()
            .map_or_else(|| "-".to_string(), |n| n.0.to_string()),
        _ => "-".to_string(),
    };

    let mut text = format!("disp\t{name_node}\t{}\n", dispositions.join("+"));

    // The instantiation payload lines — one per DISTINCT specialization of this def, in synthesis order.
    // Snapshot first (the immutable `db.instantiations` borrow cannot coexist with the `def_index_by_body`
    // reads), then keep those whose `orig_body` maps back to this def.
    let records: Vec<(StructId, usize, Vec<String>)> = db
        .instantiations
        .iter()
        .map(|inst| (inst.orig_body, inst.spec_index, inst.args.clone()))
        .collect();
    for (orig_body, spec_index, args) in records {
        if db.def_index_by_body(orig_body) != Some(def_idx) {
            continue;
        }
        let spec_name = db.defs[spec_index].name.clone();
        text.push_str("inst\t");
        text.push_str(&spec_name);
        text.push('\t');
        text.push_str(&name_node);
        text.push('\t');
        text.push_str(&args.join(";"));
        text.push('\n');
    }
    text
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
    // 3. A grammar keyword's doc.
    grammar_keyword_doc(name).map(str::to_string)
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
        crate::ast::Leaf::Int { .. } | crate::ast::Leaf::Float(_) => {
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
    Some(classify_resolved_name(db, id))
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
    // Record field name: `id` is the FIRST child of a two-element pair `(field value)` whose grandparent
    // is a `("record" …)` form. (The pair itself is a headless list; `read_record_fields` reads its
    // first child as the field label.)
    if let Struct::List(pair) = db.ast.get(parent)
        && pair.len() == 2
        && pair[0] == id
        && let Some(grand) = db.parent_of(parent)
        && db.ast.head_ctor(grand) == Some("record")
    {
        return true;
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
            Request::Query(Query::Highlight),
            Request::Query(Query::DocOf {
                name: "helper".into(),
            }),
            Request::Query(Query::DocAt { node: 11 }),
            Request::Query(Query::Instantiations {
                name: "fold-n".into(),
            }),
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
