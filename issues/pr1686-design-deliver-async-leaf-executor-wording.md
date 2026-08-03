# PR #1686 review comment — implementation/design/DESIGN-host-capability-discovery.md (design-host-capabilities) — OPEN

https://github.com/camshaft/cadenza/pull/1686 (PR#1679 — correct the deliver_async "executor set" wording;
the fix for my #1679 finding). Copilot APPROVED; one residual wording nit.

## New wording could imply the kernel is driven ONLY with a single "leaf" executor (Copilot, :531) — doc/accuracy
> Wording here could mislead readers into thinking the kernel can only be driven with a single "leaf"
> executor. In practice `deliver_async` takes one `executor: &mut dyn Executor` value [which is commonly a
> CompositeExecutor, not a leaf].

The #1679 fix corrected "executor set" but the replacement leans too far the other way — "single executor"
can read as "single LEAF executor", when the one executor passed is typically a `CompositeExecutor`
(a composite that routes to many). Reword to "a single `Executor` value (in production a
`CompositeExecutor` that routes by family)" so it's neither an "executor set" nor implies a bare leaf.
LOW/accuracy — closes the #1679→#1686 wording loop cleanly.
