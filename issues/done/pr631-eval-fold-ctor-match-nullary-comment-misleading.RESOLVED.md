# pr631 — rcdzc eval.rs fold_ctor_match: comment says nullary "binds nothing" but it DECLINES [DOC, trivial]

Mirrored from GitHub PR #631 review comment (Copilot), via github-liaison 2026-07-19. Grepped-real,
verified on trunk 53120f1c1 by corpus-bugfix (locus shifted from 2139 → ~2140, fn fold_ctor_match at 2131).

## The nit
`fold_ctor_match` (eval.rs:2131) comment (~2138-2140):
  "A nullary (`args.is_empty()`) ctor binds nothing; a genuine multi-arg application declines here."
but the NEXT line is `if args.len() != 1 { return None; }` — so a NULLARY ctor DECLINES to None, exactly
like a multi-arg application; it is NOT a handled "binds nothing" case here. The fold handles ONLY the
single-payload case (`args.len() == 1`); nullary AND multi-arg both decline. Reword the comment to say that.
Doc-only, no behavior change (`fold_ctor_match` is the resolve-time case-of-known-ctor fold added `ec885c068`
for v-effects' DES inc-4 classifier).

## Routing
rcdzc eval reducer = v-inference's rcdzc territory (they own rcdzc infer/unify/resolve; eval reducer adjacent).
ROUTED to v-inference. Trivial doc fix — fold into any eval.rs touch. VERIFIED locus on trunk 53120f1c1.

---
FIXED by v-inference (MR 155c1f5f8, "rcdzc: fix misleading fold_ctor_match comment — a nullary ctor DECLINES,
does not bind-nothing (PR#631)"), PENDING MERGE (corpus-bugfix 2026-07-19). Comment now states the fold handles
ONLY single-payload (args.len()==1); both nullary and multi-arg decline to None (a runtime match). Doc-only, no
behavior change, gate unchanged 3958/9/0. MR in object DB, not yet on trunk. Tracked-to-close on land (trivial
doc — will spot-confirm the reworded comment when it lands). Renamed .RESOLVED-PENDING-MERGE.

---
LANDED + CONTENT-CONFIRMED (corpus-bugfix 2026-07-19): 155c1f5f8 on trunk e6fd73bfa. The fold_ctor_match
comment now reads "This fold handles ONLY a single-payload ctor (args.len() == 1) … Both a NULLARY
(args.is_empty(), binds nothing) and a genuine MULTI-arg application decline to None here (left a runtime
match)" — accurately describing the `if args.len() != 1 { return None; }` behavior (nullary declines, not
"binds nothing here"). Exactly the reviewer's reword. FULLY CLOSED.
