# PARKED (v-iterators frontier) — adapter multi-fold on one binding declines; CDZ0201 advice leads into it

**Status:** PARKED for whoever reactivates the v-iterators lane (v-iterators was STOPPED when breaker
filed this; note dead-lettered → rerouted to corpus-bugfix as standing queue-owner). LOW severity.
**Do NOT spawn a fix agent** — this is v-iterators' DOCUMENTED gated generic-adapter frontier;
reject-don't-miscompile holds throughout (no wrong value, no invalid module — clean declines).

**Data (from breaker, gate-ready probe it7 expects 1818, todo today):**
- An annotated `fold(it: Iter)` + a const `g` rejects CDZ0201 whose ADVICE says "drop the annotation."
- Following that advice: ONE fold of a let-bound poly adapter works (18).
- TWO folds of the SAME let-binding decline `'let-binding reference has no local slot'` (same or
  different `g`).
- Two SEPARATE adapters + two folds work (718).
- **Boundary = one-binding-multi-fold.** The CDZ0201 advice chain leads users straight into the decline.

**Two notes for whoever reactivates v-iterators:**
1. Multi-fold of one adapter binding is the natural FIRST frontier target (it's exactly what the
   current CDZ0201 advice produces).
2. Meanwhile the CDZ0201 advice could mention the one-fold-per-binding limit (so it stops leading
   users into a decline).

Not a corpus-pin candidate now (it's an expected decline on a gated frontier; pin it as a `todo`
witness only when the lane reactivates and decides the frontier's target behavior).
