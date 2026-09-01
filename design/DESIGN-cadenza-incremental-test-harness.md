# DESIGN: cadenza incremental test harness (nix layer)

**Owner:** v-nix (CI / nix / gate layer). **Stakeholders:** v-cdz-tooling (owns the `cdz test` CLI
surface + `run_test_file`), v-fleet-tooling (gate integration + pr-sync lane wiring), v-compiler-ml +
v-rust-backend (own the rcdzc-internal compile cache, tracked separately in
`DESIGN-compiler-ml-test-shared-graph-compile-reuse.md`), concierge (operator relay).
**Status:** v1a per-project split QUEUED (candidate #2876); coordination RESOLVED (both peers answered);
v1b-α (`--warm-only` decouple) mechanism prototyped + validated, wiring pending #2876 merge; v1b-β
(new flags) + v2 deferred. Not blocking.

## Scope + non-scope (read this first — two DIFFERENT caches)
This design covers the **nix/derivation layer**: how the CI test derivations are structured so that an
unchanged input does not rerun work. It is DELIBERATELY separate from the rcdzc-internal shared-graph
compile cache (`DESIGN-compiler-ml-test-shared-graph-compile-reuse.md`, owner v-rust-backend +
v-cdz-tooling), which attacks the ~381s-per-file compile cost INSIDE a single `cdz test` invocation by
content-addressing the lowered-core emit of the shared import closure. The two compose:

- **rcdzc cache (theirs):** makes ONE `cdz test <dir>` run fast internally (shared defs lowered once).
- **nix harness (this):** makes the CI GRAPH incremental — a project/harness change that does not touch
  a given derivation's inputs does not rerun that derivation at all.

Neither subsumes the other. This doc does NOT propose a second compile cache; where compile reuse is
needed it defers to their design.

## Problem (nix layer)
The `cad-tests` job ran `cdz test` on all 4 in-tree projects (cad / compiler-ml / choreography /
iterators) from ONE union `src`, so any one-line edit to any project reran all 4 (~35m, dominated by
compiler-ml's shared-closure emit). More broadly, EVERY `cdz test` derivation today couples three
phases into one non-incremental unit:

1. resolve the seed compiler + component store (already crane-cached — good),
2. COMPILE the project: partition its `@test` files by shared import-closure, emit each closure's
   PROVIDER component + JIT (`.cwasm`) ONCE per group, plus each file's thin consumer (the expensive
   part is the per-group provider codegen — ~230s per group for the compiler-ml self-host closure,
   #1502),
3. RUN the compiled components under wasmtime and collect PASS/FAIL.

IMPORTANT (code-verified, `run_test_file` / `precompile_group` L4013-4130): the per-FILE redundant emit
is ALREADY eliminated in-process — a single `cdz test <dir>` emits each shared closure's provider once
per group (not once per file) and every file's consumer links against it. There is also a cross-INVOCATION
provider cache (`CDZ_PROVIDER_CACHE`, `<hash>.provider.wasm` + `.cwasm`) that lets a SECOND process skip
the ~230s provider codegen on a content-hash HIT. So the remaining nix-layer problem is NOT per-file
redundancy; it is that (2) and (3) live in one derivation keyed by project source over an EMPTY provider
cache: every CI `cad-tests` run starts cold (the nix store output is fresh + immutable, so the provider
cache never persists across derivations), re-paying the per-group provider codegen every time, and a
change affecting ONLY the run path (wasmtime bump, runtime store, run-harness tweak) still re-pays the
whole compile.

## v1a — per-project split (LANDED, slice (b), candidate #2876)
Split the monolithic `cad-tests` into one derivation per project, each with a narrow own-dir fileset;
`cad-tests` is now an aggregate over the 4 (required context name unchanged, no ruleset edit); the 4
are also exposed individually. A one-project edit reruns only that project (proven: editing `iterators`
leaves `cad-test-cad`'s drv byte-identical). ~4x on the common single-project-change case. This is the
cheap, landed win; everything below is the follow-on.

## v1b — compile/run decouple (this slice, for review)
The lever is the EXISTING cross-invocation provider cache: give the run phase a WARM provider cache
built by a separately-cached compile derivation, so a run-only change reuses it instead of re-paying the
per-group provider codegen. Two staged forms:

**v1b-α (zero new flags, prototype-first — v-cdz-tooling's advice):** split each per-project test into:
- **`cdz-test-compile-<project>`** — runs `cdz test --warm-only <dir>` with `CDZ_PROVIDER_CACHE=$out`:
  emits + JITs every closure group's provider ONCE and exits without running tests (the ~230s/group
  cost). Inputs: project source + seed compiler + component store. MUST root the seed-compiler output
  (v-fleet-tooling caveat: `cad-tests` feeds warm-cache roots + the crane deps-layer).
- **`cdz-test-run-<project>`** — depends on the compile `$out`, runs the normal per-file sweep with
  `CDZ_PROVIDER_CACHE=<compile-out>` so the provider codegen HITs the warm cache (consumer-only, cheap).
This decouples the dominant shared-closure cost TODAY. Its limit (verified in the tick-(dq) prototype):
the run still pays the per-file CONSUMER mono + any standalone-fallback emit, since those are NOT in the
provider cache — so v1b-α is a partial, not total, decouple. Prototype this first to de-risk the nix
wiring; it may be enough for the headline payoff on shared-closure-heavy projects (compiler-ml).

**v1b-β (new flags, closes the gap — v-cdz-tooling to spec):**
- **`cdz-test-compile-<project>`** — `cdz test --compile-only <dir> -o $out`: emits providers +
  consumers + standalone fallbacks + a manifest (`compiled.json`) into `$out`; never runs.
- **`cdz-test-run-<project>`** — `cdz test --run-precompiled $out`: loads the manifest, RE-HASH-VERIFIES
  every component against its closure-hash sidecar (FAIL CLOSED on mismatch/missing), runs, collects
  PASS/FAIL — with NO compiler in the derivation's closure at all.
The β run emits ZERO (not just skips the provider), gives a hermetic self-contained `$out`, and makes
the fail-closed verify a first-class contract. Land β after α proves the wiring.

Payoff: a change that affects ONLY the run path (wasmtime version bump, run-harness tweak, the
value-heap runtime store) reruns `run` but HITS the cached `compile` — no recompile. Conversely a
source edit reruns `compile` (unavoidable) but the split keeps the run phase honest and separately
cacheable. The `cdz-test-<project>` aggregate depends on `run`, which depends on `compile`.

### Correctness bar
The split path (warm-then-run for α, compile-then-run-precompiled for β) MUST produce identical results
to inline `cdz test <dir>` — same closure, same provider bytes, same content-address, same PASS/FAIL.
This holds by construction because the split reuses the exact same emit code paths inline uses (the
provider cache is content-addressed + validate-on-load). Guard: a gate test that runs a project BOTH
inline and split, asserting identical PASS/FAIL counts + identical component content hashes. PIN
(v-cdz-tooling): run the inline baseline with a CLEAN `CDZ_PROVIDER_CACHE` (or assert the hash) so the
comparison is not inline-HIT vs split-MISS noise. β's `--run-precompiled` additionally FAILS CLOSED on
any hash mismatch/missing component (mirrors the store load-verify). Build this gate FIRST, before
wiring either derivation.

## v2 — function/coverage-keyed skip (aspirational, NOT v1)
Skip a project's test rerun entirely when its inputs are unchanged AT THE @test-DEFINITION granularity
(not just file granularity): key each `@test`'s rerun on the content hash of its own def PLUS its
reachable closure, so editing an unrelated def in the same file does not rerun untouched tests. This is
hard (needs per-@test reachable-closure introspection — the same func-index-range introspection the
rcdzc design's prove-first witness needs) and depends on that design landing first. Explicitly deferred
so v1 is buildable now without it.

## Coordination (RESOLVED — both peers answered 2026-08-09)
- **v-cdz-tooling:** decouple confirmed TRACTABLE + squarely their surface. Corrected my initial framing:
  the cross-invocation cache persists ONLY the provider, not consumers/standalone-fallbacks — so a
  `--compile-only` cannot just copy the provider. Advised: prototype the ZERO-new-flags v1b-α
  (`--warm-only` + `CDZ_PROVIDER_CACHE`, both already landed) FIRST to de-risk the nix wiring, THEN they
  spec the v1b-β flags (`--compile-only -o <out>` / `--run-precompiled <out>`) as a dedicated MR against
  known-good wiring. Ownership: they own flag semantics + the `run_test_file` seam; I own the derivation
  wiring + the byte-identity gate; wire them the gate when it's up and they co-verify.
- **v-fleet-tooling:** GO — `cad-tests` is NOT a required context (ruleset 10560470 required set has 11
  jobs, `cad-tests` absent → advisory). The only `cad-tests` reference in fleet.rs is an incidental
  /tmp-cleanup comment; the gate targets the `local-gate` aggregate and lanes are file-path-based, so my
  internal derivation restructuring is transparent. CAVEAT: the v1b compile derivation MUST still root
  the seed-compiler output (it feeds warm-cache roots + the crane deps-layer) or we reopen the
  stale-warm cold-rebuild we closed via crane.
- **concierge:** approved me leading — v1a landed (the ~4x quick win, candidate #2876); v1b-α is the
  durable decouple; v1b-β + v2 defer to v-cdz-tooling's flags + the rcdzc compile-cache design.
  Important-not-urgent; additive derivations, aggregate context unchanged, zero ruleset churn.

## Prototype evidence (tick dq, 2026-08-09)
Prototyped v1b-α on compiler-ml with the release binary against the nix component store. The mechanism
VALIDATED: `--warm-only` populated a content-addressed cache (16 providers + `.cwasm` JIT), and a
subsequent run HIT it with zero re-write during the run. v-cdz-tooling's ConsumerOnly-mono correction is
confirmed straight from the code. Clean wall-clock could NOT be measured in the hand-built prototype: my
release binary emits a runtime content-address absent from the nix store (so most heap-value tests
errored, 19 vs 779 passed) and there was heavy concurrent fleet load. That store mismatch CANNOT occur
in the real wiring (compile + run derivations share the same seed-compiler + component-store inputs, so
binary + store agree by construction) — the honest number must come from the in-derivation measurement.

## Plan
1. v1a per-project split — QUEUED (#2876). On merge, sync + land this doc.
2. v1b-α (`--warm-only` decouple, zero new flags) — mechanism prototyped + validated. Build the
   byte-identity gate FIRST, then wire the compile (`--warm-only`, root the seed-compiler output) + run
   (cache-hit sweep) derivations, measure the honest in-derivation number, land incrementally.
3. v1b-β (new flags) — ping v-cdz-tooling to spec `--compile-only`/`--run-precompiled` against the
   known-good α wiring; they build the flags, I wire the hermetic-`$out` derivations + fail-closed gate.
4. v2 — deferred behind the rcdzc compile-cache design's func-index introspection.
