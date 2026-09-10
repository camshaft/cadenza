# DESIGN: nix-generated reducer-world wasm targets — rcdzc + per-syntax parsers, `run`-callable

**Vertical:** `v-reducer-targets` (subsystem `rcdzc`). **Origin:** operator directive 2026-09-10 (brief
`v-reducer-targets-brief.md`), refined same day: *"we should have contracts for the request/response. we
want to have this loadable on the platform and have it runnable. that's the goal."*

## 1. Goal

Make the compiler (`rcdzc`) and the per-syntax parsers (`sexpr`, `ml`) **loadable on the platform and
runnable**: each is a **wasm component** that is a **nix target**, **uploaded to the CAS store**
(`cdz-cas-http`) and **invoked by hash** through the reducer world's pure-function primitive

```wit
run: func(program: program-hash, contract: contract-id, input: payload) -> result<payload, error>;
```

so *anything on the platform* can compile a Cadenza program (or parse a surface) by holding a hash and a
contract-id — no host redeploy, no bespoke API. "Anything on the platform can compile a Cadenza program"
is the headline; the parsers make the full `source → AST → artifacts` pipeline platform-native.

## 2. The spine is CONTRACTS, not wrapped functions (operator refinement)

The reducer world already **routes and decodes by `contract-id`**. `run(program, contract, input)` hands
the guest an opaque `payload` that the guest decodes *against the exact contract it names* (`world.wit`
§"strongly typed, with a carried payload"). And **a contract IS the request/response pair**: a single
contract declaration carries BOTH an input type and an output type, and hashes to ONE `contract-id`. E.g.
`contracts/userspace/blob-get.cdz` declares `contract-descriptor(module, "cdz-platform.blob.get",
"GetRequest", "GetResult")` — one contract, input `GetRequest`, output `GetResult`. So a target is not "a
function turned into wasm" and NOT two contracts — it is **ONE contract** whose input type is the request
and whose output type is the response:

| target | contract | input type (request) | output type (response) |
|---|---|---|---|
| `rcdzc.compile` | one contract-id | the compile bundle: canonical binary AST + sidecar + kinded inputs + requested targets | the `{artifacts, diagnostics}` envelope (multi-artifact → one payload) |
| `sexpr.parse` | one contract-id | source bytes (UTF-8) | `{ast-bytes, diagnostics}` |
| `ml.parse` | one contract-id | source bytes (UTF-8) | `{ast-bytes, diagnostics}` (error-recovering → diagnostics non-empty is normal) |

- The **contract's declaration** (one `.cdz` under `contracts/userspace/`) names both its input type and
  its output type; its computed `contract-id` is what the caller passes to `run`. The input type is the
  wire shape the guest decodes `message.payload` into; the output type is the shape the guest returns (as
  the `close` reason payload, §4). A caller decodes the returned `payload` against that same contract's
  output type.
- The id is **COMPUTED** by `xtask-codegen-contracts` from the `.cdz` declaration — the same computed-id
  machinery kernel/userspace contracts already use (`contract-descriptor` → `identity_from_descriptor`
  → `Contract::new(name, types, input, output).id()`), so the id is identical on the Rust guest side and
  any Cadenza peer. (Coordinate with **v-gateway-rewrite**, which is extending `xtask-codegen-contracts`
  for userspace contracts.)

**Load + run flow (the goal, concretely):**
1. `nix build .#reducer-guest-rcdzc` → a canonicalized wasm component; `nix build .#reducer-guest-rcdzc-hash` → its content hash `H`.
2. Upload: `HttpBlobStore::publish(component_bytes)` → confirms hash `H` (CAS validates bytes hash to key).
3. Invoke: `run(H, rcdzc-compile-contract-id, encode_input(ast, sidecar, inputs, targets))` → `result<payload, error>`.
4. Decode the returned `payload` against the `rcdzc.compile` contract's OUTPUT type → `{artifacts, diagnostics}`.

## 3. The shared guest surface — all targets export the SAME WIT

Every target is a component of **`pure-reducer-world`** (`cdz-platform/wit/world.wit`):

```wit
world pure-reducer-world {
  import run;          // the pure-function primitive — a compute guest MAY call it, need not
  export guest;        // on-message / on-response / on-notification -> step
}
```

- The compute guests are **pure**: input in `message.payload`, output in the `close` reason. They hold no
  `state`/`blobs` and emit no `request`s, so `pure-reducer-world` (only `run` imported) is the exact floor.
- Importing `run` (even if unused) leaves the door open for the OPTIONAL rcdzc convenience: a
  `source → artifacts` one-shot where rcdzc-reducer calls a parser target via `run` before compiling
  (brief §Targets 1). Not in the first increment.
- All three export `cadenza:platform/guest` — the *only* per-target difference is the inner function and
  the payload codec, exactly as the operator requires ("they all should export the same interface").

## 4. How a pure `run` guest returns its value: the `close` reason

There is **no `Break`** arm (the brief's word) — the WIT outcome is `continue | close(closed{schema,
reason})` (map correction). A pure `run` guest folds one `on-message` and **closes immediately**, putting
its output in the close reason:

```
on-message(msg) -> step {
  requests: [],
  outcome: close(closed { schema: <the target's contract-id>, reason: <encoded output payload> })
}
```

`closed.schema` is the target's own contract-id (the same one passed to `run`); its OUTPUT type governs the
`reason` bytes. The host's `run` implementation reads the program's output value from that close reason
inline (this is what "returns the program's output value inline, within the same fold" means in
`world.wit` §`run`).
`on-response`/`on-notification` are not part of a pure `run` and return `close` with an empty/`faulted`
reason (a pure guest is never driven through them under `run`).

## 5. The per-target payload envelope (canonical binary, binary-AST is THE format)

Per standing directive [[binary-ast-is-the-data-exchange-format-no-exceptions]], every payload is a
**canonical binary value** matching its contract type. Reuse the existing `cadenza-compile-abi` wire
codecs where they exist; fill the gaps:

- **`sexpr.parse` / `ml.parse` response** = `{ast: ast-bytes, diagnostics: [...]}`. `ast-bytes` is the
  canonical `cadenza_ast::codec::encode(&arenas)` of the parsed `Arenas`; diagnostics reuse
  `encode_diagnostics`/`decode_diagnostics` (already in `cadenza-compile-abi`). Request = raw source bytes.
- **`rcdzc.compile` response** = the whole `CompileOutput { artifacts: Vec<Artifact>, diagnostics:
  Vec<Diagnostic> }`. `encode_diagnostics` exists; there is **NO artifact-list / whole-`CompileOutput`
  codec yet** → this is the concrete **build gap**: add `encode_compile_output` / `decode_compile_output`
  to `cadenza-compile-abi` (a canonical-binary envelope: a list of `{kind, name, bytes}` artifacts + the
  diagnostics list), and make its wire shape match the `rcdzc.compile` response contract's declared type.
- **`rcdzc.compile` request** = the kinded-input bundle. `compile(inputs: &[Artifact], targets: &[Target])`
  already takes a `&[Artifact]` (kinds `ast`/`sidecar`/`entry`/`wit-world`/…); the request codec is the
  same artifact-list encoding + the `Target` list (reuse `target::Target` wire).

The artifact-list codec is shared between the request (inputs) and response (outputs) shapes — one codec,
two directions.

## 6. The generic wrapper mechanism — ONE crate, nix stamps N components

**Constraint (operator):** NO crate per wasm target; generate the components on the fly in the flake.

**Decision:** the compute targets are **Rust** (rcdzc, `cadenza-syntax` are Rust libraries — a Cadenza
`.cdz` guest cannot call into them, so `mkCadenzaGuest` is not the path here; the "Cadenza guests, no more
Rust" rule was about the conformance *fixtures*, not the compiler). So the mechanism is:

- **ONE generic wrapper crate** (working name `cdz-reducer-guest`) that depends on `rcdzc` + `cadenza-syntax`
  and exports `cadenza:platform/guest` via `wit-bindgen`. It is a *mechanism* crate, not a per-target crate
  — the prohibition is against `rcdzc-reducer` + `sexpr-reducer` + `ml-reducer` being three crates.
- The **target is selected at build time by a cargo feature** (`target-rcdzc` | `target-sexpr` |
  `target-ml`), each feature wiring `on-message` to the inner fn + the request/response codec + the
  response-contract-id const. Exactly one feature is active per build.
- **nix enumerates the targets and stamps one component per target** from that one crate:
  `reducerTargets = [ "rcdzc" "sexpr" "ml" ]`; `mkReducerGuest { target }` = `cargo build --release
  --features target-${target}` → componentize (§7) → `wasm-tools strip -a` (canonical hash). This is the
  "generate on the fly in the nix flake" the operator asked for — the flake is the enumerator, the crate
  is the shared mechanism.

Reuse `rcdzc-wasm`'s **standalone-workspace** pattern (its own `Cargo.lock` + vendored deps + the
drift-guard fileset assertion, flake.nix ~L1654/1673/1690) — the wrapper joins that leaf workspace (it
already vendors `rcdzc` + `cadenza-syntax` + `cadenza-compile-abi`).

## 7. Componentization — the key build risk to resolve first

`packages.rcdzc-wasm` today is a plain `wasm32-wasip1` **module** (cdylib, `#[no_mangle]` FFI), NOT a
component. The reducer guest must be a **component** exporting `cadenza:platform/guest`. Two paths, decide
empirically when building rcdzc-reducer:

- **Preferred: `wasm32-unknown-unknown` + `wit-bindgen` + `wasm-tools component new`** — a pure compute
  guest needs no WASI, so target unknown-unknown and the resulting component imports ONLY `run` (matches
  `pure-reducer-world`). Requires rcdzc to compile on unknown-unknown (no WASI syscalls in the compile
  path — likely true; verify).
- **Fallback: `wasm32-wasip1` + the preview1 adapter** (`wasm-tools component new --adapt
  wasi_snapshot_preview1.reactor.wasm`) — but then the component imports WASI, which `pure-reducer-world`
  does not provide. Only acceptable if unknown-unknown is infeasible, and then the host must supply a WASI
  shim (undesirable). **Resolve this before wiring the contract** — it decides the target world.

This is the `#8658`-class hazard the brief flags: a heavy (heap) reducer-guest codegen path that was
recently miscompiled (FIXED + guarded #8663) but young. **Conformance-test** each guest: compile →
instantiate → `run` round-trip, asserting the decoded response equals a native `compile`/`read` call on
the same input. That round-trip test is this vertical's gate coverage (added to `cargo xtask check`).

## 8. Supersede the syntax.wit draft

`cadenza-syntax/wit/syntax.wit` is a DRAFT **import-library** world (a linked in-wasm dep). The operator
wants **`run`-callable reducer GUESTS**, not an import-lib. This design supersedes that draft for the
parser targets: `sexpr`/`ml` become `pure-reducer-world` guests (source-bytes in → `{ast, diagnostics}`
out via `close`), not `parse`/`query`/`doc` imports. (The import-lib world may still serve the P2
compose-into-linker use case; the two are not mutually exclusive, but the reducer-target path is the one
this vertical builds.)

## 9. Increments (build order — rcdzc FIRST, it proves the mechanism e2e)

- **B0 — this design doc.** ✅ (this file).
- **B1 — envelope codec.** `encode_compile_output`/`decode_compile_output` + shared artifact-list codec in
  `cadenza-compile-abi`; unit round-trip tests. (No wasm yet — pure Rust, fast to gate.)
- **B2 — the generic wrapper crate + rcdzc feature.** `cdz-reducer-guest` exporting `guest`, `target-rcdzc`
  wiring `on-message` → decode request → `rcdzc::compile` → `close(response)`. Native unit test of the fold.
- **B3 — nix `mkReducerGuest` + rcdzc target.** Componentize (§7 decision), `reducerTargets`, `packages.
  reducer-guest-rcdzc` (+ `-hash`), drift-guard update. Conformance round-trip check wired into `cargo
  xtask check`. **This is the e2e proof:** compile a Cadenza program via `run` off the built component.
- **B4 — the rcdzc.compile contract.** `contracts/userspace/rcdzc-compile.cdz` declaring ONE contract
  (`contract-descriptor(module, "cdz-platform.rcdzc.compile", <input-type>, <output-type>)`) whose input
  type is the compile bundle and output type is the artifacts+diagnostics envelope; regenerate the
  computed id; the wrapper consumes the generated `contract-id` const. (Coordinate with v-gateway-rewrite.)
- **B5 — CAS integration test.** publish → `run(hash, id, input)` → decode, end-to-end against `cdz-cas-http`.
- **B6 — sexpr target.** feature `target-sexpr` + `contracts/userspace/sexpr-parse.cdz` + nix target + round-trip.
- **B7 — ml target.** feature `target-ml` + contract + nix target + round-trip (diagnostics-carrying).
- **B8 (optional) — rcdzc `source→artifacts`** convenience: rcdzc-reducer calls a parser target via the
  `run` import.

Each increment lands as its own gated MR (per-commit-green). B1 is the natural first *code* unit after this
doc.

## 10. Reuse / coordinate (summary)
- Reuse: `rcdzc-wasm` (proven wasm compiler build + standalone workspace pattern), `cadenza-compile-abi`
  wire codecs, `mkStripComponent`/`worldArtifacts` (componentization + world artifacts), `cdz-cas-http`
  (`HttpBlobStore::publish`/`fetch`), `xtask-codegen-contracts` (computed ids), the host `WasmReducer`
  driver (`cdz-platform/src/host.rs`) for the round-trip test.
- Coordinate via `cargo xtask fleet send`: **v-gateway-conformance** (reducer-guest + conformance/harness
  patterns), **v-cas-http** (CAS upload/call + nix packaging precedent), **v-gateway-rewrite** (userspace
  contract-id codegen). Human-shaped decisions → concierge `ask`, never block.
