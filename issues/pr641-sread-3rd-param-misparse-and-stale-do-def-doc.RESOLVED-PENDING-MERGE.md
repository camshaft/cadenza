# pr641 — compiler-ml sread: (1) >2-param def MIS-PARSES instead of declining [PARSE reject-gap] + (2) stale read-do-def doc [DOC]

From github-liaison 2026-07-19 (PR#641 Copilot, 2 comments). Both grepped-real, verified on trunk 9edcc7d0a
by corpus-bugfix. Same family as PR#634 (compiler-ml PORT reader reject-gap).

## #1 — sread.cdz:291 read-2nd-param-or-close [PARSE reject-gap, v-compiler-ml]
Handles arity 1+2. But a 3rd param `(def (f a b c) …)`: the else-branch reads `b` as param2, then
`close-paren(s, skip-space(s, k1))` — which TOLERATES a missing `)` — so with `c` still present it does NOT
verify the next char is `)`, and `read-form` then reads `c` as the BODY instead of declining. The fn's own doc
(288-290) ADMITS "A 3rd param isn't yet supported — after 2 params a non-`)` would be mis-read." 
FIX (same shape as PR#634 typed-param): after param2, CHECK the next char is `)` (or check close-paren
succeeded); if a 3rd param is present (non-`)`), return the bodyId=-1 sentinel → DECLINE (read-do-def already
declines on bodyId -1) rather than mis-reading the 3rd param as the body. A >2-arg def must DECLINE (reject-
don't-mis-parse) until the >2-param slice lands.

## #2 — sread.cdz:~341 read-do-def doc [DOC, v-compiler-ml]
The doc says "read it (nullary or single-param)" but the body now threads `param2Id` and handles TWO-param
helpers (record-param2, slice-3d). Update the doc to "nullary, single-, or two-param".

## Routing
compiler-ml/src/sread.cdz = v-compiler-ml (PORT reader; liaison-routing rule, same as PR#634). ROUTED. #1 is
a parse reject-gap (mis-parse → wrong-parse, should decline); #2 trivial doc. VERIFIED loci on trunk 9edcc7d0a.

---
FIXED by v-compiler-ml (MR fd23c9d09, "compiler-ml: decline a >2-param def (def (f a b c) …) instead of
mis-parsing (PR#641) + doc — sread 49/0"), PENDING MERGE (corpus-bugfix 2026-07-19).
• #1: read-2nd-param-or-close now checks the char after param2 is close-paren; a 3rd param → bodyId -1
  sentinel → read-do-def declines (instead of mis-reading the 3rd param as the body). New test
  sr-module-three-param-def-declines.
• #2: read-do-def doc updated to nullary/single/two-param + record-param2.
sread 49/0, conformance-db 60/0. MR real (cites PR#641), not yet on trunk. Tracked-to-close on land;
content-confirm the 3-param decline + the new test + the doc. Renamed .RESOLVED-PENDING-MERGE.
