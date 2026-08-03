; adv-62 (breaker, 2026-08-02, HIGH wasm soundness — host-effect DUPLICATION): a let-bound host-call
; result captured by TWO OR MORE escaping closures RE-FIRES the host call per closure instead of
; evaluating once and sharing the bound value. v-effects ruled the delegation itself CORRECT
; (a host block in a non-exported helper reachable from the entrypoint delegates — no static
; reject) and diagnosed the duplication as the defect; they OWN the fix (their note, 2026-08-02).
;
; observed:  wasm runs io.get TWICE -> 'host call io.get has no recorded response (call 2 of the
;            run; 1 response supplied)'. rust declines (todo) so no cross-backend value split.
; expected:  ONE firing; both closures capture the same bound v=7 -> (7+3) + 100*(7*3) = 2110.
; brackets:  SINGLE-closure capture of the same let-bound host call in the same helper shape
;            fires once and runs (v-effects re-verified: 15). The exported-def twin is
;            corpus-pinned (21-host-closures:307). The in-exported-def TWO-closure shape (e3)
;            PASSES with one firing — the bug needs helper-def + >=2 escaping closures.
; severity:  a real program would double-fire an observable effect (read a clock twice, consume
;            two queue inputs) silently when refactoring one closure into two.
(case "e1 TWO returned closures capturing the SAME let-bound host call share ONE firing"
  (input  (do
            (effect io (op get (-> Unit Int64)))
            (def (mk)
              (host (io)
                (let ((v (io.get unit)))
                  (tuple (fn ((: x Int64)) (+ v x))
                         (fn ((: x Int64)) (* v x))))))
            (def (main (: k Int64))
              (match (mk)
                ((tuple f g) (+ (f k) (* 100 (g k))))))
            (export main)))
  (host-responses (respond io.get (: 7 Int64)))
  (host-calls (call io.get))
  (call   main (: 3 Int64)) (output (: 2110 Int64)))

; --- EXTRA FACES for the on-land pin set (breaker-verified on fix 757ae079f; VALIDATE after I sync) ---
; e1 alone pins two-closure single-firing; these add coverage the fix should also hold:
; (a) THREE closures + a direct body read all sharing ONE firing of one let-bound host call.
; (b) TWO DISTINCT let-bound host calls, each captured by its own escaping closure, firing ONCE EACH
;     and IN ORDER (host-calls [io.a, io.b]) — the order-across-captured-calls face, NOT covered by e1.
; NOTE: must VALIDATE these on a synced base (adv-62 fix 757ae079f is ahead of my current base) before
; committing — exact outputs/host-call rows to be confirmed then. Sketch shapes (refine on validate):
; (a) (host (io) (let ((v (io.get unit))) (tuple (fn (x) (+ v x)) (fn (x) (* v x)) (fn (x) (- v x)))))
;     + main destructures all three and reads v-derived results → one io.get row.
; (b) (host (io) (let ((a (io.a unit)) (b (io.b unit))) (tuple (fn (x) (+ a x)) (fn (x) (* b x)))))
;     → host-calls [io.a, io.b] in order, each once.
