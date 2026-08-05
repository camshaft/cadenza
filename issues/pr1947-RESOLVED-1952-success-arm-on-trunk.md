# PR #1947 review — cdz-kernel/src/event_ast.rs (v-agent-harness) — correctness/wire-compat [VERIFIED]

https://github.com/camshaft/cadenza/pull/1947 (fix-forward on #1938 — CloseOutcome wire backward-compat).
Copilot (id 3709433968) flags that the textual decoder now DROPS the `(success <payload>)` head that #1938
itself emitted — a gap in this PR's own backward-compat intent.

## `read_close_outcome` no longer accepts `(success <payload>)` — the shape #1938's textual encoder produced → any textual log written in the #1938 window fails to decode (Copilot, event_ast.rs:775) — correctness/wire-compat [VERIFIED]
> `read_close_outcome` no longer accepts the previously-emitted `(success <payload>)` form (it now only
> matches bare payload heads `inline|blob` or `failure`). Since this file's encoder used to produce a
> `success` wrapper, any existing textual logs/events in that shape will now fail to decode even though
> this PR's intent is to be fix-forward/backward-compatible. Consider keeping decode support for both
> shapes: treat `(success <payload>)` as `Success(...)` in addition to the legacy bare payload form.

VERIFIED in the #1947 diff. #1938 (which was on trunk) emitted the textual Success form via
`let h = b.name("success"); … b.list(vec![h, pf])` → `(success <payload>)`. #1947 changes BOTH sides:
- encode: `CloseOutcome::Success(p) => payload_form(b, p)` (bare payload, no `success` head) — correct, matches the legacy pre-#1938 `(closed <payload>)`.
- decode `read_close_outcome`: REMOVES the `"success" => …` arm, now matching only `"inline" | "blob"` (→ Success) and `"failure"`.

So there are THREE textual shapes in the wild across time: legacy pre-#1938 `(closed <inline|blob …>)`
(bare), the #1938-window `(closed (success <payload>))` (wrapped), and the new #1947 bare form (= legacy).
#1947's decoder handles the bare form (legacy + new) and `failure`, but a #1938-window `(success …)` sexpr
now hits the fallthrough → decode error. The PR's whole point is backward-compat, and it restores compat
with the LEGACY shape but drops the INTERMEDIATE (#1938) shape it's fixing forward from.

Severity depends on whether any #1938-window textual log persists — #1938 merged 03:54Z and #1947 is the
fix-forward, so the exposure window is short and may be zero in practice (and the binary codec may be the
durable one, not the textual). But the fix is a free, defensive decode arm: keep `"success" => { let [p] =
form(a, id, "success")?; Ok(Success(read_payload(a, p)?)) }` ALONGSIDE the bare `"inline"|"blob"` arm. That
accepts all three shapes and fully honors the fix-forward intent at zero cost. LOW-MED (bounded by the
#1938 window). v-agent-harness owns cdz-kernel/src — same #1938 cluster.
