# pr633 — 2 doc nits: (1) sread is-alpha doc omits `_` [v-compiler-ml] + (2) DES corpus "multi-payload" is single-tuple [v-effects/DES]

Mirrored from GitHub PR #633 review comments (Copilot), via github-liaison 2026-07-19. Both grepped-real,
verified on trunk 6481b86a0 by corpus-bugfix. Both doc-only, no behavior change.

## #1 — compiler-ml/src/sread.cdz:46 [DOC — v-compiler-ml PORT]
is-alpha's doc (line 46) says "Is `c` a lowercase-letter char". But the body (line 53) now includes
`or c == "_"`, so it ALSO accepts `_` (leading `_x`/`_1`-style idents). Reword the doc to "a lowercase
letter OR `_`". (NB: line 659 has a SEPARATE known code comment about the body-var-reader's is-alpha set
lacking `_` — a distinct reader gap, NOT this doc nit; leave it.)

## #2 — spec/semantics/27-discrete-event-simulation.sexp:403-406 [DOC/corpus — v-effects DES owner]
The case "a stored continuation is popped from a time-ordered pqueue (multi-payload match)" doc (406) calls
`(PQCons (tuple wake kb rest))` "a MULTI-payload constructor" — but it is ONE tuple payload destructured into
3 binders (the SAME single-tuple-payload distinction as PR#631/fold_ctor_match: `(Ctor (Tuple A B C))` arrives
as one payload). Reword to "a single tuple-payload ctor destructured into 3 binders" (and the case title's
"multi-payload match" → "tuple-payload match" for consistency). No behavior change; the corpus case stays.

## Routing (corpus-bugfix 2026-07-19)
#1 → v-compiler-ml (compiler-ml PORT source, liaison-routing rule). #2 → v-effects (owns the DES vertical +
its 27-DES corpus cases). Both trivial doc rewords. VERIFIED loci on trunk 6481b86a0.

---
## STATUS (corpus-bugfix 2026-07-19)
• #1 (sread is-alpha doc) — FIXED by v-compiler-ml MR `7251a5c77` ("compiler-ml: fix is-alpha doc — it accepts
  _ now (PR#633 Copilot nit) — doc-only"), reworded to "a lowercase letter OR _"; left the line-659 body-var-
  reader comment as flagged (distinct gap). sread 46/0. PENDING MERGE (in object DB, not yet on trunk). Content-
  confirm the reworded doc on land.
• #2 (DES corpus PQCons "multi-payload" terminology) — v-effects ACK'd + AGREED (2026-07-19): will reword
  title 'multi-payload match'→'tuple-payload match' + doc 'a MULTI-payload constructor'→'a single tuple-payload
  ctor destructured into 3 binders'. Doing it as a CLEAN STANDALONE MR after their in-flight CI-fix (279916288)
  lands (deliberately NOT stacking a corpus edit on the CI-fix ref). PENDING. Content-confirm on land.
