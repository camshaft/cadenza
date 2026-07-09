## 40. 🔴 `compile` has no diagnostics channel — a rejected program TRAPS instead of returning a coded diagnostic (blocks ~30 ask-30 rejections from `decline → agree`)

**Finding.** `compiler.cdz` now REJECTS ill-typed and malformed programs (ask-30's two compiler-side subsets
landed 2026-07-07: the reader arity check and the `well-typed?` type-rejection pass). But its ONLY failure
channel is a TRAP — a rejected program lowers its body to `KError → unreachable`, so `compile` returns *no bytes
and no reason*. Native, for the same input, returns a coded diagnostic:

| input | native | `compiler.cdz` |
|---|---|---|
| `(+ 1 true)` | `declined: operation on mismatched types` (CDZ0201) | traps (`unreachable`) — no code |
| `(if true 1 false)` | `declined: conditional branches have different types` | traps — no code |
| `(+ 1)` (malformed arity) | `declined: malformed \`+\` form: arity mismatch` | traps — no code |

So a rejection is now HONEST (reject-don't-miscompile holds — it no longer mis-accepts), but it cannot be
scored as agreement: the byte gate moves these ~30 cases `disagree → decline`, and they are STUCK at `decline`
because `compile`'s type says "bytes or trap", not "bytes or diagnostics".

**Why it touches the spec.** `contracts/build-tool-interface.md` is explicit: *"The build tool's derivation
entry MUST have a result-typed signature whose success arm carries the component bytes and whose failure arm
carries the diagnostics, so that success and failure are distinguished by the interface's type rather than by
an in-band sentinel such as an empty byte sequence"* (§The Tool Produces A Component, A Manifest, And
Diagnostics), and *"MUST reject an input that is not a well-formed canonical source tree with a machine-readable
diagnostic rather than an opaque failure."* A trap is precisely the "opaque failure" the spec forbids. The WIT
world already declares `compile -> result<list<u8>, list<diagnostic>>` (per the `compile`-entry comment in
`compiler.cdz`), but the seed emits `(def (compile b) …)` as plain `-> list<u8>`, and the language gives
`compiler.cdz` no way to CONSTRUCT a diagnostic value with a code.

**This is the sole remaining barrier to `agree` on the whole rejection corpus.** ask-30's acceptance signal is
"ill-typed cases move `disagree → decline` (done), then `→ agree` once the diagnostics ABI lets the compiler
return the matching `CDZ####`." The first half is now done compiler-side; this ask is the second half. It is
also what ask-30's sub-gap #2 named — promoted to its own ask because it now blocks a concrete, enumerated ~30
cases and is a distinct seed/spec change (an ABI + a value type), independent of any further type-checking.

**Two coupled pieces (both seed/spec-side — the compiler agent's domain):**
1. **The `compile` export becomes result-typed.** Emit `(def (compile b) …)` as `compile : list<u8> →
   result<list<u8>, list<diagnostic>>` (the WIT world already says so), so success carries the component bytes
   and failure carries diagnostics — distinguished by TYPE, not by an empty-bytes sentinel. `component-check`
   then reads the failure arm and can compare the code against native's rejection.
2. **A diagnostic-constructor surface** so `compiler.cdz` can build a `diagnostic` with a code + message where it
   currently emits `KError`. Needs a `diagnostic` value type (code : the `CDZ####` string or an enum; message;
   optional span) and a way for the front end to produce it. `compiler.cdz`'s `KError` node becomes "reject with
   THIS diagnostic" rather than "lower to unreachable".

**Acceptance signal.** `compile` on `(+ 1 true)` / `(if true 1 false)` / `(+ 1)` returns the FAILURE arm carrying
a diagnostic whose code matches native's (`CDZ0201` etc.), and `component-check` scores these `agree` (or a
value-equal diagnostic match) instead of `decline`. The ~30 enumerated ask-30 rejection cases move `decline →
agree`. Corpus: the rejection cases already exist (native realizes them); no new corpus needed — the gate
measures it once the failure arm is comparable.

**Evidence (2026-07-07).** Compiler-side rejections verified landing: `(+ 1 true)`, `(if true 1 false)`,
`(< 1 true)`, `(and 1 true)`, `(^ 1 true)`, `(<< 1 true)`, `(not 5)`, `(+ 1)`, `(+ 1 2 3)`, `(if true 1)`,
`(< 5)`, `(= 7)`, `(not 1 2)` — ALL now trap (reject) where they previously emitted a wrong value / partial
form; harness 0 hard / 0 error. The block is purely the missing diagnostics channel.
Related: ask-30 (the type-checker whose rejections this makes reportable), ask-11 (the front-end unknown-head
diagnostic, resolved as an honest trap — same "trap vs coded diagnostic" tension), `contracts/build-tool-interface.md`
§The Tool Produces A Component…, the `compile`-entry diagnostics-gap comment in `compiler.cdz`.

---

**🚧 SEED PROGRESS 2026-07-07 (this ask == ask-30 sub-gap 2; operator chose to build it).**
- ✅ **Envelope LANDED** (the hard/uncertain wasm-encoder part): `xtask/src/wit_envelope.rs`
  `build_compile_result_reference` + generated `COMPILE_RESULT_HEAD`/`COMPILE_RESULT_TAIL` consts — the
  component that lifts `compile` as `func(list<u8>) → result<list<u8>, list<diagnostic>>`,
  `diagnostic = record{code: string, message: string}`. Validates clean. **Gotcha solved (cost 2
  attempts):** an exported func referencing a `record` fails wasmparser's `all_valtypes_named` ("func
  not valid to be used as export") unless the record is EXPORTED as a named type AND the signature
  references the index `c.export(...)` RETURNS (not the anonymous `type_defined` index). A bare
  `list<u8>` needs no naming — why the existing envelope never hit this.
- 🔭 **Retptr layout confirmed** from the real cargo-component `compile` wrapper: core sig
  `(i32 ptr,i32 len)→i32 retptr`; return area `[disc:i32 @0][ptr:i32 @4][len:i32 @8]` (+ a `cabi_post`
  cleanup companion). Err arm element `diagnostic` = 4 i32s (2 string (ptr,len) pairs).
- ⏭ **REMAINING (next iteration):** `compile_result_wrapper_body` (marshal both arms to that layout —
  the Err arm needs a nested list→diagnostic→2-strings→bytes loop, GUARDED per the hand-emitted-wasm
  rule) + detect a `Result`-returning `compile` body in `compile_component_module` to select the
  result envelope + corpus + gates. Host `decode_compile_result` is ALREADY ready (Ok/Err + bare-list
  fallback), `run_compiler_component` composes the runtime — no host change.
- **Additive/unconsumed** so far: compiler.cdz's plain `-> list<u8>` path is untouched, gate 569/0. When
  the wrapper lands, the surface is: `compile` body returns `(Ok <bytes>)` / `(Err <list of (record
  (code "CDZ0301") (message "…"))>)` — `KError` stops being a trap. See [[diagnostics-abi-result-envelope]].

**🟢 LANDED + VERIFIED END-TO-END 2026-07-07 (seed).** The diagnostics ABI is complete: a `(def (compile
b) …)` body that returns a `Result<Bytes, list<diagnostic>>` is now emitted as `compile: list<u8> →
result<list<u8>, list<diagnostic>>` (a bare-`Bytes` body keeps the plain `list<u8> → list<u8>` seam, so
compiler.cdz is unaffected until it opts in). Three parts landed:
1. **Envelope** — `xtask` `build_compile_result_reference` + generated `COMPILE_RESULT_HEAD`/`_TAIL`.
2. **Wrapper** — `compile_result_wrapper_body` marshals the runtime `Result` into the canonical retptr
   `[disc:i32 @0][ptr:i32 @4][len:i32 @8]`; Err arm = `n*16`-byte elements, per diagnostic the two String
   fields into `[code(ptr,len)@0][message(ptr,len)@8]` (single-level copy loops).
3. **Selection** — `compile_component_module` detects a Result-shaped body (`shape_of` + `is_result_shape`).
Verified via `compile-run`:
```
(def (compile b) (Ok (Bytes.of (list 1 2 3))))                          → Ok (3 bytes) [1,2,3]
(def (compile b) (Err (list (record (code "CDZ0201") (message "…")))))  → Diagnostics [("CDZ0201","…")]
  (also: two diagnostics, empty code string, long message — all correct)
```
Host `decode_compile_result` already handled both arms (no host change). ⚠ Edge: `(Err (list))` (EMPTY)
can't shape its element → falls to the bytes path; harmless (a rejection always carries ≥1 diagnostic).
All 4 gates green (behavior 569/0, ignition byte-identical, cc-vs-Rust 574/0, cargo test). Learning:
[[diagnostics-abi-result-envelope]]. Moved open → done.

**→ For the compiler agent:** replace `Core.KError → unreachable` with a `compile` body returning `(Err
(list (record (code "CDZ0201") (message "…"))))` (and `(Ok <bytes>)` on success). The ~30 `native=rejected
/ component=ok` disagreements then become comparable — `decline → agree` once the codes match native's.

**⚠️ SUPERSEDED-SHAPE 2026-07-07 (Run 80) — the diagnostics channel is now a FROZEN CONTRACT, and it's NOT a
two-arm Result.** A sibling landed `spec/contracts/build-tool-interface.md` (frozen) + constitution Amendment
0.8.0, reshaping the derivation entry from the two-arm `compile : list<u8> → result<list<u8>, list<diagnostic>>`
this ask assumed to a **kinded-artifact interface**: `compile : list<artifact> → compile-output` where
`compile-output = record { artifacts: list<artifact>, diagnostics: list<diagnostic> }`, `artifact = { kind,
bytes }`, `diagnostic = { severity, code, message }`. Success = a component artifact present + no error-severity
diagnostic; failure = no component artifact + ≥1 error diagnostic; a warning rides alongside a produced
component. So diagnostics and byte-outputs are DISTINCT CHANNELS, not mutually-exclusive arms — and the Option-
vs-Result question flagged on ask-38 is moot (neither; it's the artifacts+diagnostics record). Rationale:
sibling learning `spec/learnings/2026-07-07-the-build-tool-interface-is-a-kinded-artifact-list-not-a-two-arm-result.md`.

**Loop re-probe (Run 80): the seed's DRIVER ABI has NOT migrated yet.** `cadenza-seed compile-run` /
`component-check` still return a single `list<u8>` (`compile → Ok (N bytes)`), and a type-rejection still emits
the 88-byte bare-`unreachable` decline stub — no `compile-output` record, no diagnostics surfaced. So the ~30
ask-30 type-rejections are STILL at `decline`, not `agree`: the contract is spec-frozen but the seed's
compile-component ABI + the checker's expectation must both migrate to the artifacts+diagnostics record before a
coded rejection can be produced and matched. That migration (seed + checker) is the remaining ask-40 work under
the new shape.
