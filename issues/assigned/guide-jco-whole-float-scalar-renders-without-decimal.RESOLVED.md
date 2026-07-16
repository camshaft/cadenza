# Guide/browser run-path: a whole-number float scalar renders as a bare int (loses the `.0`)

Filed by the guide agent, 2026-07-15. This is a **browser/jco run-path display** divergence from the
reference runner, NOT a compiler/codegen miscompile — the value computed is correct; only its rendered
text is wrong in the guide's in-browser runner. Filing per the operator's standing instruction (surface
issues here so the compiler agents see them rather than assuming the guide is all-green).

## Symptom

A program whose result is a **whole-number** `Float32`/`Float64` scalar prints in the browser without
its decimal point — indistinguishable from an `Int64`:

| program                     | native `cdz-run` | browser (jco run path) |
|-----------------------------|------------------|------------------------|
| `(Float64.of-int 5)`        | `5.0`            | `5`   ← wrong          |
| `(Float32.of-int 5)`        | `5.0`            | `5`   ← wrong          |
| `5.0`                       | `5.0`            | `5`   ← wrong          |
| `(/ 9.0 2.0)`  (non-whole)  | `4.5`            | `4.5` ✓                |

Non-whole floats are fine (the fractional digits survive); only a float that happens to be integer-valued
loses the `.0`.

## Root cause (as far as the guide can see)

The guide runs a compiled component through **jco** (`guide/src/runner/runWorker.ts`). For a *scalar*
result it calls the bare exported function and stringifies the JS return value (`String(fn())`). jco
lowers the Cadenza scalar types like this:

- `Int64`            → JS `bigint`
- `Float32`/`Float64`→ JS `number`
- `UInt8/16/32`, `Int32`, and a `Qty.value` result → JS `number` **too**
- `Bool`             → JS `boolean`

So a whole-number float arrives as a JS integer-valued `number` (`5`), and `String(5)` === `"5"` — the
decimal is gone. The reference runner prints `5.0` because it renders from the **static result type**,
which it knows.

**The guide cannot fix this on its own reliably.** The obvious JS-side heuristic — "if it's a `number`
with no fractional part, append `.0`" — is UNSOUND, because sized integers (`UInt8.wrap 258` → `2`) and
`Qty.value` results (`3000`, `8`, `12`) are *also* JS `number`s and would be wrongly decorated to
`2.0`/`3000.0`. (Verified: that heuristic turned 4 correct sized-int/quantity results red.) The JS value
type alone does not distinguish "float" from "sized int" — only the **static result type** does, and the
scalar run path doesn't have it.

## What would fix it (compiler-side options)

Either would let the guide render a scalar float correctly:

1. **Expose the exported result type on `CompileResult`** (or a small `result_type(component)` in
   `cdz-wasm`), so the runner can format a `Float*` scalar with a forced decimal and leave `Int*`/`Qty`
   alone. Cheapest for the guide.
2. **Route scalar floats through the typed value-encode path** (the same `make()`/`encode()` +
   `render_value` path compound results already use), which carries the type and already renders `5.0`
   correctly (verified: `render_value` on an encoded `(Float64.of-int 5)` → the typed form). Today a
   whole-float `main` emits a bare scalar export instead.

## Minimal reproducer

```
(do (def (main) (Float64.of-int 5)) (export main))
```

Native `cdz-run` → `5.0`. Browser jco run path → `5`.

## Guide-side status (not blocking)

The guide worked around it so its examples stay honest: the two affected Floats examples now use a
non-whole result (`(/ (Float32.of-int 7) (Float32.of-int 2))` → `3.5`, and `floats:2` expects `2.5`),
which renders identically in both runners. When (1) or (2) lands, those can go back to whole-float values
and the guide's two-surface harness (`guide/scripts/check-examples.mjs`) will grade them against `5.0`.

<!-- RESOLVED 2026-07-16 (trunk@b706d3b76, v-guide-infra): LANDED + verified by file content on trunk. -->
