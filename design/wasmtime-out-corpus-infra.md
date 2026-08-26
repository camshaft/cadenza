# Corpus-infra extension for the wasmtime-out hard tail

## Why

The wasmtime-migration campaign moves every wasmtime-using rcdzc `#[test]` out of the
compiler crate so rcdzc can drop its `wasmtime` dev-dependency. Behavioral coverage is
tested once end-to-end in the corpus: `cdz` compiles a case, `cdz-run` executes it (that
crate legitimately keeps wasmtime), and the recorded outcome is compared. Most tests
migrate as ordinary value/trap cases.

Two classes cannot: their assertion is not a returned value.

1. Memory-liveness probes (~93 sites, `tests.rs` 7056-13026). Each composes the program
   with the `debug-counters` runtime, runs an export, and asserts `rt.live_objects() == N`
   (a heap-balance invariant — no leak, no double-free). The observable is a runtime
   counter, not the program's result.

2. Resource-protocol drives (~40 sites, `tests.rs` 87011-92213, `closure_host_resource`).
   Each drives a host-held resource protocol — `make() -> handle`, then
   `call(handle, args)` one or more times, then `drop(handle)` so the dtor fires — and
   observes both the per-call results and (often) the post-drop `live_objects() == 0`.
   A single `(call export args)` corpus case cannot express the make/call/drop sequence.

`use-wasmtime = 0` in rcdzc (and dropping the dev-dep) is blocked on these until the
corpus can express both. This is the operator-mandated path: compiler tests go in the
corpus; where the corpus lacks the functionality, extend the corpus definition and
infrastructure.

## What exists today

- `cdz-run::run(component, opts)` takes `RunOpts { runtime: Option<Vec<u8>>, .. }`. The
  runtime component is INJECTED by the caller and instantiated via `instantiate_runtime`.
  Passing the `debug-counters` runtime instead of the shipped one is already supported by
  the composition path.
- The shipped runtime's `live-objects` heap export returns 0 unconditionally; the
  `debug-counters` build returns the real live-cell count. It is located by
  `DEBUG_RUNTIME_HASH` in the content-addressed store (built by `cargo xtask build`).
- The rcdzc test helper `ComposedRuntime` (tests.rs) already implements exactly the two
  drives we need to lift into `cdz-run`: `call(name, args)` + `live_objects()`, and
  `run_escape_and_drop()` (the make/encode/drop resource sequence). These are the
  reference implementations for the two extensions.
- The corpus record (`cdz-corpus::Record`) already carries per-case clauses beyond the
  value form (`wit_world`, `component_name`, `host_calls`, `host_responses`), and the
  xtask gate driver forwards them to `cdz-run` flags. The two new clauses follow that
  pattern.
- `v-platform-itest`'s arg-probe/value-capture (branch `platform-itest/arg-probe-slice`)
  drives typed args through a component-model boundary and captures observed values. Its
  drive machinery overlaps the resource-protocol drive; the resource extension shares it
  rather than building a parallel driver.

## Extension 1: live-object-balance assertion

A corpus case clause asserting the runtime's live-cell count after the run.

- Corpus surface: a `(live-objects N)` clause on a case (alongside or after its
  `(call ..)`), meaning "run to completion on the debug-counters runtime, then the heap's
  `live-objects` export reads N". N is almost always 0 (balanced); a `_known_gap` case may
  assert a specific nonzero N.
- `cdz-corpus`: add a `live_objects: Option<u32>` field to `Record`, parsed from the
  clause and rendered into the flat record stream (like `host_responses`).
- xtask gate driver: when a case carries `live-objects`, drive `cdz-run` with the
  `debug-counters` runtime (not the shipped one) and a `--report-live-objects` mode, then
  compare the reported count to N.
- `cdz-run`: extend `run`/`RunOpts` (or a sibling entry) so that after the normal run it
  reads the heap `live-objects` export and returns it (a new `Outcome` field or a distinct
  return). The composition and read already exist in `ComposedRuntime::live_objects`; this
  lifts them into the crate.
- Caching (v-nix): the exec derivation is keyed on `{artifact, expect}`. A `live-objects`
  case's expect now includes N AND the fact that it runs on the debug-counters runtime, so
  the key must incorporate the runtime identity (the debug-counters hash) — a case that
  runs on the debug runtime must not collide with the same artifact run on the shipped one.

## Extension 2: resource-protocol drive

A corpus case clause expressing the make/call/drop sequence for a host-held resource.

- Corpus surface: a clause naming the protocol steps, e.g.
  `(resource-drive (make) (call len ()) (call len ()) (drop))` with the per-step expected
  results and an optional trailing `(live-objects 0)`. The exact grammar is settled with
  v-platform-itest so it matches their capture shape.
- `cdz-run`: a `run_resource_protocol` entry that reaches the resource instance
  (`cadenza:run/run` or the closure-host instance), calls `make()` to get a handle, calls
  the named member(s) with args, drops the handle so the dtor fires, and returns the
  per-call rendered results (+ live-objects). `ComposedRuntime::run_escape_and_drop` is the
  reference; generalize it from the fixed make/encode/drop to a caller-named step list.
- Shared drive (v-platform-itest): the typed-arg coercion and value-capture used to drive
  `call(handle, args)` and render results is the same machinery arg-probe already has.
  Reuse it; do not fork a second typed-drive.

## Coordination

- v-nix owns the corpus gate/runner build and per-case caching. Loop in on: the
  debug-counters runtime selection in the exec phase, and keying the exec derivation on the
  runtime identity so `live-objects` cases cache correctly and do not collide with
  shipped-runtime runs.
- v-platform-itest owns the arg-probe/value-capture drive. Extension 2 reuses it for the
  `call(handle, args)` step and result rendering.
- v-wasmtime-migration (this vertical) leads: designs and builds the `cdz-corpus` clause
  parsing, the xtask gate wiring, and the `cdz-run` capabilities, then migrates the ~133
  hard-tail tests onto the new clauses and drops the rcdzc test helpers + the wasmtime
  dev-dep.

## Phasing

1. Extension 1 (live-object-balance) first — it is simpler (no new sequencing, just a
   post-run counter read) and unblocks the larger ~93-site block. Land the `cdz-run`
   capability + `cdz-corpus` clause + gate wiring + v-nix caching key, then migrate the
   liveness tests in batches.
2. Extension 2 (resource-protocol drive) second, co-designed with v-platform-itest, then
   migrate the ~40 resource/closure-host tests.
3. Once both classes are migrated and the raw-ABI hand-built-core-wasm tests (764-1059)
   are relocated or retired, rcdzc has zero wasmtime usage and the dev-dep is dropped.
