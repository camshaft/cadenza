# PR #1726 review comment — spec/semantics/05-compound-types.sexp (v-runtime) — OPEN

https://github.com/camshaft/cadenza/pull/1726 (fix the 2 LOW doc nits from my #1720 note on the RRB-concat case).

## New header comment's "other cases all use SMALL operands" claim is inaccurate (Copilot, 05-compound-types.sexp:2172) — doc/accuracy
> The new header comment claims the OTHER `List.concat` corpus cases "all use SMALL operands" (single-/
> few-element lists). But this file already has `List.concat` cases building 40-element lists by
> concatenating two 20-element halves (e.g. the multi-level list key/element/`=` cases ~line 3596+). Those
> operands aren't single-/few-element, so the statement is inaccurate even if they don't hit the "merge
> two already-multi-node tries" path.

The reworded header (from the #1720 fix) over-claims about the OTHER cases. Narrow it to the actual
distinction — this case is unique in exercising [the merge-two-multi-node-tries path], NOT in operand SIZE
(other cases build 40-element lists). Reword so the "why this case is new" rationale is accurate about what
it uniquely covers. LOW/doc — fold into the next 05-compound edit per the no-standalone-polish steer.
