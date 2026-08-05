# PR #2187 review — rcdzc/src/effects.rs (v-effects) — OPEN — doc [VERIFIED, LOW] (RECURRENCE of my #2147 doc-attach finding in the re-cut)

https://github.com/camshaft/cadenza/pull/2187 (FIX ao10 — Site-5 #cv-lift gated by refold-servability;
predicate EXACTLY matches the refold gate; #2147 review — a fresh re-cut of the ao10 fix). Copilot 1
inline. NOTE: this is the SAME doc-attach issue I filed on the original #2147 (comment 3717170488), which
v-effects fixed in 0a35b40b — the re-cut appears to have reintroduced it.

## the `///` block above `body_served_by_oneshot_refold` is leftover `hoist_resumptive_conditional` docs (ends with a dangling "See the"); the inserted helper now carries the wrong item's rustdoc + `hoist_resumptive_conditional` loses its doc (Copilot, effects.rs:3522) — doc [VERIFIED, LOW; RECURRENCE]
> The doc comment immediately above `body_served_by_oneshot_refold` appears to be leftover from the
> `hoist_resumptive_conditional` docs (it describes lifting a resumptive conditional, and ends with a
> dangling "See the"). Because this helper was inserted here, those lines now document the wrong item and
> make the Rustdoc misleading.

VERIFIED in the #2187 diff: the pre-existing `hoist_resumptive_conditional` `///` block ends mid-sentence
"...conditional ends up in TAIL position where the tail-resume fold threads state correctly. See the"
(diff:8 — a DANGLING "See the"), then with NO blank line the NEW `body_served_by_oneshot_refold` `///`
block starts (diff:9-20), then `fn body_served_by_oneshot_refold` (diff:21). Consecutive `///` blocks with
no separator concatenate into ONE doc comment on the NEXT item — so BOTH blocks now document
`body_served_by_oneshot_refold`, `hoist_resumptive_conditional` loses its rustdoc, and the "See the"
dangles. LOW/doc.

⚠️ RECURRENCE: this is EXACTLY the doc-attach finding I filed on the ORIGINAL #2147 (comment id 3717170488
— "the `///` block for hoist_resumptive_conditional now concatenates onto the inserted
body_served_by_oneshot_refold"), which v-effects fixed in 0a35b40b ("moved the orphaned block back above
its fn"). This #2187 is a FRESH RE-CUT of the ao10 fix (the correctness conjunct + supersede chain), and
the re-cut evidently branched from a base WITHOUT 0a35b40b's doc fix — so the doc-attach regressed. Fix per
Copilot + matching the earlier 0a35b40b fix: move the `hoist_resumptive_conditional` block (the one ending
"See the …") back to sit directly above `hoist_resumptive_conditional`, leaving only the new helper's block
on the new helper — AND complete the dangling "See the" sentence. v-effects owns rcdzc effects. PR OPEN →
foldable. (Heads-up worth noting to v-effects: the re-cut dropped a previously-landed doc fix — worth a
grep for whether 0a35b40b carried anything else the re-cut lost.)
