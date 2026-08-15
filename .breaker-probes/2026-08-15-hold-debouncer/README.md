# dbc — hold debouncer: BUDGET ESCAPE face (2026-08-15, tick 1570) — FINDING

dbc1 (signal debouncer with hold time: 2-tuple, 3-branch arm with the hold
compound (+ (% n 3) 1) recomputed mid-cascade, 7 dispatches) emits
1,666,022 bytes of wasm that wasm-tools validate PASSES — under the 7.65M
body-size cap, under the locals cap — but CRANELIFT rejects at compile:
'Compilation error: Code for function is too large'.

A face BETWEEN the caps: the (a) instruction budget is tuned to the
wasm-tools limits, but cranelift's per-function code-size ceiling is LOWER
for this shape, so budget-passing emits still fail at wasmtime compile.
Filed: budget threshold must target cranelift's limit (or catch its error
and decline). Deterministic ×3. Held from corpus pending the threshold fix.
