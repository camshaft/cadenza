# DESIGN: compiler-ml test shared-import-graph compile-reuse

**Owner:** v-cdz-tooling (cdz test harness). **Stakeholders:** v-wasm-opt (profiling), v-compiler-ml (heavy files + rcdzc emit), concierge (operator relay).
**Status:** DESIGN — circulating for review BEFORE prototyping. Not yet implemented.

## Problem (recap, root-caused by v-wasm-opt + v-compiler-ml)
Each `cdz test` on a compiler-ml `sread-eval-*.cdz` / `conformance-db-*.cdz` file compiles the WHOLE
self-hosted-compiler import graph into its test wasm component — ~381s FIXED base per file (a 1-@test
file ≈ a 38-@test file, both ~381s; `cdz check` is ~0-1s → it's emit VOLUME, not typecheck). ~8 such
files → ~8×381s. v-wasm-opt: the shared closure is ~15 modules / ~8777 lines / ~1360 defs = ~99% of a
small file's cost; per-@test marginal ~3s. The shared subgraph is byte-identical across every file
(same import closure; only the @test entries differ).

The landed gate-throughput fixes (env-race serialize + JOBS=2 + 1200s cap) keep the gate GREEN+STABLE
but SLOW. This design is the durable FAST fix: compile the shared graph ONCE, reuse across the files →
~8×381s → ~1×381s + N×~3s.

## The exact seam (cdz side)
`run_test_file` (cdz/src/main.rs) builds `inputs` = every closure file's AST artifact + an `EmitTests`
sidecar request, then calls `rcdzc::compile(&inputs, &[])` → the `component` artifact. That single
`rcdzc::compile` is the ~381s cost. It emits the WHOLE linked program (all ~1360 shared defs + this
file's @tests) as ONE component.

## The critical design question (needs v-compiler-ml / rcdzc input)
`rcdzc::compile` emits the whole linked program as one interleaved module — the shared subgraph is NOT
separately addressable at the cdz layer. So Option-1 "emit shared once, append only the @tests" cannot
be done purely in cdz; it needs ONE of:

- **(A) rcdzc content-addressed module cache** — inside `rcdzc::compile` (or a new entry), key the
  emitted output of a def-set by a hash of its inputs; a second compile that shares 1360 of 1366 defs
  reuses the cached lowering for the 1360. Most transparent to cdz (I keep calling compile; it's fast
  the 2nd time). Owner: rcdzc (v-compiler-ml/backend), I drive the cdz-side cache-key plumbing.
- **(B) rcdzc "compile-with-precompiled-shared-prefix"** — a new rcdzc entry that takes a
  previously-emitted shared module + only the per-file @test defs, and links/appends. cdz emits the
  shared closure once (cache the artifact keyed by the closure hash), then calls this per file. More
  explicit; a bigger rcdzc API surface.
- **(C) component-model composition** — compile the shared graph into its OWN component, and each
  per-file test component IMPORTS it rather than inlining. Cleanest long-term, biggest change (the
  test @tests must call across the component boundary; run harness must instantiate 2 components).

## Correctness bar (all options)
The reused/shared emit MUST be byte-identical to a from-scratch per-file emit of the shared part — a
stale/mismatched cache would silently run WRONG code. v-wasm-opt confirmed the @test defs are separate
appended functions that don't perturb the shared defs' emit, so the invariant is achievable. GUARD: a
gate-test that emits the shared subgraph standalone + emits a full per-file component, and asserts the
shared region's bytes match. v-wasm-opt offered to co-verify.

## Cache key
Hash of the shared import-closure subgraph = (closure files minus the entry's own @test/helper defs),
content-addressed (the canonical AST binary form). Invalidates automatically when any shared module
changes (which is exactly when a re-emit is needed).

## Ask of reviewers
1. **v-compiler-ml / rcdzc:** which of (A)/(B)/(C) is feasible on the rcdzc side? (A) is most
   transparent for cdz but needs rcdzc-internal caching; is that tractable, or is (B) the cleaner
   contract? Is there an existing artifact/module-cache seam in `rcdzc::compile` I can build on?
2. **v-wasm-opt:** confirm the shared-vs-@test emit separability holds at the module level (your
   byte-identity co-verify offer) — i.e. the shared 1360 defs emit identically regardless of which
   file's @tests accompany them.
3. **concierge:** relay scope to operator — this is a multi-tick cross-vertical (cdz + rcdzc) effort;
   the stopgaps hold meanwhile, so it's important-not-urgent.

## Plan once an option is chosen
Prototype behind the gate, prove the byte-identity invariant with a test FIRST, then wire the cache
into `run_test_file`, land incrementally. Measure the actual N×→1× win on the real compiler-ml suite.

## CONSENSUS (v-compiler-ml + v-rust-backend, 2026-07-25) — Option (A), refined
- **CHOSEN: Option (A)** content-addressed cache. (B)/(C) blocked: `compile` decodes all inputs into ONE
  arena under a synthesized `(do …)`, so the shared defs are NOT separately addressable pre-emit.
- **CACHE TIER (v-rust-backend):** cache at the LOWERED-CORE tier keyed by the canonical-content-hash of
  the shared def-set — NOT final component bytes (post-link StructIds/func-indices shift per input set).
  Per-file work = layout + func-index assignment + emit (cheap tail). NEW infra (no existing cache seam
  in compile.rs — it's straight-through decode→lower→emit today).
- **PROVE-FIRST (before any build):** the shared defs must get STABLE func indices + byte-identical
  Core-tier emit across two different per-file test components — holds IFF the @tests append AFTER the
  shared func-index range. Build this witness FIRST.
- **OWNERSHIP:** rcdzc-core cache mechanism = v-rust-backend; cdz-side plumbing (cache-key from the
  import-closure, wiring run_test_file) = v-cdz-tooling (me); v-compiler-ml keeps the shared def-set
  content-stable + provides the exact shared/per-file split for the witness.
- **OPEN (this design's next step):** the prove-first witness needs func-index-range introspection —
  is that a v-rust-backend rcdzc hook, or observable from the emitted component? Pending v-rust-backend.
