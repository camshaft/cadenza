# Design — Cadenza documentation publishing (docs → binary AST, docs-in-harness, guide-as-sexpr)

**Author:** design-cadenza-docs (autonomous — operator away ~2 weeks, no interactive sessions)
**Audience:** `v-metaprogramming`, `v-syntax`, `v-guide` / `v-guide-infra` / `v-guide-editor`, `v-agent-harness` / `v-agent-harness-host`, `corpus-bugfix` (PM)
**Status:** PROPOSAL — 4 load-bearing forks DECIDED by operator before they went dark; 1 fork decided by design-agent (vetoable); stakeholder input requested (see §9). Ready to route increments.

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

Three coupled threads, one unifying insight:

1. **Doc-extraction → binary AST.** A `cadenza doc`-like pass that projects a program's public
   surface (doc comments + names + signatures + resolved types) into a **derived binary AST** — a
   rustdoc-like structured index that IS a `cdzast` value, so every existing AST surface
   (`cadenza-syntax` reader/printer, ML/sexpr, metaprog `Ast`, the codec/dictionary) applies to it
   uniformly.
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

### 2.1 Doc-AST shape — DECIDED (operator): **derived doc-item projection**
Not a view over the raw program AST, and not a standalone non-`cdzast` schema. `cadenza doc` emits a
NEW binary-AST construct built from the same `cadenza-ast` arenas — new head *names*
(`doc-module`, `doc-item`, `sig`, `ty`), never a new `Struct` variant (keywords-are-data,
per the frozen 2-variant `Struct`). Rationale: reuses the arena model + codec + every AST surface;
honors the standing binary-AST-reuse order; a doc index is itself matchable/queryable as `Ast`.

```
(doc-module "mymod"
  (module-doc "Top-of-module prose.")
  (doc-item
    (name "map")
    (sig "(a -> b) -> List a -> List b")          ; printed display string
    (doc  "Applies f to each element.")
    (ty   (-> (-> a b) (List a) (List b))))        ; resolved type sub-AST
  (doc-item …))
```

### 2.2 Guide direction — DECIDED (operator): **codegen TSX from sexprs**
Author guide content as sexprs; a build step emits the TSX chapter modules. The existing React site,
`<Runnable>` live-compile, `<Exercise>`/`<Why>`, and the `check-*.mjs` gate stay intact. Sexpr
becomes source-of-truth; TSX is generated. Rejected the runtime-pull alternative: it would need a
new runtime sexpr→React renderer and rework how `<Runnable>`/exercises embed — more surface, no gate
reuse.

### 2.3 Harness publish surface — DECIDED (operator): **reuse `fs/*` + CAS blob, host mechanism-only**
Publish a doc-AST as a content-addressed blob via the existing `cdz-kernel` CAS (`blob.put → Hash`),
then register it under a name (`name_store`, e.g. `doc/<pkg>`). The doc *index/format* lives in the
wasm reducer, not the host. No new inevolvable host doc logic — honors minimize-host /
composition-in-wasm. (Seam care: `fs_exec` rejects blob-ref payloads on `fs/write`, so the
doc-publish path goes through the blob + name effect path, not `fs/write`. Confirming exact effect
path with `v-agent-harness-host` — see §9.)

### 2.4 First increment — DECIDED (operator): **doc-extraction → binary AST**
It is the foundation the other two threads consume (harness publishes it; the guide is a sibling doc
in the same model). Purely in `rcdzc` / `cadenza-syntax`, gate-able with a fold unit + corpus
round-trip.

### 2.5 Guide content model — DECIDED (design-agent, VETOABLE): **unify guide on `cadenza-ast`**
Guide chapters ARE `cdzast` binary AST too (heads like `chapter`, `h2`, `p`, `runnable`, `note`),
NOT a distinct guide-only sexpr schema. Rationale: the operator explicitly wants "documentation
around the language and platform in the harness … queryable by agents" — a single doc model spanning
program docs + agent docs + the guide gives ONE query surface and reuses one codec, which is the
whole point of the binary-AST framing. A distinct schema would fork the query path in two. Recorded
as vetoable: if guide content proves too React-shaped to model cleanly in `cdzast` heads (see the
`v-guide` hard-cases question in §9), fall back to a distinct guide-content sexpr schema that shares
only the reader.

---

## 3. Current ground truth (file/line anchors)

- **Doc comments already survive to canonical binary AST.** `cadenza-syntax/src/parser.rs`
  `take_docs_here()` (~parser.rs:640) drains leading `///` docs and splices them as `(doc "text")`
  body forms (~parser.rs:781); module-level docs become `(module-doc "text")` siblings
  (~parser.rs:699-726). `///` always LEADS its item. Lexer `Kind::DocComment`
  (`cadenza-syntax/src/lexer.rs:264-286`, `token.rs:19`). Printer round-trips them
  (`printer.rs`). **So thread (1) does NOT need to capture docs — the raw material is already in the
  wire format; it needs to PROJECT it into the derived doc-item shape.**
- **The AST value model.** `cadenza-ast/src/ast.rs`: two flat arenas — `enum Leaf` (ast.rs:31,
  interned primitives incl. `Name(String)` at the construct head) and `enum Struct` (ast.rs:149,
  **frozen at 2 variants** `Atom(LeafId)` / `List(Vec<StructId>)`). A construct = a `List` whose
  first child is an `Atom` of a `Name`. `struct Arenas` (ast.rs:180), `struct Builder` interns
  leaves (ast.rs:190).
- **Language-surface `Ast` sum type** (matched by user code): `Ast.Int/Float/Str/Bool/Name/List/Bytes`
  (`rcdzc/src/quote.rs`, `eval_ast.rs`). An ordinary exhaustiveness-checked sum type.
- **Binary codec + dictionary.** `cadenza-ast/src/codec.rs` — canonical `cdzast\x00\x01` (hand-rolled,
  total `decode`, bijection per `spec/contracts/ast-encoding.md`). `cadenza-ast/src/dict.rs` — the
  `cdzast\x00\x02` DICTIONARY transport (content-hash subtree sharing, `Hash([u8;32])`, reachable
  via `codec::decode_with_dicts`). `canon.rs` canonicalizes.
- **AST surfaces / round-trip.** `cadenza-syntax/src/convert.rs` (binary ↔ sexpr ↔ ML, all
  projections of one `Arenas`); `sexpr.rs` (sexpr reader = corpus round-trip oracle); `parser.rs` /
  `printer.rs` (ML). `spec/capabilities/self-hosting-surface.md`.
- **The guide.** `guide/` — Vite + React + TS SPA. Chapters are **TSX modules**
  (`guide/src/content/chapters/*.tsx`, ~40). Registry `guide/src/content/chapters.ts` (ordered;
  `lazy()`-loaded; two pillars `"language"` / `"platform"`; each chapter has `slug/title/blurb/
  pillar/section/exercises/Component`). Prose components `guide/src/components/Prose.tsx`
  (`H1,Lede,H2,P,C,Note`) + `<Runnable source={…}/>` (wasm live-compile), `<Exercise>`, `<Why>`.
  Build: `guide/package.json` → `tsc -b && vite build`, plus `wasm` staging. Gate:
  `guide/scripts/check-*.mjs`.
- **Harness store.** CAS blob store in `cdz-kernel/src/blob.rs` (`put(bytes)→Hash` / `get(&Hash)`,
  self-verifying, `MemBlobStore`/`DiskBlobStore`). `name_store.rs`, `log_store.rs`,
  `component_store.rs`. `fs/*` family (`FS_READ/FS_WRITE/FS_GLOB`) executed in
  `cdz-agent-host/src/fs_exec.rs`, Cedar-gated; `fs_exec` deliberately rejects blob-ref payloads on
  `fs/write` (inline bytes only). No dedicated doc store today.
- **Closest style templates:** `DESIGN-binary-ast-dictionary.md`, `DESIGN-record-update-syntax.md`.

---

## 4. The shape (decided)

### 4.1 The doc-AST model (`doc-module` / `doc-item`)
A doc document is a `doc-module` construct: a module name, an optional `module-doc`, and a sequence
of `doc-item`s. Each `doc-item` carries:
- `(name "…")` — the item's public identifier.
- `(sig "…")` — a printed display signature (via the ML printer) for human/agent rendering.
- `(doc "…")` — the doc prose (from the item's leading `///` nodes, concatenated).
- `(ty <sub-ast>)` — the resolved type as a `cdzast` sub-AST (present when extraction runs
  post-typecheck; omitted if extraction is purely syntactic — see §9.1).
- optional `(kind def|type|effect|module)`, `(visibility public|…)` for filtering.

It is `cdzast`. It canonicalizes, encodes to `\x00\x01`, round-trips through sexpr/ML, and is
matchable as `Ast` by metaprograms. A multi-module doc set MAY use the `\x00\x02` dictionary
transport to share repeated type subtrees (pending `v-metaprogramming`, §9.2).

### 4.2 The `cadenza doc` pass
Program → doc-AST. A fold over the program's `Arenas` that, for each public def/type/effect: reads
its leading `(doc …)` node(s), its head `(name …)`, prints its signature, and — post-typecheck —
attaches the resolved `(ty …)`. Output is a `doc-module` per module, written as canonical `cdzast`
bytes (and displayable as sexpr/ML for inspection). Exposed as a `cadenza doc` / `cdz doc`
subcommand.

### 4.3 The harness doc-publish surface
Reducer-owned, host-mechanism-only:
- `blob.put(doc_ast_bytes) → Hash` (existing CAS).
- register `doc/<pkg>` → `Hash` in the name store (existing).
- The **doc index** (what's published, by whom, queryable) is a reducer-owned structure on the log —
  NOT new host logic. Query = the reducer walking its own state. No new inevolvable host doc family
  unless `v-agent-harness-host` says an effect is required for the blob+name write (§9.3).

### 4.4 The guide as sexpr + codegen
Guide chapters authored as `cdzast` sexprs (heads `chapter`/`section`/`h2`/`p`/`c`/`note`/`runnable`/
`exercise`/`why`, mirroring `Prose.tsx` + the interactive components). A codegen step reads each
`*.sexp` → emits the corresponding `chapters/*.tsx` consumed by the existing `chapters.ts` registry.
`<Runnable source={…}>` blocks embed as `(runnable "…source…")`. The React site, wasm live-compile,
and `check-*.mjs` gate are unchanged — only the *source of truth* moves to sexpr, and the same sexpr
is a queryable in-harness doc.

---

## 5. Increments (top-to-bottom, the way a vertical lands them)

**I1 — doc-AST model + `cadenza doc` extraction (foundation).** Owner: `v-syntax` (+ `v-metaprogramming`
review for the head-name/codec surface). area=`cadenza-syntax`/`rcdzc`. Define the `doc-module`/
`doc-item` head-name construct; implement the projection fold (program `Arenas` → doc-AST); wire a
`cadenza doc` subcommand emitting canonical `cdzast`. **Gate:** a fold unit test (program → expected
doc-AST); a corpus round-trip (doc-AST encodes → `\x00\x01` → decodes identically, and prints to
sexpr/ML and re-reads); `cargo test -p rcdzc --lib` + `cargo xtask gate` additive-only.

**I2 — resolved-type enrichment.** Owner: `v-syntax` + `v-inference`. area=`rcdzc`. Run extraction
post-typecheck so `(ty …)` carries the resolved type sub-AST (not just the printed `sig`). **Gate:**
a unit where an inferred type appears in `(ty …)`; corpus round-trip unchanged.

**I3 — harness doc-publish (reducer-owned index).** Owner: `v-agent-harness` (kernel/reducer) +
`v-agent-harness-host` (host mechanism). area=`agent-harness`. `blob.put` + `name_store` register of a
doc-AST; a reducer-owned doc index on the log; a doc-query the reducer answers from its own state.
**Gate:** a reducer E2E — publish a doc-AST, query it back by name; Cedar-gate check; no new host
logic beyond blob/name mechanism.

**I4 — guide sexpr schema + codegen (pilot chapter).** Owner: `v-guide-infra` (codegen/build) +
`v-guide` (content). area=`guide`. Define the guide `cdzast` head schema; convert ONE pilot chapter
(minimal custom components) to `*.sexp`; a `sexp → *.tsx` codegen step feeding `chapters.ts`. **Gate:**
codegen output renders identically to the hand-written TSX; `check-*.mjs` green; the pilot chapter's
`<Runnable>` still live-compiles.

**I5 — guide bulk conversion + in-harness query.** Owner: `v-guide-infra` + `v-guide-editor`.
area=`guide`. Convert remaining chapters to sexpr; publish the guide doc-set into the harness (I3
surface) so agents query language/platform docs. **Gate:** full guide `check-*.mjs` green from
codegen; a harness query returns a guide chapter.

Each increment is independently gate-green and a MEANINGFUL merge-request (no per-line drips).

---

## 6. Seams / file anchors (where each increment cuts)

- I1: `cadenza-ast/src/ast.rs` (new head *names* only — do NOT add a `Struct`/`Leaf` variant),
  `cadenza-syntax/src/parser.rs` (reuse `take_docs_here` output), a new projection module in
  `rcdzc/src/` (doc fold), `rcdzc` CLI (`cdz doc` subcommand). Round-trip via
  `cadenza-syntax/src/convert.rs` + `codec.rs`.
- I2: `rcdzc` typecheck/resolve hand-off — attach resolved type after inference; coordinate
  `v-inference` for the resolved-type sub-AST shape.
- I3: `cdz-kernel/src/blob.rs` + `name_store.rs` (reuse, no new store), a reducer doc-index in the
  agent-harness reducer; `cdz-agent-host` only if a blob+name effect path is needed (NOT `fs/write` —
  it rejects blob refs). Cedar policy for doc-publish/doc-get.
- I4/I5: `guide/src/content/chapters/*.sexp` (new source-of-truth), a `guide/scripts/` codegen step
  → `chapters/*.tsx`, `guide/src/content/chapters.ts` registry. Do NOT alter `Prose.tsx` component
  contracts casually — the codegen targets them.

---

## 7. The gate (what protects it)

1. `cargo test -p rcdzc --lib` — 0 failed; each increment adds its unit (fold unit; round-trip;
   post-typecheck `(ty …)` unit; reducer publish/query E2E).
2. `cargo xtask gate` — diff the FAIL SET vs baseline, additive only. A doc-AST round-trip case in
   the corpus (encode → `\x00\x01` → decode identical; sexpr/ML re-read identical) — the same
   bijection guarantee as `spec/contracts/ast-encoding.md`.
3. `cargo xtask check` — fmt + clippy `-D warnings` + `codegen --check` clean.
4. Guide: `guide/scripts/check-*.mjs` green from codegen output; codegen is deterministic (same sexpr
   → byte-identical TSX).
5. Do NOT edit `cdz-runtime` `//` comments or `wit/runtime.wit` (frozen `REQUIRED_RUNTIME_HASH`).

---

## 8. Deferred extensions (with a chosen default recorded)

- **Cross-reference / link resolution** (rustdoc-style `[Type]` links across items): DEFER — v1 emits
  flat `doc-item`s; linking is a later fold over the doc-set.
- **Doc rendering to HTML/markdown site** (a `cadenza doc` HTML output like `cargo doc`): DEFER — the
  binary doc-AST is the primitive; a renderer is a downstream consumer (the guide codegen is the
  first such renderer).
- **Private/internal docs & visibility filters:** default = extract public surface only; carry
  `(visibility …)` so a later pass can include internals.

---

## 9. Open decisions (each with a chosen default — override only with operator/stakeholder sign-off)

### 9.1 Extraction timing: syntactic vs post-typecheck (asked `v-syntax`)
Default: **post-typecheck in `rcdzc`** so `(ty …)` carries resolved types; `(sig …)` printed string
present regardless. A purely-syntactic I1 (no `(ty …)`) is acceptable as a first cut with I2 adding
resolved types. Pending `v-syntax` seam preference.

### 9.2 Multi-module transport: canonical inline vs `\x00\x02` dictionary (asked `v-metaprogramming`)
Default: **canonical inline per module** for v1 (simplest, one codec path); adopt the dictionary
transport only if doc-sets prove large with heavy type-subtree repetition. Pending
`v-metaprogramming`.

### 9.3 Harness publish: pure blob+name vs a doc effect (asked `v-agent-harness-host` + `v-agent-harness`)
Default: **reducer-owned index via existing blob+name, no new host family** (minimize-host). Adopt a
`doc/*` effect only if the host owners say the blob+name write needs an effect path. Pending both.

### 9.4 Guide content model: unify on `cadenza-ast` vs distinct guide schema (design-agent call, asked `v-guide`)
Default (design-agent, VETOABLE): **unify on `cadenza-ast`** for one query surface. Fall back to a
distinct guide-content sexpr schema only if `v-guide` reports chapters too React-shaped to model in
`cdzast` heads. Pending `v-guide` hard-cases list.

### 9.5 Guide pilot chapter (asked `v-guide-infra`)
Default: pick a prose-heavy chapter with minimal bespoke components (e.g. a `platform` overview or a
short language chapter). Pending `v-guide-infra` recommendation.

---

## 10. Hand-off

- I1 (foundation) → route to `v-syntax` (with `v-metaprogramming` review). I2 → `v-syntax` +
  `v-inference`. I3 → `v-agent-harness` + `v-agent-harness-host`. I4/I5 → `v-guide-infra` +
  `v-guide`/`v-guide-editor`.
- The PM (`corpus-bugfix`) mints/assigns the owning verticals; a vertical-ready brief lands in
  `.claude/fleet/queue/design-cadenza-docs.md`.
- Proposal summary + delegation plan sent to `concierge` for operator surfacing on return.
- Open decisions §9.1–9.5 are defaulted-and-proceeding; stakeholder replies refine them without
  blocking I1.
