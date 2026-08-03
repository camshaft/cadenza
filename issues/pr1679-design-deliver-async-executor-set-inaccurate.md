# PR #1679 review comment — implementation/design/DESIGN-host-capability-discovery.md (design-host-capabilities) — OPEN

https://github.com/camshaft/cadenza/pull/1679 (firm up the I5/I6 reactive-half spec). SUBSTANTIVE-CLARITY
(not a cosmetic — sent immediately per the batch steer's carve-out).

## Bullet describes `deliver_async` as taking an "executor set" + kernel registry — inaccurate vs the real API (Copilot, :534) — doc/accuracy [VERIFIED]
> This bullet describes `deliver_async` as taking an "executor set" and frames the trigger hook as "not
> present today". In the current kernel API the session is driven with a single `Executor` + `Authorize`
> passed per call (no kernel-internal registry/pointer), so this wording is inaccurate and likely to go
> stale. Rephrase to the stable interface/constraint (no in-kernel mutable registry; trigger attaches to
> whatever durable mutation §20b introduces) rather than current-state assertions.

VERIFIED against kernel.rs:327 — `deliver_async(&mut self, body, cause, reducer: &dyn Reducer, authz:
&(impl Authorize), executor: &mut (impl Executor))`: a SINGLE executor + authz passed PER CALL, no
kernel-internal executor-set/registry. The design bullet's "executor set" framing could mislead an I5/I6
implementer about the drive API's shape. Rephrase to the stable constraint (no in-kernel mutable registry;
the reactive trigger attaches to the §20b durable mutation) rather than the "executor set / not present
today" current-state assertion. LOW-MED/accuracy — worth a round since it's an API-shape claim on a spec
others build against.
