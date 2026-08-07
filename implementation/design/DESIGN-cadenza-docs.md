# Design — Cadenza documentation publishing (docs → binary AST, docs-in-harness, guide-as-sexpr)

**Author:** design-cadenza-docs (autonomous — operator away ~2 weeks, no interactive sessions)
**Audience:** `v-metaprogramming`, `v-syntax`, `v-guide` / `v-guide-infra` / `v-guide-editor`, `v-agent-harness` / `v-agent-harness-host`, `corpus-bugfix` (PM)
**Status:** PROPOSAL — landed (PR #2559). REVISED 2026-08-07 to fold in operator refinements (doc-extraction = a COMPILER QUERY; the compiler emits STRUCTURED info only, rendering is the consumer's job) + six stakeholder replies. Increments routed: I1 → v-syntax. Open decisions §9 mostly resolved by stakeholder input.

---

## 1. The feature

Operator idea, verbatim:

> "We need a way to publish documentation for a cadenza program. Kind of like rust docs. But the
> output should be a binary AST and then we can use all of the same tooling on that. Additionally
> I'm thinking it would be great if we could publish docs in the harness so agents could document
> their stuff. And honestly having the language guide there as well would be really helpful. And so
> I've been thinking if we should extract all of our guide content into sexprs and then we'd either
> pull those in the guide or codegen the jsx. Either way having documentation around the language
> and platform in the harness is going to be super helpful."

Operator refinements (during review, folded in below):
> "For the docs I'm assuming we're making it a compiler query. That's the way to go here, just like
> any other query the compiler supports."
> "Rendering should be the job of the consumer, not the compiler. We should simply be emitting as
> much information that the consumer would need to render the documentation effectively."

Three coupled threads, one unifying insight:

1. **Doc-extraction → binary AST, as a COMPILER QUERY.** A query over the compiled program (the same
   way the compiler already supports queries — `type-of`, `resolve`, the `query.rs`/selector
   machinery) that projects the program's public surface (doc comments + names + *structured*
   signatures/types) into a **derived binary AST**. It IS a `cdzast` value, so every existing AST
   surface (`cadenza-syntax` reader/printer, ML/sexpr, metaprog `Ast`, the codec/dictionary) applies
   to it uniformly. **The compiler emits STRUCTURED information only — it does NOT render.** No
   printed/formatted strings baked into the doc-AST; a consumer that wants a display string runs the
   printer itself over the structured node.
2. **Docs-in-harness.** Sessions/reducers publish docs about their own code into the harness so
   agents can document their stuff and query each other's docs.
3. **Guide-in-harness.** Extract the existing guide content into sexprs, then **codegen the TSX**
   chapter modules from them — so the language + platform guide is itself binary-AST documentation
   living in the harness, queryable by agents by the SAME tooling as (1) and (2).

**The unifying insight:** all three are the *same doc model*. A program's doc-index, an agent's
published docs, and the language guide are all `cdzast` binary-AST documents. One model → one
codec → one query surface → agents query "all documentation" through a single tooling stack.

---

## 2. The load-bearing forks

### 2.1 Doc-AST shape — DECIDED (operator): **derived doc-item projection, structured-only**
Not a view over the raw program AST, and not a standalone non-`cdzast` schema. The doc query emits a
NEW binary-AST construct built from the same `cadenza-ast` arenas — new head *names*
(`doc-module`, `doc-item`, `sig`, `ty`), never a new `Struct` variant (keywords-are-data,
per the frozen 2-variant `Struct`). Rationale: reuses the arena model + codec + every AST surface;
honors the standing binary-AST-reuse order; a doc index is itself matchable/queryable as `Ast`.

**Structured, not rendered (operator ruling).** The emitted doc-AST carries the *structured*
signature as a type sub-AST — NOT a pre-printed display string. Rendering (turning `(sig …)` into
`(a -> b) -> List a -> List b`, HTML, markdown) is the CONSUMER's job: it runs the ML printer over
the structured node. This keeps the doc-AST a faithful data projection and avoids the compiler
choosing a rendering.

```
(doc-module "mymod"
  (module-doc "Top-of-module prose.")
  (doc-item
    (name "map")
    (doc  "Applies f to each element.")
    (sig  (-> (-> a b) (List a) (List b)))         ; STRUCTURED signature sub-AST — consumer prints it
    (kind def)
    (visibility public))
  (doc-item …))
```

`(sig …)` is the structured signature sub-AST (the source annotation as-written pre-typecheck;
enriched with the resolved type post-typecheck — see §4.2). No separate printed-string field: a
display string is `printer(sig)`, computed by the consumer, never stored.

### 2.2 Guide direction — DECIDED (operator): **codegen TSX from sexprs**
Author guide content as sexprs; a build step emits the TSX chapter modules. The existing React site,
`<Runnable>` live-compile, `<Exercise>`/`<Why>`, and the `check-*.mjs` gate stay intact. Sexpr
becomes source-of-truth; TSX is generated. Rejected the runtime-pull alternative: it would need a
new runtime sexpr→React renderer and rework how `<Runnable>`/exercises embed — more surface, no gate
reuse.

### 2.3 Harness publish surface — DECIDED (operator): **reuse CAS blob + name-store; index in the reducer; ZERO new kernel family — but ONE missing generic mechanism**
Publish a doc-AST as a content-addressed blob in the `cdz-kernel` CAS, register it under a name in
`store/*` (e.g. `doc/<pkg>`), and keep the doc *index/format* in the wasm reducer (a reducer-owned KV
on the log; query = the reducer walking its own state). No doc-publish/doc-query EFFECT FAMILY, no
inevolvable host doc logic — honors minimize-host / composition-in-wasm. Confirmed by both harness
owners: doc-publish is a COMPOSITION of existing mechanisms, not a new world-effect.

**The one real gap (v-agent-harness-host).** There is today NO reducer-facing blob-**WRITE** effect.
`store/*` only maps a name to a Hash the CAS *already* holds; nothing lets a reducer put NEW bytes
into the CAS at runtime (in the existing publish-consume E2E the blob is put by the host/test
out-of-band at build time). A reducer that GENERATES a doc-AST at runtime therefore cannot get its
bytes into the CAS. So I3 needs a small generic **`blob/put` (and likely `blob/get`) reducer-facing
effect family** — the CAS-write MECHANISM, NOT doc logic (owned by `v-agent-harness` for the
kernel family+codec + `v-agent-harness-host` for the executor wrapping `blob.rs`). It is a shared
dependency for docs-in-harness *and any reducer that produces content*. `fs/write` is the wrong path
(it touches the real filesystem, path-scoped, and rejects blob-ref payloads — the CAS is a different,
content-addressed store). Cedar: `blob/put` is authorized (writing shared CAS is world-touching);
`store/set` is already authz-gated by name prefix (SEC-F1) so `doc/` authority gates the index.

### 2.4 Doc-extraction = a COMPILER QUERY — DECIDED (operator)
The extraction mechanism (thread 1) is modeled as a **compiler query** — the same infrastructure the
compiler already exposes (`type-of`, `resolve`, the `cadenza-syntax` selector/`query.rs` surface that
`v-agent-harness-host` is wasm-compiling) — NOT a bolt-on separate pass. The doc query walks the
program (AST for the structural half; typed IR for the resolved half) and projects `doc-item`s. This
composes with the existing query surface and dovetails the resolved-type query (§4.2, I2).

### 2.5 First increment — DECIDED (operator): **doc-extraction → binary AST**
It is the foundation the other two threads consume. Per v-syntax's seam analysis (§4.2) it SPLITS:
the structural half lives in `cadenza-syntax`, the type-enrichment half in `rcdzc` — both framed as
compiler queries. Gate-able with a fold unit + corpus round-trip.

### 2.6 Guide content model — DECIDED (design-agent, VETOABLE): **unify guide on `cadenza-ast`**
Guide chapters ARE `cdzast` binary AST too (heads like `chapter`, `h2`, `p`, `note`, `runnable`),
NOT a distinct guide-only sexpr schema — one query surface spanning program docs + agent docs + the
guide. **v-guide's content survey confirms this is viable:** ~40 of 44 chapters are already a small
typed DSL (`H1/Lede/H2/P/C/Note` + `<Runnable>` + `<Exercise>` + `<Why>`) that round-trips cleanly;
the registry (`chapters.ts`) is plain data. **Carve-outs to design for** (from v-guide):
1. **Inline links** (`<Link to>`, `AppLink`/`Ch`): add a `(link <slug> "text")` node — standardizes
   what is free JSX today.
2. **Two bespoke widgets** — `ControlFlow`'s `<TryChange>` (interactive) and `Welcome`'s
   `<StatusIcon>`/`StatusLegend`: give each a schema node OR carve out those two chapters for later.
3. **App-route showcases** (`ExampleApps`, `Playground`, `CdzToolchain`, `WhatsNext` point at the
   playground/calculator/CAD/notebook ROUTES — heavy stateful React): the *prose* round-trips, but
   the apps themselves are OUT OF SCOPE for the doc model — explicitly carve them out, don't
   schematize live canvas/slider/compile widgets.
4. **`<Runnable>` stores authored source + `authoredIn` only**; codegen re-derives the sexpr↔ml
   toggle rather than freezing both forms.
Vetoable: if these carve-outs prove too large, fall back to a distinct guide-content sexpr schema
that shares only the reader. Current judgment: unify — the carve-out set is small and bounded.

---

## 3. Current ground truth (file/line anchors)

- **Doc comments already survive to canonical binary AST.** `cadenza-syntax/src/parser.rs`
  `take_docs_here()` (~parser.rs:640) drains leading `///` docs and splices them as `(doc "text")`
  body forms (~parser.rs:781); module-level docs become `(module-doc "text")` siblings
  (~parser.rs:699-726). `///` always LEADS its item. Lexer `Kind::DocComment`
  (`cadenza-syntax/src/lexer.rs:264-286`, `token.rs:19`). Printer round-trips them
  (`printer.rs`). **So thread (1) does NOT capture docs — the raw material is already in the wire
  format; the query PROJECTS it into the derived doc-item shape.** The canonical AST is STRUCTURAL and
  PRE-TYPECHECK: it carries syntactic type annotations where the source wrote them, but NO
  resolved/inferred types (v-syntax) — hence the §4.2 split.
- **The AST value model.** `cadenza-ast/src/ast.rs`: two flat arenas — `enum Leaf` (ast.rs:31,
  interned primitives incl. `Name(String)` at the construct head, and **`Bytes(Vec<u8>)`** the
  `b"…"` length-prefixed leaf) and `enum Struct` (ast.rs:149, **frozen at 2 variants**
  `Atom(LeafId)` / `List(Vec<StructId>)`). A construct = a `List` whose first child is an `Atom` of a
  `Name`. `struct Arenas` (ast.rs:180), `struct Builder` (ast.rs:190).
- **Language-surface `Ast` sum type**: `Ast.Int/Float/Str/Bool/Name/List/Bytes` (`rcdzc/src/quote.rs`,
  `eval_ast.rs`). Ordinary exhaustiveness-checked sum type.
- **Binary codec + dictionary.** `cadenza-ast/src/codec.rs` — canonical `cdzast\x00\x01` (total
  `decode`, bijection per `spec/contracts/ast-encoding.md`); it is **head-agnostic** (serializes/
  round-trips `Name` heads, never interprets them — v-metaprog). `cadenza-ast/src/dict.rs` — the
  `cdzast\x00\x02` DICTIONARY transport (content-hash subtree sharing; `DictSet::from_artifacts` /
  `resolve`, FLAT-only, hermetic). `canon.rs` canonicalizes. **v-metaprog owns the Ast + dict model
  layer; v-syntax owns the codec WIRE** (`hash_dag`/I2b is v-syntax's and NOT yet on trunk).
- **AST surfaces / round-trip.** `cadenza-syntax/src/convert.rs` (binary ↔ sexpr ↔ ML); `sexpr.rs`
  (round-trip oracle); `parser.rs`/`printer.rs` (ML); `query.rs` / selector (the query surface to
  compose with). `spec/capabilities/self-hosting-surface.md`.
- **The guide.** `guide/` — Vite + React + TS SPA. Chapters are **TSX modules**
  (`guide/src/content/chapters/*.tsx`, 44). Registry `guide/src/content/chapters.ts` (ordered,
  `lazy()`-loaded, pillars `"language"`/`"platform"`; each chapter `slug/title/blurb/pillar/section/
  exercises/Component`). Prose components `guide/src/components/Prose.tsx`. `<Runnable source={…}
  authoredIn expect wrap mode prelude/>`, `<Exercise id/prompt/starter/solution>`, `<Why>`. Build:
  `tsc -b && vite build`; wasm staging separate. Gate: `guide/scripts/check-*.mjs` —
  `check-examples.mjs` REGEX-extracts `source=`/`solution=`/`expected=` from `chapters/*.tsx`
  (~L391, readdir ~L447), with floors (<30 content files / <100 examples fails LOUD).
- **Harness store.** CAS blob store `cdz-kernel/src/blob.rs` (`put(bytes)→Hash`/`get(&Hash)`,
  self-verifying, `MemBlobStore`/`DiskBlobStore`). `store/*` family (`STORE_SET`/`resolve`) maps a
  name→hash (kernel-applied, authz-gated by name via SEC-F1) — but **maps only, does not PUT bytes**.
  `fs/*` (`FS_READ/WRITE/GLOB`, `cdz-agent-host/src/fs_exec.rs`) touches the real filesystem and
  rejects blob-ref payloads — NOT the CAS. No reducer-facing blob-write effect today (§2.3 gap).
- **Closest style templates:** `DESIGN-binary-ast-dictionary.md`, `DESIGN-record-update-syntax.md`.

---

## 4. The shape (decided)

### 4.1 The doc-AST model (`doc-module` / `doc-item`)
A doc document is a `doc-module` construct: a module name, optional `module-doc`, and a sequence of
`doc-item`s. Each `doc-item` carries STRUCTURED fields only (no rendered strings):
- `(name "…")` — the item's public identifier.
- `(doc "…")` — doc prose (from the leading `(doc …)` node(s)).
- `(sig <sub-ast>)` — the STRUCTURED signature sub-AST: the syntactic annotation as-written
  (structural pass), enriched with the resolved type post-typecheck (rcdzc pass). A consumer renders
  it via the ML printer.
- `(kind def|type|effect|module)`, `(visibility public|…)` — for filtering.
- byte payloads (if any) use the `b"…"` `Ast.Bytes` leaf (one length-prefixed node), never a
  node-per-byte list (v-metaprog).

Head-name constraints (v-metaprog): `doc-module`/`doc-item`/`sig`/`ty`/`kind`/`visibility`/`link` are
plain identifiers, none reader-reserved (only `quote`/`quasiquote`/`unquote`/`unquote-splicing` are).
Any doc-node example added to a `.sexp` corpus MUST pass the ML round-trip (the printer renders every
minted head; non-empty `Name`-head lists are fine).

It is `cdzast`: canonicalizes, encodes to `\x00\x01`, round-trips through sexpr/ML, matchable as
`Ast`. A `DocItem` Rust type is defined ONCE in `cadenza-ast` (shared) with the resolved-type field
OPTIONAL — populated by rcdzc, `None` in the syntax-only projection.

### 4.2 The doc query (structural in cadenza-syntax + type-enrich in rcdzc)
Per v-syntax's seam analysis — SPLIT by what each layer knows, both as compiler queries:
- **`cadenza-syntax` owns the STRUCTURAL query:** `doc_items(&Arenas) -> Vec<DocItem>` — an
  AST→AST projection that, per public def/type/effect, emits `{name, doc-text, syntactic-sig
  subtree, visibility, resolved-ty = None}`. Purely structural, no types needed; reusable by tools
  that don't typecheck (fast doc outline, IDE symbol list, syntax-only preview). Same kind of Arenas
  projection cadenza-syntax already does (json/markdown/cedar/toml).
- **`rcdzc` owns the TYPE-ENRICHMENT query:** post-typecheck, fill the resolved type (inferred
  return, resolved generic, effect row) into `(sig …)` — rcdzc is the only place that type exists.
- **Boundary case (v-syntax flag):** an un-annotated public def yields a `doc-item` with an EMPTY
  syntactic sig in the structural pass (so the item list is COMPLETE pre-typecheck); rcdzc fills the
  resolved type. (Chosen default — keep it in the structural pass.)

Exposed as a `cdz doc` / `cadenza doc` subcommand that runs the query and emits canonical `cdzast`
(displayable as sexpr/ML for inspection).

### 4.3 The harness doc-publish surface
Reducer-owned, host-mechanism-only, additive (§2.3):
- (NEW, generic) `blob/put(bytes) → Hash` reducer-facing effect → host does `blob.put` into the CAS,
  returns the content hash. (Plus `blob/get` for the consume side.) This is the missing CAS-write
  mechanism, owned by v-agent-harness (kernel family+codec) + v-agent-harness-host (executor).
- `store/set doc/<pkg> → Hash` (existing name register, authz-gated by name prefix).
- The **doc index** is a reducer-owned KV on the log; QUERY = the reducer folding/walking its own
  state (same shape as `control/summary` forking over reducer state). No kernel query effect.
- Cross-session doc discovery (one session querying another's index) is a later control-plane read
  (like `control/capabilities`) — deferred, flag v-agent-harness if it surfaces.
- The doc-AST blob is OPAQUE bytes to the kernel (it carries the hash, never parses the doc) — the
  doc schema lives entirely in the reducer/tooling (O4 opaque-bytes discipline).

### 4.4 The guide as sexpr + codegen
Guide chapters authored as `cdzast` sexprs (heads mirroring the typed DSL: `chapter`/`section`/`h2`/
`p`/`c`/`note`/`link`/`runnable`/`exercise`/`why`). A codegen step reads each `*.sexp` → emits
`chapters/*.tsx`. Infra constraints (v-guide-infra):
- Codegen runs BEFORE `tsc -b` (prepend `npm run codegen` or a Vite pre-step), then tsc typechecks
  the generated TSX.
- Generated TSX must be tsc-clean under the app config: `noUnusedLocals` is ON, so NO unused imports
  (TS6133 hard-fail) — verify with `tsc -b`, not `tsc --noEmit`. Emit a `// @generated DO NOT EDIT`
  header.
- Keep generated file paths/slugs stable so `chapters.ts` `lazy(import("./chapters/X.tsx"))` resolves
  — OR codegen `chapters.ts` too (registry derived from the sexp set = zero drift; preferred
  long-term).
- A `<Runnable>` is modeled `(runnable (surface sexpr) (source "…program…") (expected "…"))`; codegen
  lowers `(source "…")` → `source={`…`}` and `(expected "…")` → `expected="…"` 1:1 with today's props
  so `check-examples.mjs`'s regex extractor still matches (else a chapter silently extracts 0
  examples — but the <30-file/<100-example floors fail LOUD). Keep `expected` on the sexpr surface
  (guard #1123 asserts it only on the sexpr pass); `authoredIn="ml"` supported if needed.
- Editorial invariant tests (`arc.test`/`opener`/`tenets`/`links`/`proseEmDash`) must run against the
  sexpr form (or the generated TSX) or they go stale.

Pilot chapter (v-guide-infra): **`PlatformOverview`** — ~100 lines, ZERO Runnable/Exercise, 1 import,
pure prose — validates the codegen pipeline + registry wiring WITHOUT the Runnable-embedding problem;
add Runnable in a 2nd increment. Avoid Basics/Numbers/Floats/Data for the pilot (max component
surface).

---

## 5. Increments (top-to-bottom, the way a vertical lands them)

**I1 — doc-AST model + structural doc query (foundation).** Owner: **v-syntax** (+ v-metaprog review
of the head-name/Ast surface). area=`cadenza-syntax`/`cadenza-ast`. Define the shared `DocItem` type
+ the `doc-module`/`doc-item` head-name construct in `cadenza-ast`; implement `doc_items(&Arenas)`
structural query in cadenza-syntax (name + doc-text + syntactic-sig subtree + visibility, resolved-ty
None); wire a `cdz doc` subcommand emitting canonical `cdzast`. **Gate:** a fold unit (program →
expected doc-AST); a corpus round-trip (doc-AST → `\x00\x01` → decode identical + sexpr/ML re-read);
`cargo test -p rcdzc --lib` + `cargo xtask gate` additive-only.

**I2 — resolved-type enrichment query.** Owner: v-syntax + v-inference. area=`rcdzc`. Post-typecheck
query that fills the resolved type into `(sig …)`. **Gate:** a unit where an inferred type appears in
the enriched `doc-item`; round-trip unchanged.

**I3 — harness doc-publish (blob/put mechanism + reducer-owned index).** Owner: **v-agent-harness**
(kernel `blob/put`(+`blob/get`) family + codec) + **v-agent-harness-host** (executor wrapping
`blob.rs`). area=`agent-harness`. Then a reducer-owned doc index (publish = `blob/put` + `store/set`;
query = reducer walks its KV). **Gate:** a reducer E2E — reducer generates a doc-AST at runtime,
`blob/put`s it, `store/set`s `doc/<pkg>`, queries it back by name; Cedar-gate check; no new host doc
logic. NOTE: the `blob/put` family is a SHARED dependency (any content-producing reducer needs it) —
sequence it first within I3.

**I4 — guide sexpr schema + codegen (PlatformOverview pilot).** Owner: v-guide-infra (codegen/build/
registry/gate) + v-guide (content). area=`guide`. Define the guide `cdzast` head schema incl. the
`link` carve-out node; convert `PlatformOverview` to `*.sexp`; codegen `sexp → *.tsx` (before tsc,
`@generated` header, tsc-clean) feeding `chapters.ts`. **Gate:** codegen output renders identically to
the hand-written TSX; `check-*.mjs` green (extractor floors intact); pilot has no Runnable so the
Runnable-embed contract is validated in I5.

**I5 — Runnable embedding + guide bulk conversion + in-harness query.** Owner: v-guide-infra +
v-guide/v-guide-editor. area=`guide`. Add the `(runnable …)`/`(exercise …)` lowering (matching the
check-examples extractor); convert remaining chapters (widget carve-outs per §2.6); publish the guide
doc-set into the harness (I3 surface). **Gate:** full guide `check-*.mjs` green from codegen; a
harness query returns a guide chapter; editorial invariant tests run against the sexpr form.

Each increment is independently gate-green and a MEANINGFUL merge-request.

---

## 6. Seams / file anchors (where each increment cuts)

- I1: `cadenza-ast/src/ast.rs` (new head *Names* only — NO new `Struct`/`Leaf` variant) + a shared
  `DocItem` type in `cadenza-ast`; `cadenza-syntax` new `doc_items` projection (reuse the doc-node
  preservation from `parser.rs` `take_docs_here` + the `query.rs`/projection machinery); `rcdzc` CLI
  `cdz doc`. Round-trip via `convert.rs` + `codec.rs`.
- I2: `rcdzc` post-typecheck hand-off — fill resolved type; coordinate v-inference for the resolved
  type→sub-AST reification.
- I3: `cdz-kernel` new `blob/*` effect family + codec (v-agent-harness) wrapping `blob.rs`;
  `cdz-agent-host` `blob/put` executor (v-agent-harness-host, mirrors existing executors); reducer
  doc-index in the agent-harness reducer; `store/*` reuse. Cedar policy for `blob/put` + `doc/`
  prefix. Do NOT use `fs/write` (wrong store).
- I4/I5: `guide/src/content/chapters/*.sexp` (new source-of-truth); a `guide/scripts/` codegen step →
  `chapters/*.tsx` (+ optionally `chapters.ts`); the check-examples extractor contract. Do NOT alter
  `Prose.tsx` component contracts casually — codegen targets them.

---

## 7. The gate (what protects it)

1. `cargo test -p rcdzc --lib` — 0 failed; each increment adds its unit (structural fold; round-trip;
   post-typecheck resolved-type; reducer publish/query E2E).
2. `cargo xtask gate` — diff the FAIL SET vs baseline, additive only. A doc-AST round-trip corpus case
   (encode → `\x00\x01` → decode identical; sexpr/ML re-read identical) — same bijection as
   `spec/contracts/ast-encoding.md`. Any `.sexp` doc example must pass the ML round-trip.
3. `cargo xtask check` — fmt + clippy `-D warnings` + `codegen --check` clean.
4. Guide: `guide/scripts/check-*.mjs` green from codegen output (extractor floors intact); codegen
   deterministic (same sexpr → byte-identical TSX); generated TSX tsc-clean under `tsc -b`.
5. Do NOT edit `cdz-runtime` `//` comments or `wit/runtime.wit` (frozen `REQUIRED_RUNTIME_HASH`).

---

## 8. Deferred extensions (with a chosen default recorded)

- **Cross-reference / link resolution** (rustdoc-style `[Type]` links across doc-items): DEFER —
  v1 emits flat `doc-item`s; linking is a later fold over the doc-set. (The guide `link` node is
  separate — it's guide content, not program-doc cross-refs.)
- **Doc rendering to HTML/markdown** (`cargo doc`-style site): DEFER — the binary doc-AST is the
  primitive; a renderer is a downstream CONSUMER (per the operator's rendering-is-the-consumer's-job
  ruling). The guide codegen is the first such consumer.
- **Cross-session harness doc discovery:** DEFER — a control-plane read like `control/capabilities`;
  within a session, query is pure reducer state.
- **Multi-module dictionary transport (`\x00\x02`):** DEFER — canonical inline per module is the v1
  default (§9.2).
- **Private/internal docs:** default = extract public surface only; `(visibility …)` lets a later
  pass include internals.

---

## 9. Open decisions (resolved by stakeholder input; defaults recorded)

### 9.1 Extraction timing — RESOLVED (v-syntax): **SPLIT**
Structural query in cadenza-syntax (no types); resolved-type enrichment in rcdzc post-typecheck.
Shared `DocItem` with optional resolved-ty. Un-annotated defs kept in the structural pass with an
empty syntactic sig. (§4.2)

### 9.2 Multi-module transport — RESOLVED (v-metaprog): **canonical inline per module (v1)**
The `\x00\x02` dictionary transport is NON-CANONICAL (operator option A) and a compaction-only
optimization that costs canonical identity of the aggregate. Default canonical inline per module.
Reach for dict transport only if a large multi-module set shows measured shared-subtree redundancy;
if a stable aggregate content-address is wanted, use `hash_dag` (v-syntax's I2b, NOT yet on trunk =
a dependency to sequence).

### 9.3 Harness publish surface — RESOLVED (v-ah + v-ah-host): **reuse mechanisms + add generic `blob/put`**
No doc effect family, reducer-owned index. But add the missing generic reducer-facing `blob/put`
(+`blob/get`) CAS-write mechanism (§2.3/§4.3/I3). (§4.3)

### 9.4 Signature representation — RESOLVED (operator + v-syntax): **structured sub-AST is truth; consumer renders**
Emit the structured signature sub-AST only; NO pre-printed string in the doc-AST. A display string is
`printer(sig)`, computed by the consumer. (§2.1)

### 9.5 Guide content model — RESOLVED (design-agent + v-guide survey): **unify on `cadenza-ast`, with carve-outs**
Unify; carve out the two bespoke widgets (or schematize) + the app-route showcases (out of scope);
add a `link` node. Vetoable if the carve-out set grows. (§2.6)

### 9.6 Guide pilot chapter — RESOLVED (v-guide-infra): **`PlatformOverview`** (§4.4)

---

## 10. Hand-off

- I1 (foundation) → **v-syntax** (ASSIGNED by corpus-bugfix; + v-metaprog review). I2 → v-syntax +
  v-inference. I3 → v-agent-harness (blob/put family) + v-agent-harness-host (executor). I4/I5 →
  v-guide-infra + v-guide/v-guide-editor.
- PM (`corpus-bugfix`) routed to engaged verticals (no duplicate mint); I2–I5 staged-route on I1 land.
  concierge `ask` out to ratify the split-vs-dedicated-vertical (proceeding on split default).
- Reviewers offered: v-metaprog (binary-AST section), v-syntax (structural half), v-guide-infra
  (codegen vs gate extractor), v-guide (content correctness). Ping them per increment.
- Proposal summary + delegation plan sent to concierge for operator surfacing on return.
