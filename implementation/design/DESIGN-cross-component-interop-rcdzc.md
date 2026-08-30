# Design — cross-component Cadenza interop via shared-runtime value handles (rcdzc)

**Author:** design pass (compiler). **Audience:** the implementer picking this up, + future me.
**Status:** ✅ **REALIZED end-to-end (2026-07-14).** X0–X5d + X4b-4 all landed on `spec`. Two separately-
compiled Cadenza source components link and run across a live boundary; a runtime value (scalar OR
compound: List/String/Record/tuple/…) crosses between them as an opaque shared-runtime handle with NO
serialization; driveable from the CLI (`cdz compile --component-name` + `cdz-run --peer`). Remaining =
optional widenings (own/borrow reclamation across the boundary, N-component graphs, a multi-component
corpus shape, the outermost non-Cadenza marshaling edge). Written 2026-07-14 against `spec` @`51407497`.
**Operator ask (verbatim intent):** "a way to easily interop with Cadenza functions across
components — export a function with a specific signature, and another Cadenza component binds to it via
our import mechanisms; extend the component ABI to be a Cadenza API where any runtime value can cross
boundaries in multiple Cadenza components without too much overhead."

**Decision taken (AskUserQuestion, 2026-07-14):** the value crossing uses **shared-runtime handles**
(Transport B below), not per-call marshaling. Cadenza-to-Cadenza first; the native-component-types edge
(Transport A) is a later, outermost-boundary concern.

---

## ⭐ PIVOT (operator, 2026-07-14): UNIFY cross-component interop WITH EFFECTS — remove `extern`

**The insight (operator):** rather than a separate `(extern …)` form, REUSE the effect system. A component
declares a contract as an EFFECT (`(effect Math (op add (-> …)))`), defaults the whole component scope to
route that effect to a specific peer contract, and OVERRIDES it — for a unit test, or to do its own thing —
with an ordinary in-program `(handle Math …)`. **This unifies the whole feature set: a peer dependency and
a host effect become ONE concept — an escaping effect the manifest records.**

**Why it's sound (already 80% true in the code):** `envelope::assemble_extern` is a byte-fork of
`assemble_host`; the frozen ABI v4 already made a host-delegated effect "an ordinary imported-function
call." So at the transport/emit layer a peer call and a host-delegated effect are ALREADY the same. The
`(handle E …)` machinery (E0–E5, complete) gives the test-override for FREE — a nearer handler discharges
the effect before it escapes, so a mock needs no peer. This also aligns with the capability/manifest model:
a component's escaping-effect row = its dependencies (host OR peer), one row.

**The model:**
| was (`extern`) | unified (effects) |
|---|---|
| `(extern "cadenza:math/api" (add (-> …)))` | `(effect Math (op add (-> …)))` + a binding of `Math`→peer `cadenza:math/api` |
| binds `add` to that peer | a component-scope DEFAULT route for `Math` (the `(host (E) …)` analogue, with a TARGET) |
| — | `(handle Math …)` in a test overrides → mocked in-program, no peer |
| `Core::ExternCall` → `assemble_extern` | escaping `Math.add` → `Core::HostCall` → the SAME boundary envelope |

**Binding precedence (operator, 2026-07-14):** in-source DEFAULT (a top-level directive binds `Math`→a peer
contract) < COMPILE-REQUEST override (a `--bind Math=<iface>` input, or dropping it) < in-program `(handle
Math …)`. So source declares the default, the compiler can rebind (or drop for a test), and a handler always
wins.

**Migration decision (operator):** MIGRATE `extern`→effects and REMOVE the `(extern …)` form + merge
`Core::ExternCall` into `Core::HostCall`. What SURVIVES the merge (surface-agnostic, all reused verbatim):
the shared-runtime value-handle TRANSPORT (X5), the RUNNER (`run_with_peers`), the PROVIDER side
(`assemble_provider`/`assemble_provider_runtime` + `--component-name`), and the boundary ENVELOPE
(`assemble_extern`/`_runtime` — the peer-interface import shape, now reached from an escaping effect). What
GOES: the `(extern …)` scan, `Resolved::Extern`, the `Core::ExternCall` variant (folded into `HostCall`),
`db.extern_op_by_name`, `collect_extern_imports` (merges into the host-import collection with a per-effect
TARGET). Unification bricks: **U1** (this pivot record) → **U2** (an effect bound to a peer routes to the
boundary envelope) → **U3** (compile-request override) → **U4** (remove `extern`, merge into `HostCall`).

### Unification progress
- **U1 — pivot recorded. ✅ DONE (`spec` `c810e7e4`).**
- **U2 — an effect BOUND to a peer routes to the boundary. ✅ DONE (`spec`).** Surface: a top-level
  `(bind Effect "cadenza:pkg/iface")` directive → `db.effect_bindings` (a `scan_effect_bindings` load-time
  scan; `bind` in `TOP_LEVEL_FORMS`). Routing: in `emit`, a collected `HostImport` whose effect is bound is
  MOVED into the extern set retargeted to its bound interface (`host_param_abi` converts the params); `select`'s
  `Core::HostCall` arm resolves a bound effect against `layout.extern_index` and emits `CallExternImport`
  (else the host path). NO new IR — `Core::HostCall` IS the peer call. **Proven e2e (`u2_*`): a source
  consumer `(effect Math (op add (-> Int64 Int64 Int64))) (bind Math "cadenza:math/api") (def (main (: x
  Int64)) (host (Math) (Math.add x x)))` composed with a peer providing `add` → main(5) = add(5,5) = 10.**
  An in-program `(handle Math …)` discharges the effect before escape → the test-override, free. Byte-
  neutral (routing fires only when `effect_bindings` non-empty — gate 2041/0/0). SCOPE: scalar ops, a
  bound-effect + unbound-host-effect in one program still declines (the extern+host coexistence guard);
  extern+runtime already composes.
- **U3 — COMPILE-REQUEST override of an effect's peer binding. ✅ DONE (`spec`).** A `KIND_EFFECT_BIND`
  input artifact (newline `Effect=iface` lines) merged over `db.effect_bindings` AFTER load, WINNING over
  the in-source `(bind …)` default: `Effect=<iface>` REBINDS, `Effect=` (empty) UNBINDS (→ escape to host,
  or a test's in-program handler). **Proven e2e (`u3_*`): the SAME U2 source (bound to `cadenza:math/api`,
  an ADD peer) rebound by the request to `cadenza:mathv2/api` (a MUL peer) → main(5) = Math.add(5,5) = 5*5 =
  25, not 10.** Realizes the FULL precedence ladder the operator asked for: **in-source default <
  compile-request override < in-program `(handle …)`** (the handler discharges before escape, free from
  U2). Byte-neutral (the artifact only affects a program that supplies it — gate 2046/0/0).
- **U4 — REMOVE the `extern` surface + `Core::ExternCall`. ✅ DONE (`spec`).** Removed: the resolve step
  producing `Resolved::Extern`, the lower Apply arm producing `Core::ExternCall`, the `Resolved::Extern` and
  `Core::ExternCall` variants + all their match arms, the `(extern …)` SCAN (`db::ExternDecl`/`ExternOp`/
  `scan_extern_decl`/`extern_op_by_name`/`extern_decls`, `extern` from `TOP_LEVEL_FORMS`), the compile.rs
  extern well-formedness diagnostic + `MALFORMED_EXTERN_PREFIX`, and `collect_extern_imports`. SURVIVE
  (surface-agnostic, reached from a peer-bound effect via U2): `assemble_extern`/`_runtime`,
  `assemble_provider`/`_runtime` + `--component-name`, `layout.extern_order`/`extern_index`,
  `Lir::CallExternImport`, `serialize::core_module_with_extern*`, `host::ExternImport`/`extern_abi_val_type`,
  `run_with_peers`. Tests: the extern-SOURCE + removed-variant tests removed; the hand-built TRANSPORT
  oracles (x1a/x1b/x3/x4a/x5a) + the effects-surface e2e (u2/u3) remain (7 cross-component tests). Gate
  2049/0/0, suite 1407, clippy clean. ⚠ FOLLOW-UP: source-level COMPOUND-value coverage over the effects
  surface (old x5b/c/d proved it over `extern`; x5a still proves the compound TRANSPORT hand-built, U2/U3
  prove the effects surface at scalar); a `(bind …)` / `(effect …)` well-formedness diagnostic.

- **U5 — COMPOUND value over the EFFECTS surface, from source. ✅ DONE (`spec`).** Closes the U4 follow-up:
  a peer-bound effect whose op returns/takes a runtime compound now crosses it as the opaque `u32` handle
  over the shared runtime, from an ordinary `(effect …)`/`(bind …)`/`(host …)` — no `extern`. Two fixes to
  the front of the emit path (the transport already carried compounds since X5b): (1) `collect_host_imports`
  uses `extern_abi_val_type` (compound→U32 handle) instead of `abi_val_type` (compound→None, dropped) for a
  peer-bound effect's params/result; (2) `first_unrepresentable_host_op` widens its representable set for a
  peer-bound effect — a runtime-owned compound result/argument is a handle-crossable value, not a decline
  (the plain host boundary stays scalar-only). Test `u5_*`: a hand-built runtime-importing tuple peer
  (`pair : func(s64)->u32` building `(x,x)` on the shared heap, exported as `cadenza:pairs/api`) composed
  with a SOURCE consumer `(effect P (op pair (-> Int64 (Tuple Int64 Int64)))) (bind P "cadenza:pairs/api")
  (def (main (: x Int64)) (host (P) (. (P.pair x) 0)))` → main(9)=9 (the tuple crossed as a handle, element
  0 read back). Routes through `assemble_extern_runtime`. Gate 2076/0/0, clippy clean, byte-neutral (no
  corpus program is peer-bound). 9 cross-component tests.

- **U6 — BOTH SIDES FROM SOURCE over the effects surface. ✅ DONE (`spec`, test-only).** The full payoff:
  no hand-built peer at all. A source PROVIDER `(def (pair (: x Int64)) (tuple x x)) (export pair)` compiled
  with component-name `cadenza:pairs/api` (the `--component-name` path → `assemble_provider_runtime`, which
  survived U4 — the provider side never used `extern`, just a named export) is consumed by the U5 effects
  consumer `(effect P (op pair (-> Int64 (Tuple Int64 Int64)))) (bind P "cadenza:pairs/api") … (host (P) (.
  (P.pair x) 0))`. Composed via `run_with_peers` over ONE shared runtime → main(9)=9, two Cadenza source
  files exchanging a compound. Test `u6_*` + a `compile_provider` helper (compiles source with a
  `component_name_artifact`). No compiler change (byte-neutral); gate 2088/0/0, 10 cross-component tests.

- **U7 — DOC refresh after U4. ✅ DONE (`spec`, comment-only).** A dozen backend/front-end doc comments
  still described the removed `(extern …)` source surface / the removed `Core::ExternCall` node as if live
  (the binding surface, extern-import selection, and a stale "extern after host+runtime, a later increment"
  layout note contradicting X5's extern-FIRST order). Refreshed each to the effects-unified reality; kept
  the historical "exactly as an `(extern …)` op did" analogies as tombstones. Byte-neutral.

- **U8 — CLI PROVIDER delivery on the effects surface. ✅ DONE (`spec`, test-only).** X4b-4's CLI delivery
  was hand-verified on the removed `extern` surface and had NO automated coverage after U4. New `cdz` CLI
  test `cross_component_cli.rs` drives the real binary: `cdz compile <provider> --component-name
  cadenza:pkg/iface` on a SCALAR provider publishes the named interface instance; on a COMPOUND-returning
  provider (`pair x = (tuple x x)`) publishes the interface AND imports `cadenza:runtime/heap` (the
  `assemble_provider_runtime` path); the control (no `--component-name`) keeps the export top-level. Shape
  asserted by dependency-free byte inspection (the `cdz-run --peer` consumer-run half needs wasmtime + the
  store, kept in `cdz-run`; `u6_*` proves the full run). Byte-neutral; gate 2105/0/0.

- **U9 — a consumer binds TWO+ DISTINCT PEER INTERFACES. ✅ DONE (`spec`).** Lifts the single-interface
  decline (`mod.rs`). Generalized `assemble_extern`/`assemble_extern_runtime` to take `op_ifaces: &[&str]`
  (the interface each op in `extern_order` is imported from) instead of one `peer_iface`: the distinct
  interfaces (first-appearance order) become component instances/types `0..g`, each op aliases out of ITS
  interface's instance, and the boundary lift's comp-type index shifts `1`→`g` (peer-only) / `2`→`g+1`
  (peer+runtime). The core module is UNCHANGED — the one merged `"peer"` core instance still exports every
  lowered op FLAT by name, so op names must be globally unique across the bound interfaces; a cross-interface
  collision DECLINES (`mod.rs`, "unique across the peer interfaces"). G=1 reproduces the byte-exact X3/X5
  shape (gate 2107/0/0, byte-neutral). Helpers `distinct_ifaces`/`iface_index`/`peer_group_ops`. Tests:
  `u9_*` — a consumer binds M→`cadenza:math/api` (scalar `neg`) AND P→`cadenza:pairs/api` (compound `pair`),
  `main(9)=neg(pair(9).0)=-9` (a value from EACH peer, via `assemble_extern_runtime` g=2) — plus `u9b_*` the
  same-op-name collision decline. 12 cross-component tests.

- **U10 → U11 — a MIDDLE component is BOTH consumer AND provider (an A→B→C chain). ✅ DONE (`spec`).**
  U10 first made the consumer+provider combination an honest DECLINE; U11 turned it into a working feature.
  A component B that binds a peer (A) AND is compiled with `--component-name` now imports A's interface,
  computes, and BUNDLES its own boundary export into a named interface instance for a downstream consumer
  (C) — the fused envelope. Implemented as an `Option<&str> publish_iface` param on `assemble_extern` /
  `assemble_extern_runtime`: `None` (a pure consumer) exports each boundary func TOP-LEVEL (byte-identical
  to X3/X5), `Some(iface)` bundles the lifted funcs into a component instance (the `assemble_provider`
  shape — comp instance `g` peer-only / `g+1` with runtime) and exports it under `iface`. The runner
  (`run_with_peers`) now binds each EARLIER peer's interface into later peers' linkers (peers in dependency
  order, `bind_peer_ifaces_into`), so B (a peer) can import A (a peer). Test `u11_*`: A publishes
  `cadenza:pairs/api` (`pair`), B binds it + publishes `cadenza:mid/api` (`mid x = pair(x).0 + 1`), C binds
  `cadenza:mid/api` → `main(9)=10`, a value flowing A→B→C. Byte-neutral (`publish_iface=None` is the old
  shape); gate 2141/0/0. 14 cross-component tests + 8 cdz-run tests.

- **U12 — a DIAMOND graph: one shared provider feeds two middles. ✅ DONE (`spec`, test-only).** Composes
  the multi-interface consumer (U9) with the fused consumer+provider chain (U11) over a SHARED base: a
  provider A (`base x = x*2`, `cadenza:base/api`) is consumed by TWO middle components B (`bee x = base(x)+1`,
  `cadenza:b/api`) AND C (`cee x = base(x)+10`, `cadenza:c/api`), and a top component D binds BOTH B and C
  (`main x = bee(x)+cee(x)`). No compiler change — the U11 runner (each EARLIER peer's interface bound into
  later peers' linkers, dependency order) already forwards A into BOTH B's and C's linkers and both middles
  into D. Test `u12_*`: `main(5) = (10+1)+(10+10) = 31`. Confirms the peer-forwarding composes for a
  non-linear graph. 15 cross-component tests.

- **U13 — RECLAIM a peer-returned compound projected across the boundary. ✅ DONE (`spec`).** A peer-bound
  effect returning a COMPOUND (U5/U11) hands the consumer a fresh OWNED shared-runtime handle; when the
  consumer projects a scalar field (`(host (P) (. (P.pair x) 0))`), `arr-get` BORROWS the aggregate to read
  the element but nothing released it — it LEAKED until run-end (harmless one-shot, a leak for a long-lived
  host). Fix in the `Core::Proj` emit: when the operand is a fresh OWNED temporary (`heap_operand_ownership`
  == Owned — now including `Core::HostCall` alongside `Core::Call`/constructors) AND the element is SCALAR
  (`get-int`/`get-bool` COPY the value out), stash the aggregate in a scratch slot, read, then `drop` it
  (rc--, cascade). A NESTED-COMPOUND element (the `arr-get` borrow IS the child) is left as-is (a deferred
  leak, never a use-after-free); a projection off a BORROWED binding drops nothing (the owner reclaims it).
  Byte-neutral for the corpus (an in-process projection off a local constructor optimizes the tuple away
  before the reclaim path; the leak only existed for an opaque peer-returned compound). Test `u13_*`
  (peer-effect projection emits `drop`, borrowed-param projection does not); full rcdzc suite 1448/0, gate
  2152/0/0, all cross-component runs correct under wasmtime (no use-after-free).

- **U14 — reclaim across a NESTED-COMPOUND projection. ✅ DONE (`spec`).** Extends U13 to the case U13 left
  as a deferred leak: projecting a COMPOUND field out of an owned peer-returned aggregate. The `arr-get`
  result IS the borrowed child, so the `Core::Proj` emit now `dup`s the returned child (rc++) so it survives,
  THEN `drop`s the parent — the parent's storage + every other child is reclaimed and the returned child
  stays live under its own retained reference. `collect_used_ops` imports `dup` for a nested-compound
  reclaim. Test `u14_*`: a peer returns `((x,x+1), x+2)`, the consumer `(. (. (P.nest x) 0) 1)` projects the
  inner tuple (nested → dup+drop-parent) then reads its scalar (→ drop-inner) → `main(9)=10`, verified under
  wasmtime (the inner tuple stays live across the outer's drop — no use-after-free). The `u13_*` unit test
  also asserts the nested case emits both `dup` and `drop`. Full rcdzc 1453/0, gate 2157/0/0.

- **U15 — a LET-BOUND peer-returned compound read twice then reclaimed. ✅ DONE (`spec`, test-only).**
  Confirms the general Perceus `let`-drop machinery (`Core::Let` emit: a dead heap binding is `drop`'d at
  scope end unless `binding_escapes`) already reclaims a peer-RESULT binding — no origin special-case needed.
  Test `u15_*`: `(let ((t (P.pair x))) (+ (. t 0) (. t 1)))` binds the owned peer tuple, reads BOTH fields
  via borrowing `arr-get`s off the `LocalRef` (so U13/U14 add no drop — a borrowed operand), both reads see
  a LIVE tuple (no premature drop between them), and the dead binding is reclaimed once at scope end →
  `main(9)=18` under wasmtime. Locks the corner as a regression test. 17 cross-component tests, gate 2169/0/0.

- **U16 — a compound ARGUMENT crosses TO a peer (the inbound direction). ✅ DONE (`spec`, test-only).**
  U5/U6/U11 cross compound RESULTS out of a peer; U16 closes the other direction — the CONSUMER builds a
  runtime tuple and passes it INTO the peer's op, crossing as its `u32` handle over the shared runtime (NOT
  marshaled bytes, so no `value-decode` needed), and the PROVIDER reads both fields. The collect-time wiring
  (`extern_abi_val_type` on a peer-bound op's PARAMS, `host.rs`) has existed since U5 but was never exercised
  end-to-end for an argument. Test `u16_*`: provider `sum t = (. t 0)+(. t 1)` over `cadenza:adder/api`,
  consumer `(host (S) (S.sum (tuple x (+ x 1))))` → `main(9)=sum((9,10))=19` under wasmtime. Completes the
  "value crosses BOTH directions" story for Transport B. 18 cross-component tests, gate 2173/0/0.

- **U17 — HANDLE PASS-THROUGH: a peer-produced handle flows straight to another peer. ✅ DONE (`spec`,
  test-only).** Composes U16 (compound arg in) with U5 (compound result out) across TWO boundary crossings
  in ONE body: `(host (P) (host (S) (S.sum (P.pair x))))` — the tuple A mints flows straight into B, never
  inspected by the consumer. Exercises ownership-transfer-on-argument: the handle is in a CONSUMING position,
  so the consumer NEITHER drops it (double-free) NOR leaks it — ownership transfers to B. Test `u17_*`:
  producer `pair x = (x, x+1)` / consumer `sum t = (. t 0)+(. t 1)` → `main(9)=sum(pair(9))=19` under
  wasmtime (correct across two crossings, no double-free/leak). 19 cross-component tests, gate 2176/0/0.

🎉 **THE UNIFICATION IS COMPLETE.** Cross-component interop IS the effect system: a contract is an
`(effect …)`, a peer dependency is that effect `(bind …)`-ed to a peer interface, a test overrides with a
`(handle …)` or a compile-request `--bind`. ONE concept — an escaping effect the manifest records — for
both host effects and peer dependencies. `extern` is gone.

⚠ The X0–X5d/X4b-4 work below LANDED as the `extern` surface — it is the low-level mechanism the effects
surface now sits on. The transport/envelope/runner/provider stay; the `extern` front-end is removed in U4.

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
- **X4b-4 — runner/CLI delivery. ✅ DONE (`spec`) — DRIVEABLE FROM THE COMMAND LINE.** Two CLI surfaces:
  (1) `cdz compile <prov> --component-name cadenza:pkg/iface` (a `--component-name` flag on
  `rcdzc_cli::CompileArgs` + the `cdz` driver → a `KIND_COMPONENT_NAME` artifact via
  `component_name_artifact`, the `--entry` pattern) publishes a provider's interface; (2) `cdz-run <cons>
  --peer cadenza:pkg/iface=<prov.wasm>` (a repeatable `--peer interface=path`) composes the consumer with
  its peers via `run_with_peers` — resolving the shared runtime if the consumer OR any peer needs it.
  **Proven through the real BINARIES: `cdz compile prov.sexp --component-name cadenza:math/api` + `cdz
  compile cons.sexp` + `cdz-run cons.wasm --call main --arg 5 --peer cadenza:math/api=prov.wasm` → -5; the
  compound case (a tuple crossing, runtime auto-resolved from the store) → 9.** The whole vertical is now
  operator-driveable end-to-end, no Rust harness. (A multi-component CORPUS/gate shape — corpus is
  single-`(input)`, `DESIGN-package-linking.md §8.1` names the same gap — remains a separate tooling task;
  the behavior is proven by the CLI + the Rust integration tests.) ⚠ `.cdz`/`.ml` parse as ML; use `.sexp`
  for the s-expr surface at the CLI.

**X5 — COMPOUND values cross as shared `value` handles (the payoff). IN SUB-BRICKS.**
- **X5a — share ONE runtime instance across consumer + peers. ✅ DONE (`spec`).** `run_with_peers` (X4a)
  gave each peer its own fresh linker/instance; now it instantiates the value-heap runtime ONCE
  (`instantiate_runtime`, split out of `compose_runtime`) and binds it (`bind_runtime_into`) into EVERY
  component that imports it — the consumer AND each peer (they all pin the same content hash → same import
  name). So a handle one produces indexes the ONE shared heap the other reads (component-abi.md §A
  Cross-Component Handle Is Meaningful Only In The Shared Runtime Instance). **Proven (`x5a_*`): a peer
  builds `[42]` on the shared heap and returns the u32 handle; a consumer (also importing the runtime)
  reads element 0 back through the shared instance → 42.** cdz-run refactor + test-only; gate 1932 pass / 0
  fail / 0 regressions (the ordinary single-component run path is unchanged). This is the prerequisite for
  a `value` handle crossing.
- **X5b — a compound value crosses as a shared handle. ✅ DONE (`spec`) — THE PAYOFF.** A runtime
  compound crosses between peers as its opaque `u32` heap handle over the shared runtime (X5a), NO
  serialization. Pieces: (1) `host::extern_abi_val_type` maps a runtime-owned compound
  (tuple/record/sum/list/map/set/string/bytes/bigint/rational + erased nominal/qty) to `AbiValType::U32`
  (the handle), a scalar to its scalar rep — so `collect_extern_imports` carries a compound arg/result as
  the handle instead of dropping it. (2) `envelope::assemble_extern_runtime` — the peer analogue of
  `assemble_host_runtime`: imports the peer interface (as `"peer"`) AND the runtime (as `"heap"`), for a
  consumer that receives a compound handle and INSPECTS it (a projection imports `arr-get`/`get-int`).
  (3) `serialize::core_module_with_extern_runtime` + `core_module_impl` reordered extern-FIRST (`0..e`
  peer, `e..e+k` runtime; extern+host never coexist so host/`CallHostImport` unaffected — byte-neutral,
  gate 1938/0/0). (4) `emit` lifts the extern+runtime decline → routes to the fusion core+envelope. **Proven
  e2e (`x5b_*`): a peer builds a runtime `(tuple x x)` and returns the handle; a SOURCE consumer `(extern
  "cadenza:pairs/api" (pair (-> Int64 (Tuple Int64 Int64)))) … (. (pair x) 0)` receives it as its static
  type over the shared runtime → main(7) = 7.** 🔑 ABI clause softened: a compound crosses as "an opaque
  handle into the shared runtime" (concrete form — runtime handle valtype or a `value` resource — a
  declared-default choice), matching how the runtime already backs handles as bare `u32`. SCOPE: the
  compound crosses as the u32 handle; widening the value matrix + the PROVIDER-side compound export from
  SOURCE (currently the source provider hits the parameterized-heap-return limit; a hand-built peer proved
  the consumer side) are follow-ups (X5c).
- **X5c — provider-side compound export from source. ✅ DONE (`spec`).** A source PROVIDER returning a
  runtime compound now publishes it as a `u32` handle through its interface. Pieces: (1) the host
  resource-escape is SKIPPED for a provider (`db.component_name.is_none()` guard) — a compound crosses to a
  peer as a handle, not the host `list<u8>` escape; (2) the boundary-export loop, for a provider, maps a
  compound result/param to its `u32` handle (`host::extern_abi_val_type`); (3) `envelope::assemble_provider_runtime`
  — a provider whose exports BUILD runtime values (imports the runtime) bundles the lifted funcs into the
  interface instance (the `assemble_with_imports` + `assemble_provider` fusion); `emit` routes a provider to
  it when `imports` non-empty, else the bare `assemble_provider`. **Proven e2e (`x5c_*`): BOTH sides from
  source — a provider `(def (pair (: x Int64)) (tuple x x)) (export pair)` (component-name cadenza:pairs/api)
  + a consumer `(extern "cadenza:pairs/api" (pair (-> Int64 (Tuple Int64 Int64)))) … (. (pair x) 0)` →
  main(9) = 9.** 🪤 byte bug fixed by `wasm-tools print`: the bundled instance is component-instance 1 (the
  imported runtime instance is 0), so export index 1, not 0. Byte-neutral (provider paths fire only when
  `component_name` set — gate 1953/0/0).
- **X5d — value-matrix widening coverage. ✅ DONE (`spec`, test-only).** `extern_abi_val_type` maps every
  runtime-owned type to the u32 handle uniformly, so String/List/Record (and by construction Map/Set/Sum/
  nested) cross the SAME shared-handle way with no new machinery. Locked in by `x5d_*` (all two-source):
  a `(List Int64)` (consumer reads `List.len` → 2), a runtime `String` (consumer reads `String.byte-len`),
  a `(Record (a …) (b …))` (consumer reads field `b`). REMAINING (follow-up, non-blocking): own/borrow
  RECLAMATION across the boundary (a handle a peer produces + the consumer drops — leak-free discipline,
  the `heap_operand_ownership` analogue), and Map/Set/Sum/nested explicit coverage.

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
