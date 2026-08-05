# PR #2312 review — cdz-kernel/tests/reducer_cadenza_b3_e2e.rs (v-agent-harness) — OPEN — test-robustness [VERIFIED, LOW-MED]

https://github.com/camshaft/cadenza/pull/2312 (B3 kernel-e2e — real reducer_b3 bumps kv "count" + emits one
Http effect through the bound handle-ABI kv; the B3 climb my #2290 params-arity + async KV underpin). Copilot
1 inline (id 3724779579, reducer_cadenza_b3_e2e.rs:140).

## `resolve_runtime_deps` supplies `RUNTIME_HEAP_COMPONENT` bytes for EVERY declared dep (`deps.iter().cloned().map(|d| (d, bytes.clone()))`) — if a reducer ever declares >1 dep it silently gives the heap component to unrelated deps → confusing compose/link failures (Copilot, reducer_cadenza_b3_e2e.rs:140) — test-robustness [VERIFIED, LOW-MED]
> `resolve_runtime_deps` uses `RUNTIME_HEAP_COMPONENT` bytes for *every* declared dep. If the reducer ever
> declares more than one dep, this will silently supply the heap component for unrelated deps, which is
> incorrect and will lead to confusing compose/link failures. Make the direct-path override explicitly
> require a single dep (or resolve only the heap dep via the override and fall back to `CDZ_STORE` for the
> rest).

VERIFIED in the #2312 diff: `resolve_runtime_deps` (diff:168) — when `RUNTIME_HEAP_COMPONENT` is set (:169)
it `return deps.iter().cloned().map(|d| (d, bytes.clone())).collect()` (:172), i.e. the SAME heap bytes for
every dep, without checking the dep is actually `cadenza:runtime/heap`. For b3 today (a single heap dep) it's
correct, but the helper is silently wrong for any future >1-dep reducer: an unrelated dep would receive heap
bytes → a deep compose/link failure instead of a targeted error. LOW-MED / test-robustness. This mirrors the
fail-loud-on->1-runtime-dep discipline already in the NON-test `apply_handle_lowered` (which errors "MORE THAN
ONE cadenza:runtime/heap dep") — the test helper should match that rigor. Fix per Copilot: either assert a
single dep on the `RUNTIME_HEAP_COMPONENT` override path (`assert_eq!(deps.len(), 1)` / targeted panic), OR
resolve only the heap dep via the override and fall back to `CDZ_STORE` for the rest. v-agent-harness owns
cdz-kernel/tests. PR OPEN → foldable pre-merge.
