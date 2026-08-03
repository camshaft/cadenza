# PR #1692 review comments — cdz-agent-host/src/host.rs (v-agent-harness-host) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1692 (MERGED).

## 1. `[`ControlEffect`]` rustdoc link won't resolve → rustdoc::broken_intra_doc_links CI risk (Copilot, host.rs:90) — doc/CI
> The rustdoc link `[`ControlEffect`]` likely won't resolve in this module (not in scope), which can trip
> `rustdoc::broken_intra_doc_links` in CI.

Fully-qualify the link (`[`cdz_kernel::effect::ControlEffect`]`) or import it, so rustdoc resolves it and
the lint stays green. LOW/doc (but a real CI-lint trip if that lint is denied).

## 2. Test grants `EffectKind::Emit` but host serves `effect_ct::NOW` + comment overstates the grant's role (Copilot, host.rs:876) — test-precision
> This test grants `EffectKind::Emit` but the host only serves `effect_ct::NOW`; the comment suggests the
> grant is required for the seed to complete, but `control/capabilities` [is kernel-answered, doesn't need
> the grant].

Mismatched grant + misleading comment — the seed's capabilities answer is kernel-inline (doesn't consult
the grant), so the `Emit` grant is neither what the host serves (`NOW`) nor required. Align the grant to
what's actually exercised + fix the comment. LOW/test-precision.

## 3. Final assertion only checks *some* non-empty inline payload — could pass on the wrong result (Copilot, host.rs:897) — test-coverage
> The final assertion only checks that some non-empty inline payload was recorded, which could pass even
> if the seeded EffectResult isn't actually the `control/capabilities` answer.

Weak assertion (same class as #1660): tighten to assert the payload IS the capabilities manifest (decode
+ check a known field / family), not merely non-empty. LOW-MED/test-coverage.
