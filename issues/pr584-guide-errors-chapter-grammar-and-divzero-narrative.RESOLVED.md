# pr584 — guide Errors chapter: grammar + division-by-zero narrative inconsistency (2 Copilot)

Mirrored from GitHub PR #584 review comments (Copilot). Both VERIFIED against `git show trunk`.
PR: https://github.com/camshaft/cadenza/pull/584 (10-MR publish batch)
File: `guide/src/content/chapters/Errors.tsx` — guide/narrative territory.

## id 3608237783 (Errors.tsx:15) — grammar "or up a missing key"
> Grammar: the phrase "or up a missing key" reads like a missing word; it should be "look up a
> missing key".

VERIFIED: "…when you look past the end of a list or up a missing key or ask for a result…". Reads as
an elided "look" (shared verb) but is awkward; "look up a missing key" is clearer. Readability nit.

## id 3608237788 (Errors.tsx:43) — division-by-zero grouped with Option-absence contradicts line 15
> This paragraph groups "dividing by zero" with Option-style absence, but the preceding paragraph (and
> the numeric-model semantics) describe division by zero as a halting/trapping operation rather than
> an Option-valued absence. Consider removing division-by-zero from this list to keep the chapter's
> narrative consistent.

VERIFIED: line 15 says a genuinely-undefined op "like dividing by zero … halts rather than inventing a
value". But line 43 says "Reading past the end of a list, dividing by zero, and a lookup that misses
are all ordinary values your program handles in Cadenza rather than crashes" — directly contradicts
line 15 (div-by-zero TRAPS, it is NOT an Option-valued absence). Real doc-consistency bug; fix = drop
div-by-zero from the line-43 list. This is the more substantive of the two.

## Owner
`guide/src/content/chapters/` = guide narrative → v-guide-editor (area=guide). Both fold into one edit.

---
RESOLVED (corpus-bugfix 2026-07-19, verified on trunk 0d8b661f7): both fixed in guide/src/content/chapters/Errors.tsx.
• Grammar (line 14): now "or look up a missing key" — the missing "look" word added.
• Div-by-zero narrative (lines 17-19): now "A genuinely undefined operation like dividing by zero is a different
  story that HALTS rather than inventing a value … so this chapter is about the kind of absence you can handle" —
  correctly distinguishes division-by-zero (halting/trapping) from Option-style absence, resolving the contradiction
  with the preceding paragraph the reviewer flagged. Guide-narrative nits resolved by v-guide-editor. No action.
