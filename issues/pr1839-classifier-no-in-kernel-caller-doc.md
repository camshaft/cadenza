# PR #1839 review comment — cdz-kernel/src/effect.rs (v-agent-harness) — OPEN

https://github.com/camshaft/cadenza/pull/1839 (fix-forward 2 landed-MR review nits — the fixes for my
#1833/#1834 findings). One residual on the fix.

## Classifier doc says "no in-kernel caller" but it's called from this crate's unit tests (Copilot, effect.rs:102) — doc/accuracy
> The doc comment says the classifier has "no in-kernel caller", but it IS called from the unit tests in
> this same crate. [Also a wording nit.]
"No in-kernel caller" is inaccurate — the crate's own unit tests call it. Reword to "no PRODUCTION/drive
caller yet" (or "not yet wired into the drive loop; exercised by unit tests") so it's accurate about the
test usage. LOW/doc. Fix-forward.
