# PR #2203 review — cdz-kernel/src/wasm_host.rs (v-agent-harness) — OPEN — 2 MED + 1 LOW-MED + 1 LOW [VERIFIED]

https://github.com/camshaft/cadenza/pull/2203 (B1 e2e — drive the REAL reducer_b1 through
apply_handle_lowered; skips until v-nix wires the env). Copilot 4 inline on the handle-lowered fold path
(§19e). This is the real reducer-ABI integration path — my #2050/#2122/#2151 area.

## the doc says "requires exactly one resolved `cadenza:runtime/heap` dep", but the loop SILENTLY OVERWRITES `runtime_instance` on every prefix match → multiple resolved deps → last-one-wins (nondeterministic), wrong runtime instance, broken heap sharing (Copilot, wasm_host.rs:1445) — correctness [VERIFIED, MED]
> The doc comment says this path "requires exactly one" resolved `cadenza:runtime/heap` dep, but the loop
> silently overwrites `runtime_instance` if multiple resolved deps match the prefix. That can
> nondeterministically select the wrong runtime instance and break heap sharing. Fail loud if more than
> one runtime dep is present.
VERIFIED in the #2203 diff: doc "Requires exactly one resolved runtime dep exporting `cadenza:runtime/heap`"
(diff:56); loop `for (import_name, bytes) in &self.resolved_deps { … if
import_name.starts_with("cadenza:runtime/heap") { runtime_instance = Some(inst); } }` (diff:79-90) — every
prefix match reassigns `runtime_instance`, so with >1 match the LAST wins, and `resolved_deps` iteration
order isn't guaranteed stable → nondeterministic wrong-instance pick. No fail-loud, no first-wins. MED
(silent-overwrite/fail-loud class — same as my #2160 multi-address bypass). Fix per Copilot: fail loud if
>1 resolved dep matches the prefix (or assert exactly-one), matching the doc's "exactly one".

## on `HeapHandle::bind` failure, `apply_handle_lowered` returns `Kv::new()` (EMPTY) — losing the caller's base KV, violating the "any error hands the base KV back" atomicity contract (Copilot, wasm_host.rs:1487) — correctness/data-loss [VERIFIED, MED]
> On `HeapHandle::bind` failure, `apply_handle_lowered` returns `Kv::new()`, losing the caller's base KV.
> This breaks the function's own atomicity contract ("any error hands the base KV back") and is
> inconsistent with `ComponentReducer::apply`… Consider changing `HeapHandle::bind` to return the consumed
> `Store<T>` on error (so the caller can recover `ReducerHost::into_kv()`)…
VERIFIED in the diff: EVERY other error path returns the real KV — instantiate-fail `store.into_data()
.into_kv()` (diff:120-121), set_fuel-fail `heap.into_store().into_data().into_kv()` (diff:137-138). But the
bind-fail arm does `return Err((e, Kv::new()))` (diff:132) with the code's OWN comment admitting it: "bind
failed → the store is consumed by bind's error path; can't recover the KV, so surface with an empty KV"
(diff:129-131). So a `HeapHandle::bind` failure DROPS the caller's base KV → the caller (who `mem::take`-d
it out expecting atomic restore on error) loses state. MED/data-loss (violates the function's stated
contract + diverges from `ComponentReducer::apply` which always restores). Fix per Copilot: make
`HeapHandle::bind` return the consumed `Store<T>` in its `Err` so this path can recover
`into_data().into_kv()` — never drop the base KV.

## `call_apply_lowered` reuses `call_u32s`, which labels non-trap failures `Instantiate("heap-op call failed …")` — misleading when the failure is in the reducer's `apply` (Copilot, wasm_host.rs:897) — error-classification [VERIFIED-plausible, LOW-MED]
> …classifies non-trap failures as `Instantiate("heap-op call failed …")`. When the failure is actually
> in the reducer's `apply` call, that message is misleading and makes debugging reducer-ABI mismatches
> harder. Consider classifying with reducer-specific context (still preserving the OutOfFuel/trap split).
VERIFIED-plausible: `call_apply_lowered` reuses `call_u32s` (the heap-op helper), so a reducer-apply-call
failure inherits the "heap-op call failed" label — misdirects debugging of a reducer-ABI mismatch. LOW-MED
(same error-mislabel class as my #2122 `call_u32s` blanket-Trap finding). Fix: wrap with reducer-apply
context, keeping the OutOfFuel/trap split.

## the "declares no `cadenza:runtime/heap` dep" error fires when no RESOLVED dep was composed — misleading if the reducer DID declare it but the host didn't attach/resolve it (Copilot, wasm_host.rs:1453) — error-wording [VERIFIED, LOW]
> …the message says the reducer "declares no … dep", but the condition is that `self.resolved_deps` lacked
> it. Misleads when the reducer declares the dep but the host forgot to attach/resolve it via
> `with_resolved_deps`.
VERIFIED: the arm (diff:92-97) fires on `runtime_instance == None` (no RESOLVED dep composed), but the
message is "handle-lowered reducer declares no cadenza:runtime/heap dep to marshal on" (diff:97). A reducer
CAN declare the dep while the host omitted `with_resolved_deps` → the message misattributes to the reducer.
LOW/error-wording. Fix: "no RESOLVED cadenza:runtime/heap dep composed (attach it via with_resolved_deps)".

The two MED (silent-overwrite + bind-fail KV-loss) matter — both on the real reducer fold path. v-agent-
harness owns cdz-kernel/src. PR OPEN → all foldable pre-merge.
