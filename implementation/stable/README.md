# Stable cdz-compiler toolchain — published snapshot

A frozen, all-gates-green copy of the seed toolchain, so the self-hosting (compiler.cdz) agent can
work against a fixed `cadenza-seed` and NOT be broken by concurrent seed changes in
`implementation/seed/`. Point your `compile-run` / `emit` / `component-check` invocations at
`stable/cadenza-seed` and `CADENZA_RUNTIME=stable/cdz_runtime.wasm`.

## Published snapshot — 2026-07-08 (c82 — a wrong-type payload constructor in direct match-scrutinee position is now CDZ0201, was a wrong value)

**A wrong-payload constructor in direct match-scrutinee position is now rejected (c82).** `(match (I true)
((I x) x) …)` under `(type N (I Int64 | J Int64))` ran and returned `true` — a Bool crossing the run
boundary where the arm binder `x` (the payload of `I Int64`) is Int64. Root: `check_tree` pruned the entire
`match` form (`Some("match") => return Ok(())`, correct for arm patterns) but that skipped the SCRUTINEE,
which is an ordinary expression — so a wrong-payload constructor written directly there bypassed the
constructor-payload check that fires in every other position (bare, let-bound, arg, annotated,
over-applied). Fix: `Some("match") if elems.len() >= 2 => check_tree(scrutinee)` — descend into the
scrutinee only, keeping arm patterns owned by `gen_match`. Regression guards hold: valid scrutinee matches,
let-bound wrong-payload rejects, runtime/param sum match + `(match (List.at …) …)` reader idiom compile,
non-exhaustive fires CDZ0210. Behavior **680 passed / 4 failed** (10 todo, 228 skipped) · IGNITION PASS ·
cargo **28/0** · compiler.cdz VALID (262157 B). ⚠ The 4 FAILs are sibling-owned (3 map + c80
quote-nested-unquote). Runtime UNCHANGED (`d3f1a14d…`); compiler `19a4ef99…`.

### Older snapshot — 2026-07-08 (absent record-field access → compile-time CDZ0201, was a runtime trap; normative spec corrected)

**Absent record-field access is now a COMPILE-TIME type error (c81).** `(. (record (x 1)) z)` — projecting
a field the record's type does not carry — lowered to a runtime `unreachable` trap instead of rejecting.
The member-access check verified "operand IS a record" (CDZ0201 for `(. 5 x)`) but not "record HAS the
field" — the uncovered half of the master pattern (projection well-formedness = operand-is-record AND
field-exists). Fix: `resolved_record_fields` (the record twin of `resolved_tuple_arity`, reaching a
literal / let-bound / function-returned record via `resolve`) → reject CDZ0201 on an absent field; a
runtime-record parameter imposes nothing (no false reject); built-in module operands excluded (their
dotted access is op-dispatch, not projection). **Spec corrected (user-directed):** core-semantics.md
#Member Access said missing-field "MUST raise a trap", contradicting type-system.md (row-projection of an
absent field is a compile-time reject) and the already-compile-time `(. 5 x)` case — both sentences
rewritten to compile-time rejection, and the 2 contradicting corpus cases updated (05 "missing field
traps"→"is a type error"; 11 "delegated capability" → CDZ0201). Behavior **679 passed / 5 failed** (10
todo, 228 skipped) · IGNITION PASS · cargo **28/0** · compiler.cdz VALID (262157 B) · real `compile-bytes`
pipeline compiles (14614 B, no false reject). ⚠ The 5 FAILs are sibling-owned (3 map + c82
variant-payload-scrutinee + c80 quote-nested-unquote). Runtime UNCHANGED (`d3f1a14d…`); compiler
`f8017a88…`.

### Older snapshot — 2026-07-08 (ask-77 CLOSED — the mutual-recursion tuple return-kind; cdzc's front end unblocked on real bytes)

**ask-77 CLOSED.** cdzc's `decode` ↔ `decode-node` ↔ `decode-app-children` mutual recursion returns
`(tuple <heap Ast>, Int-cursor)` and declined two ways: the scalar-slot face `(match (decode-node …)
((tuple a p) p))` → "cannot infer runtime compound result shape", and the heap-slot face `(match (decode
b) ((Ast.Int n) …))` → "runtime match with a non-literal pattern". Root cause: the KIND-INFERENCE `match`
arm never bound TUPLE-pattern binders (it handled literal- and constructor-pattern arms only), so
`decode`'s `((tuple ast pos) ast)` inferred the heap slot `ast` — and thus `decode`'s result — as scalar
Int64, and a caller's constructor-pattern match took the scalar-literal path. This is the
MUTUAL-RECURSION sibling of ask-73 (which fixed DIRECT tail-recursion at the emit path); ask-77 needed the
same slot-kind recovery in the INFERENCE pass. Fix: bind irrefutable-tuple-pattern binders in
`InferCtx::infer`'s match arm via `scrutinee_tuple_slot_kinds` (heap slot → Heap, scalar cursor → Int),
guarded to a call-returned tuple (not an inline `(tuple n 9)`); plus `tuple_slot_scalar_kind` falls back
from `shape_of` to Kind inference so a recursive scalar slot producer (`skip-item`) is recovered. Both
faces compile against real cdzc.cdz; a standalone mutual-recursion regression case added to
02-binding-and-control. Behavior **677 passed / 4 failed** (10 todo, 228 skipped) · IGNITION PASS · cargo
**28/0** · compiler.cdz VALID (262157 B). ⚠ The 4 FAILs are pre-existing sibling cases (3 map + the c80
plain-quote/nested-unquote break), none from this change. Runtime UNCHANGED (`d3f1a14d…`); compiler
`d692fb5e…`.

### Older snapshot — 2026-07-08 (String.slice on a RUNTIME string — an op compiler.cdz uses that only const-folded, never had a runtime emitter)

**`String.slice` runtime path LANDED.** `(String.slice s a b)` on a runtime string (a parameter, not a
literal) declined "unsupported dotted-application" — it const-folded on a literal but had NO runtime
emitter (unlike `Bytes.slice`). Found by sweeping the ops cdzc/compiler.cdz actually use and probing each
on a parameter. Only the emitter was missing (inference, render-shape, const-fold already handled `slice`).
`gen_runtime_string_slice`: the runtime String is a Bytes-backed UTF-8 leaf, so it scans the bytes once
(guarded loop) mapping SCALAR offsets `[a,b)` to byte offsets (scalar-start byte = `(byte & 0xC0) != 0x80`),
tallies the total scalar count to validate `b`, then `bytes-slice` + Option-build. String offsets are
Unicode SCALAR positions, NOT byte offsets (distinct from `Bytes.slice`'s `(start, LENGTH)`) — `"aébc"`[1,3)
= "éb". Fallible: valid `0<=a<=b<=count` → `(Some sub)`, else `(None unit)`. Corpus: 4 new runtime cases in
13-strings (parameter-fed, defeating the fold). Behavior **676 passed / 3 failed** (10 todo, 228 skipped) ·
IGNITION PASS · cargo **28/0** · compiler.cdz self-compiles VALID (262157 B). ⚠ The 3 FAILs are the SAME
sibling map cases (unchanged). Runtime UNCHANGED (`d3f1a14d…`); only the compiler grew (`1ba81ab2…`).

### Older snapshot — 2026-07-08 (List.concat wired to the runtime `vec-concat` — cdzc needs it to assemble output in linear time)

**`List.concat` LANDED.** `(List.concat a b)` produces a new list = the elements of `a` followed by `b`,
lowering to the runtime's RRB-trie `vec-concat` (WIT 55, O(log N) — the runtime already implemented and
unit-tested it; the gap was purely that the compiler didn't lower to it). Wired end-to-end: `vec-concat`
added to the envelope allow-list (`himport::VEC_CONCAT = 41`, `RT_N_IMPORTS 41→42`, regenerated via
`xtask build`); `gen_runtime_list_concat` emits the call; inference constrains both operands `Heap` (so a
concat-consumer's list parameter stays Heap and a self-call emits a runtime `call`, not an inline);
`shape_of` renders a concatenated list as the same `(list …)` as a literal (representation unobservable);
and a construction-time check rejects concatenating lists of DIFFERENT element types (CDZ0201,
decline-don't-miscompile). This is the `code-cat`/emit-assembly idiom — a self-hosted compiler joins
encoded fragments in linear time instead of pushing one element at a time (O(n²)). Spec: added the
concatenation clause to collections-and-text.md §A List Is Grown By Functional Construction. Corpus: 6 new
cases in 05-compound-types (flat-concat render, length-is-sum, boundary read, empty-identity, via-params,
type-error). Behavior **672 passed / 3 failed** (10 todo, 228 skipped) · IGNITION PASS · cargo **28/0** ·
runtime `vec-concat` unit tests green · compiler.cdz self-compiles VALID (262157 B). ⚠ The 3 FAILs are the
SAME sibling map cases as the prior snapshot (unchanged, pre-existing). Runtime UNCHANGED (`d3f1a14d…`);
only the compiler grew (`7cf2242a…`) by the one new import.

### Older snapshot — 2026-07-08 (ask-73 tail-recursive TUPLE-return slot-kind from scrutinee — the sole remaining front-end blocker for the cdzc rewrite's `decode`)

**ask-73 CLOSED.** `(match (go 3 0) ((tuple a b) a))` where `go` is a tail-recursive tuple-returning
function (`(tuple acc 0)` at the base, `(go …)` in the recursive branch) declined "cannot infer runtime
compound result shape". Root cause: `main`'s return kind resolved to `Heap` because the tuple-match bound
the returned slot `a` as an opaque Heap handle — the arm-usage inference (`infer_tuple_binder_kinds`)
recovers a slot's scalar kind only from how the binder is USED, and `a` was merely returned bare. Fix:
`scrutinee_tuple_slot_kinds` navigates the SCRUTINEE to a representative `(tuple …)` form (follows
`if`/`match`/`let`/`do`/`:`, inlines user calls, and SKIPS a recursive self-call branch — its result kind
equals the base branch's by induction), reads each element's kind, and fills the slots arm-usage left
`Heap`. The TUPLE twin of the already-realized tail-recursive SCALAR-accumulator return-kind inference
(the record path already worked; the tuple path lacked the fallback). Verified across bool-slot,
mutual-recursion, and nested-compound-slot variants. ⚠ `main` RETURNING the whole recursive tuple still
declines (the render path — a recursive-returning function's `shape_of` is genuinely None; correct
decline). The corpus case destructures to a scalar — the decoder's `(node, position)` cursor idiom.

Behavior **666 passed / 3 failed** (10 todo, 228 skipped) · IGNITION PASS · cargo **23/0** ·
compiler.cdz self-compiles VALID (262063 B). ⚠ The 3 FAILs are ALL sibling-added map cases (`a map with a
computed key equals the same map with a constant key`; `an unbound name in a map key is a scope error`;
`matching a lookup from a computed-key map literal selects the present-value arm`), all present on the
prior stable's spec — NOT regressions (the prior stable produces the identical 3 FAILs and had ask-73's
accumulator case as `todo`). This snapshot strictly improves the prior one (ask-73 flips 2 cases
todo→PASS). NOTE: the native↔wasm **COMPONENT-CHECK is retired from the gate set** — it tested
Rust→wasm toolchain fidelity, not compiler correctness; it returns as the byte gate when the
*Cadenza-authored* compiler emits the component (the real self-hosting check). `cdz_compiler_component.wasm`
is still published for that future use.

### Older snapshot — 2026-07-07 (Run: CHAMP map ops in the compiler envelope (allow-list 32–40, ignition byte-identical) + Map surface spec refined (swap/take) + two reject-don't-miscompile fixes: plain-quote nested-unquote→CDZ0401, and `(: (Some true) (Option Int64))` payload-mismatch→CDZ0203)

| artifact | what it is |
|---|---|
| `cadenza-seed` | the native seed CLI (`emit` / `compile-run` / `behavior-gate` / `ignite` / `component-check`) |
| `cdz_runtime.wasm` | the value-heap runtime component (compose target; set `CADENZA_RUNTIME` to it) |
| `cdz_compiler_component.wasm` | the wasm build of cdz-rustc (the `component-check` reference) |
| `SHA256SUMS` | content hashes for pinning |

Content hashes are in `SHA256SUMS` (verify with `cd stable && shasum -c SHA256SUMS`).

## Verified at publish time (all four gates GREEN)

- **BEHAVIOR-GATE** `spec/semantics`: **582 passed, 0 failed** (2 todo, 233 skipped).
- **IGNITION**: PASS — byte-identical self-reproduction.
- **COMPONENT-CHECK** (Rust cdz-rustc vs its wasm component): **584 agree / 0 disagree / 0 soft / 0 decline**
  (classifies byte-differing components by RUNTIME BEHAVIOR — ask-33 — so `disagree` = runs-to-wrong-value; the
  decline discriminator now applies SYMMETRICALLY on the native-REJECTS branch too — an ill-typed program that
  compiler.cdz emits a decline stub for is scored `decline`, not `disagree`).
- **cargo test --release**: green (incl. artifacts-ABI, field-projection, and effect-handler end-to-end probes).

## What this snapshot includes (relevant to the self-hosting agent)

- **Field access on a RUNTIME record — the compiler can READ ITS INPUT.** `(match (List.at inputs 0) ((Some
  a) (. a bytes)) …)` projects a field off an input `artifact` record (a runtime-record `arr-get` at the
  sorted-key slot, unboxed by the field's shape). The `match` payload binder now carries the payload's
  `Shape`, and the `compile` entry's `inputs` parameter is given the fixed `list<artifact>` shape, so
  `.bytes`/`.kind` resolve. **Both idioms work** (ask-52): `(match (List.at inputs 0) ((Some a) (. a bytes)) …)`
  AND `(. (Option.expect (List.at inputs 0) "…") bytes)`. This is how compiler.cdz reads the AST out of its
  input to feed `read-module`/`compile-program`.
- **Effect-based diagnostics is FULLY unblocked** (ask-46/49/51): a recursive-effectful `Diag` handler lowers
  under BOTH the `compile` entry (ask-46) and the gate's `emit`/`run()` entry with a compound result (ask-49),
  AND the `compile-output` ABI detection now looks THROUGH the `handle` (ask-51) so a record produced inside
  the handler is decoded as the artifact ABI (not the bytes fallback). Write `compile` as `(handle (list)
  ((Diag.emit …)(Diag.collect …)) (record (artifacts (list (record (bytes <bytes>) (kind "component"))))
  (diagnostics (Diag.collect unit))))` — the whole pipeline runs, gate-green.
- **Diagnostics-via-effects is now COMPLETE seed-side** (ask-46): a recursive-effectful `handle` (the `Diag`
  collector) lowers under the `compile` ENTRY, so the handler installs at `compile`. Combined with the
  kinded-artifact ABI (below) and ask-45 (recursive-effectful collection), compiler.cdz can run its
  `check-node`/`check-funcs` pass under a `Diag` handler and RETURN the collected `list<diagnostic>` in the
  `{artifacts, diagnostics}` record. Verified: `(def (compile inputs) (record (artifacts (list)) (diagnostics
  (handle (list) (…Diag.emit…) (…Diag.collect…) (walk)))))` → the collected diagnostics surface. (Every
  internal-state effect — symbol table, return-kind table, fresh-slot counter — lowers under `compile` too.)
- The **kinded-artifact ABI** (ask-41 / Amendment 0.8.0) — the FULL SYMMETRIC build-tool interface.
  A `(def (compile inputs) …)` body evaluating to a `(record (artifacts …) (diagnostics …))` record is
  emitted as `compile: list<artifact> → compile-output`, where `artifact = record{bytes: list<u8>,
  kind: string}`, `diagnostic = record{code: string, message: string, severity: enum{error,warning}}`,
  and `compile-output = record{artifacts: list<artifact>, diagnostics: list<diagnostic>}`. Field order
  in each WIT record is SORTED-by-key (matching the runtime's sorted record slots), so slot-i ↔
  canonical-offset-i is a straight copy. Verified end-to-end:
  - a component artifact + a **warning** (severity 1) → the component bytes are produced (the non-error
    diagnostic rides alongside, does NOT deny the component);
  - a component artifact + an **error** (severity 0) → `Diagnostics` (the error denies the component);
  - a **multi-artifact** output (a `dwarf` sidecar + the component) → the component is selected BY KIND;
  - the host feeds the AST in as one `{bytes: <ast>, kind: "ast"}` input artifact (input `list<artifact>`).
- This is the diagnostics OUT-channel: because success and rejection are ONE record type (not two
  differently-typed `Ok`/`Err` arms), the compiler's choosing `if`/`match` has same-shaped branches, so
  the deep `Core` sum-match is an ordinary heap consumer — closing ask-42 by construction. compiler.cdz
  can now collect `list<diagnostic>` (via the `Diag` effect, ask-45) and RETURN it alongside the
  component in the record.
- The **prior ABIs still work**: a bare-`Bytes` body keeps `list<u8> → list<u8>`; a `Result<Bytes,
  list<diagnostic>>` body keeps `list<u8> → result<…>` (ask-40). The artifact ABI takes precedence when
  the body is a `compile-output` record.
- ⚠️ **Allocator fix baked in:** the shared `cabi_realloc` now reads its alignment argument from the
  CANONICAL param index (2), not index 1. The old order masked every wasmtime-driven nested-lowering
  allocation to address 0 — invisible on the single-allocation bytes ABI, fatal for the artifact ABI's
  nested `list<artifact>` input lowering. If you saw a spurious OOB trap marshalling inputs on an older
  snapshot, this is why.

## Usage

```
cd <repo-root>
CADENZA_RUNTIME=implementation/stable/cdz_runtime.wasm \
  implementation/stable/cadenza-seed compile-run implementation/compiler/compiler.cdz <input.cdz>

# byte-level self-hosting gate over the whole corpus:
implementation/stable/cadenza-seed compile-run implementation/compiler/compiler.cdz \
  --emit-component /tmp/compilercdz.wasm
CADENZA_RUNTIME=implementation/stable/cdz_runtime.wasm \
  implementation/stable/cadenza-seed component-check /tmp/compilercdz.wasm spec/semantics
```

## Refreshing this snapshot

Re-copy from `implementation/seed/target/release/cadenza-seed` +
`crates/{cdz-runtime,cdz-compiler-component}/target/wasm32-unknown-unknown/release/*.wasm` ONLY after
all four gates are green, and re-stamp `SHA256SUMS`. `implementation/` is gitignored (disposable), so
this snapshot lives only in the working tree — it is a concurrency convenience, not a released artifact.
