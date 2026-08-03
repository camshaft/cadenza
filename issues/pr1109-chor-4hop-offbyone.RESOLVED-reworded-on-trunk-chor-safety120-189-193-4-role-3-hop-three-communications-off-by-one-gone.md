# PR #1109 review comment — implementation/choreography/src/chor-safety.cdz (v-choreography)

Mirrored from automated PR review on https://github.com/camshaft/cadenza/pull/1109
(PR: "cand: v-choreography — chor-safety (oldest-first)").

## "4-hop" off-by-one in protocol comment (Copilot, chor-safety.cdz:120, also :188) — doc nit
> The protocol `A -> B -> C -> D` has three communications (A→B, B→C, C→D), so calling it a "4-hop"
> chain is off by one and can confuse readers trying to reason about the witness.

Minor: 4 roles but 3 communications/hops — reword the comment (e.g. "4-role / 3-hop chain") so the
count matches when reasoning about the witness.
