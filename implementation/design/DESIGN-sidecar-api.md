# Design — the compiler sidecar API: driving compilation as a program over pure column reads

**Author:** compiler front-end. **Audience:** whoever builds the program that *drives* the compiler —
selecting which artifacts to materialize, querying static facts, and applying validated rewrites.
**Status:** **RUNG 2 LANDED (a first slice); rungs 3–4 are design.** The interim-Rust-host slice of §7
is realized: a `sidecar` kinded INPUT artifact carrying a request list, decoded in
`compile()`, with `Emit`/`Query(TypeOf|UsesOf)` requests running over the shared columns
(`rcdzc/src/sidecar.rs`; wired at `rcdzc/src/compile.rs`). It builds directly on machinery that is real
*today*: the kinded-artifact `compile(inputs, targets)` entry (`rcdzc/src/compile.rs`,
`rcdzc/src/abi.rs`), the columns model (`spec/architecture/query-engine.md`), and the query/rewrite
vocabulary sketched in [`DESIGN-query-engine.md`](./DESIGN-query-engine.md). The branching `drive`
program (rung 3) and the effect sugar (rung 4) remain design. Line references are landmarks, not
promises they won't drift.

This doc resolves the open question left by `DESIGN-query-engine.md` and the debug-info design: **how
declarative should the sidecar be, and do we need effects to make it work.** The short answers, argued
below: *both declarative and imperative are the same interface at different amounts of body*, and *no —
effects are unnecessary because the compiler's state is already a set of pure, total column reads.*

---

## 1. TL;DR — one insight, one signature, one non-requirement

**The insight (why this is easy now).** `spec/architecture/query-engine.md` fixes that the compiler's
whole state is a set of **columns keyed by node identity**, and that *every* static fact — a node's
type, a name's resolution, the effect row, where a symbol is used, **and the emitted artifact itself**
— is a **pure, total, deterministic read** of a column ("A Static Fact Is A Column Read"; "The Artifact
Is The Terminal Column"). "Give me the component bytes" and "give me the type of this node" are the
*same operation* over different columns.

**The signature.** A sidecar is one pure function:

```
drive : Ast -> List Request
```

where each `Request` is either *materialize an output column* (`Emit`), *read a fact column* (`Query`),
or *apply a validated edit* (`Rewrite`). A fully declarative sidecar returns a constant list; a powerful
one branches on query results before deciding what to return. **Same type, both ends** — a constant list
is the degenerate case of a program, exactly as "lower to a component" is the degenerate sidecar in
`DESIGN-query-engine.md`.

**The non-requirement.** Because facts are pure reads, `if this-expr has this-type then …` is ordinary
`match type-of(n) { … }` — **branching needs no effects and no mutable state.** Effects would buy
ergonomics, not power; a mutable "place" would be strictly worse (it breaks the two invariants that make
the model sound: *a query retains no cache* and *incremental = batch by construction*). So the answer to
"do we need effects" is **no**, and that is the feature, not a limitation.

---

## 2. The starting point already built — `targets` is the "Emit half"

`compile()` today is:

```rust
pub fn compile(inputs: &[Artifact], targets: &[Target]) -> CompileOutput   // compile.rs:33
```

- `inputs` = kinded artifacts in (`ast` today; `spans`, cache, dependencies are the open set —
  build-tool-interface.md §The Tool's Inputs Are A Kinded Artifact List).
- `targets: &[Target]` (`Wasm` → `component`, `Rust` → `rust`; `backend/mod.rs:28`) = *which output
  columns to materialize.*
- `CompileOutput { artifacts, diagnostics }` = artifacts-out + always-live diagnostics
  (`abi.rs:89`).

**`targets` is already the `Emit` half of the Request model — a flat, static, no-branching, no-query
list of "materialize these columns."** The sidecar API is the single generalization of that one
parameter: replace the fixed `&[Target]` with *a program that returns a list of requests*, where `Emit`
is today's `Target`, and `Query` / `Rewrite` are the two new request kinds. Everything else — the
kinded-input list, the `{artifacts, diagnostics}` return, the decline-don't-miscompile fault path —
stays exactly as it is. This is the concrete realization of `DESIGN-query-engine.md`'s
`engine(target, sidecar, output)` framing, pinned to the entry that exists.

---

## 3. The request vocabulary

One sum, three arms. (ML surface; the real types are ordinary Cadenza sums once self-host lands — §7.)

```
Request =
  | Emit    ArtifactKind EmitOpts     -- materialize a terminal column
  | Query   Q                          -- read a fact column
  | Rewrite (Ast -> Option Ast)        -- a validated transaction (§5)

ArtifactKind = Component | Rust | Dwarf | SourceMap | Manifest | TypeInfo | ...   -- open set

Q =
  | TypeOf     NodeId    -- the solved-type column
  | ResolveOf  NodeId    -- name resolution (which binder / prelude entry)
  | EffectRow  NodeId    -- the inferred manifest/effect row at a point
  | ScopeAt    NodeId    -- visible bindings + their types  ("variable scope tracking")
  | UsesOf     Symbol    -- reverse resolution: every occurrence that resolves to this symbol
                         --   ("all the locations a function or type is used")
  | SizeOf     ArtifactKind EmitOpts   -- a fact ABOUT a terminal column (see §4, case 3)
```

Each `Q` is a read of a column the compiler already fills (or a trivial derivation over one — `UsesOf`
is the transpose of the resolution column; `ScopeAt` reads the binder columns visible at a node). The
oracle contract (`tooling-and-lsp.md` §An Agent Queries The Compiler For Any Static Fact) *already
obliges* every one of these to be total, deterministic, and equal-to-full-compile — so the sidecar API
is the surface that *exposes* an obligation the compiler already carries, not new analysis.

**Each of the user's motivating cases lands as a request:**

| The ask | Sidecar |
|---|---|
| a type query | `[Query (TypeOf n)]` |
| full wasm + separate DWARF | `[Emit Component _, Emit Dwarf _]` |
| variable scope tracking | `[Query (ScopeAt n)]` |
| everywhere a fn/type is used | `[Query (UsesOf "foo")]` |

Note every one is a **constant list** — no branching. That is the whole "declarative end": the four
examples the user gave are pure manifests. Branching is only ever for *policy* — conditional rewrites,
lint-with-autofix, "emit debug only if the module declares an effect," "codemod only where the type is
`T`."

---

## 4. Why branching needs no effects — three cases, all pure

The tempting place for effects is *reacting to a result mid-run*. Every sub-case stays pure because the
oracle is a pure read available **inside** the sidecar, not only at the engine boundary.

**Case 1 — branch on a static fact (the common case).**
```
fn drive(ast) =
  concat-map(defs-of(ast), fn(d) =>
    match type-of(d) {                    -- pure column read
      Fn(_, row) => if effectful(row)     -- ordinary control flow over the value
                    then [Rewrite (wrap-in-logging d)]
                    else [],
      _ => [] })
```
`type-of` is pure ⇒ `match`/`if` over it is pure. This is the entire "if this expr has this type then do
X" surface, with zero effects.

**Case 2 — branch on the validity of a tree the sidecar just built.** "Apply this rewrite only if the
result still type-checks." Because the checker is a pure query, the sidecar can ask it about a
*candidate* tree it constructed:
```
let candidate = wrap-in-logging(d) in
if well-typed?(candidate)               -- pure: run inference on the candidate, read the type column
then [Rewrite (const candidate)]
else [Query (diagnostics-of candidate)]
```
The engine's re-parse-and-recheck of an accepted rewrite (§5) then becomes a *final safety net*, not the
only place validity is knowable.

**Case 3 — branch on an output artifact.** "Compile; if the component exceeds N bytes, emit differently."
This is the one case a single flat list can't express — but it is *still pure*, because "the artifact is
the terminal column" and "any fact is a column read": `SizeOf Component opts` is a legitimate pure query
that drives the backend producer and reads back a fact about its column.
```
if size-of(Component, default) > threshold
then [Emit Component minimal-opts]
else [Emit Component default]
```
Bounded exactly as the evaluator bounds its own reduction (decline-not-diverge,
`reference-compiler.md`): a sidecar that loops compiling-and-inspecting *declines* rather than hangs.

**The only thing genuinely outside pure column reads is real I/O** — reading other source files, a
workspace-wide index, an on-disk incremental cache. And the frozen contract *forbids* the compiler from
importing host functions to do that on purpose: "its host-import world is empty; its derivation is a pure
function of its input" (build-tool-interface.md §The Compiler Imports No Host Function). Those inputs
are supposed to arrive as **kinded input artifacts** — which is precisely the multi-unit / cache /
dependency motivation of the kinded-artifact-list reshaping
(`spec/learnings/2026-07-07-the-build-tool-interface-is-a-kinded-artifact-list-not-a-two-arm-result.md`).
So "effects for I/O" isn't merely unnecessary here; it would *violate the frozen contract*. The
input-artifact list is the sanctioned mechanism, and a workspace query is a fan-out over per-unit engine
runs, not a new in-run capability.

---

## 5. Rewrite is a validated transaction (unchanged from `DESIGN-query-engine.md` §5)

A `Rewrite` request returns a modified `Ast`; the engine **re-parses and type-checks it before
accepting** — well-formed-or-reject. The edit lands as a validated, diagnostic-free tree, or is rejected
with a specific diagnostic and *no change is written*. Never a half-applied patch. This is the
highest-value authorship affordance (the external-research "code the transforms" result) and it is free
here: the AST + the pure checker are exactly what a validated transaction needs. §4 case 2 lets the
sidecar *also* consult that checker before proposing the edit; the engine's recheck is the backstop.

---

## 6. Where effects *could* live — optional internal sugar, never a host import

If we later want the imperative *feel* — `perform (Query (TypeOf n))` threaded through a `do`-block
instead of accumulating a `List Request` by hand — that is allowed, under one hard constraint:

> It MUST be an **internally-handled effect** (a handler the engine installs and discharges), **never a
> host import, and never a mutable cell.**

An internal handler threads the request-accumulator functionally, so purity, determinism, and
reproducibility are untouched, and it *dogfoods Cadenza's own effect system* — a nice demonstration, and
consistent with the effects design (`DESIGN-effects-rcdzc.md`: `handle` reduced away by the fold, the
sidecar's effects never escape to the manifest). But it is **pure sugar over `drive : Ast -> List
Request`**, identical in power. Therefore: **write the requests-as-data form first**, so the effectful
form is provably just a rendering of it — and so we are never tempted to smuggle real I/O in through a
`perform` that should have been an input artifact. The mutable-place option the user floated is rejected
outright: it is the effectful form's semantics with none of its safety, and it directly contradicts "A
Query Computes And Retains No Cache Of Its Own."

---

## 7. Staging — folds onto the rungs that already exist

**Rung 2 (buildable now, no self-host) — a first slice LANDED.** Generalize the `targets` parameter.
Add a `sidecar` **input artifact kind** carrying a *request list* (the pure-manifest form — no branching
yet), and the new `Query` **output kinds** alongside `component` / `rust` / `dwarf`. This rides the
existing kinded-artifact `compile()` entry with **zero new invocation surface**. *Realized so far*
(`rcdzc/src/sidecar.rs`, wired in `compile()`): the `sidecar` input kind + a total leb128 request-list
codec; `Emit(Wasm|Rust)` (today's `Target`, reached through the list — `targets` is now the degenerate
Emit-only case); `Query::TypeOf` → a `type-info` artifact (a def's rendered type, read from the type
column via `infer::def_scheme`); `Query::UsesOf` → a `uses` artifact (referencing node ids, the
resolution-column transpose). A query answers **alongside** a failed emit (proven end-to-end: a
CDZ0203 program yields no component but still returns the `TypeOf` answer) and a malformed list
**declines**. *Not yet built at this rung:* `ScopeAt` / `EffectRow` / `SizeOf`, `Rewrite`, and the
debug-info enablement directive (`DESIGN-debug-info-rcdzc.md` §9) — all additive on this seam. The debug
design already committed to this exact shape, so this rung unifies the two designs.

**Rung 3 (with self-host).** The branching `drive` program becomes real Cadenza — `Request`/`Q` are
ordinary sums, the sidecar is compiled by `rcdzc` and run via the component ABI, and it can call
`type-of` / `well-typed?` inline (§4). This needs the same generics + recursion-over-`Ast` the
combinator library needs (`DESIGN-query-engine.md` §7); **nothing about the engine changes** — the
manifest was the return type all along, and the requests were data from rung 2.

**Rung 4 (optional).** The `do`-notation / `perform` sugar of §6, if the ergonomics prove worth it. Pure
rendering of rung-3 semantics.

---

## 8. Fold into the frozen build-tool contract — additive, no new arms

Nothing here breaks `spec/contracts/build-tool-interface.md`:

- The `sidecar` request list is **one more kinded input artifact** — the input channel is already "an
  open set … without changing the entry's arity" (§The Tool's Inputs Are A Kinded Artifact List).
- `Query` results are **more kinded output artifacts** (a type table, a uses list) — the output set is
  already "open to additive extension" (§The Tool Produces A Component, A Manifest, And Diagnostics),
  same shape as the blessed DWARF / source-map / manifest sidecars.
- A `sidecar` kind the tool does not recognize is **a diagnostic, not a silent drop** (§The kind of an
  artifact … reported as a diagnostic) — reject-don't-miscompile.
- Success/failure is unchanged: a requested artifact present + no error diagnostic = success.

So the contract-level change is: *name the `sidecar` input kind and the `Query`-result output kinds.*
Both are additive extensions of already-open sets — no version increment, no new return arm. (Contrast
the two-arm→kinded-list reshaping, which *was* non-additive and took Amendment 0.8.0.)

---

## 9. What to build first (if greenlit)

Rung 2, in order — each testable against the corpus and the existing `convert` output projection:

1. **`sidecar` input kind + a `Request` list decoder.** Start with `Emit`-only requests (a superset of
   today's `targets`), proving the driver reads a request list and dispatches to backends unchanged.
2. **`Query` requests + their output kinds.** `TypeOf` and `UsesOf` first (they read the type and
   resolution columns directly); render results through the schema-hashed value envelope
   (`DESIGN-query-engine.md` §6) so one driver projects arbitrary query results. `ScopeAt`, `EffectRow`
   next.
3. **`Rewrite` + the validated-transaction wrapper** (re-parse + re-check; reject-or-emit) — reuse the
   `DESIGN-query-engine.md` §9 build list.
4. **`SizeOf` and the bounded compile-inspect loop** (§4 case 3) — the one place a flat list can't reach;
   proves the pure-branch-on-artifact story and the decline-not-diverge bound.
5. Defer the branching `drive` program (rung 3) behind generics, and the `perform` sugar (rung 4)
   indefinitely.

---

## 10. Open questions (for a later pass)

- **Request ordering / dependencies.** A `Rewrite` followed by a `Query` — does the query see the
  pre- or post-rewrite tree? Proposal: a request list is *ordered*, each request reads the tree as left
  by the prior ones, and the engine runs them as a left fold over the (Ast, accumulated-artifacts) state
  — still pure, still one pass. Needs pinning before rung 3.
- **`UsesOf` across units.** Within one arena it is the resolution-column transpose; workspace-wide it is
  a fan-out over per-unit runs (§4), which needs the multi-unit input-artifact story landed.
- **`SizeOf`/compile-inspect bound.** The exact decline threshold for a compile-inspect loop — reuse the
  evaluator's static call-graph bound, or a request-count budget? (`reference-compiler.md` names the
  evaluator bound; a request budget may be simpler for the driver.)
- **Effect-sugar surface (rung 4).** Only design once rung 3 exists and the ergonomic need is real — do
  not invent the `do`-notation before the request-data form proves the vocabulary (same discipline as
  `DESIGN-query-engine.md`'s "do not invent syntax first").
