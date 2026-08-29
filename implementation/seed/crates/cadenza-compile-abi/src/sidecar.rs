//! The sidecar request/query WIRE — the `Request` (materialize an output column) and `Query` (read a
//! fact column) enums that make up one sidecar request list, plus the [`encode`]/[`decode`] CODEC that
//! serializes a request list to/from the canonical binary AST (`cadenza_ast::codec`). This is the wire
//! contract both sides of the compiler boundary agree on: `rcdzc` decodes a request list + runs it, and
//! `cdz` (in a `!standalone` build) builds + encodes the identical list to hand `cdz-compile`. Only the
//! query IMPLEMENTATION (`run_query`, which reads a live `Db`) stays in `rcdzc`; the vocabulary + its
//! codec live here. `rcdzc` `pub use`s them, so `rcdzc::Request`/`Query`/`encode`/`decode` (and the
//! `crate::sidecar::` paths) stay byte-stable.
//!
//! NOTE: the doc comments below reference `rcdzc`-internal producers (`layout::…`, `backend::…`) that
//! describe HOW each variant is materialized; those intra-doc links resolve only in `rcdzc` (they are
//! plain prose here). The sibling-variant / `Target` links resolve locally.

use cadenza_ast::ast::{Arenas, Builder, IntValue, Leaf, Radix, Struct, StructId};

/// One request in a sidecar's list. Either MATERIALIZE an output column (`Emit`) or READ a fact column
/// (`Query`). `Rewrite` (the validated-transaction arm of `DESIGN-sidecar-api.md`) is a later rung and
/// not modeled here yet.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Request {
    /// Materialize a backend artifact — the generalization of a `Target`. Carried as the target's own
    /// enum so the emit path is literally today's, reached through the request list instead of the
    /// fixed `targets` slice.
    Emit(crate::Target),
    /// Materialize a UNIT-TEST wasm component — the artifact a `cdz test` build produces. It is the same
    /// wasm target as `Emit(Wasm)` EXCEPT the boundary is laid out from the program's `@test` NULLARY
    /// definitions (`layout::compute_tests`) instead of its `(export …)` clauses: every test crosses as a
    /// nullary entry the runner invokes (a failing test traps; a passing one returns unit). Kept a
    /// distinct request (not a `Target`) because it changes the EXPORT SOURCE — a layout concern — not the
    /// backend; the ordinary `(export …)` compile is untouched, so a normal build never carries a test's
    /// report-effect import.
    EmitTests,
    /// Materialize N PER-FILE unit-test components in ONE compile — the `cdz test <dir>` compile-reuse
    /// path. Like [`EmitTests`], but instead of ONE component over ALL linked `@test` defs, it lowers the
    /// linked closure ONCE and emits one component PER FILE: `db.test_defs()` is bucketed by
    /// `linkage.file_of(sig_occ)`, and each non-empty bucket becomes a layout-view (`compute_tests_for`)
    /// rooted at that file's tests, emitted as a `component` artifact NAMED by the file (its `link` path).
    /// This replaces N per-file `cdz test` compiles (each re-lowering the whole ~1360-def closure) with ONE
    /// shared-arena lowering + N cheap layout-views — behavior-identical (same tests, same per-file layout,
    /// same node-anchored diagnostics), just without the redundant re-lowering. A SINGLE-file (unlinked)
    /// compile has one bucket and behaves like `EmitTests` with a file-named artifact. See
    /// `DESIGN-shared-backend-space.md` (the shared-arena lower-once design).
    EmitTestsPerFile,
    /// OPTION C — materialize a SHARED-CLOSURE PROVIDER component + N per-file CONSUMER components in ONE
    /// compile (the composed `cdz test <dir>` emit-reuse path). Unlike [`EmitTestsPerFile`] — where each
    /// per-file component EMBEDS the whole imported closure (so the gate pays O(tests × closure-size) on
    /// emit+JIT) — this hoists the shared closure into its OWN component ONCE: the UNION of every file's
    /// cross-component call-edges ([`layout::cross_component_edges_union`]) becomes a provider component
    /// ([`layout::compute_provider_for_edges`], `db.component_name` set) exporting those edges by their source
    /// boundary name, and each per-file `@test` bucket becomes a CONSUMER component
    /// ([`layout::compute_tests_consumer`]) that EXCLUDES the shared defs and IMPORTS them from the provider.
    /// A downstream runner instantiates the ONE provider as a peer and links each consumer against it over the
    /// shared runtime (`run_with_peers`). Emits: one `component-provider` artifact (the provider), one
    /// `component-name` sidecar (the provider's published interface string), and N `component` artifacts (the
    /// consumers, each named by its file's `link` path — the SAME name-demux as `EmitTestsPerFile`). GUARDED
    /// to a SINGLE directory: `db.file_path` is the import-STEM (dir-blind, load-bearing for link resolution),
    /// so a multi-dir tree with same-stem files would collide the demux (pr881) — a build spanning >1 dir, or
    /// with no cross-edge to hoist, DECLINES so the caller falls back to the per-file `EmitTests`/`EmitTestsPerFile`
    /// path (behavior-identical, just without the closure-sharing win). See
    /// `backend/wasm/DESIGN-option-c-shared-closure-component.md`.
    EmitTestsComposed,
    /// OPTION C follow-on — emit ONLY the per-file CONSUMER components (importing the shared-closure provider
    /// interface), SKIPPING the provider emit. The provider-CACHE path: a runner that already holds the shared
    /// closure's provider component (built once, cached across invocations keyed on the closure's cross-edge
    /// content-hashes) requests this to emit just the cheap consumer(s) for the file(s) under test, then links
    /// them against the cached provider at run time (`run_with_peers`). Unlike [`EmitTestsComposed`] — which
    /// emits the provider + consumers together (paying the expensive whole-closure lower+emit every time) —
    /// this emits NO `component-provider` (the caller supplies the cached one); it emits the N `component`
    /// consumers (named by `db.file_path`) + the `component-name` iface sidecar (so the runner knows which
    /// cached provider to pair). The win: v-compiler-ml's SINGLE-FILE `cdz test` no longer re-lowers the
    /// ~1360-def closure — the consumer layout ([`layout::compute_tests_consumer`]) excludes the cross-edges
    /// and needs only the file's `@test` defs + the cross-edge SET + the iface, NOT the provider lowering.
    /// Same single-dir/stem-collision guard + fallback as `EmitTestsComposed`. See
    /// `backend/wasm/DESIGN-option-c-shared-closure-component.md` (the single-file provider-cache follow-on).
    EmitTestsConsumerOnly,
    /// The COMPILER-DRIVEN test SHRED (the operator's "emit main + a target per test" model): emit ONE
    /// whole-library MAIN component (every reachable non-`@test` def, exported under the closure interface —
    /// the shared library the tests call, lowered ONCE) plus ONE thin CONSUMER component PER `@test` (each
    /// exports just its one test and imports the whole library from main). A runner then executes each test
    /// target linked against main (`cdz-run test-<k>.wasm --peer <iface>=main.wasm`), grading by exit code —
    /// a per-TEST ca-derivation matrix (parallel + content-addressed) instead of one coarse per-project run.
    /// Reuses [`layout::compute_provider_for_edges`] (main, over the whole-library edge set) +
    /// [`layout::compute_tests_consumer`] (each test, a single-def slice over that same boundary) — the SAME
    /// machinery [`EmitTestsComposed`] runs, but the boundary is the WHOLE library (not just cross-FILE edges),
    /// so a SAME-FILE suite (no cross-file imports) still gets a non-empty main + uniform per-test linking. See
    /// `design/DESIGN-cdz-plugin-dispatch.md` §S6b. Runs on its OWN branch (multiple artifacts from one shared
    /// lowering), like the composed request.
    EmitTestsShred,
    /// The STANDALONE variant of [`EmitTestsShred`]: emit NO main; each `@test` becomes a SELF-CONTAINED
    /// component (its reachable library INLINED/included via [`layout::compute_tests_for`]), run with NO
    /// `--peer`. The operator-approved caching-aware HYBRID uses this for SMALL-closure suites
    /// (iterators/cad/choreography) — no cross-component peer boundary, so a compound-param `@test` that would
    /// DECLINE at the peer boundary (the deferred #4031 limit) shreds CLEANLY here (full coverage), at the cost
    /// of re-embedding each test's closure (fine for a small closure; the shared-main [`EmitTestsShred`] +
    /// grouping stays for the big compiler-ml closure). Same per-test artifact + manifest shape as
    /// [`EmitTestsShred`], but `main-file` = "" for every entry (there is no main). See §S6b.
    EmitTestsShredStandalone,
    /// The TWO-STAGE variant of [`EmitTestsShred`] for standalone-everywhere heavy suites (compiler-ml/cad):
    /// emit cadenza-ast FRAGMENTS, not wasm. ONE shared-closure fragment (`closure.cdzb`, a no-export
    /// `(do (def..)..)` of the reachable non-`@test` library, via [`backend::cadenza::emit_fragment`] with
    /// `include_type_decls=true`, lowered ONCE — its byte-stable bytes are v-nix's CA key) + one per-`@test`
    /// fragment (`test-<name>.cdzb`, `emit_fragment({test}, include_type_decls=false)`). The per-test wasm is
    /// built LATER by v-nix's fan-out as `rcdzc closure.cdzb test-<name>.cdzb --export <name>` (the `--export`
    /// splice, #5405) — so the ~1360-fn closure's expensive lower+opt happens ONCE + CA-caches, and each test
    /// is cheap codegen: O(closure_once + tests×body) instead of standalone's O(tests×closure). See §S6b
    /// (two-stage) + `backend/cadenza::emit_fragment` (#5401).
    EmitTestsShredTwoStage,
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
    /// Each exported definition's RESOLVED type as a STRUCTURED cdzast sub-AST — the STRUCTURED companion
    /// of `Exports` (which renders types to a STRING). The cadenza-docs I2 fact (`(C)` architecture):
    /// `rcdzc` cannot depend on `cadenza-syntax` (copy-don't-depend), so `cdz doc` runs the I1 doc-item
    /// projection on the `cadenza-syntax` side, asks THIS query for the resolved types, and grafts each as
    /// the doc-item's `(ty …)` — the type reaches docs as a queryable/linkable sub-AST, not a printed
    /// string. Bulk (no args) so `cdz doc` makes ONE round-trip. Answered as the `KIND_EXPORT_TYPES` blob:
    /// `u32_le count` then per export `u32_le name_len | name | u32_le ty_len | ty_bytes`, where `ty_bytes`
    /// is `codec::encode` of a standalone arena rooted at `eval::encode_ty_payload(db, &scheme.ty)` — a
    /// full cdzast artifact the consumer's byte-identical codec decodes directly. Covers all THREE exported
    /// item kinds: a value DEF (its `def_scheme.ty` via `encode_ty_payload` → a `(-> …)`/scalar), a TYPE
    /// decl (`typeval_of` → a `(Sum …)`/`(Record …)`), and an EFFECT decl — the effect is a GROUPED
    /// op-signature payload `(effect (op <name> <arrow>)…)` (an effect has no single `Ty`; each op's arrow
    /// is encoded like a def/type, via `effect_op_types_node`). Resolved by trying the def slot, then the
    /// type-decl, then the effect-decl by name. An export whose type does not resolve in any kind — a def
    /// with an ill-typed annotation, an effect with no resolvable op — is OMITTED (graceful-degrade: the
    /// doc-item's `(ty …)` is optional). Deterministic (export order). Co-owned with v-syntax.
    ExportedTypes,
    /// The `@test`-marked definitions the test runner would run — name + property-vs-nullary flag + source
    /// file, so `cdz test --list` reads them from THIS binary query (no rcdzc link in `cdz`; operator
    /// no-rcdzc-in-`cdz` step, v-cdz-crate-split). Answered as the [`KIND_TEST_LIST`] blob. Enumerated from
    /// `db.test_defs()` (already package-scoped by the linked closure); `is_property` = `!params.is_empty()
    /// || name.ends_with("-gen")` (canonical `compile_tests`). Bulk (no args) — one round-trip.
    TestList,
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
    /// The DOCUMENT OUTLINE — every TOP-LEVEL declaration in the module, classified by what it declares,
    /// so an editor can render a symbol tree / breadcrumb (`textDocument/documentSymbol`). Reads the SAME
    /// declaration columns a compile fills — `db.defs` (value/function definitions), `db.type_decls` (sum
    /// types), `db.effect_decls` (effects), `db.modules` (namespaces) — so a symbol equals what the
    /// compiler scanned, never a second lexical pass. This is the SUPERSET companion of `Exports` (which
    /// lists only the `(export …)`-named subset with their solved types): `Symbols` lists EVERY
    /// declaration — private ones included — by kind, which is what an outline shows. Answered as one
    /// `name<TAB>kind<TAB>name-node-id` line per declaration; `kind` is a fixed closed vocabulary
    /// (`SymbolKind` — `value`/`function`/`type`/`effect`/`module`), the node id is the declaration's NAME
    /// occurrence so a consumer can jump to it. Node-id-keyed and span-free like the other queries; the
    /// order is declaration order within each column, columns grouped (defs, then types, effects, modules),
    /// so it is a deterministic function of the program. TOTAL — a module with no declarations yields the
    /// empty result. INTERNAL defs (do-local / module-member callables `modules::register_callable`
    /// synthesizes) are OMITTED: they are not top-level source declarations an outline names.
    Symbols,
    /// The `@param` WIDGET MANIFEST — one record per well-typed `@param` site `(: (@ (param <kv>) name)
    /// Type)`, the data a HOST (browser/CAD/notebook) reads to render a control per program parameter.
    /// Reads `param_sidecar::scan_manifest` (the read-only `@param`-site scan — the SAME sites the
    /// generate half turns into a typed `Param` effect, so the manifest's rendered type and the generated
    /// accessor's result type share one source of truth), then renders each record's DECLARED TYPE via the
    /// type column (`infer::type_of` → `Ty::render_name` — the render needs the `Db`, which lives on this
    /// query side, not the arena-only scan). Answered as one record per line, TAB-separated:
    /// `name  widget  type  range-lo-node  range-hi-node  options-node  default-node  name-node` — `widget`
    /// is the bare widget atom or `-`; `type` is the rendered declared type; the four value columns are the
    /// arena node ids of `(: range [lo hi])`'s two elements, `(: options […])`'s list, and `(: default v)`'s
    /// value (`-` when the kv is absent), which the CONSUMER (holding the shared `StructId` arena + span
    /// table) renders + maps to `file:line:col` — node-id-keyed and span-free like the sibling queries
    /// (`query-engine.md`: the compiler emits node IDENTITY, the front-end owns spans + value rendering).
    /// `name-node` is the param NAME occurrence, mapped to `file:line:col`. TOTAL: a program with no
    /// `@param` sites yields the empty result.
    ParamManifest,
    /// The EMITTED-FUNCTION LAYOUT — every reachable definition's absolute wasm FUNCTION INDEX paired with
    /// a CONTENT-HASH of its AST subtree, in func-index order. FORCES monomorphization then runs
    /// `layout::compute` (like `Instantiations`), so the reported set + order equal what a real emit lays.
    /// Answered as one line per reachable def, `func-index<TAB>content-hash<TAB>def-name`, preceded by a
    /// single MARKER line `defs-begin<TAB><import_base><TAB>-` giving the def-region base (runtime-op
    /// imports occupy `0..import_base` ahead of the first defined function), so a consumer knows where the
    /// def region starts and compares the right range. The content-hash is a stable hash of the def's
    /// `(def …)` AST subtree (structure + leaves), so two builds that SHARE a def (byte-identical source)
    /// report the SAME hash — the observation the compile-reuse prove-first witness diffs (a shared def
    /// must get the SAME func-index AND the SAME content-hash across two per-file test builds) and the
    /// basis for the Option-A content-addressed cache key. Roots on `(export …)`; a program with NO export
    /// falls back to the `@test`-rooted layout (`layout::compute_tests`, what `EmitTests`/`cdz test` lays),
    /// so a pure-`@test` file reports the marker + its `@test`-reachable def rows. TOTAL: the result is
    /// empty ONLY when BOTH layouts decline (no export AND no `@test`); an empty-but-laid-out program yields
    /// just the marker line. Deterministic (func-index order).
    FuncLayout,
    /// The Option-C shared-closure CONTENT-HASH for the `@test` dir — a `u64` (hex) folding each cross-edge
    /// def's content-hash over the UNION cross-edge set ([`closure_content_hash`] over
    /// [`crate::layout::cross_component_edges_union`]). The provider-CACHE decision key: a `cdz test <dir>`
    /// runner reads this to decide a cache HIT (compare to the cached provider's key) BEFORE emitting —
    /// LAYOUT-ONLY (`compute_tests` + the cross-edge union + the fold), NO provider emit, so it costs the
    /// ~layout pass (≈ func-layout), NOT the ~1360-def provider lower it lets the runner SKIP on a hit
    /// (`EmitTestsConsumerOnly` + the cached provider). The SAME canonical hash the `EmitTestsComposed`
    /// miss-path `closure-hash` sidecar emits — one definition, so the decision key, the persist key, and a
    /// runner's own validation all agree (drift-guard). TOTAL: a build with no cross-edge (single-file / no
    /// shared closure) reports the fold over the empty set (a defined, stable "no closure" hash), never errors.
    ClosureHash,
}

/// Decode a request list from its wire bytes. Total: a truncated or malformed list yields `None` (the
/// caller turns that into a decline diagnostic), never a panic. Mirrors `codec::decode`'s discipline
/// exactly so the format ports to the self-host.
pub fn decode(bytes: &[u8]) -> Option<Vec<Request>> {
    // The request list is a BINARY-AST value (operator ruling 2026-08-26: "the sidecar absolutely needs
    // to use the binary AST" — not a bespoke tag+LEB128 vocabulary). Decode via the SAME `crate::codec`
    // a program's AST uses — rcdzc's copy; a delegating `cdz` builds the byte-identical tree via
    // cadenza-syntax's copy (copy-don't-depend, no shared crate). The ROOT is an `Ast.List` of per-request
    // forms. Total: a malformed tree / unknown head / bad operand yields `None` (the caller declines).
    let a = cadenza_ast::codec::decode(bytes)?;
    let Struct::List(forms) = a.get(a.root).clone() else {
        return None;
    };
    forms.iter().map(|&f| decode_request(&a, f)).collect()
}

/// Decode one request FORM: `(emit <target>)` | `(emit-tests…)` | `(query <selector> <args>…)`.
fn decode_request(a: &Arenas, form: StructId) -> Option<Request> {
    let Struct::List(children) = a.get(form) else {
        return None;
    };
    Some(match a.as_name(*children.first()?)? {
        "emit" => Request::Emit(target_from(a.as_name(*children.get(1)?)?)?),
        "emit-tests" => Request::EmitTests,
        "emit-tests-per-file" => Request::EmitTestsPerFile,
        "emit-tests-composed" => Request::EmitTestsComposed,
        "emit-tests-consumer-only" => Request::EmitTestsConsumerOnly,
        "emit-tests-shred" => Request::EmitTestsShred,
        "emit-tests-shred-standalone" => Request::EmitTestsShredStandalone,
        "emit-tests-shred-two-stage" => Request::EmitTestsShredTwoStage,
        "query" => Request::Query(decode_query(
            a,
            a.as_name(*children.get(1)?)?,
            &children[2..],
        )?),
        _ => return None,
    })
}

/// Map a `(query <selector> <args>…)` selector + operands to a `Query`. A NAME-arg query reads an
/// `Ast.Str`; a NODE-arg query reads an `Ast.Int` (the `u32` StructId); a nullary query takes none.
fn decode_query(a: &Arenas, selector: &str, args: &[StructId]) -> Option<Query> {
    Some(match selector {
        "type-of" => Query::TypeOf {
            name: qname(a, args)?,
        },
        "uses-of" => Query::UsesOf {
            name: qname(a, args)?,
        },
        "doc-of" => Query::DocOf {
            name: qname(a, args)?,
        },
        "instantiations" => Query::Instantiations {
            name: qname(a, args)?,
        },
        "type-at" => Query::TypeAt {
            node: qnode(a, args)?,
        },
        "resolve-of" => Query::ResolveOf {
            node: qnode(a, args)?,
        },
        "scope-at" => Query::ScopeAt {
            node: qnode(a, args)?,
        },
        "doc-at" => Query::DocAt {
            node: qnode(a, args)?,
        },
        "diagnostics" => Query::Diagnostics,
        "highlight" => Query::Highlight,
        "exports" => Query::Exports,
        "exported-types" => Query::ExportedTypes,
        "test-list" => Query::TestList,
        "symbols" => Query::Symbols,
        "param-manifest" => Query::ParamManifest,
        "func-layout" => Query::FuncLayout,
        "closure-hash" => Query::ClosureHash,
        _ => return None,
    })
}

/// A `(query <selector> "name")` operand — the first arg as an `Ast.Str`.
fn qname(a: &Arenas, args: &[StructId]) -> Option<String> {
    a.as_str(*args.first()?).map(str::to_string)
}

/// A `(query <selector> <node>)` operand — the first arg as an `Ast.Int` narrowed to a `u32` StructId.
fn qnode(a: &Arenas, args: &[StructId]) -> Option<u32> {
    u32::try_from(a.as_int(*args.first()?)?.to_i64()?).ok()
}

/// The `Target` an `(emit <target>)` selector names.
fn target_from(name: &str) -> Option<crate::Target> {
    use crate::Target;
    Some(match name {
        "wasm" => Target::Wasm,
        "wasm-debug" => Target::WasmDebug,
        "dwarf" => Target::Dwarf,
        "rust" => Target::Rust,
        "rust-async" => Target::RustAsync,
        "cadenza" => Target::Cadenza,
        _ => return None,
    })
}

/// Encode a request list to its wire bytes — the counterpart to `decode`, used by a driver (and the
/// tests) to build a sidecar input. `decode(encode(rs)) == rs`.
pub fn encode(requests: &[Request]) -> Vec<u8> {
    // Build the request list as a BINARY-AST value (root = an `Ast.List` of per-request forms) and encode
    // it with the SAME `crate::codec` a program's AST uses — the counterpart to `decode`, so
    // `decode(encode(rs)) == rs`, and byte-identical to the tree a delegating `cdz` builds via
    // cadenza-syntax's codec copy.
    let mut b = Builder::new();
    let forms: Vec<StructId> = requests
        .iter()
        .map(|req| encode_request(&mut b, req))
        .collect();
    let root = b.list(forms);
    cadenza_ast::codec::encode(&b.finish(root))
}

/// Build one request FORM: `(emit <target>)` | `(emit-tests…)` | `(query <selector> <args>…)`.
fn encode_request(b: &mut Builder, req: &Request) -> StructId {
    match req {
        Request::Emit(t) => {
            let head = b.name("emit");
            let target = b.name(target_name(*t));
            b.list(vec![head, target])
        }
        Request::EmitTests => nullary_form(b, "emit-tests"),
        Request::EmitTestsPerFile => nullary_form(b, "emit-tests-per-file"),
        Request::EmitTestsComposed => nullary_form(b, "emit-tests-composed"),
        Request::EmitTestsConsumerOnly => nullary_form(b, "emit-tests-consumer-only"),
        Request::EmitTestsShred => nullary_form(b, "emit-tests-shred"),
        Request::EmitTestsShredStandalone => nullary_form(b, "emit-tests-shred-standalone"),
        Request::EmitTestsShredTwoStage => nullary_form(b, "emit-tests-shred-two-stage"),
        Request::Query(q) => encode_query(b, q),
    }
}

/// A `(<head>)` form — an emit-tests variant with no operands.
fn nullary_form(b: &mut Builder, head: &str) -> StructId {
    let h = b.name(head);
    b.list(vec![h])
}

/// Build a `(query <selector> <arg>…)` form: a NAME-arg query carries an `Ast.Str`, a NODE-arg query an
/// `Ast.Int`, a nullary query none. Mirror-inverse of `decode_query`.
fn encode_query(b: &mut Builder, q: &Query) -> StructId {
    let (selector, arg): (&str, Option<StructId>) = match q {
        Query::TypeOf { name } => ("type-of", Some(atom_str(b, name))),
        Query::UsesOf { name } => ("uses-of", Some(atom_str(b, name))),
        Query::DocOf { name } => ("doc-of", Some(atom_str(b, name))),
        Query::Instantiations { name } => ("instantiations", Some(atom_str(b, name))),
        Query::TypeAt { node } => ("type-at", Some(atom_int(b, *node))),
        Query::ResolveOf { node } => ("resolve-of", Some(atom_int(b, *node))),
        Query::ScopeAt { node } => ("scope-at", Some(atom_int(b, *node))),
        Query::DocAt { node } => ("doc-at", Some(atom_int(b, *node))),
        Query::Diagnostics => ("diagnostics", None),
        Query::Highlight => ("highlight", None),
        Query::Exports => ("exports", None),
        Query::ExportedTypes => ("exported-types", None),
        Query::TestList => ("test-list", None),
        Query::Symbols => ("symbols", None),
        Query::ParamManifest => ("param-manifest", None),
        Query::FuncLayout => ("func-layout", None),
        Query::ClosureHash => ("closure-hash", None),
    };
    let head = b.name("query");
    let sel = b.name(selector);
    let mut children = vec![head, sel];
    children.extend(arg);
    b.list(children)
}

/// The selector name for an `(emit <target>)` form.
fn target_name(t: crate::Target) -> &'static str {
    use crate::Target;
    match t {
        Target::Wasm => "wasm",
        Target::WasmDebug => "wasm-debug",
        Target::Dwarf => "dwarf",
        Target::Rust => "rust",
        Target::RustAsync => "rust-async",
        Target::Cadenza => "cadenza",
    }
}

/// An `Ast.Str` leaf atom for a query name operand.
fn atom_str(b: &mut Builder, s: &str) -> StructId {
    b.atom_leaf(Leaf::Str(s.into()))
}

/// An `Ast.Int` leaf atom (decimal) for a query node-id operand.
fn atom_int(b: &mut Builder, n: u32) -> StructId {
    b.atom_leaf(Leaf::Int {
        value: IntValue::from_i64(i64::from(n)),
        radix: Radix::Dec,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Round-trips over the encode/decode codec now that it lives here — one representative Emit(Target),
    // a nullary EmitTests* variant, and a Query with a String and a u32 operand. Mirrors the discipline
    // of rcdzc's fuller sidecar round-trip suite (which still exercises this codec through the re-export).
    #[test]
    fn encode_decode_round_trips_representative_requests() {
        let requests = vec![
            Request::Emit(crate::Target::Wasm),
            Request::EmitTestsShred,
            Request::Query(Query::TypeOf {
                name: "foo".to_string(),
            }),
            Request::Query(Query::TypeAt { node: 42 }),
            Request::Query(Query::TestList),
        ];
        assert_eq!(decode(&encode(&requests)), Some(requests));
    }

    #[test]
    fn decode_rejects_malformed_bytes() {
        // Total decode: garbage yields None (a decline), never a panic.
        assert_eq!(decode(&[0xff, 0xff, 0xff, 0xff]), None);
        // An empty request list round-trips to the empty vec.
        assert_eq!(decode(&encode(&[])), Some(vec![]));
    }
}
