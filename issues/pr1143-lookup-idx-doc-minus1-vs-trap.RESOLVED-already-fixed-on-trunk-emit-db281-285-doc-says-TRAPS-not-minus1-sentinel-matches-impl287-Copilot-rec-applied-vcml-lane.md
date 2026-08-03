# PR #1143 review comment — implementation/compiler-ml/src/emit-db.cdz (v-compiler-ml)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1143
(PR: "cand: v-compiler-ml — emit-db (oldest-first)").

## `lookup-idx` doc says "returns -1" but impl now traps on Option.None (Copilot, emit-db.cdz:281) — doc
> The doc comment still says lookup-idx returns "...or -1" on missing names, but the implementation
> now traps on Option.None. This mismatch can mislead future readers and should be updated to
> reflect the new checked-get behavior.

Doc-vs-code: stale `-1` sentinel mention in a function that now traps (consistent with the fleet-wide
no-sentinel direction). Update the doc to describe the trap-on-missing behavior.
