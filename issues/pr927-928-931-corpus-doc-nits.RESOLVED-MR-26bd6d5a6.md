# PR#927/#928/#931 review comments — three corpus doc nits (corpus-bugfix)

Three Copilot review comments, all `spec/semantics/*.sexp` case docs → corpus-bugfix.

## Comment 1 — PR#927, 02-binding:5513 (grammar), id 3684987948; blame `f7683dd1a`

"Grammar: in the docstring, 'resolver that let' should be present tense 'resolver that lets'."

Confirmed (trunk d5df868bc): doc "…a resolver that let the arm's shadow leak backward…" — the parallel
clause is present tense ("or forward-scoped w wrongly breaks"), so "let" → "lets". Doc-only.

## Comment 2 — PR#928, 11-modules:1963 (wrong intermediate), id 3685062459; blame `2031bacce`

"The doc's worked example contains an inconsistent intermediate result: it says '46 at k=0' but the
case's expected output is 16, and the same sentence immediately recomputes to 16."

Confirmed (trunk d5df868bc): doc "…150 + 16 → 166 at k=5; 46 at k=0 — 0·3=0·10 + 16… recompute: k=0:
Circle 0 → 0 → 0·10=0 + 16 = 16)". The "46 at k=0" is a stale wrong intermediate — the same sentence
then recomputes the correct 16 (and the case's k=0 output is 16). Drop/fix "46 at k=0" → "16 at k=0".
Doc-only, pin correct.

## Comment 3 — PR#931, 18-units:3232 (dangling witness ref), id 3685308993; blame `d5df868bc`

"This comment says 'Witness 1 — the accepted arm-local form runs …' but no such witness case follows;
the next forms are exponent-law unit cases."

Confirmed (trunk d5df868bc): the comment "; Witness 1 — the accepted arm-local form runs (pins the
SEMANTICS the inline form must match):" is immediately followed by the case "a nested unit power
multiplies exponents — (m^2)^2 …" (a free-abelian exponent-law case), NOT an arm-local-form witness. The
"Witness 1 — arm-local form" comment is orphaned/misplaced (looks like a leftover from a
handler-arm-slot-typing probe note above it — "Lane guess: v-inference … Probed on trunk fc2b91731").
Fix: remove or correct the dangling "Witness 1 — arm-local form" line so it doesn't mislabel the
adjacent exponent-law cases. Doc/comment-only.

Owner: **corpus-bugfix** (all three `spec/semantics/*.sexp`; `f7683dd1a` / `2031bacce` / `d5df868bc`).
Three independent doc fixes.
