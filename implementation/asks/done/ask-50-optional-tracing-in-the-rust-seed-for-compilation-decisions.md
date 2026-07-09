## 50. 🟡 Add OPTIONAL `tracing` to the Rust seed compiler for compilation decisions — feature-gated OFF so the wasm build is untouched

**Operator direction (2026-07-07).** "Add `tracing` to the Rust compiler so we can get traces out for
compilation decisions. Make it optional since we probably don't want that when compiling it to wasm."

**Why it matters.** The conformance loop currently root-causes declines *blind* — it edits a probe program,
re-runs `emit`/`compile-run`, reads the one-line `declined: …` message, and reverse-engineers *which* codegen
path produced it. The compiler makes hundreds of silent decisions per compile (kind inference, scalar-vs-runtime
retry, payload-shape recovery, path selection at every `gen_*`), and the loop can only see the final decline
string. A trace of the decision path — "inferred return kind Heap here, took the runtime retry, declined at
`gen_member` because shape was None" — would collapse most ask investigations from many probe iterations to one.
It is the same "make the compiler's internal reasoning observable" theme as ask-48's machine-branchable KIND, but
for the DEV desk rather than the diagnostics ABI.

---

### The purity constraint (why "optional" is load-bearing, and how it's satisfied)

`crates/cdz-compiler` is a **pure `ast_bytes → component_bytes` core** with only two deps (`ciborium`,
`unicode-normalization`), deliberately kept wasm32-targetable so it can be wrapped as the
`cdz-compiler-component` and byte-checked against the Cadenza-authored compiler. Any tracing MUST NOT appear in
the wasm build — both to keep the core minimal and to preserve `component-check` byte-identity.

**This is cleanly achievable — I verified the isolation holds (confidence: HIGH):**

- The wasm wrapper `crates/cdz-compiler-component` is **EXCLUDED** from the seed workspace
  (`Cargo.toml: exclude = ["crates/cdz-compiler-component", "crates/cdz-runtime"]`) and is built by a *separate*
  `cargo component build --target wasm32-unknown-unknown` invocation in its own dir (`xtask/src/main.rs:174-194`).
  It declares `cdz-compiler = { path = "../cdz-compiler" }` with **no features**. Because Cargo feature
  unification only acts *within a single build graph*, and the wasm build is a distinct graph that never sees the
  native seed's feature selection, enabling a feature in the native seed build **cannot leak** into the wasm
  build. So a default-off feature on `cdz-compiler` is exactly the right knob.
- `tracing` (the facade) is `no_std`-friendly and wasm-safe; the parts that are native-only (`tracing-subscriber`,
  the fmt layer, env filtering) belong in `cadenza-seed`, never in the core.

### Proposed shape (confidence: HIGH on structure, MEDIUM on exact ergonomics)

1. **`cdz-compiler`** gains an OPTIONAL dep + feature, default OFF:
   ```toml
   [dependencies]
   tracing = { version = "0.1", optional = true, default-features = false }

   [features]
   trace = ["dep:tracing"]
   ```
   Instrument decision points with `tracing::trace!`/`debug!`. To keep the source readable AND emit literally
   nothing when the feature is off, wrap the calls in a tiny local macro that expands to the `tracing` event under
   `#[cfg(feature = "trace")]` and to `{}` otherwise (or `use tracing::trace;` behind the cfg). Either way, with
   the feature off there is **no `tracing` dep and no codegen** — the wasm bytes are unchanged.

2. **`cadenza-seed`** (native, already has host deps) gains its own passthrough feature + the subscriber:
   ```toml
   [features]
   trace = ["cdz-compiler/trace", "dep:tracing-subscriber"]
   [dependencies]
   tracing-subscriber = { version = "0.3", optional = true, features = ["env-filter"] }
   ```
   Initialize the subscriber once at CLI startup, **filtered by an env var** (`CADENZA_TRACE` or reuse `RUST_LOG`)
   so tracing is off unless explicitly requested even in a `--features trace` build.

3. **The wasm wrapper depends on `cdz-compiler` with the feature OFF** (it already does — no features). Nothing to
   change there; the point is just to *not* add `trace` to it.

### The one sharp edge — output stream (confidence: HIGH this must be gotten right)

⚠️ **The subscriber MUST write to `stderr`, and default filtering MUST be OFF.** This project has been bitten
**twice** by stray output on the compile path — ask-44 and ask-47 were stray `eprintln!`s that polluted the
self-hosting byte-extraction. The harness reads the compiler's output from **stdout**
(`harness/run_corpus.py:156,180,202` parse `r.stdout` for `ran → Value("…")` / `compile → Ok`), and
`run_component` concatenates `r.stdout+r.stderr`. So:
- `tracing_subscriber::fmt().with_writer(std::io::stderr)` — never stdout (stdout carries the extracted bytes).
- Default level OFF (env-gated) so a plain `cargo build` / gate run is byte-for-byte identical to today. Traces
  only appear on an explicit `--features trace` build **with** `CADENZA_TRACE=…` set — i.e. an operator/loop
  debugging run, never a gate run.
- Even then, keep stdout clean so `2>/dev/null` recovers the exact bytes.

### Where to instrument first (highest debugging leverage — confidence: HIGH these are the hot spots)

The core decision surface is `crates/cdz-compiler/src/codegen.rs` (~12.7k lines, **240** `decline(`/`reject(`
call sites). Ranked by how often the loop needs them:

1. **`decline()` / `reject()` helpers (`codegen.rs:68`, `:75`)** — instrument the *helpers themselves* with
   `tracing::debug!` so **every** decline/reject is logged with its message and code *for free*, without touching
   240 call sites. Add a span (see below) so each carries its enclosing function + mode. This single change is the
   80/20 — it's precisely the "why did it decline, and from which path" question the loop asks every cycle.
2. **The scalar → runtime-mode retry fork (`compile_module`, ~`codegen.rs:1259-1298`).** `compile_module` walks
   the whole tree in *scalar* mode, and on a scalar-path decline **RETRIES the entire pass in runtime mode**. So a
   single `compile_program` can walk every function TWICE, and a `declined: …` you observe may be from the first
   (scalar) pass that the runtime retry then resolves — or the final fatal one. Emit a span with a
   `mode = scalar | runtime` field wrapping each pass, so a reader can tell which pass a decline came from. This
   ambiguity has directly cost the loop time (it's the confusion behind the ask-42 "which pass declined"
   investigation).
3. **Kind inference decisions** (`InferCtx`, the `Kind` lattice, return-kind/back-prop, arm-unification) — the
   subject of ~a dozen resolved asks (ask-12/14/17/18/32, the payload-kind recovery cluster). A `trace!` when a
   name/branch/arm's inferred `Kind` is chosen or upgraded (e.g. Int64 → Heap) would make those investigations
   direct instead of inferential.
4. **Per-function spans in `compile_module`'s reachable-set loop (`~codegen.rs:1620-1708`)** — a span per function
   compiled, with its name + reachability + deferred-decline, so all events nest under the function they came from.

`tracing`'s span model fits this exactly: a `#[instrument]` (or manual span) per function-compile and per
scalar/runtime pass, with `trace!` events at the decision points, yields a tree that mirrors the compile.

### Acceptance signal

- `cargo build -p cadenza-seed` (default) and the four gates are **byte-identical to today** — ignition still
  byte-reproduces, `component-check` still 575/0/0, the wasm component is unchanged (feature off ⇒ no tracing in
  the graph).
- `cargo build -p cadenza-seed --features trace` then
  `CADENZA_TRACE=debug cadenza-seed emit <declining-probe>.cdz 2>/tmp/trace.log` writes a decision trace to
  **stderr** (`/tmp/trace.log`), while **stdout** still carries the exact `declined: …` / bytes the harness
  reads — verify `2>/dev/null` recovers today's stdout unchanged.
- The trace for a known decline (e.g. the ask-49 minimal repro) names the enclosing function, the scalar-vs-runtime
  pass, and the decline message.

### Confidence summary

| Claim | Confidence | Basis |
|---|---|---|
| A default-off feature on `cdz-compiler` leaves the wasm build byte-identical | **HIGH** | wasm crate is workspace-EXCLUDED, built in a separate graph, depends on cdz-compiler with no features → feature unification can't leak (verified in `Cargo.toml` + `xtask/main.rs`) |
| Instrumenting the `decline`/`reject` helpers is the 80/20 | **HIGH** | 240 call sites funnel through 2 helpers (`codegen.rs:68,75`); one edit covers all |
| Subscriber must go to stderr + default-off, or it breaks byte extraction | **HIGH** | harness parses `r.stdout` (`run_corpus.py:156,180,202`); ask-44/47 are prior stray-output regressions on this exact path |
| The scalar/runtime retry fork needs a `mode` span field | **HIGH** | `compile_module` retries the whole pass in runtime mode (`codegen.rs:1259-1298`); a decline is ambiguous without it |
| Exact feature/dep ergonomics (macro vs cfg, env var name) | **MEDIUM** | idiomatic but a design choice for the compiler agent; `CADENZA_TRACE` vs `RUST_LOG` is taste |
| `tracing` is wasm-safe if ever wanted in the core | **MEDIUM** | facade is `no_std`-friendly; not needed here since feature stays off for wasm |

**Status.** 🟡 Seed — operator-requested tooling. Pure additive dev-experience change; no spec impact, no gate
impact when off. Force-multiplier for every future decline investigation (this loop's whole job). Related:
ask-48 (machine-branchable diagnostic KIND — the runtime-ABI analogue of this dev-desk observability),
ask-44/ask-47 (the stray-output regressions that make the stderr/default-off constraint non-negotiable).

---

## ✅ DONE 2026-07-07 (conformance loop) — optional tracing landed, feature-gated, wasm byte-identical

**Landed exactly as scoped.** `cdz-compiler` gains `tracing = { optional = true }` + `[features] trace`.
Instrumented the **`decline`/`reject` HELPERS** (the 80/20 — all ~240 sites funnel through 2 fns) with
`#[cfg(feature="trace")] tracing::debug!(target: "cdz::decline"|"cdz::reject", …)`, plus the scalar→runtime
retry fork in `compile_module` (`target: "cdz::pass"`, `mode = scalar|runtime`) so a decline's PASS is
unambiguous. `cadenza-seed` gains the passthrough `trace` feature + a `#[cfg(feature="trace")] init_trace()`
that inits `tracing_subscriber::fmt()` → **STDERR**, env-gated by `CADENZA_TRACE` (default `off`); a
`#[cfg(not)]` no-op twin.

**Verified:**
- DEFAULT build: `cargo tree -p cadenza-seed` shows NO `tracing`; `cargo tree --target wasm32-unknown-unknown`
  (the component) shows none either — feature can't leak across the excluded wasm graph. All four gates
  byte-identical: BEHAVIOR 572/0, IGNITION byte-identical, COMPONENT-CHECK 577 agree/0 disagree/0 soft/0
  decline, cargo test green.
- `--features trace` build: `CADENZA_TRACE=debug … emit <declining>.cdz 2>t.log 1>o.log` → t.log has
  `cdz::pass mode="scalar"` then `cdz::decline: declined msg=non-constant float equality…`; o.log keeps the
  exact `declined: …` / bytes; `2>/dev/null` recovers today's stdout unchanged (the ask-44/47 stray-output
  constraint held).

**Acceptance signal MET** on all three bullets. 📦 STABLE refreshed. Learning:
`optional-tracing-in-the-seed-compiler`. Follow-on (optional): extend instrumentation to kind-inference
decisions (InferCtx / return-kind back-prop) when a future inference ask needs it — the helpers-level 80/20 is
in and covers the every-cycle "why/which-pass did it decline" question.
