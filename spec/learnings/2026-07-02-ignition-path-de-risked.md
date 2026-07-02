# The ignition path is real in this environment; the gate mechanism works for Rust

*2026-07-02*

**What happened.** Before synthesizing the Rust seed, two throwaway spikes de-risked the load-bearing
assumptions the whole toolchain rests on (build.md Phase 2). Both passed, and their findings are
recorded here so a later build inherits the resolved assumptions instead of re-running the spikes.

*Spike 1 — the cross-language gate.* A two-requirement markdown spec, cited from Rust with duvet's
`//=` / `//#` markers (implementation and `type=test`), reported all citations matched. Rewording one
requirement's sentence ("sum" → "total") made duvet hard-error with
`× could not find text in section "addition-is-correct" of spec/toy.md`, naming **both** stale
citations at their exact source line (`src/main.rs:4:5`, `:25:9`) — proving the quoted-sentence
identity model the conformance gate depends on works for Rust exactly as for the spec tree.

*Spike 2 — reproducible derivation (the ignition bar in miniature).* A trivial Cadenza program built
as a binary AST (the `[container-version, prelude, root]` triple, canonical prelude order), then:
derived by embedding a tiny reference interpreter over the AST into a runnable wasm artifact whose
imports mirror the program's manifest exactly; run in-process with the host `emit-event` import bound,
observing the emitted event; checked against the interpreter (oracle) — they agreed; and re-derived,
producing a byte-identical artifact (same SHA-256). The full source→derive→run→agree→re-derive path is
real and executed in this environment, not modeled.

**Why it matters (the non-obvious findings, so we do not relearn them):**

- **duvet exits 0 even when citations fail to match.** Spike 1's stale-citation run printed two
  `could not find text` errors and `encountered 2 errors`, yet the process exit code was `0`. A gate
  runner MUST parse duvet's output (or the JSON report) for `could not find text` / `encountered N
  errors` rather than trust `$?`. (This complements the `//#`-marker gotcha already in
  `commands/setup-gate.md`.)
- **The runtime is embeddable here and the component model is available.** The `wasmtime` crate
  (v37, `runtime` + `cranelift` + `component-model`, `default-features = false`) compiles and runs
  in-process; `wasmtime::component::Component::new` builds a trivial component, so the seed's real
  target (a wasm component, not just a core module) is reachable. The `wasmtime` **CLI** is not needed
  and is absent — "embeddable component-model runtime" (options/execution-model/) means the library.
- **Two Rust/wasmtime mechanics the seed will hit:** with `default-features = false`, `Module::new`
  wants wasm **bytes**, so compile text with the `wat` crate first (a bare WAT string fails with a
  "magic header" error). Host functions passed to `Linker::func_wrap` must be `Send + Sync`, so thread
  observation state through `Store<T>` and `Caller::data_mut()` rather than a captured
  `Rc<RefCell<…>>`.
- **Reproducibility is immediate when codegen order is source-determined.** Emitting imports in
  canonical (sorted) manifest order and baking the interpreter's result deterministically gave a
  byte-identical re-derivation with no extra effort — confirming reproducible-derivation.md's
  "codegen order is source-determined" is a cheap invariant to hold when the derivation is a pure
  function of the (already canonical) AST.

**The requirement it drove.** No spec change — the spikes *confirmed* the existing contracts are
realizable as written (build-tool-interface.md §"Derivation By Embedding The Reference Interpreter",
host-interface-binding.md §"Imports Mirror The Manifest Exactly", reproducible-derivation.md,
conformance-gate.md). Their operational findings are recorded here and mirrored in
`implementation/DECISIONS.md` so the seed synthesis (Phase 3) starts from proven ground rather than
re-derived assumptions. The throwaway spike code lived under `implementation/spikes/` (gitignored) and
is not part of the seed.
