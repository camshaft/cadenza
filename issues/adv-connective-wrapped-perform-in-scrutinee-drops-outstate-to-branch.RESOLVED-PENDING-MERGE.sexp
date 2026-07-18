; BREAKER FINDING 2026-07-17 (trunk d1d09dfcc worktree binary @195a3fc38 incl. the #11/#15 outstate
; fixes) — SILENT MISCOMPILE (wrong value, both branches of the family): a perform nested inside a
; CONNECTIVE (`and`/`or`) that sits in an IF-CONDITION or MATCH-SCRUTINEE advances the handler state,
; but the taken BRANCH's later perform sees the PRE-condition state — the connective's out-state is
; dropped at the scrutinee boundary.
;
;   (handle St 0 ((tick (u) s (resume (+ s 1) (+ s 1))))
;     (if (and b (> (St.tick) 0)) (St.tick) -99))     ; b=true
;   cond: tick fires (resumes 1, state -> 1); branch tick must resume 2.  OBSERVED: 1 (branch saw s=0).
;
; All three scrutinee faces confirmed wrong (each expect 2, observed 1):
;   - (if (and b (> (St.tick) 0)) (St.tick) -99)            b=true   [and, if-cond]
;   - (if (or b (> (St.tick) 5)) -99 (St.tick))             b=false  [or, if-cond — left false, right
;                                                            evaluates, not >5 -> else; state advanced]
;   - (match (and b (> (St.tick) 0)) (true (St.tick)) …)    b=true   [and, match-scrutinee]
; CONTROLS all correct:
;   - BARE effectful compare in the condition (no connective): (if (> (St.tick) 0) (St.tick) …) -> 2 ✓
;   - LET-BOUND connective: (let ((c (and b (> (St.tick) 0)))) (if c (St.tick) …)) -> 2 ✓
;   - PURE right operand: (if (and b (> 1 0)) (St.tick) …) -> 1 ✓ (no state to lose)
;   - Short-circuit laziness itself: b=false skips the right tick entirely (sc10 -> -1) ✓
; So: the bare-compare path threads the condition's out-state to the branches, and the let-bound form
; threads it; only the CONNECTIVE-wrapped perform inside the scrutinee position loses it. The and/or
; lowering (nested-if desugar) evaluated in scrutinee position drops its out-state at the boundary the
; bare compare threads through — likely the same class as the FIXED #11 (hoist_resumptive_conditional
; / beta_reduce do-preservation, eval.rs Site-4) but on the connective-desugar path that the fix's
; witnesses (helper calls) never exercised.
;
; SEVERITY: silent wrong value under an idiomatic guard shape — `(if (and precheck (effectful-test)) …)`
; is how real code writes guarded effectful dispatch.
;
; Expected: 2 on every face below (the branch perform sees the condition's advanced state).
(case "a connective-wrapped perform in an if condition threads its state advance to the taken branch"
  (doc    "`(if (and b (> (St.tick) 0)) (St.tick) -99)` with b=true, counter seeded 0, arm resumes and
           advances s+1: the condition's tick resumes 1 (state -> 1), so the then-branch's tick resumes
           2. The connective is the ONLY wrinkle — a bare `(> (St.tick) 0)` condition and a let-bound
           `and` both already thread correctly (controls) — yet the and-wrapped form returns 1: the
           connective lowering's out-state is dropped at the scrutinee boundary. The or/match faces
           fail identically; a pure-right `and` control gives 1 correctly.")
  (input  (do
            (effect St (op tick (-> Unit Int64)))
            (def (main (: b Bool))
              (handle St 0
                ((tick (u) s (resume (+ s 1) (+ s 1))))
                (if (and b (> (St.tick) 0))
                    (St.tick)
                    -99)))
            (export main)))
  (call   main (: true Bool))
  (output (: 2 Int64)))

; ---
; ROUTED to v-effects (corpus-bugfix 2026-07-17, VERIFIED trunk: run(true) -> 1, expected 2). Silent
; wrong-value miscompile: connective (and/or) wrapping a perform in an if-cond/match-scrutinee drops its
; state-advance at the scrutinee boundary; the branch perform sees pre-condition state. 3rd face on the
; connective-desugar path (#11 de52b100c / #15 c3704c06a never exercised it — likely eval.rs
; hoist_resumptive_conditional / do-preservation). Not spawning (3-fixer cap). Promote when fixed.

; ── BREAKER DESUGAR SWEEP (2026-07-17, trunk d1d09dfcc) — sharpens the fix locus ──
; FAILING faces (all return 1, expect 2): and/or DIRECTLY in if-cond or match-scrutinee, INCLUDING nested
;   (and b (and (> (St.tick) 0) b)) with the perform in the MIDDLE operand.
; PASSING controls: not()-wrapping an effectful compare in if-cond (2 — `not` is NOT part of the broken
;   desugar); connective in CALL-ARG feeding a fn whose result is branched (102); connective in LET-INIT with
;   a second let (2). So the loss is PRECISELY the connective desugar INLINE in scrutinee/cond position — the
;   same expression one BINDING away threads perfectly.
; V-EFFECTS ROOT-CAUSE (confirmed): And thread arm (effects.rs:3451) desugars (and lhs rhs)->(if lhs rhs
;   false); the If thread arm (3412) returns post-CONDITION state as the if's out-state, dropping the branch
;   advance. Wrong only when that desugared if is itself an outer if's cond / a match scrutinee (a strict-first
;   position whose out-state the outer branches observe). `not` threads its operand directly (no if-desugar), so
;   it's unaffected — matching the control. FIX (when v-effects stack drains): thread the connective-desugar's
;   TAKEN-branch out-state through when the connective sits in a strict-first (cond/scrutinee) position.

; ── V-EFFECTS DEEPER READ (2026-07-17, read-only while stack pending) ──
; PUZZLE to resolve when building: the If thread arm (effects.rs:3390-3412) returns `cur` = the
; POST-CONDITION state (line 3412), NOT the taken-branch's out-state — so naively BOTH the connective-in-
; scrutinee AND the let-bound-connective forms should drop the branch advance. Yet breaker confirms let-bound
; threads correctly (2) while inline-in-scrutinee drops (1). So the real mechanism is NOT simply "If returns
; post-cond state" — there must be a hoist (hoist_resumptive_conditional?) or a let-init-specific path that
; lifts/threads the connective's branch advance in the let-bound case but not when the connective is inline in
; an outer if-cond/match-scrutinee. FIX INVESTIGATION START: (1) trace both forms' threaded output with a
; debug dump; (2) find why let-bound captures the advance; (3) apply the same to the scrutinee/cond position
; (likely: when threading an if/match whose COND/SCRUTINEE is a connective-desugar that advances state, the
; if's out-state must be the taken-branch's, or hoist the connective's rhs advance out to a let before the if).
; NOTE: `not` is unaffected (threads its operand directly, no if-desugar) — matches the control.

; ---
; RESOLVED-PENDING-MERGE (corpus-bugfix 2026-07-17, per v-effects): FIXED in MR 42ed25544. Fix = hoist
; Site 5: bind a performing condition/scrutinee to a let so it becomes a let-init that Site 4 distributes
; (brings the inline form to parity with the already-working let-bound twin). run(true) -> 2 (was 1);
; all faces (and/or in cond, nested, short-circuit) verified; corpus + unit test + 3 baselines shipped.
; Promote/close on land.
