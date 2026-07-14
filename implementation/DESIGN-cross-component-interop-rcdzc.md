# Design — cross-component Cadenza interop via shared-runtime value handles (rcdzc)

**Author:** design pass (compiler). **Audience:** the implementer picking this up, + future me.
**Status:** proposal / handoff — **nothing landed.** Written 2026-07-14 against `spec` @`51407497`.
**Operator ask (verbatim intent):** "a way to easily interop with Cadenza functions across
components — export a function with a specific signature, and another Cadenza component binds to it via
our import mechanisms; extend the component ABI to be a Cadenza API where any runtime value can cross
boundaries in multiple Cadenza components without too much overhead."

**Decision taken (AskUserQuestion, 2026-07-14):** the value crossing uses **shared-runtime handles**
(Transport B below), not per-call marshaling. Cadenza-to-Cadenza first; the native-component-types edge
(Transport A) is a later, outermost-boundary concern.

---

## 0. The load-bearing distinction (READ FIRST)

Two different things both get called "linking," and this design is about the second:

- **Package linking (BUILT — `DESIGN-package-linking.md`, `link.rs`).** `(import "path" (f g))` splices N
  `.cdz` **source files** into ONE arena → ONE `Db` → ONE component. An imported name is just another
  file's ordinary `Def` in the same merged arena; it β-reduces/monomorphizes and inlines exactly like a
  local call. Its scope fence is explicit: *"nothing crosses a component boundary… zero component-ABI /
  envelope work."* There is **no external/unresolved-symbol notion.**
- **Cross-*component* interop (THIS DOC — does not exist).** Component A exports `f`; a **separately
  compiled** component B imports A's *interface* and calls `f` across a **live component boundary**, with
  values passing between them. The import taxonomy today is a closed set of exactly two kinds
  (`envelope.rs:1-23`, `mod.rs:4101-4108`): `cadenza:runtime/heap` (the value-heap runtime, ABI-exempt)
  and host-effect imports (`"host"`). There is **no third "peer Cadenza interface" import kind**, and
  `cdz-run` instantiates exactly one program + one *fresh* runtime instance per run
  (`cdz-run/src/lib.rs:202,391`) — no path composes two program components, let alone shares a heap.

So this is genuinely net-new: an external-symbol front-end, a fourth envelope shape, and shared-runtime
composition. But the **hardest** part — a runtime value crossing a component boundary as an opaque
resource handle, in and back out — is **already built** by the closures-across-host vertical
(`DESIGN-closure-host-resource-rcdzc.md`; see §3).

---

## 1. The two transports (why we picked handles)

A value crossing A→B goes one of two ways:

| | Transport A — native component types | Transport B — shared-runtime handles (**chosen**) |
|---|---|---|
| Wire form | frozen table: `List T`→`list<T'>`, record→`record`, sum→`variant` (`options/type-mapping/component-model-types.md`) | opaque `value` **resource handle** (i32 + refcount) into one shared heap |
| Overhead | canonical-ABI **copy per call** (marshal/unmarshal each hop) | **no serialization** — a handle is a heap index; both ends read it via runtime accessors |
| Interop reach | ANY component (Rust/JS/Python) | **Cadenza-to-Cadenza only** (shared runtime) |
| Blocked on | building the missing `value-decode` lift for compound args (`DESIGN-closure-host-resource-rcdzc.md:764`) | shared-runtime composition + a well-known `value` resource type |
| Reuses | native lift/lower (partly unbuilt) | the closures-across-host resource + round-trip machinery (built) |

**Why B is the operator's ask.** "Any runtime value crosses without too much overhead" is precisely the
no-serialization property, and it is *already* what the frozen ABI says happens at the **internal runtime
boundary**: a runtime value "crosses as an opaque handle whose interpretation belongs solely to the
runtime" and is "meaningful only within the single run and runtime instance that produced it"
(`component-abi.md §A Runtime Value Crosses As An Opaque Handle`). Transport B **extends that internal
boundary across a composition of Cadenza components** rather than inventing a new value encoding. It also
banks the closures work: a compound value already crosses OUT of a component as an `own<t>`/`borrow<t>`
resource for every shape (scalar/bytes/compound/sum/collection), and the round-trip proof shows a resource
handle crossing **out of one export and back into another**, dispatched via `resource.rep`.

**Non-goal (this phase):** Transport A. When a non-Cadenza consumer must call a Cadenza export, the value
marshals through the frozen type-mapping table at that outermost edge. That is a separate vertical and is
blocked on `value-decode`; it does not gate any Cadenza↔Cadenza work.

---

## 2. The model

### 2a. The `value` resource — one opaque handle type for every compound

Because the runtime is **tagless and name-free** (`component-abi.md §The Runtime Does Not Name Or Render
Values`) and Cadenza has **no type erasure** (both ends know a value's static type at compile time), a
single opaque `value` resource type is sufficient for every compound that crosses. There is no need for a
resource-per-monomorphized-type (contrast the closure work, which mints a resource per signature so the
host can dispatch `call`; here the receiving Cadenza component knows the static type and reads the handle
directly).

- **Scalars** (`Bool`, `s8..s64`, `u8..u64`, `f64`, `Char`, `Unit`) cross **by value** — native
  component-model scalars, no handle, zero overhead. (Same as the frozen table's scalar rows.)
- **Compounds** (`String`, `Bytes`, `List`, `Set`, `Map`, `Tuple`, `Record`, `Sum`, `BigInt`,
  `Rational`, nested) cross as a **`value` resource handle**.

The `value` resource is published at a **well-known interface** the compiler emits programs against —
naturally `cadenza:runtime/values` (a sibling of `cadenza:runtime/heap`, `runtime_abi.rs:61`), or a
`value` resource re-exported by the heap interface itself. Both A and B import the SAME `value` resource
type, so a handle A produces is a valid `value` for B. This is the literal reading of the operator's
"extend the component ABI to be a Cadenza API": the runtime interface gains an opaque `value` resource,
and cross-component compound signatures are typed against it.

### 2b. Shared runtime instance — the load-bearing composition rule

The frozen ABI: a runtime handle is "meaningful only within the single run and runtime **instance** that
produced it." Transport B **honors this literally** by composing A, B, and ONE runtime instance into a
single instantiation graph — one heap, shared. A handle B builds is meaningful to A because they index the
**same** instance's memory.

```
        ┌───────────────────────────────────────────┐
        │  composition (one wasmtime instantiation)   │
        │                                             │
        │   ┌─────────┐   value handle    ┌────────┐  │
        │   │  comp B │ ───(borrow<value>)──▶│ comp A │  │
        │   └────┬────┘                    └────┬───┘  │
        │        │ heap ops                     │ heap │
        │        ▼                              ▼      │
        │      ┌───────────────────────────────────┐  │
        │      │  ONE cadenza:runtime/heap instance │  │
        │      │  (one linear memory / one heap)    │  │
        │      └───────────────────────────────────┘  │
        └───────────────────────────────────────────┘
```

This is an **additive** reinterpretation, not a break: today's single-program run is the degenerate
one-program composition. The "single run" the contract names becomes "one composed run of the linked
graph."

### 2c. Ownership across the boundary — `borrow<value>` is the default

Reuse the closures ownership discipline exactly (`C-HOST-5`, `C-HOST-6`):

- **Argument B→A:** B built the value (owns it on the shared heap); A **borrows** it
  (`borrow<value>`) — reads it through runtime accessors, does **not** free it; B reclaims it. This is the
  natural callback shape and matches the repeatable-`borrow` posture. **The wasmtime-37 borrow trap is
  already dodged** (`C-HOST-6`, `closures-across-host-boundary.md`): a `borrow` method uses its param
  directly as the rep (no `resource.rep`), so `borrow<value>` is sound today.
- **Result A→B:** A produces the value (`own<value>` transfer out); B receives ownership and later drops
  it → the runtime `drop` reclaims. Same alloc/drop balance the resource escape already proves leak-free.

### 2d. The signature contract — monomorphized, per the frozen ABI

`component-abi.md §Generics Do Not Cross The Boundary`: A's exported `f` must be **monomorphic** at the
interface. A cross-component import binds a **concrete** signature. If A exports a generic `map`, B imports
a specific instantiation (`map` at `(List Int64) -> (List Int64)`), and A must have emitted that
monomorphization as an export. (Cross-component *monomorphization on demand* — B asks A for an
instantiation A didn't emit — is out of scope; the exported set is fixed at A's compile time. This mirrors
package-linking's fence against cross-package monomorphization.)

---

## 3. What we reuse (the closures-across-host machinery — mostly built)

The single biggest de-risking fact: **a Cadenza runtime value already crosses a component boundary as a
resource handle, in every shape, and back.** From `closures-across-host-boundary.md` /
`rcdzc-r1-resource-encode-linking-findings.md`:

- **Value escape (R1/R2/VM):** a compound crosses OUT as an `own`/`borrow` resource; the value-heap
  runtime is imported and composed; `make`/`encode`/method envelopes are byte-emitted and run under
  wasmtime. The `assemble_runtime_resource` family (`envelope.rs`) is the reference shape.
- **Round-trip (C-HOST-4):** a resource handle crosses **out of one export and back into a consumer
  export**, dispatched via `resource.rep` → cell → `call_indirect`. "producer + consumer are in the SAME
  core module → the closure's lifted lambda IS in-program → the type index resolves by signature." The
  cross-*component* case splits producer and consumer into two components sharing the runtime — the same
  handle-threading, now across the composition boundary.
- **Host + runtime import composition (`assemble_host_runtime`, `envelope.rs:625`):** an envelope that
  imports BOTH an effect interface AND the value-heap runtime, binding both into the program core. This is
  the exact template for "import a peer interface AND the shared runtime."
- **Borrow-repeatable handles (C-HOST-6):** `borrow<t>` works (trap dodged), so `borrow<value>` args are
  sound.
- **Ownership/leak discipline (`heap_operand_ownership`, own-owns-and-drops):** the alloc/drop balance for
  a handle crossing a boundary is solved and leak-tested.

What is genuinely NEW is only: the **import** direction of a *peer Cadenza interface* (vs. the runtime/host
imports and the run/closure *exports* built so far), the **external-symbol** front-end, and **multi-program
composition** in the runner.

---

## 4. Increment plan (oracle-first, byte-neutral, decompose-don't-defer)

Calibrated against the closures arc, which repeatedly proved a "large vertical" decomposes into
safe, gate-guarded bricks once attempted. Each brick lands independently onto `spec`.

**X0 — this design doc + additive spec note. ✅ DONE (`spec`).** Landed: (1) this doc; (2)
`component-abi.md` **Contract version 5** + a new `## Cross-Component Value Exchange` section (the shared-
runtime `value`-handle transport, borrow-arg/own-result ownership, monomorphic exchanged signature,
shared-instance handle meaning) — additive (a representation the shared-runtime mode previously lacked; a
single-program run stays byte-identical to v4); (3) `spec/capabilities/cross-component-interop.md` — the
program-level binding surface (explicit peer import, export-list visibility, concrete interface, value
same-on-both-sides, no-authority-widening, decline-don't-miscompile), binding to — not re-pinning — the
frozen value-form + type-mapping + host-interface-binding contracts.

**X1 — the COMPOSITION ORACLE (test-only, the de-risker).** A `wasmtime ComponentBuilder` reference: two
hand-built core modules A and B, both importing the SAME runtime instance's heap ops and a shared `value`
resource type; B calls A's export passing a heap handle; **prove it RUNS under wasmtime with one shared
heap** (a value built in B, read in A, correct result). This is the oracle-first move the closures work
used at every seam (`C-HOST-1 ORACLE`, round-trip oracle, distinct-sig oracle) — it validates the novel
runtime-sharing mechanism *before* any compiler change. Scalar-arg/scalar-result first, then a `value`-arg
variant. **This brick alone answers "does the shared-instance model actually work under wasmtime."**

**X2 — the `Core::ExternCall` IR foundation. ✅ DONE (`spec`).** New `Core::ExternCall { interface, op,
args, result }` (mirror of `Core::HostCall`), threaded through every exhaustive `Core` match: the six
compiler-forced arg-descending sites (`compile.rs` reached-poisons, `layout.rs` closure-codes + callees,
`select.rs` `binding_escapes` + `collect_used_ops`) descend into its args like a call; the wasm `select`
emit and the Rust backend both DECLINE cleanly ("a cross-component call to `iface.op` is not yet emitted
(X3/X4)"); `tail_positions_have_call` counts it as a call. The host-import collectors (`collect_host_imports`,
`first_unrepresentable_host_op`, `collect_host_arg_strings`) and the `subtree_reaches_host_call` predicate
correctly leave it out — a peer call is NOT a host effect. **Byte-neutral** (nothing constructs an
`ExternCall` on the normal path yet — gate 1876 pass / 0 fail / 0 regressions / 0 newly-passing). The
resolver front-end that CONSTRUCTS one — a cross-component `(import "comp" (f))` resolving to an external
symbol (a `Resolved::Extern`, an external `link::Import`) rather than an inlined sibling `Def`, cyclic-import
rejection reusing `find_import_cycle` — moves to **X4** alongside the runner (so the trigger, the envelope,
and the composition land together and are testable end-to-end). ⚠ This is exactly the external-symbol notion
`DESIGN-package-linking.md §0` deliberately avoided.

**X3 — the peer-interface IMPORT envelope. ✅ DONE (`spec`).** The **fourth** envelope shape,
`envelope::assemble_extern(core, exports, peer_iface, extern_fns)` — a fork of `assemble_host`
(structurally identical: import an instance-type declaring each peer op, alias + lower each to a core
func, bind them into the program core, export the consumer's own boundary), differing only in binding the
core under module `"peer"` (new `PEER_MODULE` const, matching what a consumer core imports its peer ops
from) and importing a peer Cadenza interface rather than a host effect. Index spaces documented in the
fn. Boundary + op names kebab-normalized (`kebab_extern_name`). Peer ops carry monomorphic signatures.
Proven by the X3 oracle: the consumer envelope is emitted by `assemble_extern` (not hand-built), composed
with an interface-exporting provider, and RUN under wasmtime — `main(5) = f(5)*10 = 60`. Scalar
args/result (a `value`-handle op is X5); no runtime fused yet (the `assemble_host_runtime` analogue is a
later increment). 🔑 FINDING: a component-model interface import checks parameter NAMES structurally, so
the peer's exported signature and the consumer's import declaration must AGREE on param names — X4's
front-end must emit both sides with a consistent convention (`assemble_extern` uses `p0`, `p1`, … from
`host_op_comp_functype`; the X3 provider lifts `f` with `p0` to match). Byte-neutral (new pub fn unused by
production — gate 1881 pass / 0 fail / 0 regressions). `select`-emits-`ExternCall`-as-`call` is wired in
X4 (it needs the front-end trigger + the layout's extern-import order, which land together).

**X4a — the cdz-run multi-component composition primitive. ✅ DONE (`spec`).** `cdz_run::run_with_peers(
consumer, peers, opts)` + a `cdz_run::Peer { bytes, interface }` descriptor: instantiate each peer
component, forward its exported interface's funcs (discovered off the peer instance type, never hard-coded
— the `compose_runtime` discipline) into the consumer's like-named import, all in ONE shared `wasmtime`
store; compose the consumer's runtime if it imports one. `bind_host_imports` gained a `skip: &[String]` so
a peer interface bound as a peer is not ALSO bound as a host effect (a double-bind is a linker error — the
bug this surfaced + fixed). Proven by the X4a test: the X3 provider and `assemble_extern` consumer, built
as SEPARATE valid components, composed by `run_with_peers` → `main(5) = f(5)*10 = 60`. This is the shape
the front-end (X4b) produces (each `.cdz` → its own component). Byte-neutral (new fn + a skip param passed
`&[]` on the existing path; gate 1888 pass / 0 fail / 0 regressions). ⚠ the earlier "version header out of
order" was a reused-`wasmparser::Validator` (one validator can't validate two components) — a fresh
validator per component; `wasm-tools validate` confirmed both standalone. ⚠ stale-runtime false alarm on a
fresh worktree — `cargo xtask build` before gating (515 false regressions → 0). SCOPE: scalar peer ops, a
runtime-free peer; sharing ONE runtime instance across consumer + peers is X5.

**X4b — the front-end trigger (source → `Core::ExternCall` → `assemble_extern` → run e2e), IN SUB-BRICKS.**
🔑 SURFACE DECISION (operator, 2026-07-14): a DISTINCT form `(extern "iface" (op (-> …)) …)` — NOT an
overload of `(import …)` (which means intra-package source splice) — with the peer's monomorphic
signatures declared INLINE (the declared sig IS the contract; a peer whose export mismatches declines at
composition; no wasmparser in the compile path, no external interface artifact needed). Sub-bricks:
- **X4b-1 — the `extern` SCAN. ✅ DONE (`spec`).** `db::ExternDecl { interface, occ, ops: Vec<ExternOp{
  name, name_occ, ty }> }` + `scan_extern_decl` (the `scan_effect_decl` analogue; each clause `(NAME
  TYPE)`, no `op` keyword); `Db::extern_decls` populated at load; `extern` registered in
  `TOP_LEVEL_FORMS`/`TOP_LEVEL_KEYWORDS`. Byte-neutral (table populated, nothing consumes it — gate 1894
  pass / 0 fail / 0 regressions; tests `x4b1_*`).
- **X4b-2 — resolve → `Core::ExternCall`. ✅ DONE (`spec`).** New `Resolved::Extern { interface: String,
  op: String, ty: StructId }` variant; `resolve_name` step 3d resolves an extern op name to it via a new
  `db.extern_op_by_name` query (after sum/effect/variant decls, before prelude); `infer::compute` types it
  as its declared sig (`typeval_of(ty)`); `lower`'s `Apply` arm produces `Core::ExternCall` (result = the
  sig's result after N args, via `fn_result_after`); a BARE (unapplied) extern declines (a first-class
  extern-op value is later). Every exhaustive `Resolved` match got an arm (7 forced: eval collect_callees,
  infer compute + type_errors, lower compute + ref_escapes_whole + uses_in, compile walk_for_dead_traps —
  all leaf/no-op except compute). Byte-neutral: a program with `(extern …)` type-checks + lowers to
  `Core::ExternCall`, which the backend DECLINES cleanly pending X4b-3 (never a type reject). Tests
  `x4b2_*` (resolves to `Resolved::Extern`, lowers to `Core::ExternCall` w/ interface+op+result; a
  well-typed application declines-at-emit not type-rejects). Gate 1911 pass / 0 fail / 0 regressions;
  suite 1366; clippy clean.
- **X4b-3 — backend emit. ✅ DONE (`spec`) — THE SOURCE→RUN MILESTONE.** `emit` collects the extern-import
  set (`host::collect_extern_imports` → `host::ExternImport`), records `layout.extern_order` (the
  `host_order` analogue + `with_extern_order`/`extern_index`), shifts `import_base` to include externs;
  `select` emits `Core::ExternCall` → args + `Lir::CallExternImport(index)`; the core imports peer ops from
  module `"peer"` (`serialize::core_module_with_extern` + `extern_import_functype`/`extern_import_item`);
  `emit` routes an extern-only program to `assemble_extern` (X3) with `p0,p1,…` param names
  (`extern_op_comp_functype`, matching the X3 finding). **Proven end-to-end: a SOURCE consumer `(extern
  "cadenza:math/api" (neg (-> Int64 Int64))) (def (main (: x Int64)) (neg x))` compiles to a valid
  component importing `cadenza:math/api`, and composed with a provider via `run_with_peers` → `main(5) =
  neg(5) = -5`** (test `x4b3_*`). The first end-to-end cross-component call FROM SOURCE. SCOPE: an
  extern-ONLY consumer (a single peer interface; no host/runtime fusion — those decline cleanly), scalar
  args/result. Byte-neutral (only extern-using programs take the new path — gate 1924 pass / 0 fail / 0
  regressions after `xtask build`; suite 1371; clippy clean). ⚠ stale-runtime false alarm again (536 →
  0 after `xtask build`).
- **X4b-provider — the component's OWN interface name. ✅ DONE (`spec`) — TWO SOURCE COMPONENTS LINK.**
  🔑 (operator): the PROVIDER publishes its exports under the interface name the consumer binds to, named
  by the compile REQUEST — a `KIND_COMPONENT_NAME` input artifact (the `KIND_ENTRY` pattern), read in
  `compile()` into `Db::component_name`. `envelope::assemble_provider` wraps the scalar boundary exports
  as a named INTERFACE INSTANCE (a component-instance export-items section `5` bundling the lifted funcs,
  exported under the interface name via `export_instance_item`) instead of bare top-level funcs; `emit`
  routes there when `db.component_name` is set (bare scalar case). **Proven e2e (`x4b_provider_*`): a
  PROVIDER `(def (neg (: x Int64)) (- 0 x)) (export neg)` compiled with `component-name = cadenza:math/api`
  + a CONSUMER `(extern "cadenza:math/api" (neg (-> Int64 Int64))) … (neg x)`, BOTH from source, composed
  via `run_with_peers` → main(5) = -5.** 🪤 two byte bugs fixed by `wasm-tools print`: the top-level
  instance export AND each instance MEMBER name are component EXTERN names (`0x00 <len> <name>` +
  `export_instance_item`'s trailing `0x00`), not bare strings. Byte-neutral (only a `component-name`
  request takes the provider path — gate 1926 pass / 0 fail / 0 regressions). SCOPE: scalar/unit exports
  (a compound interface member is a later increment); bare (no runtime) provider.
- **X4b-4 — runner/CLI delivery + e2e.** The library path (`run_with_peers`) is proven; what remains is
  the CLI/tooling surface so the operator drives it without a Rust test: `cdz` delivers the peer set + the
  `component-name`/`extern` artifacts. Multi-component gate/corpus shape (corpus is single-`(input)` —
  `DESIGN-package-linking.md §8.1` names the same gap). The RESOLVER + EMIT + RUNNER are all done (the
  two-source e2e passes as a Rust integration test); X4b-4 is the CLI ergonomics layer over them.

**X5 — COMPOUND values cross as shared `value` handles (the payoff).** Extend X3/X4: a compound arg crosses
as `borrow<value>`, a compound result as `own<value>`. B builds a `List` on the shared heap, hands the
handle to A as `borrow<value>`; A reads it via the runtime accessors as its statically-known `(List
Int64)`; A returns a `value` B receives. **Reuses the round-trip rep/borrow mechanics wholesale** (§3). This
is where "any runtime value crosses multiple Cadenza components without serialization" actually lands.
Widen across the value matrix (String/Bytes/Map/Set/Tuple/Record/Sum/BigInt/Rational/nested) the way the
closures result-matrix was widened — each a small corpus/coverage brick once the mechanism is in.

**X6+ — widenings (optional, non-blocking):** own-vs-borrow policy corners; N-component graphs (a diamond
import); a value's lifetime across a multi-hop call chain (A→B→C sharing one heap); the outermost
**Transport A** marshaling edge for a non-Cadenza consumer (blocked on `value-decode`); cross-component
effect/capability forwarding (a handler in B attenuating A's row — ties to
`capabilities-and-effects.md`).

---

## 5. Governance & scope fences

- **Frozen contracts touched:** `component-abi.md` (v3/v4) and `host-interface-binding.md`. The shared-
  `value`-handle transport is an **additive** change (a new well-known import + a representation the
  shared-runtime mode previously lacked) — permitted additively (`component-abi.md §Additive Evolution`),
  but it is a **coordinated spec act**: land the additive clause (X0) before the envelope brick (X3).
- **Do NOT invent a second value encoding.** The canonical byte form + its decode inverse are frozen
  (`deterministic-value-form.md`); the program-level interchange surface (`to-bytes`/`from-bytes`, tagged)
  is spec'd (`value-interchange.md`, `options/value-interchange/schema-hashed-envelope.md`). Transport B
  carries **handles, not bytes**, so it neither uses nor competes with interchange — but the outermost
  Transport-A edge, when built, MUST use the canonical form, not a bespoke one.
- **Monomorphic interfaces only** (`component-abi.md §Generics Do Not Cross`). No cross-component
  monomorphization-on-demand; A's exported instantiations are fixed at A's compile time.
- **Decline-don't-miscompile** (`reference-compiler.md §Outcomes Are Ordered By Safety): every
  not-yet-handled shape (a generic import, a compound before X5, a resource/closure-typed value crossing,
  a cross-component effect) DECLINES cleanly, never miscompiles. `cdz check` won't catch a wrong-handle
  miscompile — **`wasm-tools validate` / run under wasmtime is the oracle** (the
  materialize-a-constant-non-scalar lesson from BigInt: audit every handle-into-a-slot site).

## 6. The one-paragraph statement

Let two separately-compiled Cadenza components be **composed against a single shared value-heap runtime
instance**, so a runtime value — an opaque handle into that one heap — passes between them **without
serialization**. A component's compound exports/imports are typed against a **single well-known `value`
resource** (the runtime interface's opaque handle type); scalars still cross by value. A component
**imports a peer's monomorphized interface** the way it already imports the host and the runtime, and calls
across it via a new external-symbol path; the receiving component reads a handed-in `borrow<value>` through
the runtime accessors as its statically-known type (no tag, no erasure, no decode). This reuses the
closures-across-host resource + round-trip machinery for the boundary crossing and the value-heap-runtime
composition envelope for the shared instance; the genuinely new work is the external-symbol front-end, a
fourth (peer-interface) import envelope, and multi-program shared-runtime composition in the runner. It
binds to — does not re-pin — the frozen component-ABI, value-form, and type-mapping contracts, and the
shared-`value`-handle transport enters those contracts as an additive representation.
