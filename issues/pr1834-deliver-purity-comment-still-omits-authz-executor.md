# PR #1834 review comment — cdz-kernel/src/kernel.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1834 (reword the deliver-determinism test comment — the fix for
my #1828 finding). My recommended reword was itself incomplete.

## "pure function of (starting state, input, reducer)" still omits `authz` + `executor` (Copilot, kernel.rs:1399) — doc/accuracy
> The comment claims `deliver` is a pure function of (starting state, input, reducer), but `deliver` ALSO
> takes `authz` and an `executor` and can produce different outcomes/errors when those differ. Even though
> this test uses a Timer-only reducer (no executor calls), the statement is stronger than the API
> guarantees.
My #1828 note suggested "(starting state, input, reducer)" — but that's STILL incomplete: `deliver` also
takes `authz` + `executor`, both of which affect the outcome. So the "pure function of X" framing needs
ALL inputs, or (cleaner) drop the "pure function of" claim and say what the TEST actually pins ("this
Timer-only reducer path is deterministic given the starting state — no executor/authz calls"). LOW/doc —
my earlier reword suggestion was itself under-specified; Copilot's fuller list is correct. Fix-forward.
