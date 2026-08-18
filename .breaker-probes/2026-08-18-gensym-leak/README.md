# FINDING: internal gensym leaks as CDZ0101 unbound-name (2026-08-18)

Minimal trigger (pyg1-min.sexp):
  (handle E n
    ((tick (v) s (+ (resume v s) v)))     ; op-ARG used in the post-resume toll
    (let ((a (E.tick 3)))
      (E.tick a)))                         ; first ANSWER fed as second op's arg
=> cdz: error [CDZ0101]: unbound name `##a356281`
UNIFORM wasm AND rust (same gensym name). Both ingredients required:
- pyg1-ctl-toll.sexp: toll uses a constant instead of v -> COMPILES.
- pyg1-ctl-arg.sexp:  arg is a constant instead of a    -> COMPILES.
- nested feed (E.tick (E.tick 3)) with the v-toll -> clean fold-boundary
  DECLINE (correct behavior class).
So: op-arg-in-toll x answer-fed-arg through a LET = the refold emits a
reference to an internal binder that is no longer in scope. This is an
ILL-FORMED LOWERING surfaced as a user-facing unbound-name error — not a
decline, not a wrong answer: a compiler bug of the ICE class. Filed with
v-effects. The pyv1 corpus pin (op-arg toll, constant args) and pyr10
(let-bound answers, no arg feed) bracket it from the passing sides.

UPDATE (tick 1802, v-effects localization note 039213): confirmed + held
as banked ICE todo. The `##` is a DOUBLE-FRESHEN (freshen_local_binders
once per refold level; two performs = two levels) but that is a SYMPTOM —
even single-freshen desyncs the let binder from its refs under the
refold's splice/rebuild, and the fold-time type_errors guard reports 0
faults (the dangling name appears downstream). Fix direction: re-anchor
binder refs post-rebuild (pyr7-style forget+re-resolve) OR narrow-decline
this shape to match its nested-feed sibling. Deliberately NOT rushed —
the refold is heavily corpus-pinned. Escalation trigger agreed: ping if
it blocks a natural corpus case.

SCOPE LADDER (tick 1802, for the fix's test set — all leak, uniform class):
- pyg2-derived.sexp:  fed arg is a DERIVED local (b = a+1)   -> ##b
- pyg2-dualuse.sexp:  answer used in fold AND as fed arg     -> ##a
- pyg2-skiplevel.sexp: fed binder skips a level (3 performs) -> ###a
  (TRIPLE hash — freshen count scales with refold depth, consistent
  with the once-per-level localization.)
The class is: ANY let-bound value derived from a prior answer, fed as an
op argument, under an op-arg-consuming post-resume toll — depth shows up
as hash count. Fix test set should cover all four (pyg1-min + these).
