# PR #2050 review — cdz-kernel/src/wasm_host.rs (v-agent-harness) — MERGED — 2 substantive + 1 LOW [VERIFIED] (batched)

https://github.com/camshaft/cadenza/pull/2050 (invoke_component — multi-export artifact-set generic invoke,
slice 1). Copilot 3 inline: a dropped-`post_return` correctness bug, a `{val:?}`-in-error DoS vector, and
an empty-interface error-clarity nit. All VERIFIED on trunk.

## `invoke_component` DROPS `post_return`'s result (`let _ = func_handle.post_return(...)`) while a sibling call in the same file propagates it — hides guest traps/out-of-fuel/resource-cleanup failures + can leak resources (Copilot, wasm_host.rs:585) — correctness [VERIFIED]
> `invoke_component` ignores the result of `post_return`. In this file, other component calls propagate
> `post_return` errors (e.g. dep forwarding), and `post_return` can surface traps/out-of-fuel/resource
> cleanup failures. Silently dropping it can hide guest failures and potentially leak resources.

VERIFIED on trunk: `invoke_component` does `let _ = func_handle.post_return(&mut store);` (wasm_host.rs:583)
— result discarded. The dep-forwarding path in the SAME file propagates it: `f.post_return(&mut ctx)?;`
(:460). `post_return` runs the component's cleanup and can surface a trap / out-of-fuel / cleanup failure;
dropping it means a guest whose post-return traps looks successful, and cleanup failures (resource leaks)
go unreported. MED/correctness. Fix: propagate — `func_handle.post_return(&mut store).map_err(|e|
export_err(format!("post_return failed: {e}")))?;` matching the :460 pattern.

## `InvokeExport` error reasons embed `{val:?}` / `{other:?}` of UNTRUSTED guest output → a huge wrong-shape value explodes log/error size = DoS vector (Copilot, wasm_host.rs:603) — security/DoS [VERIFIED]
> These InvokeExport reasons include `{val:?}`. If a component returns a large value of the wrong shape,
> this can explode log/error sizes and become a DoS vector (especially since this is untrusted guest
> output). Prefer a bounded message that doesn't dump the full value.

VERIFIED: `decode_artifact_list` builds errors `format!("result is {val:?}, not a list<…>")` (wasm_host.rs
~600) and `format!("artifact field {want:?} is {other:?}, not a string")` (~608). `val`/`other` are
wasmtime `Val`s decoded from GUEST output — a malicious/buggy guest returning a massive value (e.g. a
multi-MB list, or deeply nested) gets fully `Debug`-formatted into the error string → unbounded
memory/log blowup on the error path. Same DoS class as the #1852 unbounded-set finding. MED/security. Fix:
a bounded message — report the Val's VARIANT/discriminant (`val.ty()` or a `match` naming the kind) and/or
a length-capped repr, never `{:?}` the full untrusted value. (`items.len()` for a list, the field name for
a record — enough to diagnose without dumping.)

## empty-interface (top-level export) lookup error says `interface "" exports no func …` — confusing (Copilot, wasm_host.rs:553) — error-clarity [VERIFIED, LOW]
> When `interface` is empty (top-level export lookup), the error reason currently says `interface ""
> exports no func ...`, which is confusing because there is no interface in that mode. It should report a
> missing top-level function export instead.

VERIFIED: the func-lookup miss builds `format!("interface {interface:?} exports no func {func:?}")`
(wasm_host.rs:552); with `interface == ""` that renders `interface "" exports no func …`. LOW/error-clarity.
Fix: branch on empty interface — `if interface.is_empty() { "component exports no top-level func {func:?}" }
else { "interface {interface:?} exports no func {func:?}" }`. v-agent-harness owns cdz-kernel/src.
