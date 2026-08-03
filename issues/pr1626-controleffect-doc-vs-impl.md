# PR #1626 review comment — cdz-kernel/src/effect.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1626 (control/* partition + ControlEffect channel, beat 3b).

## ControlEffect docs name wrong method + drop the control/ prefix (Copilot, effect.rs:126) — doc/accuracy
> The `ControlEffect` docs say control effects are returned from `Session::deliver_async` and that
> `request.content_type.family` is e.g. `summary`, but the implementation returns them from
> `deliver_async_control` and the family strings include the `control/` prefix (e.g. `control/summary`).

VERIFIED via the comment vs the beat-3b partition work (families ARE `control/`-prefixed per #1599/#1614).
Public-facing doc — fix the method name (`deliver_async_control`) and the family example (`control/summary`)
so API consumers aren't misled. LOW/doc.
