# Option C — the shared import-closure as its own component (compile-reuse, emit-side)

**Status: operator-greenlit 2026-07-28** (concierge relay; operator: "green light everything, no need to
ask me to fund improvements"). Build incrementally, behavior-identical, gate green per increment.

## Problem (measured)

`cdz test <dir>` over the compiler-ml self-host: each per-file `@test` component EMBEDS the whole imported
closure (a `@test` calls `run-src` → the sread-eval interpreter → the ~1360-def closure is reachable), so
the gate cost is **O(tests × closure-size)** on EMIT + wasmtime JIT (>98% of gate wall-time; front-end+lower
is <2%, per v-cdz-tooling's profile). The perf-cliff now costs LANDINGS (b145r conformance-db-cx timed out
under load, non-deterministic). `EmitTestsPerFile` (landed) deduped the LOWER (one Db, memoized) but each
view still EMITS its full reachable closure → doesn't collapse the cliff. Persistent-arena "Stage 2" only
amortizes the <2% → correctly OFF.

## The fix

Emit the shared closure **ONCE** as its own wasm component that EXPORTS its defs under an interface; each
per-file `@test` component IMPORTS that interface instead of embedding the closure. Then a test component's
emit+JIT covers only its `@test` bodies + import shims — the closure is emitted/JIT'd once, not N times.

**Transport is already solved (risk retired):** peer-linking. `component-abi.md §"A Cross-Component Handle
Is Meaningful Only In The Shared Runtime Instance"` + `cdz-run::run_with_peers`/`run_with_peers_hosted`
(lib.rs:458/470) bind every component's value-heap-runtime import to ONE shared runtime instance, so heap
handles (CHAMP/rope, opaque i32s) cross freely — the closure-component and each @test-component index the
same heap. Option C's shape IS the consumer↔peer model peer-linking runs today: the @test component
(consumer) imports the shared-closure component (peer)'s exported interface.

## Emit-side mechanism (my lane), riding existing provider/consumer emit

- A PROVIDER component (`db.component_name` set) already publishes its exports to peers over the shared
  runtime, compound results crossing as `u32` handles through the provider interface (wasm/mod.rs:195).
- A CONSUMER already imports a peer's interface via `(bind …)` → `db.effect_bindings`; a bound call is a PEER
  call, not a host call (wasm/mod.rs:484-495).
- Option C = the shared closure emitted as a PROVIDER exporting its reachable defs; each @test component a
  CONSUMER whose `Core::Call` edges INTO shared defs are routed as peer calls (callee → an imported func).

## ⚠ THE REAL RISK — X5b (do the witness FIRST, before broad consumer emit)

`run_with_peers` establishes X5a (the shared runtime INSTANCE — proven, handles index one heap). But
`cdz-run` lib.rs:450 scopes it "scalar peer ops today; a value-handle op RIDES this shared instance" —
**X5b (a value-HANDLE crossing a PEER-INTERFACE edge) is the frontier, possibly unproven.** Host-op
String/rope handles cross today (lib.rs:605/655), so the transport exists — but a PEER-INTERFACE call
returning a List/rope handle (which closure defs do) may need run-side work. If so, that's v-cdz-tooling's
increment and it GATES Option C. So the FIRST thing to build is a minimal handle-crossing witness, split:
- **v-rust-backend (emit witness):** a `@test` calls a shared-closure fn that RETURNS a List/rope handle;
  emit the closure-component + @test-component pair; assert it emits clean (validates).
- **v-cdz-tooling (run witness):** `run_with_peers` instantiate closure-peer + @test-consumer, the cross-
  edge returns a handle, assert the @test reads the RIGHT value through the shared heap.
Do BOTH early. If X5b is clean → proceed to broad (c). If it needs run-side work → v-cdz-tooling's gate first.

**✅ 2026-07-28 X5b RESOLVED CLEAN (both halves) — does NOT gate C.** EMIT half (v-rust-backend, `0e12b38d5`):
a List-returning provider op + a List-consuming consumer both emit VALID components via existing machinery.
RUN half (v-cdz-tooling, `3c6ee2f36`): `run_with_peers` over that pair — a variable-length List HANDLE crosses
the peer-interface edge and `List.len` dereferences it correctly through the ONE shared runtime instance
(main(5)→3), NO run-side work. The lib.rs:450 "scalar peer ops today" note was conservative; a List/rope
(CHAMP/RRB) handle rides the shared instance fine (as U6's tuple did). ⇒ GO on (a)/(b)/(c) broad.

## Interface-name emission (decided w/ v-cdz-tooling)

DISCOVER the funcs (run_with_peers forwards a peer's exported-interface funcs off the instance type, lib.rs:445
— no hard-coded list). But `Peer{interface: <name>}` needs the interface NAME string. So: emit the closure's
export interface under a name v-cdz-tooling can READ — either a `KIND_COMPONENT_NAME`-style sidecar naming the
closure's export interface (cleanest) or off the component type. NOT hard-coded in either lane. ONE interface =
the whole shared closure (granularity decided).

## Increments (each behavior-identical, gated, landable) — REORDERED: X5b witness first

1. **(a) PARTITION** — split the reachable-def set into `shared-closure` vs `per-file @test bodies`.
   Extends the `file_of(sig_occ)` bucketing already in `EmitTestsPerFile`: a def whose file is an IMPORTED
   closure file (not the entry/@test file) is shared; a @test body + its file-local helpers are per-file.
   Pure analysis, testable in isolation (assert the partition on the 2-file toy + a sread-eval-shaped fixture)
   BEFORE any cross-component emit. **← increment 1, start here.**
2. **(b) PROVIDER emit** — emit the shared-closure partition as a component exporting its defs under a
   generated interface (reuse the `component_name` provider path; the export set is the shared defs, not just
   top-level `(export …)`). Witness: it validates + its exported funcs are callable.
3. **(c) CONSUMER emit** — each @test component imports the closure interface; route `Core::Call` edges into
   shared defs as peer calls (the `effect_bindings` peer-call path). Witness (v-cdz-tooling flagged): a
   handle-returning cross-component edge (today's shared-instance scope note says "scalar peer ops"; a
   value-handle op is called out as supported but wants a witness).
4. **(d) DRIVER (v-cdz-tooling)** — `run_test` instantiates the ONE shared-closure component as a peer +
   links N @test components against it (`run_with_peers`, closure as the peer every test imports). Their
   lane; co-design the interface shape (what the provider exports ↔ what each consumer imports).

## Increment (b) recon — the provider-export ABI wall (2026-07-28)

The provider path exports `layout.exports` as interface funcs; each export's params + result must have a
cross-component boundary rep via `host::extern_abi_val_type` (wasm/host.rs:109). Coverage: scalars (by
value) + EVERY heap compound (tuple/record/sum/list/map/set/String/Bytes/BigInt/Rational + erased nominal)
as an opaque `u32` handle. Returns **None for `Unit` and a bare FUNCTION type**. So:
- Increment (b) builds a provider layout whose "exports" are the `shared` partition defs (a `compute`-like
  fn per shared def, mirroring `compute_tests_for`'s ExportPlan-per-def), then routes it through the existing
  provider emit.
- 🪤 **OPEN CONSTRAINT for (b)/(c):** a shared def that TAKES or RETURNS a bare function value (a
  higher-order def) has NO extern ABI rep → can't cross the peer interface. If the compiler-ml closure has
  higher-order shared defs, those can't be shared-component-exported — they'd have to stay per-file
  (emitted in each @test component), or the closure boundary needs a function-handle ABI (bigger). MUST
  CHECK: does the sread-eval closure export higher-order defs across the file boundary? (A def only called
  WITHIN the closure isn't an export edge — only cross-file `Core::Call` edges into shared defs need an
  interface func; a higher-order def called only intra-closure is fine, emitted inside the provider.) So the
  interface = the shared defs that a PER-FILE @test component actually calls (the cross-component call
  edges), not ALL shared defs. Partition (a) gives shared-vs-own; (b) further needs the cross-edge set =
  shared defs called from `own` defs. Refine (b): export only the shared defs on a cross-file call edge;
  a higher-order such edge is the constraint to surface (likely rare — closures are usually called with
  scalar/heap args, returning heap values, per the X5b witness).

## Increment (c) recon — the CONSUMER-emit fork (2026-07-28)

Layout+provider side DONE: (a) partition, (b)(i) cross_component_edges, (b)(ii) compute_shared_closure_
provider, (b)(iii) provider emits a valid component (source-named boundary via `source_boundary_name`).
Increment (c) is the hard core: each per-file @test component must (1) NOT emit the shared cross-edge defs
(they live in the provider — the whole point), and (2) route its `Core::Call`s into those defs through the
IMPORTED interface. 🪤 The existing consumer machinery (`db.effect_bindings`, wasm/mod.rs:490) routes an
escaping EFFECT OP to a peer import (`ExternImport{interface,op,params,result}` — effect-op-shaped, emitted
via `CallImport`). But a cross-edge is a plain `Core::Call` to a DEF, not an effect perform — a different
shape. There is NO "external def" concept for a plain `Core::Call` today. So (c) forks:
- **(A) Core-rewrite:** rewrite each cross-edge `Core::Call{callee: shared_def}` into a peer-op call
  (synthesize an effect-op-style import named `source_boundary_name(shared_def)` on the closure interface).
  Reuses the whole existing peer-import emit; the rewrite is the new work. Risk: threading a synthesized
  effect through the consumer layout cleanly.
- **(B) emit-level external-def:** teach layout/select to treat the cross-edge def-set as IMPORTED funcs
  (exclude from the emitted/reachable set; emit an interface import per edge; resolve the `Core::Call`'s
  `CallImport` to it). More localized to the backend, no Core rewrite, but adds an "external def" notion to
  select's call emit + the layout's import set.
✅ **DECISION LOCKED 2026-07-28: (B).** v-cdz-tooling confirmed their (d) run-link is A/B-agnostic (run_with_peers
binds by interface NAME, funcs discovered off the peer type) AND gave a run-specific reason to prefer (B):
under (A) each cross-edge becomes a synthesized EFFECT-OP performance, which POLLUTES their observed-host-op
list (breaks property-test gen-int counting + trap-vs-fail diagnosis); a plain `CallImport` (B) is INVISIBLE
to the effect machinery. So (B) is chosen. v-wasm-opt heads-up'd on the select call-emit change.
🔑 (historical rationale) Leaning (B): no Core mutation (the Core stays the honest
program; only the EMIT differs per-component), localizes Option C to the backend, and the cross-edge set +
`source_boundary_name` I already have feed it directly. But (B) touches select's call-emit (v-wasm-opt-
adjacent). Co-design the mechanism with v-cdz-tooling (their run_test driver (d) links against whatever
interface shape (c) emits) + likely a v-wasm-opt heads-up on the select call-emit change. This is the
increment to get right — surface the fork, don't rush.

## ⚠ (c) CORRECTNESS CONSTRAINT — cross-edge import index MUST match provider export order (v-wasm-opt, 2026-07-28)

The consumer's cross-edge `CallImport` index MUST resolve to the SAME import the provider EXPORTS — the
collect≡emit import-set agreement (the FINDING #22 shifted-index-invalid-module family). A cross-edge
CallImport whose index disagrees with the provider's export ORDER = an invalid module, and ONLY a
MULTI-cross-edge witness catches it (a single edge can't expose an ordering mismatch). So the (c) build MUST:
(1) order the consumer's imported interface funcs to MATCH the provider's exported order (both derived from
the same cross-edge set in the same `layout.order` order — keep ONE canonical ordering, don't re-derive
independently); (2) include a MULTI-cross-edge test (a @test calling ≥2 shared defs) so a mis-ordered import
index is caught. v-wasm-opt will co-review the select.rs diff + run the drift-guard battery (opt-sweep
0-divergence + a cross-edge-vs-local emit witness confirming a NON-cross-edge Core::Call is byte-identical
pre/post — the guarded path leaves existing calls untouched) when the (c) MR is up.

## (c)(B) recon — REUSE the extern_order machinery (2026-07-28)

The consumer emit (B) can ride the EXISTING extern-import infrastructure, sharpening the build:
- `Layout.extern_order: Vec<(String,String)>` (interface,op pairs) + `with_extern_order` + the extern
  CallImport emit already exist — fed today by `db.effect_bindings` (an escaping effect op → `extern_imports`
  → `extern_order`), NOT a plain `Core::Call` (`Core::ExternCall` was REMOVED in U4). So the peer-import
  section + index resolution machinery is live and reusable.
- (c)(B) = at select.rs:10089 (`Core::Call` → `Lir::Call(layout.abs(callee))`), add: if `callee` ∈ the
  cross-edge set, resolve `(closure_interface, source_boundary_name(callee))` to its `extern_order` index
  and emit `Lir::CallImport` (the extern path) INSTEAD of `Lir::Call`; the callee is excluded from the
  consumer's `order` (not emitted). The consumer layout adds the cross-edges to `extern_order` (interface =
  the closure component-name; op = source_boundary_name), and EXCLUDES them from `order`'s reachability
  closure (finish_layout's worklist must STOP at a cross-edge callee — record it as an extern import, don't
  add it to `order`).
- ⚠ INDEX-AGREEMENT (v-wasm-opt): the consumer's extern_order for the cross-edges MUST be in the SAME
  canonical order as the provider's export set (both from the cross-edge set in `layout.order` order) — a
  mismatch = invalid module, multi-cross-edge witness catches it.
- SUB-STEPS: (i) consumer layout — finish_layout variant that excludes cross-edges from `order` + adds them
  to extern_order (canonical order); (ii) select — the Core::Call external-def branch → extern CallImport;
  (iii) the multi-cross-edge witness + v-wasm-opt drift-guard co-review. Each gate-able.

## (c)(ii) recon — the select external-def branch (2026-07-28)

The peer-call pattern to mirror is at select.rs:10881 (`Core::HostCall` bound to a peer): `layout.extern_index(iface, op)` → emit args → `Lir::CallExternImport(index)`. The cross-edge `Core::Call` branch mirrors it exactly. Insertion at select.rs:~10089 (`Core::Call{callee,args}` → `layout.abs(callee)` → `Lir::Call`):
- IF `callee` is a cross-edge (imported), resolve `layout.extern_index(closure_iface, source_boundary_name(callee_name))` → emit args → `Lir::CallExternImport(index)` + return; ELSE the existing `layout.abs`→`Lir::Call`.
🔑 select needs an O(1) "is `callee` a cross-edge + what's its extern index?" — `extern_order` is `(iface,op)` STRINGS but select has a `callee` DEF INDEX. So add a `Layout` field `cross_edge_import: HashMap<usize (def), usize (extern_order position)>` (empty for a non-consumer layout → the branch never fires → byte-identical). `compute_tests_consumer` populates it: for each `boundary_hit` def (in CANONICAL layout.order order — the index-agreement invariant), push `(closure_iface, source_boundary_name(def_name))` to `extern_order` + map `def → its extern position`. The closure_iface name is the SAME the provider publishes (a fixed convention or passed in — co-designed w/ v-cdz-tooling's Peer{interface}).
- SUB-STEPS: (c)(ii-a) Layout.cross_edge_import field + compute_tests_consumer populates extern_order + the map (layout-side, testable: consumer layout's extern_order lists the cross-edge, map resolves def→index); (c)(ii-b) select Core::Call external-def branch (the CallExternImport emit); (c)(ii-c) the emit driver (compile.rs) routes a consumer @test build through compute_tests_consumer + emits the consumer component; (c)(ii-d) multi-cross-edge witness + v-wasm-opt index-agreement co-review. Each gate-able.
- ⚠ the closure_iface NAME must match the provider's export interface name (both a fixed convention, e.g. from db.component_name on the provider side) — the provider↔consumer contract. Decide the convention when wiring (ii-a)/(ii-c).

## Invariants

- BEHAVIOR-IDENTICAL: same test pass/fail, same located diagnostics — only the emission SHAPE changes
  (shared-closure + importing @test components vs each embedding the closure). Gate green per increment.
- Stage 2 (persistent-arena gate restructure) stays OFF — Option C is emit-side, needs no gate change.
- Stopgaps (JOBS=2 + 1200s cap) hold until Option C lands.

## Co-design split
- **v-rust-backend (me):** (a) partition, (b) provider emit, (c) consumer cross-component call edges.
- **v-cdz-tooling:** (d) run_test multi-component instantiate/link (closure as peer); the interface-shape
  contract between provider export ↔ consumer import.
