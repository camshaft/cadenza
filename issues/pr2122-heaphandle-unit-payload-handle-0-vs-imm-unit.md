# PR #2122 review — cdz-kernel/src/wasm_host.rs (v-agent-harness) — OPEN — 1 correctness (MED) + 2 LOW [VERIFIED]

https://github.com/camshaft/cadenza/pull/2122 (HeapHandle — host-side value-heap marshalling, slice 1).
Copilot 3 inline: a unit-payload-encoding bug, a call error-classification, and staging docs.

## unit payload for a nullary/None variant is encoded as handle `0`, but the runtime's unit is a dedicated inline handle (`IMM_UNIT`, currently 2) — `0` is a NULL handle → invalid sum payload (Copilot, wasm_host.rs:624 & :1854) — correctness [VERIFIED, MED]
> This comment implies the unit payload for a nullary/`None` variant is handle `0`, but the runtime uses a
> dedicated inline-unit handle (`arr-alloc(0)` / `IMM_UNIT`, currently `2`). Treating `0` as unit would be
> interpreted as a null handle/token in the runtime and can build an invalid sum payload.

VERIFIED in the diff: the sum_new test does `heap.sum_new(1, 0)` with the comment "None option payload:
sum-new(1=None disc, unit=**0**)" (diff:253-254), and the sum_new doc says a None option is `sum-new(1,
unit)` — but it passes `0` AS the unit. Per Copilot the runtime's canonical unit is `IMM_UNIT` (a dedicated
inline-unit handle, currently 2), NOT 0; handle `0` is a null handle/token, so `sum_new(1, 0)` builds a
None-option whose payload is NULL rather than unit → a malformed value-heap sum. So both the doc and the
test mis-encode the unit payload. MED (correctness in the new marshalling; a downstream reader of that sum
gets a null where unit is expected). Fix per Copilot: use the real unit handle — `arr_alloc(0)` / the
`IMM_UNIT` constant — for the nullary/None payload, in both the doc example and the test (`heap.sum_new(1,
imm_unit)`). v-agent-harness should confirm the canonical unit-handle value + thread it through.

## `call_u32s` maps ALL `Func::call`/`post_return` failures to `ComponentError::Trap`, but wasmtime returns non-trap errors for host-side issues (signature mismatch) (Copilot, wasm_host.rs:567) — error-classification [VERIFIED, LOW-MED]
> …wasmtime will also return non-trap errors for host-side issues like a signature mismatch. Classifying
> those as traps makes debugging interface/version mismatches harder and is inconsistent with the more
> careful error classification elsewhere in this module.
VERIFIED — `call_u32s` blanket-maps to `Trap`. A signature/type mismatch (a host-side WIT-version drift,
not a guest trap) then looks like a guest trap, misdirecting debugging. LOW-MED. Fix: classify non-trap
call errors distinctly (mirror the `invoke_component`/dep-forwarding error handling in this module —
Trap only for genuine guest traps, else an Instantiate/host error).

## `SLICE 1`/`next slice` plan-notes in the module doc (Copilot, wasm_host.rs:490 & :630) — doc-staleness [VERIFIED, LOW]
> This doc comment is written as a slice/plan note … likely to go stale once follow-up slices land. Prefer
> documenting the stable current behavior (which ops are exposed and why).
LOW/staging-doc class — describe the ops exposed now; drop "SLICE 1"/"next slice" forward refs. v-agent-
harness owns cdz-kernel/src. The unit-handle-0 is the finding that matters.
