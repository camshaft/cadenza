; Effects and handlers (part 3 of 3) — continuation of 14-effects-and-handlers.sexp, split 2026-08-11
; for parallel-append throughput (glob-enumerated spec/semantics/*.sexp; baselines key on description). Same genre.

(case "op2 an Option RESUME value from a single-site arm — Some carries the excess, None answers the shortfall row"
  (input  (do
            (effect O (op get (-> Int64 (Option Int64))))
            (def (main (: n Int64))
              (handle O n
                ((get (k) s (resume (if (> k s) (Some (- k s)) (None)) (+ s 1))))
                (+ (match (O.get 10) ((Some d) d) ((None) -100))
                   (* 10 (match (O.get 0) ((Some d) d) ((None) -100))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -995 Int64))
  (call   main (: 20 Int64)) (output (: -1100 Int64)))

(case "dw1 a list literal mixing an Int32-annotated element with a wider-inferred literal unifies to (List Int32) — every element renders at the unified element width, uniform across backends (fuzzer cdz-smith differential: rust once emitted a heterogeneous vec![i32,i64])"
  (input  (do
            (def (main) (list (: 127 Int32) 32767))
            (export main)))
  (call   main) (output (: (list 127 32767) (List Int32))))

;; ── conditional aborts under HEAP-state outers + closure crossings + do-def/relay (breaker ab/cc/dd/cn) ──
;; ab = a branch-conditional abort (Bail INNER, heap-state handler OUTER — the folding direction;
;; abort THROUGH an inner resumptive handler stays not-yet-reducible): scalar/string/map states,
;; the abort VALUE as an outer draw, and TWO sequential abort regions (first-only/neither/both).
;; cc = closures crossing handler boundaries the SOUND way (pure init, outer-let captures):
;; pre-built capture applied twice inside, escape-A-apply-under-B, sequential captures, composed
;; g(f(draw)), bound-outside crossing a nested shadow, threaded through a RECURSIVE helper, and
;; carried in a TUPLE. (The performing-init/factory-arg faces are finding #10, held.)
;; dd/cn = do-def orderings (consecutive defs; a bare discarded draw before a LET; a def bound to
;; a whole nested shadow) and seed relays (parent-draw seeds three deep; the inner result flowing UP).

(case "ab1d the scalar twin — a conditional abort under a SCALAR-state outer, pre-abort advance committed"
  (input  (do
            (effect L (op emit (-> Int64)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: n Int64))
              (+ (handle L 10
                   ((emit () s (resume s (+ s 1))))
                   (handle Bail 0
                     ((bail (v) s v))
                     (do
                       (L.emit)
                       (let ((g (if (> n 3) (Bail.bail 99) 0)))
                         (+ g (+ (L.emit) 500))))))
                 (* 1000 n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5099 Int64))
  (call   main (: 0 Int64)) (output (: 511 Int64)))

(case "ab1e a branch-conditional abort under a STRING-state outer handler — the taken abort skips the post-abort emit, the untaken row grows the rope"
  (input  (do
            (effect L (op emit (-> Int64)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: n Int64))
              (+ (handle L "x"
                   ((emit () s (resume (String.byte-len s) (String.concat s "yz"))))
                   (handle Bail 0
                     ((bail (v) s v))
                     (do
                       (L.emit)
                       (let ((g (if (> n 3) (Bail.bail 99) 0)))
                         (+ g (+ (L.emit) 500))))))
                 (* 1000 n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5099 Int64))
  (call   main (: 0 Int64)) (output (: 503 Int64)))

(case "ab2 a conditional abort under a MAP-state outer — the pre-abort insert is committed either way"
  (input  (do
            (effect R (op touch (-> Int64 Int64)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: n Int64))
              (+ (handle R (map (1 10))
                   ((touch (k) s (resume (Map.len s) (Map.insert s k k))))
                   (+ (R.touch 5)
                      (handle Bail 0
                        ((bail (v) s v))
                        (let ((g (if (> n 3) (Bail.bail 77) 0)))
                          (+ g (R.touch 6))))))
                 (* 1000 n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5078 Int64))
  (call   main (: 0 Int64)) (output (: 3 Int64)))

(case "ab3 the abort VALUE is itself a draw from the outer STRING-state handler — the pre-abort dispatch commits before the unwind"
  (input  (do
            (effect L (op emit (-> Int64)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle L "abc"
                ((emit () s (resume (String.byte-len s) (String.concat s s))))
                (handle Bail 0
                  ((bail (v) s v))
                  (let ((g (if (> n 3) (Bail.bail (L.emit)) 0)))
                    (+ g (* 10 (L.emit)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3 Int64))
  (call   main (: 0 Int64)) (output (: 30 Int64)))

(case "ab4 TWO sequential abort regions under one STRING-state outer — an aborted region leaves the rope where it was"
  (input  (do
            (effect L (op emit (-> Int64)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle L "q"
                ((emit () s (resume (String.byte-len s) (String.concat s "z"))))
                (+ (handle Bail 0
                     ((bail (v) s v))
                     (let ((g (if (> n 3) (Bail.bail 50) 0)))
                       (+ g (L.emit))))
                   (* 100 (handle Bail 0
                            ((bail (v) s v))
                            (let ((h (if (> n 100) (Bail.bail 7) 0)))
                              (+ h (L.emit))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 150 Int64))
  (call   main (: 0 Int64)) (output (: 201 Int64))
  (call   main (: 200 Int64)) (output (: 750 Int64)))

(case "cc1 a closure over the fn PARAM built before the handle, applied twice inside with draws — capture stable, draws advance"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (let ((f (fn ((: x Int64)) (* x n))))
                (handle St 3
                  ((next () s (resume s (+ s 2))))
                  (+ (f (St.next)) (f (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 40 Int64))
  (call   main (: 2 Int64)) (output (: 16 Int64)))

(case "cc2 a closure escaping handle A is applied inside handle B — the A-capture is stable while B's draws feed the args"
  (input  (do
            (effect A (op a (-> Int64)))
            (effect B (op b (-> Int64)))
            (def (main (: n Int64))
              (let ((f (handle A n
                         ((a () s (resume s (+ s 1))))
                         (let ((k (A.a))) (fn ((: x Int64)) (+ x k))))))
                (handle B 100
                  ((b () t (resume t (* t 2))))
                  (+ (f (B.b)) (f (B.b))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 310 Int64))
  (call   main (: 0 Int64)) (output (: 300 Int64)))

(case "cc4 TWO closures over SEQUENTIAL draws inside one region — each captures its own read, applied after both bind"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (let ((a (St.next)))
                  (let ((f (fn ((: x Int64)) (+ x a))))
                    (let ((b (St.next)))
                      (let ((g (fn ((: x Int64)) (* x b))))
                        (+ (f 100) (g 10))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 165 Int64))
  (call   main (: 0 Int64)) (output (: 110 Int64)))

(case "cc5 composed closures over three draws — g(f(draw)) where f and g each captured an earlier read"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (let ((a (St.next)))
                  (let ((f (fn ((: x Int64)) (+ x a))))
                    (let ((b (St.next)))
                      (let ((g (fn ((: x Int64)) (* x b))))
                        (g (f (St.next)))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 72 Int64))
  (call   main (: 0 Int64)) (output (: 2 Int64)))

(case "cc6b a closure bound OUTSIDE the outer handle applied inside a nested SHADOW region and after it — capture crosses both boundaries"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (let ((f (fn ((: x Int64)) (+ x (* n 100)))))
                (handle St n
                  ((next () s (resume s (+ s 1))))
                  (+ (handle St 50
                       ((next () t (resume t (* t 2))))
                       (f (St.next)))
                     (f (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1055 Int64))
  (call   main (: 0 Int64)) (output (: 50 Int64)))

(case "cc7 a draw-capturing closure threaded through a RECURSIVE helper — applied per frame, the leaf applies it to a fresh draw"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (walk (: d Int64) (: f (-> Int64 Int64)))
              (if (<= d 0)
                  (f (St.next))
                  (+ (f d) (walk (- d 1) f))))
            (def (main (: n Int64))
              (handle St 100
                ((next () s (resume s (+ s 1))))
                (let ((k (St.next)))
                  (walk n (fn ((: x Int64)) (* x k))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 10700 Int64))
  (call   main (: 0 Int64)) (output (: 10100 Int64)))

(case "cc8 a closure carried in a TUPLE beside a scalar — destructured in one match and applied around advancing draws"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (match (tuple (fn ((: x Int64)) (* x n)) 7)
                  ((tuple f c) (+ (f (St.next)) (+ c (f (St.next))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 62 Int64))
  (call   main (: 2 Int64)) (output (: 17 Int64)))

(case "dd1b consecutive do-DEF draws — both binders hold their reads, the tail draw sees the doubled-twice state"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (* s 2))))
                (do
                  (def a (St.next))
                  (def b (St.next))
                  (+ (* 100 a) (+ (* 10 b) (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 620 Int64))
  (call   main (: 1 Int64)) (output (: 124 Int64)))

(case "dd1d a bare DISCARDED draw before a let-bound draw — the discard advances the state the binder reads"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (* s 2))))
                (do
                  (St.next)
                  (let ((a (St.next)))
                    (+ (* 100 a) (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1020 Int64))
  (call   main (: 1 Int64)) (output (: 204 Int64)))

(case "dd2 a do-DEF bound to a whole nested SHADOW handle — the def holds the inner region's value, the tail draw reads outer"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (do
                  (def inner (handle St 40
                               ((next () t (resume t (* t 3))))
                               (+ (St.next) (St.next))))
                  (+ inner (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 165 Int64))
  (call   main (: 0 Int64)) (output (: 160 Int64)))

(case "cn1 a THREE-deep seed RELAY — each nested shadow's seed is a draw from its parent, strides differ per depth"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (handle St (St.next)
                  ((next () s (resume s (+ s 10))))
                  (handle St (St.next)
                    ((next () s (resume s (+ s 100))))
                    (+ (St.next) (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 110 Int64))
  (call   main (: 0 Int64)) (output (: 100 Int64)))

(case "cn2 the inner shadow's RESULT flows up into the outer computation — a let-bound region value scaled beside an outer draw"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (* s 2))))
                (let ((up (handle St 7
                            ((next () t (resume t (+ t 1))))
                            (+ (St.next) (St.next)))))
                  (+ (* up 10) (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 155 Int64))
  (call   main (: 0 Int64)) (output (: 150 Int64)))

;; ── INLINE performing if-conditions + tuple snapshots (breaker ic/tr) ────────────────────────────
;; The if-condition position is SOUND for inline performs (unlike the match/guard positions of
;; findings #8/#9): ic2 = an inline performing condition (the taken branch reads the advanced
;; state); ic3 = chained else-if conditions each drawing (later conditions fire only on miss);
;; ic4 = a performing-condition HELPER called twice; ic5 = a recursive LOOP whose exit condition
;; draws per iteration (state-determined trip count); ic6 = a draw-conditioned branch selecting
;; BETWEEN two effects (the untaken effect's state untouched). tr1 = a TUPLE snapshot resume
;; value (state, state*10) built from one live state per dispatch.

(case "ic2 an INLINE performing if-condition — the taken branch's draw reads the condition-advanced state"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (* s 2))))
                (if (> (St.next) 4)
                    (+ 100 (St.next))
                    (- 0 (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 110 Int64))
  (call   main (: 2 Int64)) (output (: -4 Int64)))

(case "ic3 CHAINED if-else-if where each condition draws — three rows land in three different arms"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 3))))
                (if (> (St.next) 10)
                    111
                    (if (> (St.next) 5)
                        (+ 200 (St.next))
                        (- 0 (St.next))))))
            (export main)))
  (call   main (: 11 Int64)) (output (: 111 Int64))
  (call   main (: 4 Int64)) (output (: 210 Int64))
  (call   main (: 0 Int64)) (output (: -6 Int64)))

(case "ic4 a helper with a performing IF-condition called twice — each call re-evaluates the condition against the advanced state"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (pick) (if (> (St.next) 6) (St.next) (- 0 (St.next))))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 2))))
                (+ (pick) (* 100 (pick)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 1309 Int64))
  (call   main (: 0 Int64)) (output (: -602 Int64))
  (call   main (: 3 Int64)) (output (: 895 Int64)))

(case "ic5 a recursive LOOP whose exit condition draws per iteration — the iteration count is state-determined"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (spin (: acc Int64))
              (if (> (St.next) 20)
                  acc
                  (spin (+ acc 1))))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 5))))
                (spin 0)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 5 Int64))
  (call   main (: 21 Int64)) (output (: 0 Int64))
  (call   main (: 11 Int64)) (output (: 2 Int64)))

(case "ic6 a draw-conditioned branch selects BETWEEN two effects — the untaken effect's state is untouched by that row"
  (input  (do
            (effect A (op a (-> Int64)))
            (effect B (op b (-> Int64)))
            (def (main (: n Int64))
              (handle A n
                ((a () s (resume s (+ s 1))))
                (handle B 100
                  ((b () t (resume t (* t 2))))
                  (+ (if (> (A.a) 3) (A.a) (B.b))
                     (* 10 (if (> (A.a) 3) (A.a) (B.b)))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 75 Int64))
  (call   main (: 0 Int64)) (output (: 2100 Int64))
  (call   main (: 2 Int64)) (output (: 2100 Int64)))

(case "tr1 a TUPLE snapshot resume value — each dispatch returns (state, state*10), two snapshots differ by the stride"
  (input  (do
            (effect St (op snap (-> (Tuple Int64 Int64))))
            (def (main (: n Int64))
              (handle St n
                ((snap () s (resume (tuple s (* s 10)) (+ s 1))))
                (match (St.snap)
                  ((tuple a b) (match (St.snap)
                                 ((tuple c d) (+ (* 1000 a) (+ (* 100 b) (+ (* 10 c) d)))))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 4060 Int64))
  (call   main (: 0 Int64)) (output (: 20 Int64)))

;; ── DEPTH-stress towers + multiply-consumed draws (breaker dn/rr) ────────────────────────────────
;; dn = handler towers past the pinned depths: dn1 a FIVE-deep same-effect shadow tower (strides
;; 1-5); dn2b an abort under FOUR resumptive frames (extends the three-frame pin; the conditional
;; variant in a strict-plus tail stays not-yet-reducible); dn3 an alternating A/B/A/B tower where
;; BOTH effects share the op name `next` (routing is by effect identity, not op name); dn4
;; SKIP-LEVEL performs crossing two foreign frames to the outermost handler; dn5 a tuple built
;; inside the inner region from BOTH effects' draws, destructured outside. rr = one draw
;; multiply-consumed (squared/scaled/summed) and the square of a draw difference.

(case "dn1 a FIVE-deep same-effect shadow tower — one draw per level plus a doubled draw at the innermost"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (+ (St.next)
                   (handle St 10
                     ((next () s (resume s (+ s 2))))
                     (+ (St.next)
                        (handle St 100
                          ((next () s (resume s (+ s 3))))
                          (+ (St.next)
                             (handle St 1000
                               ((next () s (resume s (+ s 4))))
                               (+ (St.next)
                                  (handle St 10000
                                    ((next () s (resume s (+ s 5))))
                                    (+ (St.next) (St.next))))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 21120 Int64))
  (call   main (: 0 Int64)) (output (: 21115 Int64)))

(case "dn2b an abort under FOUR resumptive frames — the unwind abandons all four pending sums (extends the three-frame pin)"
  (input  (do
            (effect A (op a (-> Int64)))
            (effect B (op b (-> Int64)))
            (effect C (op c (-> Int64)))
            (effect D (op d (-> Int64)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Bail 0
                ((bail (v) s v))
                (handle A 1 ((a () s (resume s (+ s 1))))
                  (handle B 10 ((b () s (resume s (+ s 1))))
                    (handle C 100 ((c () s (resume s (+ s 1))))
                      (handle D 1000 ((d () s (resume s (+ s 1))))
                        (+ (A.a) (+ (B.b) (+ (C.c) (+ (D.d) (Bail.bail 7)))))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 7 Int64)))

(case "dn3 an ALTERNATING A/B/A/B tower where both effects share the op NAME next — each perform homes to its effect's innermost handler"
  (input  (do
            (effect A (op next (-> Int64)))
            (effect B (op next (-> Int64)))
            (def (main (: n Int64))
              (handle A n
                ((next () s (resume s (+ s 1))))
                (handle B 10
                  ((next () s (resume s (+ s 2))))
                  (handle A 100
                    ((next () s (resume s (+ s 3))))
                    (handle B 1000
                      ((next () s (resume s (+ s 4))))
                      (+ (A.next) (+ (B.next) (+ (A.next) (B.next)))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 2207 Int64)))

(case "dn4 SKIP-LEVEL performs from the innermost region — A's draws cross the B and C frames to the outermost handler twice"
  (input  (do
            (effect A (op a (-> Int64)))
            (effect B (op b (-> Int64)))
            (effect C (op c (-> Int64)))
            (def (main (: n Int64))
              (handle A n
                ((a () s (resume s (* s 2))))
                (handle B 10
                  ((b () s (resume s (+ s 1))))
                  (handle C 100
                    ((c () s (resume s (+ s 1))))
                    (+ (A.a) (+ (A.a) (+ (B.b) (C.c))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 125 Int64))
  (call   main (: 1 Int64)) (output (: 113 Int64)))

(case "dn5 a tuple built INSIDE the inner region from BOTH effects' draws, destructured OUTSIDE — data crosses the region boundary"
  (input  (do
            (effect A (op a (-> Int64)))
            (effect B (op b (-> Int64)))
            (def (main (: n Int64))
              (handle A n
                ((a () s (resume s (+ s 1))))
                (match (handle B 50
                         ((b () t (resume t (* t 2))))
                         (tuple (B.b) (+ (A.a) (B.b))))
                  ((tuple x y) (+ (* 100 x) (+ (* 10 y) (A.a)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6056 Int64))
  (call   main (: 0 Int64)) (output (: 6001 Int64)))

(case "rr1 one draw consumed THREE times (squared, scaled, summed) — a single dispatch, the binder multiply-read"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (let ((d (St.next)))
                  (+ (* d d) (+ (* 10 d) (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 81 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -3 Int64)) (output (: -23 Int64)))

(case "rr2 the SQUARE of a difference of two draws — composite arithmetic over one advancing thread, zero row included"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (* s 3))))
                (let ((a (St.next)))
                  (let ((b (St.next)))
                    (* (- b a) (- b a))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 100 Int64))
  (call   main (: -2 Int64)) (output (: 16 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))

;; ── deep pure arithmetic in arm slots + STRING scrutinee routing (breaker aa/sc) ─────────────────
;; aa = heavy pure expressions in the two arm slots: aa1 the resume VALUE is (v+s)^2 - v*s over
;; the op arg AND state; aa2 a QUADRATIC next-state (s^2+1, a negative seed squaring positive);
;; aa3 REMAINDER cycling (n=3 and n=10 give identical answers — period alignment IS the witness);
;; aa4 a THREE-WAY comparison encoded 1/10/100; aa5 DIVISION with a subtracting stride. sc =
;; STRING scrutinee routing (the za dispatch idiom on the String kind): sc1 literal string arms
;; over a let-bound draw (the hi arm re-performs); sc2 EQUALITY of two draws routing the branch
;; (the n=3 row crosses the threshold BETWEEN draws).

(case "aa1 the resume VALUE is a deep pure expression over the op arg AND state — (v+s)^2 - v*s per dispatch"
  (input  (do
            (effect E (op f (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((f (v) s (resume (- (* (+ v s) (+ v s)) (* v s)) (+ s v))))
                (+ (E.f 3) (E.f 2))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 133 Int64))
  (call   main (: 0 Int64)) (output (: 28 Int64))
  (call   main (: -1 Int64)) (output (: 19 Int64)))

(case "aa2 a QUADRATIC next-state (s^2+1) — three dispatches, the state squaring away from the seed"
  (input  (do
            (effect E (op g (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((g () s (resume s (+ (* s s) 1))))
                (+ (E.g) (+ (E.g) (E.g)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 33 Int64))
  (call   main (: 0 Int64)) (output (: 3 Int64))
  (call   main (: -3 Int64)) (output (: 108 Int64)))

(case "aa3 REMAINDER arithmetic in the resume value — (% s 7) cycles as the +5 stride wraps the modulus"
  (input  (do
            (effect E (op g (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((g () s (resume (% s 7) (+ s 5))))
                (+ (E.g) (+ (* 10 (E.g)) (* 100 (E.g))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 613 Int64))
  (call   main (: 0 Int64)) (output (: 350 Int64))
  (call   main (: 10 Int64)) (output (: 613 Int64)))

(case "aa4 a THREE-WAY comparison arm — the resume value encodes gt/eq/lt as 1/10/100 against the advancing state"
  (input  (do
            (effect E (op probe (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((probe (v) s (resume (+ (if (> v s) 1 0) (+ (if (= v s) 10 0) (if (< v s) 100 0))) (+ s 1))))
                (+ (E.probe 5) (+ (E.probe 6) (E.probe 0)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 120 Int64))
  (call   main (: 6 Int64)) (output (: 300 Int64))
  (call   main (: 0 Int64)) (output (: 102 Int64)))

(case "aa5 DIVISION in the resume value with a subtracting stride — quotients shrink as the state walks down"
  (input  (do
            (effect E (op g (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((g () s (resume (/ s 3) (- s 4))))
                (+ (E.g) (+ (* 10 (E.g)) (* 100 (E.g))))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 790 Int64))
  (call   main (: 9 Int64)) (output (: 13 Int64)))

(case "sc1 STRING literal-arm dispatch on a draw — the hi arm re-performs and measures, the lo arm is constant"
  (input  (do
            (effect St (op name (-> Int64 String)))
            (def (main (: n Int64))
              (handle St n
                ((name (k) s (resume (if (> s k) "hi" "lo") (+ s 1))))
                (let ((w (St.name 3)))
                  (match w
                    ("hi" (+ 100 (String.byte-len (St.name 0))))
                    ("lo" 200)
                    (_o 300)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 102 Int64))
  (call   main (: 1 Int64)) (output (: 200 Int64)))

(case "sc2 EQUALITY of two string draws routes the branch — the n=3 row crosses the threshold between draws"
  (input  (do
            (effect St (op name (-> Int64 String)))
            (def (main (: n Int64))
              (handle St n
                ((name (k) s (resume (if (> s k) "big" "sm") (+ s 2))))
                (let ((w1 (St.name 4)))
                  (let ((w2 (St.name 4)))
                    (if (= w1 w2) (String.byte-len (String.concat w1 w2)) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64))
  (call   main (: 3 Int64)) (output (: -1 Int64))
  (call   main (: 0 Int64)) (output (: 4 Int64)))

;; ── NEGATIVE division/remainder uniformity + the overflow boundary (breaker nm) ──────────────────
;; Truncated-vs-floored conventions flip signs on negative operands — a classic silent-divergence
;; hazard between backends. nm1 pins TRUNCATED remainder (dividend sign) bare AND through a
;; handler arm (incl. the exact-multiple row); nm2 pins division truncating toward ZERO the same
;; two ways; nm3 pins the ONLY overflowing division (Int64.min / -1) as a CDZ0304 constant-fold
;; reject; nm4 pins Int64.min over RUNTIME divisors (sign-flip to +2^62, identity to MIN).

(case "nm1 NEGATIVE remainder is TRUNCATED (sign of the dividend) uniformly — bare and through a handler arm"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (+ (* 1000 (% n 7))
                 (handle St n
                   ((next () s (resume (% s 7) (- s 5))))
                   (+ (St.next) (* 10 (St.next))))))
            (export main)))
  (call   main (: -10 Int64)) (output (: -3013 Int64))
  (call   main (: 10 Int64)) (output (: 3053 Int64))
  (call   main (: -7 Int64)) (output (: -50 Int64)))

(case "nm2 NEGATIVE division TRUNCATES toward zero uniformly — bare and through a handler arm"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (+ (* 1000 (/ n 3))
                 (handle St n
                   ((next () s (resume (/ s 3) (+ s 2))))
                   (+ (St.next) (* 10 (St.next))))))
            (export main)))
  (call   main (: -7 Int64)) (output (: -2012 Int64))
  (call   main (: 7 Int64)) (output (: 2032 Int64)))

(case "nm3 the only overflowing DIVISION (Int64.min / -1) is a CONSTANT-fold reject — the runtime-divisor form runs for every other divisor"
  (input  (do
            (def (main) (/ -9223372036854775808 -1))
            (export main)))
  (error  CDZ0304))

(case "nm4 Int64.min divided by RUNTIME divisors — every non-(-1) divisor has an exact Int64 quotient"
  (input  (do
            (def (main (: d Int64)) (/ -9223372036854775808 d))
            (export main)))
  (call   main (: 2 Int64)) (output (: -4611686018427387904 Int64))
  (call   main (: -2 Int64)) (output (: 4611686018427387904 Int64))
  (call   main (: 1 Int64)) (output (: -9223372036854775808 Int64)))

;; ── Float64 effect boundaries + BOOL handler states (breaker fe/bs) ──────────────────────────────
;; fe = Float64 through the effect machinery using EXACT binary fractions (so float equality is
;; legitimate): fe1 a halving state whose three draws sum to exactly 1.75x the seed; fe2 a
;; CLAMP-style arm with a rising floor (single-site resume; the two-site x Float64 form stays
;; not-yet-reducible, matching the two-site x Option boundary); fe3 an Int64 and a Float64 handler
;; INTERLEAVED (the float identity gates the int readout); fe4 float comparisons ROUTING an
;; integer match (the second draw's sign picks the arm). bs = Bool states completing the scalar
;; kind coverage: bs1 a TOGGLE (complementary bit patterns per seed); bs2 a LATCH (once false,
;; every later check answers false).

(case "fe1 a Float64 handler state HALVING per dispatch — three draws sum to exactly 1.75x the seed (exact binary fractions)"
  (input  (do
            (effect F (op next (-> Float64)))
            (def (main (: n Int64))
              (handle F (Float64.of-int n)
                ((next () s (resume s (* s 0.5))))
                (let ((a (F.next)))
                  (let ((b (F.next)))
                    (let ((c (F.next)))
                      (if (= (+ a (+ b c)) (* (Float64.of-int n) 1.75)) 1 0))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64))
  (call   main (: -12 Int64)) (output (: 1 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))

(case "fe2 a CLAMP-style Float64 arm (single-site resume) — below-state args are lifted to the rising floor"
  (input  (do
            (effect F (op clip (-> Float64 Float64)))
            (def (main (: n Int64))
              (handle F 0.0
                ((clip (v) s (resume (if (< v s) s v) (+ s 1.0))))
                (let ((a (F.clip (Float64.of-int n))))
                  (let ((b (F.clip -2.5)))
                    (let ((c (F.clip 3.5)))
                      (if (= (+ a (+ b c)) (+ (Float64.of-int n) 4.5)) 7 (if (= (+ a (+ b c)) 8.0) 8 9)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7 Int64))
  (call   main (: -3 Int64)) (output (: 9 Int64))
  (call   main (: 0 Int64)) (output (: 7 Int64)))

(case "fe3 an Int64 handler and a Float64 handler INTERLEAVED — integer ticks and exact float halving thread independently"
  (input  (do
            (effect I (op tick (-> Int64)))
            (effect F (op half (-> Float64)))
            (def (main (: n Int64))
              (handle I n
                ((tick () s (resume s (+ s 1))))
                (handle F 8.0
                  ((half () t (resume t (* t 0.5))))
                  (let ((i1 (I.tick)))
                    (let ((f1 (F.half)))
                      (let ((i2 (I.tick)))
                        (let ((f2 (F.half)))
                          (if (= (+ f1 f2) 12.0) (+ (* 10 i1) i2) -1))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))

(case "fe4 float comparisons route an integer match — the sign of the second draw picks the arm, exact identities verify inside"
  (input  (do
            (effect F (op next (-> Float64)))
            (def (main (: n Int64))
              (handle F (Float64.of-int n)
                ((next () s (resume s (- s 1.5))))
                (let ((a (F.next)))
                  (let ((b (F.next)))
                    (match (if (< b 0.0) 0 1)
                      (0 (if (= (- a b) 1.5) 11 12))
                      (_o (if (= (+ a b) (- (* 2.0 (Float64.of-int n)) 1.5)) 21 22)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 21 Int64))
  (call   main (: 1 Int64)) (output (: 11 Int64))
  (call   main (: 0 Int64)) (output (: 11 Int64)))

(case "bs1 a BOOL toggle state — four draws alternate true/false from an input-dependent seed"
  (input  (do
            (effect T (op flip (-> Bool)))
            (def (main (: n Int64))
              (handle T (> n 3)
                ((flip () s (resume s (not s))))
                (+ (if (T.flip) 1 0)
                   (+ (if (T.flip) 10 0)
                      (+ (if (T.flip) 100 0) (if (T.flip) 1000 0))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 101 Int64))
  (call   main (: 0 Int64)) (output (: 1010 Int64)))

(case "bs2 a LATCH — once an op arg is false, the state stays false and every later check answers false"
  (input  (do
            (effect T (op check (-> Bool Bool)))
            (def (main (: n Int64))
              (handle T true
                ((check (v) s (resume (and s v) (and s v))))
                (+ (if (T.check true) 1 0)
                   (+ (if (T.check (> n 3)) 10 0)
                      (+ (if (T.check true) 100 0) (if (T.check true) 1000 0))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1111 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))

;; ── HANDLE expressions in literal-element and argument positions (breaker hr) ────────────────────
;; A whole handle region as a VALUE in construction positions: hr1 a record-literal FIELD beside a
;; pure field; hr2 a MIDDLE list element between pure elements; hr3 a map-literal VALUE stored
;; under a key and looked up after; hr4 another effect's op ARGUMENT (B's region computes what A's
;; dispatch consumes, A's state carrying to a second dispatch).

(case "hr1 a HANDLE expression as a record-literal FIELD value — the region's result sits beside a pure field"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (let ((r (record (= a (handle St n
                                    ((next () s (resume s (+ s 1))))
                                    (+ (St.next) (St.next))))
                               (= b 7))))
                (+ (* 100 (. r a)) (. r b))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1107 Int64))
  (call   main (: 0 Int64)) (output (: 107 Int64)))

(case "hr2 a HANDLE expression as a middle LIST element — the region's result sits between pure elements"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (el (: xs (List Int64)) (: i Int64))
              (match (List.at xs i) ((Some v) v) ((None) 0)))
            (def (main (: n Int64))
              (let ((xs (list 3
                              (handle St n
                                ((next () s (resume s (* s 2))))
                                (+ (St.next) (St.next)))
                              9)))
                (+ (el xs 0) (+ (* 10 (el xs 1)) (* 100 (el xs 2))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1053 Int64))
  (call   main (: 1 Int64)) (output (: 933 Int64)))

(case "hr3 a HANDLE expression as a map-literal VALUE — the region's result is stored under a key and looked up after"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (let ((m (map (1 (handle St n
                                 ((next () s (resume s (+ s 3))))
                                 (+ (St.next) (St.next))))
                            (2 50))))
                (+ (match (Map.lookup m 1) ((Some v) v) ((None) -1))
                   (* 100 (match (Map.lookup m 2) ((Some v) v) ((None) -1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5013 Int64))
  (call   main (: 0 Int64)) (output (: 5003 Int64)))

(case "hr4 a whole HANDLE region as another effect's op ARGUMENT — B's region computes the value A's dispatch consumes"
  (input  (do
            (effect A (op scale (-> Int64 Int64)))
            (effect B (op next (-> Int64)))
            (def (main (: n Int64))
              (handle A 10
                ((scale (v) s (resume (* v s) (+ s 1))))
                (+ (A.scale (handle B n
                              ((next () t (resume t (* t 2))))
                              (+ (B.next) (B.next))))
                   (A.scale 1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 161 Int64))
  (call   main (: 0 Int64)) (output (: 11 Int64)))

;; ── WRAPPING arithmetic through the effect thread (breaker wa) ───────────────────────────────────
;; Int64.wrapping-add composed with handler state: wa1 wrapping as the NEXT-STATE (a near-MAX seed
;; wraps to a concrete near-MIN value; the unwrapped row lands the mismatch arm); wa2 wrapping the
;; op ARG with state in the resume value (wrap / exact-MAX-boundary / identity rows); wa3 the wrap
;; WALK — three draws step exactly MAX, MIN, MIN+1 while checked '+' operates beside the wrapped
;; values. (Comparing wrapped values by DIFFERENCE would overflow the checked '-'; these pin
;; concrete constants instead.)

(case "wa1 wrapping-add as the next-state — a near-MAX seed wraps to a concrete near-MIN value on the second draw"
  (input  (do
            (effect W (op bump (-> Int64)))
            (def (main (: n Int64))
              (handle W 9223372036854775800
                ((bump () s (resume s (Int64.wrapping-add s n))))
                (if (= (W.bump) 9223372036854775800)
                    (if (= (W.bump) -9223372036854775806) 1 2)
                    3)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1 Int64))
  (call   main (: 3 Int64)) (output (: 2 Int64)))

(case "wa2 wrapping-add of the op ARG and state in the resume value — wrap, exact-MAX, and identity rows"
  (input  (do
            (effect W (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle W n
                ((add (v) s (resume (Int64.wrapping-add v s) s)))
                (W.add 9223372036854775800)))
            (export main)))
  (call   main (: 10 Int64)) (output (: -9223372036854775806 Int64))
  (call   main (: 7 Int64)) (output (: 9223372036854775807 Int64))
  (call   main (: 0 Int64)) (output (: 9223372036854775800 Int64)))

(case "wa3 the wrap WALK — three draws from a MAX seed step MAX, MIN, MIN+1 through wrapping-add(+1), checked '+' beside"
  (input  (do
            (effect W (op cyc (-> Int64)))
            (def (main (: n Int64))
              (handle W 9223372036854775807
                ((cyc () s (resume s (Int64.wrapping-add s 1))))
                (let ((a (W.cyc)))
                  (let ((b (W.cyc)))
                    (let ((c (W.cyc)))
                      (+ (if (= a 9223372036854775807) 1 0)
                         (+ (if (= b -9223372036854775808) 10 0)
                            (+ (if (= c -9223372036854775807) 100 0) n))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 116 Int64))
  (call   main (: 0 Int64)) (output (: 111 Int64)))

;; ── cross-DEF handler composition + shadow-boundary ARG homing (breaker ed/sb) ───────────────────
;; ed = handlers meeting function boundaries: ed1 two defs each installing their OWN handler for
;; one effect; ed2 a handled def CALLED from inside another def's handle (call-boundary shadowing);
;; ed3 a RECURSIVE def handling per frame under a handled main (the base case draws from the
;; deepest frame); ed4 ONE shared performing helper under TWO different live handlers — each call
;; homes to its caller's region (dynamic scoping). sb = where an op ARG is drawn vs where its
;; dispatch lands: sb1 both home to the INNER handler at a same-effect shadow boundary
;; (n-independent result = outer thread untouched); sb3 the arg draws from B while the dispatch
;; homes to A (both states advance across paired dispatches); sb4 a COMPOSITE seed — B's draw
;; feeds A's add and the result seeds a fresh B shadow.

(case "ed1 TWO defs each install their OWN handler for one effect — main calls both, seeds and arms fully independent"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (f1 (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (+ (St.next) (St.next))))
            (def (f2 (: n Int64))
              (handle St (* n 10)
                ((next () s (resume s (* s 2))))
                (+ (St.next) (St.next))))
            (def (main (: n Int64))
              (+ (f1 n) (* 100 (f2 n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 15011 Int64))
  (call   main (: 1 Int64)) (output (: 3003 Int64)))

(case "ed2 a handled def CALLED from inside another def's handle — the callee's region shadows the caller's mid-body"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (inner (: k Int64))
              (handle St (* k 100)
                ((next () s (resume s (+ s 7))))
                (+ (St.next) (St.next))))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (+ (St.next) (+ (inner 2) (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 418 Int64))
  (call   main (: 0 Int64)) (output (: 408 Int64)))

(case "ed3 a RECURSIVE def that handles per frame, called from a handled main — the base case draws from the deepest frame's handler"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (tower (: d Int64))
              (if (<= d 0)
                  (St.next)
                  (handle St (* d 1000)
                    ((next () s (resume s (+ s 1))))
                    (+ (St.next) (tower (- d 1))))))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (* s 3))))
                (+ (St.next) (tower 2))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 4006 Int64))
  (call   main (: 0 Int64)) (output (: 4001 Int64)))

(case "ed4 one shared performing helper called under TWO different live handlers — each call homes to its caller's region"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (helper (: k Int64))
              (+ (St.next) k))
            (def (region (: k Int64))
              (handle St (* k 10)
                ((next () s (resume s (+ s 1))))
                (+ (helper 500) (helper 6000))))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 2))))
                (+ (St.next) (+ (region 3) (helper 70)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6643 Int64))
  (call   main (: 0 Int64)) (output (: 6633 Int64)))

(case "sb1 an op ARG drawn at a shadow boundary — both the arg draw and the consuming dispatch home to the INNER handler" (input (do
  (effect St (op add (-> Int64 Int64)) (op next (-> Int64)))
  (def (main (: n Int64))
    (handle St n
      ((add (v) s (resume (+ v s) s))
       (next () s (resume s (+ s 1))))
      (handle St 100
        ((add (v) s (resume (* v s) s))
         (next () s (resume s (+ s 10))))
        (St.add (St.next)))))
  (export main)))
  (call main (: 5 Int64)) (output (: 11000 Int64))
  (call main (: 0 Int64)) (output (: 11000 Int64)))

(case "sb3 the op ARG draws from effect B while the dispatch homes to effect A — two paired dispatches, both states advance"
  (input  (do
            (effect A (op add (-> Int64 Int64)))
            (effect B (op next (-> Int64)))
            (def (main (: n Int64))
              (handle A 10
                ((add (v) s (resume (+ v s) (+ s 1))))
                (handle B n
                  ((next () t (resume t (* t 2))))
                  (+ (A.add (B.next)) (A.add (B.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 36 Int64))
  (call   main (: 0 Int64)) (output (: 21 Int64)))

(case "sb4 the innermost seed is a COMPOSITE of two effects' dispatches — B's draw feeds A's add, the result seeds a fresh B shadow"
  (input  (do
            (effect A (op add (-> Int64 Int64)))
            (effect B (op next (-> Int64)))
            (def (main (: n Int64))
              (handle A 10
                ((add (v) s (resume (+ v s) (+ s 1))))
                (handle B n
                  ((next () t (resume t (* t 2))))
                  (handle B (A.add (B.next))
                    ((next () t (resume t (- t 3))))
                    (+ (B.next) (B.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 27 Int64))
  (call   main (: 0 Int64)) (output (: 17 Int64)))

;; ── list WALKS composed with draws (breaker lf) ──────────────────────────────────────────────────
;; Recursive index-walks (the fold idiom here) meeting the effect thread: lf1 a PURE walk over a
;; DRAW-BUILT list scaled by an earlier draw; lf2 a PERFORMING walk (each visit draws, element
;; order paired with state order); lf3 an arm pushing TWO elements per dispatch where both pushed
;; values read the PRE-push binding (the arm's state is immutable — no sequencing surprise); lf4 a
;; per-element dispatch comparing each element against the RISING state (amplify-or-pass); lf5 two
;; lists in LOCKSTEP with a per-pair 2-arg dispatch, length-guarded via projection helpers (the
;; nested-Option-match x performing-recursive-callee form stays not-yet-reducible — the nesting,
;; not the arity, is the trigger).

(case "lf1 a recursive index-walk over a DRAW-BUILT list, scaled by an earlier draw — the walk itself is pure"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (sum-scaled (: xs (List Int64)) (: i Int64) (: k Int64))
              (match (List.at xs i)
                ((Some v) (+ (* v k) (sum-scaled xs (+ i 1) k)))
                ((None) 0)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (let ((k (St.next)))
                  (let ((xs (list (St.next) (St.next) (St.next))))
                    (sum-scaled xs 0 k)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64))
  (call   main (: 2 Int64)) (output (: 24 Int64)))

(case "lf2 a PERFORMING recursive walk — each element visit draws, pairing element order with state order"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (visit (: xs (List Int64)) (: i Int64))
              (match (List.at xs i)
                ((Some v) (+ (* v (St.next)) (visit xs (+ i 1))))
                ((None) 0)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (visit (list 3 5 7) 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 94 Int64))
  (call   main (: 0 Int64)) (output (: 19 Int64)))

(case "lf3 the arm pushes TWO elements per dispatch (both lengths read from the PRE-push list) — the third draw reads length 5"
  (input  (do
            (effect L (op push2 (-> Int64)))
            (def (main (: n Int64))
              (handle L (list n)
                ((push2 () s (resume (List.len s) (List.push (List.push s (List.len s)) (* (List.len s) 10)))))
                (do
                  (L.push2)
                  (L.push2)
                  (L.push2))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 5 Int64)))

(case "lf4 a per-element dispatch comparing each element against the RISING state — amplify-or-pass per visit"
  (input  (do
            (effect St (op weigh (-> Int64 Int64)))
            (def (walk (: xs (List Int64)) (: i Int64))
              (match (List.at xs i)
                ((Some v) (+ (St.weigh v) (walk xs (+ i 1))))
                ((None) 0)))
            (def (main (: n Int64))
              (handle St n
                ((weigh (v) s (resume (if (> v s) (* v 100) v) (+ s 1))))
                (walk (list 2 9 4) 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 906 Int64))
  (call   main (: 1 Int64)) (output (: 1500 Int64))
  (call   main (: 8 Int64)) (output (: 15 Int64)))

(case "lf5 TWO lists walked in LOCKSTEP with a per-pair 2-arg dispatch — length-guarded via projection helpers"
  (input  (do
            (effect St (op mix (-> Int64 Int64 Int64)))
            (def (pair-or (: xs (List Int64)) (: i Int64))
              (match (List.at xs i) ((Some v) v) ((None) -1)))
            (def (zipwalk (: xs (List Int64)) (: ys (List Int64)) (: i Int64))
              (if (< i (List.len xs))
                  (+ (St.mix (pair-or xs i) (pair-or ys i)) (zipwalk xs ys (+ i 1)))
                  0))
            (def (main (: n Int64))
              (handle St n
                ((mix (a b) s (resume (+ (* a b) s) (+ s 1))))
                (zipwalk (list 1 2 3) (list 10 20 30) 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 158 Int64))
  (call   main (: 0 Int64)) (output (: 143 Int64)))

;; ── enumeration as OBSERVATION across mutating dispatches (breaker mi) ───────────────────────────
;; The static enumeration pins dump once; these observe enumerations ACROSS a mutating sequence:
;; mi1 Map.to-list dumps BEFORE and AFTER keyed inserts (collision rows shrink the delta); mi2
;; SORTED Set enumeration after a draw-keyed insert — positional reads with first/last/collision
;; placements; mi3 the arm aggregates map VALUES via a to-list walk inside the arm (the n=1 row
;; overwrites the seed value — insert-replace semantics through the effect thread).

(case "mi1 enumeration DELTA across dispatches — dumps before and after keyed inserts, collision rows shrink the delta"
  (input  (do
            (effect Db (op put (-> Int64 Int64)) (op dump (-> (List (Tuple Int64 Int64)))))
            (def (main (: n Int64))
              (handle Db (map (1 10))
                ((put (k) m (resume (Map.len m) (Map.insert m k (* k 2))))
                 (dump () m (resume (Map.to-list m) m)))
                (let ((before (List.len (Db.dump))))
                  (do
                    (Db.put n)
                    (Db.put 7)
                    (+ (* 100 (List.len (Db.dump))) (* 10 before))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 310 Int64))
  (call   main (: 1 Int64)) (output (: 210 Int64))
  (call   main (: 7 Int64)) (output (: 210 Int64)))

(case "mi2 SORTED Set enumeration survives a draw-keyed insert — positional reads, the collision row exposes the missing third slot"
  (input  (do
            (effect Sx (op add (-> Int64 Int64)) (op dump (-> (List Int64))))
            (def (at-or (: xs (List Int64)) (: i Int64))
              (match (List.at xs i) ((Some v) v) ((None) -1)))
            (def (main (: n Int64))
              (handle Sx (Set.of (list 20 8))
                ((add (v) s (resume (Set.len s) (Set.insert s v)))
                 (dump () s (resume (Set.to-list s) s)))
                (do
                  (Sx.add n)
                  (let ((xs (Sx.dump)))
                    (+ (* 100 (at-or xs 0)) (+ (* 10 (at-or xs 1)) (at-or xs 2)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 600 Int64))
  (call   main (: 30 Int64)) (output (: 1030 Int64))
  (call   main (: 8 Int64)) (output (: 999 Int64)))

(case "mi3 the arm AGGREGATES map values via a to-list walk — the n=1 row OVERWRITES the seed value, shrinking the total"
  (input  (do
            (effect Db (op put (-> Int64 Int64)) (op total (-> Int64)))
            (def (sum-snd (: xs (List (Tuple Int64 Int64))) (: i Int64))
              (match (List.at xs i)
                ((Some p) (match p ((tuple k v) (+ v (sum-snd xs (+ i 1))))))
                ((None) 0)))
            (def (main (: n Int64))
              (handle Db (map (1 100))
                ((put (k) m (resume (Map.len m) (Map.insert m k k)))
                 (total () m (resume (sum-snd (Map.to-list m) 0) m)))
                (do
                  (Db.put n)
                  (Db.put 3)
                  (Db.total))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 108 Int64))
  (call   main (: 1 Int64)) (output (: 4 Int64))
  (call   main (: 3 Int64)) (output (: 103 Int64)))

;; ── string CONTENT built, sliced, gated, and measured through the effect thread (breaker sg) ─────
;; sg1 a string BUILT by a walk of draws (EXACT content compared — dropped/doubled/reordered
;; dispatches visible in the string); sg2 a stable PREFIX slice of a growing rope (String.slice
;; start,END; returns Option); sg3 a ONE-SHOT string lock (the arm string-compares and consumes
;; the key); sg4 TWO-SIDED rope growth (the op arg's sign picks append vs prepend); sg5 the
;; growing rope as a MAP KEY per dispatch; sg6 the rope's LENGTH PARITY routing its own growth
;; (self-referential feedback); sg7 byte-len vs scalar-len DIVERGING on a multi-byte rope (the
;; first UTF-8-width pin through the effect thread).

(case "sg1 a string BUILT by a recursive walk of draws — one H/L character per dispatch, exact content compared"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (build (: d Int64) (: acc String))
              (if (<= d 0)
                  acc
                  (build (- d 1) (String.concat acc (if (> (St.next) 4) "H" "L")))))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 2))))
                (let ((w (build 3 "")))
                  (if (= w "LHH") 1 (if (= w "HHH") 2 (if (= w "LLH") 3 0))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1 Int64))
  (call   main (: 5 Int64)) (output (: 2 Int64))
  (call   main (: 1 Int64)) (output (: 3 Int64)))

(case "sg2 a stable PREFIX slice of a growing rope — String.slice (start,END) of the state per dispatch, prefix identical across growth"
  (input  (do
            (effect St (op grow (-> String)))
            (def (main (: n Int64))
              (handle St "ab"
                ((grow () s (resume (match (String.slice s 0 2) ((Some p) p) ((None) "?"))
                                    (String.concat s "cd"))))
                (let ((p1 (St.grow)))
                  (let ((p2 (St.grow)))
                    (if (= p1 p2) (String.byte-len (String.concat p1 p2)) -1)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 4 Int64)))

(case "sg3 a ONE-SHOT string lock — the arm string-compares op arg vs state, consuming the key on the first match"
  (input  (do
            (effect Lock (op try (-> String Int64)))
            (def (main (: n Int64))
              (handle Lock "key"
                ((try (w) s (if (= w s) (resume 1 "used") (resume 0 s))))
                (+ (Lock.try (if (> n 3) "key" "nope"))
                   (+ (* 10 (Lock.try "key")) (* 100 (Lock.try "used"))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 101 Int64))
  (call   main (: 0 Int64)) (output (: 110 Int64)))

(case "sg4 TWO-SIDED rope growth — the op arg's sign picks append-right vs prepend-left, exact content across three sign patterns"
  (input  (do
            (effect St (op tag (-> Int64 String)))
            (def (main (: n Int64))
              (handle St "M"
                ((tag (side) s (resume s (if (> side 0)
                                             (String.concat s "R")
                                             (String.concat "L" s)))))
                (do
                  (St.tag n)
                  (St.tag (- 0 n))
                  (St.tag n)
                  (let ((w (St.tag 0)))
                    (if (= w "LMRR") 1 (if (= w "LLMR") 2 (if (= w "LLLM") 3 0)))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64))
  (call   main (: -1 Int64)) (output (: 2 Int64))
  (call   main (: 0 Int64)) (output (: 3 Int64)))

(case "sg5 the GROWING string state is a MAP KEY per dispatch — each draw looks up the current rope in a literal map"
  (input  (do
            (effect St (op adv (-> Int64)))
            (def (main (: n Int64))
              (handle St "a"
                ((adv () s (resume (match (Map.lookup (map ("a" 10) ("ab" 20) ("abb" 30)) s)
                                     ((Some v) v)
                                     ((None) -1))
                                   (String.concat s "b"))))
                (+ (St.adv) (+ (* 10 (St.adv)) (* 100 (St.adv))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 3210 Int64)))

(case "sg6 the rope's LENGTH PARITY routes its own growth — odd appends two, even appends one; four draws read 1,3,4,5"
  (input  (do
            (effect St (op step (-> Int64)))
            (def (main (: n Int64))
              (handle St "x"
                ((step () s (resume (String.byte-len s)
                                    (if (= (% (String.byte-len s) 2) 0)
                                        (String.concat s "a")
                                        (String.concat s "bb")))))
                (+ (St.step) (+ (* 10 (St.step)) (+ (* 100 (St.step)) (* 1000 (St.step)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 7531 Int64)))

(case "sg7 byte-len vs scalar-len DIVERGE on a growing multi-byte rope — each dispatch reads the difference (one per accent)"
  (input  (do
            (effect St (op grow (-> Int64)))
            (def (main (: n Int64))
              (handle St "é"
                ((grow () s (resume (- (String.byte-len s) (String.scalar-len s))
                                    (String.concat s "é"))))
                (+ (St.grow) (+ (* 10 (St.grow)) (* 100 (St.grow))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 321 Int64)))

;; ── NESTED and mixed tuples through the effect boundary (breaker tt) ─────────────────────────────
;; tt1 a nested-tuple op ARG destructured two levels inside the arm; tt2 nested-tuple RESUME
;; values — the fully-nested body match declines (the nested-match x multi-dispatch boundary), the
;; outer-match + inner-PROJECTION form folds; tt3 a tuple ROTATED through two chained dispatches
;; (each rotation folding the live state into the moved slot); tt4 a MIXED String+Int tuple state
;; (rope grows in one slot, the counter folds the op arg in the other); tt5 an inner pair SWAPPED
;; on counter parity — nested match works fine IN THE ARM (the body x multi-dispatch conjunction
;; is the boundary, not nesting per se).

(case "tt1 a NESTED-tuple op ARG destructured two levels inside the arm — both dispatches read the live state"
  (input  (do
            (effect E (op deep (-> (Tuple (Tuple Int64 Int64) Int64) Int64)))
            (def (main (: n Int64))
              (handle E n
                ((deep (p) s (match p
                               ((tuple inner c) (match inner
                                                  ((tuple a b) (resume (+ (* 100 a) (+ (* 10 b) (+ c s)))
                                                                       (+ s 1))))))))
                (+ (E.deep (tuple (tuple 1 2) 3)) (E.deep (tuple (tuple 4 5) 6)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 590 Int64))
  (call   main (: 0 Int64)) (output (: 580 Int64)))

(case "tt2 NESTED-tuple resume values across two dispatches — outer destructured by match, inner read by PROJECTION"
  (input  (do
            (effect E (op snap (-> (Tuple Int64 (Tuple Int64 Int64)))))
            (def (main (: n Int64))
              (handle E n
                ((snap () s (resume (tuple s (tuple (* s 10) (+ s 1))) (+ s 2))))
                (match (E.snap)
                  ((tuple a inner)
                   (let ((b (. inner 0)))
                     (let ((c (. inner 1)))
                       (match (E.snap)
                         ((tuple d inner2)
                          (+ a (+ b (+ c (+ d (+ (. inner2 0) (. inner2 1))))))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 146 Int64))
  (call   main (: 0 Int64)) (output (: 26 Int64)))

(case "tt3 a tuple ROTATED through two chained dispatches — each rotation folds the state into the moved slot"
  (input  (do
            (effect E (op rot (-> (Tuple Int64 Int64 Int64) (Tuple Int64 Int64 Int64))))
            (def (main (: n Int64))
              (handle E n
                ((rot (p) s (match p
                              ((tuple a b c) (resume (tuple b c (+ a s)) (+ s 1))))))
                (match (E.rot (E.rot (tuple n 2 3)))
                  ((tuple x y z) (+ (* 100 x) (+ (* 10 y) z))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 408 Int64))
  (call   main (: 0 Int64)) (output (: 303 Int64)))

(case "tt4 a MIXED String+Int tuple state — the arm grows the rope and folds the op arg into the counter per dispatch"
  (input  (do
            (effect E (op log (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E (tuple "go" n)
                ((log (v) s (match s
                              ((tuple w k) (resume (+ (String.byte-len w) k)
                                                   (tuple (String.concat w "!") (+ k v)))))))
                (+ (E.log 100) (+ (* 10 (E.log 0)) (* 100 (E.log 5))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 11987 Int64))
  (call   main (: 0 Int64)) (output (: 11432 Int64)))

(case "tt5 an inner PAIR swapped on counter parity — a nested-tuple state machine, both slots and the counter observed"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple (tuple n 100) 0)
                ((tick () s (match s
                              ((tuple pr c) (match pr
                                              ((tuple x y) (resume (+ x c)
                                                                   (if (= (% c 2) 0)
                                                                       (tuple (tuple y x) (+ c 1))
                                                                       (tuple (tuple x y) (+ c 1))))))))))
                (+ (E.tick) (+ (* 10 (E.tick)) (+ (* 100 (E.tick)) (* 1000 (E.tick)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 19215 Int64))
  (call   main (: 0 Int64)) (output (: 14210 Int64)))

;; ── arm-INTERNAL computation shapes (breaker al) ─────────────────────────────────────────────────
;; What an arm may compute before resuming: al1 chained LET locals feeding both slots; al2 a PURE
;; helper called from the arm for both slots; al3 a two-site resume where EACH branch binds its own
;; local (per-branch strides); al4 ONE shared local feeding BOTH slots (single-eval — the
;; copy-vs-alias hazard shape); al5 an arm-local named the SAME as a body-side binder (hygiene for
;; arm-internal lets, extending the op-param/state-binder hygiene pin).

(case "al1 chained LET locals inside the arm — intermediate names feed both the resume value and the next-state"
  (input  (do
            (effect E (op f (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((f (v) s (let ((doubled (* v 2)))
                            (let ((shifted (+ doubled s)))
                              (resume shifted (+ s doubled))))))
                (+ (E.f 3) (E.f 5))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 32 Int64))
  (call   main (: 0 Int64)) (output (: 22 Int64)))

(case "al2 a PURE helper called from the arm for BOTH slots — the arm delegates its computation to a def"
  (input  (do
            (effect E (op f (-> Int64 Int64)))
            (def (mix (: a Int64) (: b Int64)) (+ (* a a) b))
            (def (main (: n Int64))
              (handle E n
                ((f (v) s (resume (mix v s) (mix s 1))))
                (+ (E.f 2) (E.f 3))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 44 Int64))
  (call   main (: 0 Int64)) (output (: 14 Int64)))

(case "al3 a two-site resume where EACH branch has its own LET local — gap/overshoot named per path, different strides"
  (input  (do
            (effect E (op f (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((f (v) s (if (> v s)
                              (let ((gap (- v s))) (resume (* gap 10) (+ s 1)))
                              (let ((over (- s v))) (resume over (+ s 2))))))
                (+ (E.f 8) (+ (E.f 3) (E.f 9)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 43 Int64))
  (call   main (: 0 Int64)) (output (: 170 Int64))
  (call   main (: 10 Int64)) (output (: 16 Int64)))

(case "al4 ONE arm-local feeds BOTH slots — the score accumulates into the base while the count rides beside"
  (input  (do
            (effect E (op f (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E (tuple n 0)
                ((f (v) s (match s
                            ((tuple base count)
                             (let ((score (+ (* v v) base)))
                               (resume (+ score count) (tuple score (+ count 1))))))))
                (+ (E.f 2) (+ (E.f 3) (E.f 1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 49 Int64))
  (call   main (: 0 Int64)) (output (: 34 Int64)))

(case "al5 an arm-LOCAL named the same as a body-side binder — hygiene keeps the two w's separate across dispatches"
  (input  (do
            (effect E (op f (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((f (v) s (let ((w (* v 10)))
                            (resume (+ w s) (+ s 1)))))
                (let ((w 7000))
                  (+ (E.f 2) (+ w (E.f 3))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7061 Int64))
  (call   main (: 0 Int64)) (output (: 7051 Int64)))

;; ── FRESH regions in sequence: re-entry and value chaining (breaker fs) ──────────────────────────
;; fs1 the SAME handle expression re-entered per recursion round — a fresh region each iteration,
;; seeds keyed by depth (state bleed between iterations would corrupt the depth-keyed sums); fs2
;; two SEQUENTIAL handles where the second's seed is computed from the first's RESULT; fs3 THREE
;; value-chained regions with distinct arms (+1 / x2 / -5) — a chain-order error is visible in
;; the pipeline value.

(case "fs1 the SAME handle expression re-entered per recursion round — a FRESH region each iteration, seeds keyed by depth"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (round (: i Int64))
              (if (<= i 0)
                  0
                  (+ (handle St (* i 10)
                       ((next () s (resume s (+ s 1))))
                       (+ (St.next) (St.next)))
                     (round (- i 1)))))
            (def (main (: n Int64))
              (round n))
            (export main)))
  (call   main (: 3 Int64)) (output (: 123 Int64))
  (call   main (: 1 Int64)) (output (: 21 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))

(case "fs2 two SEQUENTIAL handles where the second's SEED is computed from the first's RESULT — regions chained by value"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (let ((r1 (handle St n
                          ((next () s (resume s (+ s 1))))
                          (+ (St.next) (St.next)))))
                (handle St (* r1 2)
                  ((next () s (resume s (- s 3))))
                  (+ (St.next) (* 10 (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 212 Int64))
  (call   main (: 0 Int64)) (output (: -8 Int64)))

(case "fs3 THREE value-chained regions — each seed is the previous region's result, arms +1 / x2 / -5"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (let ((r1 (handle St n
                          ((next () s (resume s (+ s 1))))
                          (+ (St.next) (St.next)))))
                (let ((r2 (handle St r1
                            ((next () s (resume s (* s 2))))
                            (+ (St.next) (St.next)))))
                  (handle St r2
                    ((next () s (resume s (- s 5))))
                    (+ (St.next) (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 61 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))

;; ── SYMBOLS through the effect thread (breaker sy) ───────────────────────────────────────────────
;; Interned symbols as state, keys, selectors, and commands: sy2 a symbol TOGGLE state (equality
;; routes the answer while the arm flips fast/slow); sy3 the symbol state as a MAP KEY per
;; dispatch (the a->b->c walk ends at a missing key); sy4 sequential draws written into record
;; FIELDS via chained Record.with symbol selectors; sy5 symbol op args as COMMANDS (inc/dbl/nop
;; route both the answer and the transition — the interpreter-dispatch idiom).

(case "sy2 a SYMBOL toggle state — equality routes the resume value while the arm flips fast/slow per dispatch"
  (input  (do
            (effect M (op mode (-> Int64)))
            (def (main (: n Int64))
              (handle M (if (> n 3) #"fast" #"slow")
                ((mode () s (resume (if (= s #"fast") 100 1) (if (= s #"fast") #"slow" #"fast"))))
                (+ (M.mode) (+ (M.mode) (M.mode)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 201 Int64))
  (call   main (: 0 Int64)) (output (: 102 Int64)))

(case "sy3 the SYMBOL state is a MAP KEY per dispatch — the a→b→c walk ends at a missing key"
  (input  (do
            (effect R (op route (-> Int64)))
            (def (main (: n Int64))
              (handle R #"a"
                ((route () s (resume (match (Map.lookup (map (#"a" 10) (#"b" 20)) s)
                                       ((Some v) v)
                                       ((None) -1))
                                     (if (= s #"a") #"b" #"c"))))
                (+ (R.route) (+ (* 10 (R.route)) (* 100 (R.route))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 110 Int64)))

(case "sy4 sequential draws written into record FIELDS via Record.with symbol selectors — chained functional updates"
  (input  (do
            (effect P (op pick (-> Int64)))
            (def (main (: n Int64))
              (handle P n
                ((pick () s (resume s (+ s 1))))
                (let ((r (record (= x 10) (= y 20))))
                  (let ((r2 (Record.with r #"x" (P.pick))))
                    (let ((r3 (Record.with r2 #"y" (P.pick))))
                      (+ (* 100 (. r3 x)) (. r3 y)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 506 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))

(case "sy5 SYMBOL op args as COMMANDS — inc/dbl/nop route both the answer and the state transition"
  (input  (do
            (effect C (op cmd (-> Symbol Int64)))
            (def (main (: n Int64))
              (handle C n
                ((cmd (w) s (resume (if (= w #"inc") (+ s 1) (if (= w #"dbl") (* s 2) 0))
                                    (if (= w #"inc") (+ s 1) (if (= w #"dbl") (* s 2) s)))))
                (+ (C.cmd #"inc") (+ (* 10 (C.cmd #"dbl")) (* 100 (C.cmd #"nop"))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 126 Int64))
  (call   main (: 0 Int64)) (output (: 21 Int64)))

;; ── contracts checking EFFECTFUL values (breaker uv) ─────────────────────────────────────────────
;; The pinned @requires/@ensures x effects cases guard a performing BODY; these point the contract
;; at effect-derived VALUES: uv2 a @requires-guarded fn FED BY DRAWS (the contract checks each
;; draw at call time, boundary row included); uv3 the handler ARM calls a @requires-guarded helper
;; (the contract checks the LIVE STATE per dispatch); uv4 a RELATIONAL @ensures (ret > x) over a
;; TWO-draw body (the postcondition compares the effectful result to the argument, twice).

(case "uv2 a @requires-guarded fn fed by DRAWS — the contract checks effectful argument values at each call"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (@ (requires (> x 3)) (def (f (: x Int64)) (* x 10)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (+ (f (St.next)) (f (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 110 Int64))
  (call   main (: 4 Int64)) (output (: 90 Int64)))

(case "uv3 the handler ARM calls a @requires-guarded helper — the contract checks the live state per dispatch"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (@ (requires (> x 0)) (def (safe-dbl (: x Int64)) (* x 2)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume (safe-dbl s) (+ s 1))))
                (+ (St.next) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 22 Int64))
  (call   main (: 1 Int64)) (output (: 6 Int64)))

(case "uv4 a RELATIONAL @ensures (ret > x) over a TWO-draw body — the postcondition compares the effectful result to the arg, twice"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (@ (ensures (> ret x)) (def (above (: x Int64)) (+ x (+ (St.next) (St.next)))))
            (def (main (: n Int64))
              (handle St 1
                ((next (u) s (resume s (+ s 1))))
                (+ (above n) (above 100))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 115 Int64))
  (call   main (: 0 Int64)) (output (: 110 Int64)))

;; ── SCALE stress: dispatch width, shadow depth, table width (breaker lo) ─────────────────────────
;; lo1 THIRTY dispatches in one region (an order of magnitude past the corpus's usual handful —
;; the arithmetic-series sum is exact); lo2 a NINE-level same-effect shadow tower (strides 1-8,
;; per-level seeds k*100, the deepest doubling); lo3 EIGHT ops in one effect called in SHUFFLED
;; order (dispatch-table width, exactly one op advancing mid-sequence so the pre/post split is
;; visible).

(case "lo1 THIRTY dispatches in one region — the fold scales past the corpus's usual handful, arithmetic sum exact"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (+ (St.next) (St.next))))))))))))))))))))))))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 585 Int64))
  (call   main (: 0 Int64)) (output (: 435 Int64)))

(case "lo2 a NINE-level shadow tower (outer + eight) — one draw per level, the deepest doubling, strides 1-8"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (+ (St.next) (handle St 100 ((next () s (resume s (+ s 1)))) (+ (St.next) (handle St 200 ((next () s (resume s (+ s 2)))) (+ (St.next) (handle St 300 ((next () s (resume s (+ s 3)))) (+ (St.next) (handle St 400 ((next () s (resume s (+ s 4)))) (+ (St.next) (handle St 500 ((next () s (resume s (+ s 5)))) (+ (St.next) (handle St 600 ((next () s (resume s (+ s 6)))) (+ (St.next) (handle St 700 ((next () s (resume s (+ s 7)))) (+ (St.next) (handle St 800 ((next () s (resume s (+ s 8)))) (+ (St.next) (St.next))))))))))))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 4413 Int64))
  (call   main (: 0 Int64)) (output (: 4408 Int64)))

(case "lo3 EIGHT ops in one effect, called in shuffled order — dispatch-table width, one op advancing mid-sequence"
  (input  (do
            (effect W (op a (-> Int64)) (op b (-> Int64)) (op c (-> Int64)) (op d (-> Int64))
                      (op e (-> Int64)) (op f (-> Int64)) (op g (-> Int64)) (op h (-> Int64)))
            (def (main (: n Int64))
              (handle W n
                ((a () s (resume (+ s 1) s))
                 (b () s (resume (+ s 2) s))
                 (c () s (resume (+ s 3) s))
                 (d () s (resume (+ s 4) s))
                 (e () s (resume (+ s 5) s))
                 (f () s (resume (+ s 6) s))
                 (g () s (resume (+ s 7) s))
                 (h () s (resume (+ s 8) (+ s 10))))
                (+ (W.a) (+ (W.h) (+ (W.b) (+ (W.g) (+ (W.c) (+ (W.f) (+ (W.d) (W.e))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 136 Int64))
  (call   main (: 0 Int64)) (output (: 96 Int64)))

;; ── Option COMPOSITIONS through the effect thread (breaker oc) ───────────────────────────────────
;; oc1 an Option-of-TUPLE accumulator state (None seeds, Some carries (total,count) advancing
;; both); oc2 an Option FLOWING between two ops of one effect (find produces, use consumes it as
;; an op ARG); oc3 a DOUBLE-wrapped Option resume value — None vs Some(None) vs Some(Some v) all
;; distinguished by nested body matches. (oc3's nested body matches x two dispatches FOLD — the
;; lf5/tt2 decline specifically needs the performing RECURSIVE callee, sharpening that boundary
;; once more.)

(case "oc1 an Option-of-TUPLE accumulator — None seeds on first step, Some carries (total,count) advancing both"
  (input  (do
            (effect O (op step (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle O (None)
                ((step (v) s (match s
                               ((None) (resume 0 (Some (tuple v 1))))
                               ((Some p) (match p
                                           ((tuple tot cnt) (resume (+ tot cnt)
                                                                    (Some (tuple (+ tot v) (+ cnt 1))))))))))
                (+ (O.step n) (+ (* 10 (O.step 3)) (* 100 (O.step 7))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1060 Int64))
  (call   main (: 0 Int64)) (output (: 510 Int64)))

(case "oc2 an Option FLOWS between two ops of one effect — find produces Some/None, use consumes it as an op ARG"
  (input  (do
            (effect O (op find (-> Int64 (Option Int64))) (op use (-> (Option Int64) Int64)))
            (def (main (: n Int64))
              (handle O n
                ((find (k) s (resume (if (> k s) (Some (- k s)) (None)) (+ s 1)))
                 (use (m) s (match m
                              ((Some v) (resume (* v 10) s))
                              ((None) (resume (- 0 s) s)))))
                (+ (O.use (O.find 10)) (O.use (O.find 0)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 43 Int64))
  (call   main (: 20 Int64)) (output (: -43 Int64)))

(case "oc3 a DOUBLE-wrapped Option resume value — None vs Some(None) vs Some(Some v) all distinguished by the body"
  (input  (do
            (effect O (op probe (-> Int64 (Option (Option Int64)))))
            (def (main (: n Int64))
              (handle O n
                ((probe (k) s (resume (if (< k 0)
                                          (None)
                                          (if (> k s) (Some (Some (- k s))) (Some (None))))
                                      (+ s 1))))
                (+ (match (O.probe 10)
                     ((Some inner) (match inner ((Some v) v) ((None) -1)))
                     ((None) -100))
                   (* 1000 (match (O.probe -5)
                             ((Some inner) (match inner ((Some v) v) ((None) -1)))
                             ((None) -100))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -99995 Int64))
  (call   main (: 15 Int64)) (output (: -100001 Int64)))

;; ── Result-STYLE sums (Ok/Err) through the effect thread (breaker rt) ────────────────────────────
;; rt1 Ok/Err resume values with a DEPENDENT second dispatch (the falling state's sign flips Ok to
;; Err; Ok-Ok / Ok-Err / Err rows); rt2 an Err SHORT-CIRCUITING a recursive Result walk (Ok
;; accumulates tail-recursively, the first Err multiplies out and stops — a single-level match on
;; a performing call inside a recursive callee folds; only the NESTED form declines); rt3 the Err
;; VALUE seeding a RECOVERY region (a fallback same-effect handle inside the Err arm).

(case "rt1 Ok/Err resume values with a DEPENDENT second dispatch — the sign of the falling state flips Ok to Err"
  (input  (do
            (type Res (Ok Int64) (Err Int64))
            (effect E (op run (-> Int64 Res)))
            (def (main (: n Int64))
              (handle E n
                ((run (k) s (resume (if (> s 0) (Ok (* k s)) (Err s)) (- s 2))))
                (match (E.run 3)
                  ((Ok a) (match (E.run 5)
                            ((Ok b) (+ a b))
                            ((Err e2) (+ a (* 1000 e2)))))
                  ((Err e) (* 100 e)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 30 Int64))
  (call   main (: 1 Int64)) (output (: -997 Int64))
  (call   main (: -3 Int64)) (output (: -300 Int64)))

(case "rt2 an Err SHORT-CIRCUITS a recursive Result walk — Ok accumulates, the first Err multiplies out and stops"
  (input  (do
            (type Res (Ok Int64) (Err Int64))
            (effect E (op try (-> Int64 Res)))
            (def (chain (: i Int64) (: acc Int64))
              (if (> i 3)
                  acc
                  (match (E.try i)
                    ((Ok v) (chain (+ i 1) (+ acc v)))
                    ((Err e) (* acc e)))))
            (def (main (: n Int64))
              (handle E n
                ((try (k) s (resume (if (> s 0) (Ok (* k 10)) (Err k)) (- s 1))))
                (chain 1 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 60 Int64))
  (call   main (: 2 Int64)) (output (: 90 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))

(case "rt3 the Err VALUE seeds a RECOVERY region — a fallback same-effect handle runs inside the Err arm"
  (input  (do
            (type Res (Ok Int64) (Err Int64))
            (effect E (op go (-> Res)))
            (def (main (: n Int64))
              (handle E n
                ((go () s (resume (if (> s 3) (Ok (* s 10)) (Err (+ s 100))) s)))
                (match (E.go)
                  ((Ok v) v)
                  ((Err e) (handle E e
                             ((go () s (resume (Ok (+ s 1)) s)))
                             (match (E.go)
                               ((Ok v2) (* v2 2))
                               ((Err _e2) -1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 50 Int64))
  (call   main (: 2 Int64)) (output (: 206 Int64)))

;; ── ae: STRICT ARGUMENT-EVALUATION ORDER under the effect thread ─────────────
;; Every face pins left-to-right, exactly-once argument evaluation with an
;; advancing state as the witness. ae1 uses SUBTRACTION antisymmetry (a swap
;; flips the sign, not just the value); ae2/ae3 use positional 100/10/1 digit
;; encodings across a pure call and an op's own argument list; ae4 routes the
;; middle argument through a performing def; ae5/ae6 pin the short-circuit
;; halves of `and`/`or` — the skipped draw must leave the state untouched,
;; proved by a trailing draw's x10 digit.

(case "ae1 SUBTRACTION of two same-op draws as a 2-ary op's args — the antisymmetry pins left-to-right order exactly"
  (input  (do
            (effect E (op next (-> Int64)) (op pair (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (* s 2)))
                 (pair (a b) s (resume (- a b) s)))
                (E.pair (E.next) (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -5 Int64))
  (call   main (: -3 Int64)) (output (: 3 Int64)))

(case "ae2 draw-PURE-draw argument positions to a pure 3-ary fn — the middle constant sits between two advancing draws"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (mix3 (: a Int64) (: b Int64) (: c Int64)) (+ (* 100 a) (+ (* 10 b) c)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (mix3 (E.next) 7 (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 576 Int64))
  (call   main (: 0 Int64)) (output (: 71 Int64)))

(case "ae3 THREE same-op draws as a 3-ary OP's own args — order pinned inside the op's argument list itself"
  (input  (do
            (effect E (op next (-> Int64)) (op mix (-> Int64 Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (mix (a b c) s (resume (+ (* 100 a) (+ (* 10 b) c)) s)))
                (E.mix (E.next) (E.next) (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 567 Int64))
  (call   main (: 0 Int64)) (output (: 12 Int64)))

(case "ae4 draw / performing-HELPER / draw argument positions — the middle arg performs through a def boundary"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (mix3 (: a Int64) (: b Int64) (: c Int64)) (+ (* 100 a) (+ (* 10 b) c)))
            (def (bump) (E.next))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (mix3 (E.next) (bump) (E.next))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 345 Int64))
  (call   main (: -1 Int64)) (output (: -99 Int64)))

(case "ae5 short-circuit AND with DRAWS on both sides — the skipped right draw leaves the state thread untouched"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((c (if (and (> (E.next) 0) (> (E.next) 0)) 100 200)))
                  (+ c (* 10 (E.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 170 Int64))
  (call   main (: -3 Int64)) (output (: 180 Int64)))

(case "ae6 short-circuit OR with DRAWS on both sides — the right draw fires only when the left is false"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((c (if (or (> (E.next) 0) (> (E.next) 0)) 100 200)))
                  (+ c (* 10 (E.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 160 Int64))
  (call   main (: -3 Int64)) (output (: 190 Int64)))

;; ── ae (cont.): argument order through STRUCTURE and COMPOUND expressions ────
;; ae7 nests a draw under a pure unary call (nesting must not reorder); ae8/ae10
;; carry draw order into LIST and TUPLE element positions and read them back;
;; ae9 pins record field-INIT order via the CDZ0201-advised projection form
;; (its match forms hit the two known bind-once declines — witnesses banked);
;; ae11/ae12 put whole compound expressions in argument position: a do-block's
;; DISCARDED interior draw still advances the state, and an if-condition draw
;; decides whether a second draw fires before the next argument.

(case "ae7 draw NESTED under a pure unary call in the first arg slot, second slot a bare draw — nesting must not reorder"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (dbl (: x Int64)) (* 2 x))
            (def (tens (: a Int64) (: b Int64)) (+ (* 10 a) b))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (tens (dbl (E.next)) (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 106 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))

(case "ae8 LIST literal of three draws — element positions carry the draw order into the structure"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((xs (list (E.next) (E.next) (E.next))))
                  (match (List.at xs 0)
                    ((Some a) (match (List.at xs 1)
                      ((Some b) (match (List.at xs 2)
                        ((Some c) (+ (* 100 a) (+ (* 10 b) c)))
                        ((None) 0)))
                      ((None) 0)))
                    ((None) 0)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 345 Int64))
  (call   main (: -2 Int64)) (output (: -210 Int64)))

(case "ae9 RECORD literal of two draws read back by PROJECTION — field-init order in the literal drives the state thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((r (record (= a (E.next)) (= b (E.next)))))
                  (+ (* 10 (. r a)) (. r b)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64))
  (call   main (: -4 Int64)) (output (: -43 Int64)))

(case "ae10 TUPLE literal of three draws matched immediately as scrutinee — positions survive the construct-then-destructure round trip"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (match (tuple (E.next) (E.next) (E.next))
                  ((tuple a b c) (+ (* 100 a) (+ (* 10 b) c))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 234 Int64))
  (call   main (: 0 Int64)) (output (: 12 Int64)))

(case "ae11 a DO-block as an argument — its DISCARDED interior draw still advances the state before the block's value draw"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (tens (: a Int64) (: b Int64)) (+ (* 10 a) b))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (tens (E.next) (do (E.next) (E.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 57 Int64))
  (call   main (: 0 Int64)) (output (: 2 Int64)))

(case "ae12 an IF-expression as an argument whose CONDITION draws — the taken branch decides whether a second draw fires before the next arg"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (tens (: a Int64) (: b Int64)) (+ (* 10 a) b))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (tens (if (> (E.next) 0) (E.next) 100) (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 67 Int64))
  (call   main (: -2 Int64)) (output (: 999 Int64)))

;; ── ha: CROSS-HANDLER argument evaluation order ──────────────────────────────
;; Draws in one argument list that dispatch to DIFFERENT live handlers. ha1
;; sends two outer draws through an inner op's dispatch boundary; ha2 chains
;; outer -> inner -> outer through two frames; ha3 interleaves outer-inner-outer
;; in a single pure call's argument list; ha4 makes an inner op's own arguments
;; dispatch to the SAME inner handler (state doubles between them) before an
;; outer draw stamps the hundreds digit.

(case "ha1 an INNER op's arguments draw from the OUTER handler — two outer draws cross the inner dispatch boundary in order"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect I (op tens (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle I 0
                  ((tens (a b) s (resume (+ (* 10 a) b) s)))
                  (I.tens (O.next) (O.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))

(case "ha2 outer-draw feeds the INNER op, whose result feeds an OUTER op again — a three-hop cross-handler value chain"
  (input  (do
            (effect O (op next (-> Int64)) (op send (-> Int64 Int64)))
            (effect I (op dbl (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1)))
                 (send (v) s (resume (+ v s) s)))
                (handle I 0
                  ((dbl (x) s (resume (* 2 x) s)))
                  (O.send (I.dbl (O.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 16 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -3 Int64)) (output (: -8 Int64)))

(case "ha3 INTERLEAVED outer-inner-outer draws in ONE argument list — each draw dispatches to its own handler in sequence"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect I (op pick (-> Int64)))
            (def (mix3 (: a Int64) (: b Int64) (: c Int64)) (+ (* 100 a) (+ (* 10 b) c)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle I 7
                  ((pick () s (resume s s)))
                  (mix3 (O.next) (I.pick) (O.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 576 Int64))
  (call   main (: 0 Int64)) (output (: 71 Int64)))

(case "ha4 an inner op's arguments dispatch to the SAME inner handler — same-effect draws inside the op's own arg list, then an outer draw"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect I (op get (-> Int64)) (op tens (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (+ (handle I 3
                     ((get () m (resume m (* 2 m)))
                      (tens (a b) m (resume (+ (* 10 a) b) m)))
                     (I.tens (I.get) (I.get)))
                   (* 100 (O.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 536 Int64))
  (call   main (: 0 Int64)) (output (: 36 Int64)))

;; ── rv: handler-arm RESUME VALUES that themselves perform ────────────────────
;; The arm's resume-value expression dispatches to the ENCLOSING handler at
;; dispatch time. rv1 resumes with a single outer draw (two asks advance the
;; outer thread once each); rv2 resumes with a SUBTRACTION of two outer draws
;; (antisymmetry pins their order inside the arm) against a doubling outer
;; state. The mirror face — a performing next-STATE expression — is a known
;; tail-resumptive-fold decline (non-tail resume); witnesses banked, not cases.

(case "rv1 the inner handler ARM resumes with an OUTER draw — the resume VALUE expression performs against the enclosing handler"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect I (op ask (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle I 0
                  ((ask () t (resume (O.next) t)))
                  (+ (* 10 (I.ask)) (I.ask)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))

(case "rv2 the inner arm's resume value SUBTRACTS two outer draws — cross-handler order inside the arm, with a doubling outer state"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect I (op ask (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (* 2 s))))
                (handle I 0
                  ((ask () t (resume (- (O.next) (O.next)) t)))
                  (+ (I.ask) (* 10 (O.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 195 Int64))
  (call   main (: 1 Int64)) (output (: 39 Int64))
  (call   main (: -3 Int64)) (output (: -117 Int64)))

;; ── tl: THREE-LEVEL handler chains ───────────────────────────────────────────
;; Three live frames at once. tl1 draws from all three in one expression (each
;; draw dispatches past the inner frames to its own handler); tl2 pipelines a
;; value innermost -> middle -> outermost; tl3 puts the rv face at depth (the
;; MIDDLE frame's arm resumes with an OUTERMOST draw, dispatched from under a
;; third live frame); tl4 handles the SAME effect at two depths — the inner
;; handle shadows for its extent and the outer thread is untouched after it
;; closes.

(case "tl1 THREE live handlers, each drawn in one expression — every draw dispatches past the two inner frames to its own"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect M (op step (-> Int64)))
            (effect I (op pick (-> Int64)))
            (def (mix3 (: a Int64) (: b Int64) (: c Int64)) (+ (* 100 a) (+ (* 10 b) c)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle M 4
                  ((step () m (resume m (+ m 2))))
                  (handle I 7
                    ((pick () t (resume t t)))
                    (+ (* 1000 (O.next)) (mix3 (M.step) (I.pick) (O.next)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5476 Int64))
  (call   main (: 0 Int64)) (output (: 471 Int64))
  (call   main (: -1 Int64)) (output (: -530 Int64)))

(case "tl2 a THREE-frame value pipeline — innermost pick feeds middle dbl feeds outermost send, then an outer draw stamps the hundreds"
  (input  (do
            (effect O (op next (-> Int64)) (op send (-> Int64 Int64)))
            (effect M (op dbl (-> Int64 Int64)))
            (effect I (op pick (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1)))
                 (send (v) s (resume (+ v s) s)))
                (handle M 0
                  ((dbl (x) m (resume (* 2 x) m)))
                  (handle I 7
                    ((pick () t (resume t t)))
                    (+ (O.send (M.dbl (I.pick))) (* 100 (O.next)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 519 Int64))
  (call   main (: 0 Int64)) (output (: 14 Int64))
  (call   main (: -20 Int64)) (output (: -2006 Int64)))

(case "tl3 the MIDDLE frame's arm resumes with an OUTERMOST draw — rv-face at depth, dispatched from under a third live frame"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect M (op grab (-> Int64)))
            (effect I (op pick (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle M 0
                  ((grab () m (resume (O.next) m)))
                  (handle I 7
                    ((pick () t (resume t t)))
                    (+ (* 100 (M.grab)) (+ (* 10 (M.grab)) (I.pick)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 567 Int64))
  (call   main (: 0 Int64)) (output (: 17 Int64))
  (call   main (: -2 Int64)) (output (: -203 Int64)))

(case "tl4 SAME effect handled at two depths — the inner handle shadows for its extent, the outer thread resumes after it closes"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (handle E 50
                     ((next () s (resume s (+ s 5))))
                     (E.next))
                   (* 10 (E.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 100 Int64))
  (call   main (: 0 Int64)) (output (: 50 Int64))
  (call   main (: -4 Int64)) (output (: 10 Int64)))

;; ── av: ABORT values that perform ────────────────────────────────────────────
;; The arm aborts (no resume) but its escaping value dispatches to an enclosing
;; handler on the way out. av1 aborts with a single outer draw (the aborted
;; body's +999 is dead); av2 aborts with a SUBTRACTION of two outer draws under
;; a doubling outer state (antisymmetry pins order inside the aborting arm);
;; av3 stacks three frames and aborts with a MIDDLE draw — the middle thread
;; advances and later draws observe it. The re-entrant face — the abort value
;; dispatching to the SAME handler's own sibling op — is CDZ0401 by design (a
;; handler's arms sit outside its own extent); witness banked, not a case.

(case "av1 the inner arm ABORTS with an OUTER draw as the abort value — the escaping value performs on the way out"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect Bail (op out (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (+ (* 100 (handle Bail 0
                            ((out () t (O.next)))
                            (+ (Bail.out) 999)))
                   (* 10 (O.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 560 Int64))
  (call   main (: 0 Int64)) (output (: 10 Int64))
  (call   main (: -2 Int64)) (output (: -210 Int64)))

(case "av2 the abort value SUBTRACTS two outer draws under a DOUBLING outer state — antisymmetry pins order inside the aborting arm"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect Bail (op out (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (* 2 s))))
                (+ (* 100 (handle Bail 0
                            ((out () t (- (O.next) (O.next))))
                            (+ (Bail.out) 999)))
                   (O.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -480 Int64))
  (call   main (: 1 Int64)) (output (: -96 Int64))
  (call   main (: -3 Int64)) (output (: 288 Int64)))

(case "av3 in a THREE-stack the innermost arm aborts with a MIDDLE draw — the escaping value advances the middle thread, later draws see it"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect M (op step (-> Int64)))
            (effect Bail (op out (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle M 4
                  ((step () m (resume m (+ m 2))))
                  (+ (* 100 (handle Bail 0
                              ((out () t (M.step)))
                              (+ (Bail.out) 999)))
                     (+ (* 10 (M.step)) (O.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 465 Int64))
  (call   main (: 0 Int64)) (output (: 460 Int64))
  (call   main (: -7 Int64)) (output (: 453 Int64)))

;; ── hi: a HANDLE installed INSIDE a handler arm ──────────────────────────────
;; A whole handler lifecycle (install -> dispatch -> close) nested within one
;; dispatch of an enclosing handler. hi1 resumes with the fresh handle's result;
;; hi2's installed body ALSO draws from the arm's enclosing frame; hi3 nests the
;; rv face (the installed handle's own arm resumes with an outer draw); hi4
;; nests the av face (the installed handle aborts with an outer draw, its
;; body's +999 dead).

(case "hi1 an arm INSTALLS a fresh handle and resumes with its result — a whole handler lifecycle inside one dispatch"
  (input  (do
            (effect O (op boost (-> Int64)) (op next (-> Int64)))
            (effect J (op get (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((boost () s (resume (+ (handle J 3 ((get () t (resume t t))) (J.get)) s) s))
                 (next () s (resume s (+ s 1))))
                (+ (* 10 (O.boost)) (O.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 85 Int64))
  (call   main (: 0 Int64)) (output (: 30 Int64))
  (call   main (: -2 Int64)) (output (: 8 Int64)))

(case "hi2 the arm-installed handle's body draws from the arm's ENCLOSING frame — fresh inner frame and outer dispatch in one arm"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect M (op grab (-> Int64)))
            (effect J (op get (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle M 0
                  ((grab () m (resume (handle J 9
                                         ((get () t (resume t t)))
                                         (+ (J.get) (O.next)))
                                       m)))
                  (+ (* 10 (M.grab)) (O.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 146 Int64))
  (call   main (: 0 Int64)) (output (: 91 Int64))
  (call   main (: -3 Int64)) (output (: 58 Int64)))

(case "hi3 the arm-installed handle's OWN arm resumes with an OUTER draw — the rv face nested inside another handler's dispatch"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect M (op grab (-> Int64)))
            (effect J (op ask (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle M 0
                  ((grab () m (resume (handle J 0
                                        ((ask () t (resume (O.next) t)))
                                        (+ (J.ask) (* 10 (J.ask))))
                                      m)))
                  (+ (M.grab) (* 100 (O.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 765 Int64))
  (call   main (: 0 Int64)) (output (: 210 Int64))
  (call   main (: -1 Int64)) (output (: 99 Int64)))

(case "hi4 the arm-installed handle ABORTS with an outer draw — the av face nested inside another handler's dispatch"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect M (op grab (-> Int64)))
            (effect Bail (op out (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle M 0
                  ((grab () m (resume (handle Bail 0
                                        ((out () t (O.next)))
                                        (+ (Bail.out) 999))
                                      m)))
                  (+ (* 10 (M.grab)) (O.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -4 Int64)) (output (: -43 Int64)))

;; ── ms: parity-built SUM scrutinees ──────────────────────────────────────────
;; An op returns a THREE-variant Mode sum constructed in the arm from state
;; parity (% s 3) — which variant comes back depends on where the thread is.
;; ms1 scores two sequential performing scrutinees (the second sees the
;; advanced state and can change variant); ms2's match arms THEMSELVES draw
;; after the scrutinee advanced the thread; ms3 walks a recursion drawing a
;; Mode per level (single-level match in a performing recursive callee — the
;; folding side of the rt2 boundary); ms4 nests a pure payload-parity match
;; inside the C arm. The recursion-x-abort face (a walk that Bails mid-descent)
;; is a cross-function fold decline; witness banked, not a case.

(case "ms1 an op returns a THREE-variant sum built from state parity — two sequential performing scrutinees, the second sees the advanced state"
  (input  (do
            (type Mode (A) (B Int64) (C Int64 Int64))
            (effect E (op mode (-> Mode)))
            (def (score (: m Mode))
              (match m
                ((A) 7)
                ((B x) (* 10 x))
                ((C x y) (+ (* 100 x) y))))
            (def (main (: n Int64))
              (handle E n
                ((mode () s (resume (match (% s 3)
                                      (0 (A))
                                      (1 (B s))
                                      (_ (C s s)))
                                    (+ s 1))))
                (+ (* 1000 (score (E.mode))) (score (E.mode)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 7010 Int64))
  (call   main (: 1 Int64)) (output (: 10202 Int64))
  (call   main (: 2 Int64)) (output (: 202007 Int64))
  (call   main (: 4 Int64)) (output (: 40505 Int64)))

(case "ms2 match ARMS themselves draw after the performing scrutinee advanced the state — arm-selected continuation of the same thread"
  (input  (do
            (type Mode (A) (B Int64) (C Int64 Int64))
            (effect E (op mode (-> Mode)) (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((mode () s (resume (match (% s 3)
                                      (0 (A))
                                      (1 (B s))
                                      (_ (C s s)))
                                    (+ s 1)))
                 (next () s (resume s (+ s 1))))
                (+ (* 100 (match (E.mode)
                            ((A) (E.next))
                            ((B x) (+ x (E.next)))
                            ((C x y) (+ x y))))
                   (E.next))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 102 Int64))
  (call   main (: 1 Int64)) (output (: 303 Int64))
  (call   main (: 2 Int64)) (output (: 403 Int64))
  (call   main (: 6 Int64)) (output (: 708 Int64)))

(case "ms3 parity-sum dispatch inside a RECURSIVE walk — each level draws a Mode and accumulates by variant as the thread advances"
  (input  (do
            (type Mode (A) (B Int64) (C Int64 Int64))
            (effect E (op mode (-> Mode)))
            (def (walk (: k Int64))
              (if (<= k 0)
                  0
                  (+ (match (E.mode)
                       ((A) 7)
                       ((B x) x)
                       ((C x y) (* x y)))
                     (walk (- k 1)))))
            (def (main (: n Int64))
              (handle E n
                ((mode () s (resume (match (% s 3)
                                      (0 (A))
                                      (1 (B s))
                                      (_ (C s s)))
                                    (+ s 1))))
                (walk 4)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 19 Int64))
  (call   main (: 1 Int64)) (output (: 16 Int64)))

(case "ms4 the C arm RE-MATCHES its own payload's parity — a nested pure match inside an arm of the performing-scrutinee match"
  (input  (do
            (type Mode (A) (B Int64) (C Int64 Int64))
            (effect E (op mode (-> Mode)) (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((mode () s (resume (match (% s 3)
                                      (0 (A))
                                      (1 (B s))
                                      (_ (C s s)))
                                    (+ s 1)))
                 (next () s (resume s (+ s 1))))
                (+ (* 10 (match (E.mode)
                           ((A) 7)
                           ((B x) x)
                           ((C x y) (match (% x 2)
                                      (0 (+ 1000 y))
                                      (_ (+ 2000 y))))))
                   (E.next))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 10023 Int64))
  (call   main (: 5 Int64)) (output (: 20056 Int64))
  (call   main (: 0 Int64)) (output (: 71 Int64))
  (call   main (: 1 Int64)) (output (: 12 Int64)))

;; ── fa: FOLD-style accumulators through a performing recursion ───────────────
;; The draw feeds an accumulator PARAMETER rather than the return path. fa1's
;; accumulator doubles then absorbs each level's draw (non-commutative: order
;; and count pinned in one value); fa2 threads TWO accumulators (running sum
;; and prefix-sum-of-prefix-sums) that must stay in step; fa3 draws TWICE per
;; level and mixes 10*first + second, pinning the intra-level order.

(case "fa1 a FOLD-style accumulator threads through a performing recursion — acc doubles then absorbs each level's draw"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (fold (: k Int64) (: acc Int64))
              (if (<= k 0)
                  acc
                  (fold (- k 1) (+ (* 2 acc) (E.next)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (* 10 (fold 3 0)) (E.next))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 114 Int64))
  (call   main (: 0 Int64)) (output (: 43 Int64))
  (call   main (: -2 Int64)) (output (: -99 Int64)))

(case "fa2 TWO accumulators through one performing recursion — running sum and prefix-sum-of-prefix-sums stay in step with the draws"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (fold2 (: k Int64) (: a Int64) (: b Int64))
              (if (<= k 0)
                  (+ (* 100 a) b)
                  (let ((d (E.next)))
                    (fold2 (- k 1) (+ a d) (+ b (+ a d))))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (fold2 3 0 0)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 610 Int64))
  (call   main (: 0 Int64)) (output (: 304 Int64))
  (call   main (: -1 Int64)) (output (: -2 Int64)))

(case "fa3 each fold level draws TWICE and mixes them asymmetrically — 10*first + second pins the intra-level draw order"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (fold (: k Int64) (: acc Int64))
              (if (<= k 0)
                  acc
                  (fold (- k 1) (+ acc (+ (* 10 (E.next)) (E.next))))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (fold 3 0)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 102 Int64))
  (call   main (: 0 Int64)) (output (: 69 Int64))
  (call   main (: -3 Int64)) (output (: -30 Int64)))

;; ── fa (cont.): sum-typed and two-effect accumulators ────────────────────────
;; fa4's accumulator IS a Mode sum cycling A->B->C->A, capturing a draw as
;; payload in the A and B arms (the C->A arm is draw-free — four levels end at
;; B holding the third draw); fa5 folds across TWO effects, each level
;; multiplying one draw from each independently-advancing thread.

(case "fa4 the accumulator IS a sum — each level's draw moves it around an A->B->C->A cycle capturing payloads on the way"
  (input  (do
            (type Mode (A) (B Int64) (C Int64 Int64))
            (effect E (op next (-> Int64)))
            (def (spin (: k Int64) (: acc Mode))
              (if (<= k 0)
                  acc
                  (spin (- k 1)
                        (match acc
                          ((A) (B (E.next)))
                          ((B x) (C x (E.next)))
                          ((C x y) (A))))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (* 10 (match (spin 4 (A))
                           ((A) 7)
                           ((B x) (* 10 x))
                           ((C x y) (+ (* 100 x) y))))
                   (E.next))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 304 Int64))
  (call   main (: 0 Int64)) (output (: 203 Int64))
  (call   main (: -2 Int64)) (output (: 1 Int64)))

(case "fa5 a TWO-effect fold — each level's step multiplies a draw from each thread, both threads advancing independently"
  (input  (do
            (effect P (op next (-> Int64)))
            (effect Q (op next (-> Int64)))
            (def (fold (: k Int64) (: acc Int64))
              (if (<= k 0)
                  acc
                  (fold (- k 1) (+ acc (* (P.next) (Q.next))))))
            (def (main (: n Int64))
              (handle P n
                ((next () s (resume s (+ s 1))))
                (handle Q 100
                  ((next () t (resume t (+ t 10))))
                  (fold 3 0))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 1010 Int64))
  (call   main (: 0 Int64)) (output (: 350 Int64))
  (call   main (: -1 Int64)) (output (: 20 Int64)))

;; ── ch: op-result CHAINS feeding op arguments ────────────────────────────────
;; Results flow hop-to-hop through performing argument positions. ch1 chains
;; FOUR ops of one effect in a single nested expression, each hop bumping the
;; shared state by a different increment (+2/+3/+5/+1), a probe pinning the
;; final state; ch2 flattens the SAME chain through one let per hop — the two
;; forms must agree exactly (nested-argument desugaring == explicit
;; sequencing); ch3 sends the hops across TWO effects (F.b of G.p of F.a),
;; each thread advancing only on its own hops.

(case "ch1 a FOUR-op result chain in one nested expression — each op transforms the last result while bumping the shared state differently"
  (input  (do
            (effect E (op a (-> Int64)) (op b (-> Int64 Int64)) (op c (-> Int64 Int64)) (op d (-> Int64 Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((a () s (resume s (+ s 2)))
                 (b (x) s (resume (+ x s) (+ s 3)))
                 (c (x) s (resume (* 2 x) (+ s 5)))
                 (d (x) s (resume (+ x s) (+ s 1)))
                 (probe () s (resume s s)))
                (+ (* 100 (E.d (E.c (E.b (E.a))))) (E.probe))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 2413 Int64))
  (call   main (: 0 Int64)) (output (: 1411 Int64))
  (call   main (: -4 Int64)) (output (: -593 Int64)))

(case "ch2 the same FOUR-op chain flattened through LETs — one binding per hop must equal the nested form exactly"
  (input  (do
            (effect E (op a (-> Int64)) (op b (-> Int64 Int64)) (op c (-> Int64 Int64)) (op d (-> Int64 Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((a () s (resume s (+ s 2)))
                 (b (x) s (resume (+ x s) (+ s 3)))
                 (c (x) s (resume (* 2 x) (+ s 5)))
                 (d (x) s (resume (+ x s) (+ s 1)))
                 (probe () s (resume s s)))
                (let ((va (E.a)))
                  (let ((vb (E.b va)))
                    (let ((vc (E.c vb)))
                      (let ((vd (E.d vc)))
                        (+ (* 100 vd) (E.probe))))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 2413 Int64))
  (call   main (: 0 Int64)) (output (: 1411 Int64))
  (call   main (: -4 Int64)) (output (: -593 Int64)))

(case "ch3 the chain hops CROSS two effects — F.b of G.p of F.a, each thread advancing only on its own hops"
  (input  (do
            (effect F (op a (-> Int64)) (op b (-> Int64 Int64)) (op fp (-> Int64)))
            (effect G (op p (-> Int64 Int64)) (op gp (-> Int64)))
            (def (main (: n Int64))
              (handle F n
                ((a () s (resume s (+ s 2)))
                 (b (x) s (resume (+ x s) (+ s 3)))
                 (fp () s (resume s s)))
                (handle G 100
                  ((p (x) t (resume (+ x t) (+ t 10)))
                   (gp () t (resume t t)))
                  (+ (* 10 (F.b (G.p (F.a)))) (+ (F.fp) (G.gp))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 1177 Int64))
  (call   main (: 0 Int64)) (output (: 1135 Int64))
  (call   main (: -6 Int64)) (output (: 1009 Int64)))

;; ── cv: op-result chains INSIDE handler arms ─────────────────────────────────
;; The ch face composed with the rv face: a chain of enclosing-handler ops
;; evaluated within an inner arm's resume-value expression. cv1 runs O.b of
;; O.a inside the arm (probe pins the double advance); cv2 dispatches TWO asks
;; — the second chain starts where the first left the outer thread; cv3 routes
;; the chain through a PURE fn mid-hop (the pure call must not detach the
;; thread).

(case "cv1 an inner arm resumes with a CHAIN of two outer ops — O.b of O.a evaluated inside the arm, probe pins the double advance"
  (input  (do
            (effect O (op a (-> Int64)) (op b (-> Int64 Int64)) (op probe (-> Int64)))
            (effect I (op ask (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((a () s (resume s (+ s 2)))
                 (b (x) s (resume (+ x s) (+ s 3)))
                 (probe () s (resume s s)))
                (handle I 0
                  ((ask () t (resume (O.b (O.a)) t)))
                  (+ (* 10 (I.ask)) (O.probe)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 67 Int64))
  (call   main (: 0 Int64)) (output (: 25 Int64))
  (call   main (: -5 Int64)) (output (: -80 Int64)))

(case "cv2 TWO asks each running the in-arm chain — the second chain starts where the first left the outer thread"
  (input  (do
            (effect O (op a (-> Int64)) (op b (-> Int64 Int64)))
            (effect I (op ask (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((a () s (resume s (+ s 2)))
                 (b (x) s (resume (+ x s) (+ s 3))))
                (handle I 0
                  ((ask () t (resume (O.b (O.a)) t)))
                  (+ (* 100 (I.ask)) (I.ask)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 616 Int64))
  (call   main (: 0 Int64)) (output (: 212 Int64))
  (call   main (: -3 Int64)) (output (: -394 Int64)))

(case "cv3 the in-arm chain routes through a PURE fn mid-hop — O.b of dbl of O.a, the pure call must not detach the thread"
  (input  (do
            (effect O (op a (-> Int64)) (op b (-> Int64 Int64)) (op probe (-> Int64)))
            (effect I (op ask (-> Int64)))
            (def (dbl (: x Int64)) (* 2 x))
            (def (main (: n Int64))
              (handle O n
                ((a () s (resume s (+ s 2)))
                 (b (x) s (resume (+ x s) (+ s 3)))
                 (probe () s (resume s s)))
                (handle I 0
                  ((ask () t (resume (O.b (dbl (O.a))) t)))
                  (+ (* 10 (I.ask)) (O.probe)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 87 Int64))
  (call   main (: 0 Int64)) (output (: 25 Int64))
  (call   main (: -4 Int64)) (output (: -99 Int64)))

;; ── bc: BOOLs and comparisons across the dispatch boundary ───────────────────
;; bc1 sends a comparison-of-a-draw INTO a Bool-taking op whose arm negates
;; the state (history made visible); bc2 checks monotonicity of three draws
;; under a parity-dependent step (+2 even / -3 odd) with and-chained
;; comparisons; bc3 receives Bool FROM an op (state parity) into a two-flag if
;; ladder — a mod-3-dependent step decorrelates consecutive parities so all
;; four paths are reachable, one per pinned input.

(case "bc1 a comparison of a draw feeds a BOOL-taking op whose arm NEGATES the state — the bool crosses the dispatch boundary"
  (input  (do
            (effect E (op next (-> Int64)) (op judge (-> Bool Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (judge (b) s (resume (if b 100 200) (- 0 s)))
                 (probe () s (resume s s)))
                (+ (E.judge (< (E.next) 3)) (E.probe))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 97 Int64))
  (call   main (: 5 Int64)) (output (: 194 Int64))
  (call   main (: -1 Int64)) (output (: 100 Int64)))

(case "bc2 monotonicity of THREE draws under a parity-dependent step — the and of two comparisons decides, the final state rides along"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (if (= (% s 2) 0) (+ s 2) (- s 3))))
                 (probe () s (resume s s)))
                (let ((a (E.next)))
                  (let ((b (E.next)))
                    (let ((c (E.next)))
                      (+ (if (and (< a b) (< b c)) 1000 2000) (E.probe)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1006 Int64))
  (call   main (: 1 Int64)) (output (: 2002 Int64))
  (call   main (: 4 Int64)) (output (: 1010 Int64))
  (call   main (: -5 Int64)) (output (: 1996 Int64)))

(case "bc3 an op RETURNS Bool consumed by a two-flag if ladder — a mod-3-dependent step makes all four paths reachable"
  (input  (do
            (effect E (op flag (-> Bool)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((flag () s (resume (= (% s 2) 0) (if (= (% s 3) 0) (+ s 1) (+ s 2))))
                 (probe () s (resume s s)))
                (let ((f1 (E.flag)))
                  (let ((f2 (E.flag)))
                    (+ (* 10 (if f1 (if f2 10 20) (if f2 30 40))) (E.probe))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 106 Int64))
  (call   main (: 0 Int64)) (output (: 203 Int64))
  (call   main (: 3 Int64)) (output (: 306 Int64))
  (call   main (: 1 Int64)) (output (: 404 Int64)))

;; ── wx: EXTREME Int64 values through the thread ──────────────────────────────
;; wx1 wraps the STATE at Int64.max via wrapping-add — three draws straddle the
;; seam (MAX-1, MAX, MIN) and comparisons observe the discontinuity; wx2 sends
;; the MIN/MAX literals through dispatch as op ARGUMENTS (echoed back exact,
;; count arm tallies trips); wx3 rides them as SUM payloads (variant chosen by
;; state parity, both payload slots verified exact).

(case "wx1 the state thread WRAPS at Int64.max — three draws straddle the wraparound and the comparisons see the seam"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: u Int64))
              (handle E 9223372036854775806
                ((next () s (resume s (Int64.wrapping-add s 1))))
                (let ((d1 (E.next)))
                  (let ((d2 (E.next)))
                    (let ((d3 (E.next)))
                      (+ (if (> d2 d1) 100 200)
                         (+ (if (< d3 d2) 10 20)
                            (if (< d3 0) 1 2))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 111 Int64)))

(case "wx2 Int64.min and Int64.max cross the dispatch as OP ARGUMENTS and come back exact — a count arm tallies the trips"
  (input  (do
            (effect E (op keep (-> Int64 Int64)) (op count (-> Int64)))
            (def (main (: u Int64))
              (handle E 0
                ((keep (x) s (resume x (+ s 1)))
                 (count () s (resume s s)))
                (+ (if (= (E.keep -9223372036854775808) -9223372036854775808) 100 900)
                   (+ (if (= (E.keep 9223372036854775807) 9223372036854775807) 10 90)
                      (E.count)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 112 Int64)))

(case "wx3 Int64.min and Int64.max ride as SUM payloads through dispatch — variant chosen by state parity, payloads verified exact"
  (input  (do
            (type Ext (B Int64) (C Int64 Int64))
            (effect E (op wrap (-> Ext)) (op count (-> Int64)))
            (def (main (: u Int64))
              (handle E 0
                ((wrap () s (resume (if (= (% s 2) 0)
                                        (B -9223372036854775808)
                                        (C 9223372036854775807 -9223372036854775808))
                                    (+ s 1)))
                 (count () s (resume s s)))
                (+ (* 100 (match (E.wrap)
                            ((B x) (if (= x -9223372036854775808) 1 9))
                            ((C x y) 7)))
                   (+ (* 10 (match (E.wrap)
                              ((B x) 8)
                              ((C x y) (if (and (= x 9223372036854775807) (= y -9223372036854775808)) 2 6))))
                      (E.count)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 122 Int64)))

;; ── wx (cont.): wrapping ARITHMETIC at the dispatch boundary ─────────────────
;; wx4 doubles the op argument with wrapping-mul inside the ARM (MAX -> -2,
;; MIN -> 0, small exact); wx5 steps the STATE by wrapping-sub of MAX per draw
;; (draws 0, MIN+1, 2 — the seam crossed in both directions); wx6 chains
;; wrapping increments hop-to-hop (MAX-1 crosses the seam inside a nested
;; three-op chain).

(case "wx4 the ARM doubles its argument with wrapping-mul — MAX wraps to -2, MIN to 0, a small value stays exact, count rides along"
  (input  (do
            (effect E (op dbl (-> Int64 Int64)) (op count (-> Int64)))
            (def (main (: u Int64))
              (handle E 0
                ((dbl (x) s (resume (Int64.wrapping-mul x 2) (+ s 1)))
                 (count () s (resume s s)))
                (+ (E.dbl 9223372036854775807)
                   (+ (E.dbl -9223372036854775808)
                      (+ (E.dbl 3) (* 10 (E.count)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 34 Int64)))

(case "wx5 the state STEPS by wrapping-sub of MAX each draw — two hops cross the seam in opposite directions"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: u Int64))
              (handle E 0
                ((next () s (resume s (Int64.wrapping-sub s 9223372036854775807))))
                (let ((d1 (E.next)))
                  (let ((d2 (E.next)))
                    (let ((d3 (E.next)))
                      (+ (if (< d2 0) 1 5)
                         (+ (if (> d3 0) 10 50)
                            (if (= d3 2) 100 900))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 111 Int64)))

(case "wx6 wrapping increments CHAINED hop-to-hop — MAX-1 crosses the seam inside a nested three-op chain, count pins the trips"
  (input  (do
            (effect E (op step (-> Int64 Int64)) (op count (-> Int64)))
            (def (main (: u Int64))
              (handle E 0
                ((step (x) s (resume (Int64.wrapping-add x 1) (+ s 1)))
                 (count () s (resume s s)))
                (let ((v (E.step (E.step (E.step 9223372036854775806)))))
                  (+ (if (= v -9223372036854775807) 100 900)
                     (+ (if (< v 0) 10 90) (E.count))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 113 Int64)))

;; ── dv: DIVISION and MODULO edges via the thread ─────────────────────────────
;; dv1 exercises truncated / (toward zero) and dividend-sign % over negative
;; draws; dv2 takes the DIVISOR from the arm with a state-dependent SIGN
;; (alternating +-3, including the sign-crossing quotient); dv3 divides a
;; RUNTIME draw by -1 across the full non-MIN range (negation exact at
;; MIN+1), the MIN draw guarded to its own branch. Note: MIN / -1 with BOTH
;; operands as tail-resumptive op results still const-folds to the CDZ0304
;; compile reject — the folder sees through the arms; witness banked.

(case "dv1 truncated division and dividend-sign modulo over DRAWS — negative dividends exercise the toward-zero rule through dispatch"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 3))))
                (let ((a (E.next)))
                  (let ((b (E.next)))
                    (+ (* 1000 (/ a 4))
                       (+ (* 100 (% a 4))
                          (+ (* 10 (/ b 4)) (% b 4))))))))
            (export main)))
  (call   main (: -7 Int64)) (output (: -1310 Int64))
  (call   main (: 5 Int64)) (output (: 1120 Int64))
  (call   main (: -9 Int64)) (output (: -2112 Int64)))

(case "dv2 the DIVISOR comes from the arm with a state-dependent sign — quotient and remainder track the alternating divisor exactly"
  (input  (do
            (effect E (op next (-> Int64)) (op getdiv (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (getdiv () s (resume (if (= (% s 2) 0) 3 -3) (+ s 1)))
                 (probe () s (resume s s)))
                (let ((a (E.next)))
                  (let ((d (E.getdiv)))
                    (+ (* 100 (/ a d))
                       (+ (* 10 (% a d)) (- (E.probe) n)))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 212 Int64))
  (call   main (: -8 Int64)) (output (: 182 Int64))
  (call   main (: 4 Int64)) (output (: -88 Int64)))

(case "dv3 division by -1 of a RUNTIME draw — negation across the full non-MIN range, the MIN draw guarded to its own branch"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s s)))
                (let ((x (E.next)))
                  (if (= x -9223372036854775808)
                      777
                      (/ x -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -5 Int64))
  (call   main (: -9223372036854775807 Int64)) (output (: 9223372036854775807 Int64))
  (call   main (: -9223372036854775808 Int64)) (output (: 777 Int64)))

;; ── st: SAME-EFFECT shadow towers ────────────────────────────────────────────
;; Extends the two-deep tl4 pin. st1 handles one effect at THREE depths — each
;; draw resolves to the innermost open frame, outers resume as inners close;
;; st2 places draws BETWEEN the installs (each thread advances only while it
;; is the innermost, the sum pins the interleave); st3's SHADOWING frame has
;; an arm that draws the same effect — the dispatch escapes its own extent and
;; lands on the frame it shadows (the 999 state is never read).

(case "st1 the SAME effect handled at THREE depths — each draw resolves to the innermost open frame, outer frames resume as inner ones close"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (handle E 50
                     ((next () s (resume s (+ s 5))))
                     (+ (handle E 700
                          ((next () s (resume s (+ s 7))))
                          (E.next))
                        (* 10 (E.next))))
                   (* 100 (E.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1700 Int64))
  (call   main (: 0 Int64)) (output (: 1200 Int64))
  (call   main (: -3 Int64)) (output (: 900 Int64)))

(case "st2 draws BETWEEN the installs of a three-deep tower — each thread advances only while it is the innermost, sum pins the interleave"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (E.next)
                   (+ (handle E 50
                        ((next () s (resume s (+ s 5))))
                        (+ (E.next)
                           (+ (handle E 700
                                ((next () s (resume s (+ s 7))))
                                (E.next))
                              (E.next))))
                      (E.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 816 Int64))
  (call   main (: 0 Int64)) (output (: 806 Int64))
  (call   main (: -3 Int64)) (output (: 800 Int64)))

(case "st3 the SHADOWING frame's arm draws the SAME effect — its dispatch escapes its own extent and lands on the frame it shadows"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (handle E 999
                  ((next () m (resume (E.next) m)))
                  (+ (* 10 (E.next)) (E.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -4 Int64)) (output (: -43 Int64)))

;; ── li: DRAW-DRIVEN list operations ──────────────────────────────────────────
;; Collection indices and shapes decided by the thread. li1's draws choose
;; BOTH the List.update target and the List.at read index; li2 builds a list
;; from pushed draws then reads at a draw-picked index (construction and
;; consumption share one thread); li3's draw PARITY picks prepend vs push
;; while building — the final shape encodes the whole draw sequence.

(case "li1 draws choose BOTH the List.update target and the List.at read index — the collection edit follows the thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 2))))
                (let ((xs (list 10 20 30 40 50)))
                  (let ((i1 (% (E.next) 5)))
                    (let ((i2 (% (E.next) 5)))
                      (let ((ys (List.update xs i1 7)))
                        (match (List.at ys i2)
                          ((Some v) (+ (* 100 v) (+ (* 10 i1) i2)))
                          ((None) -1))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 3002 Int64))
  (call   main (: 3 Int64)) (output (: 1030 Int64))
  (call   main (: 4 Int64)) (output (: 2041 Int64)))

(case "li2 a list BUILT from pushed draws then read at a draw-picked index — construction and consumption share one thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 3))))
                (let ((xs (List.push (List.push (List.push (list) (E.next)) (E.next)) (E.next))))
                  (let ((i (% (E.next) 3)))
                    (match (List.at xs i)
                      ((Some v) (+ 100 (+ (* 10 v) i)))
                      ((None) -1))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 100 Int64))
  (call   main (: 1 Int64)) (output (: 141 Int64))
  (call   main (: 2 Int64)) (output (: 182 Int64)))

(case "li3 draw PARITY picks prepend vs push while building — the final shape encodes the whole draw sequence"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (build (: k Int64) (: xs (List Int64)))
              (if (<= k 0)
                  xs
                  (let ((d (E.next)))
                    (build (- k 1) (if (= (% d 2) 0) (List.prepend xs d) (List.push xs d))))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((xs (build 4 (list))))
                  (match (List.at xs 0)
                    ((Some h) (match (List.at xs 3)
                      ((Some t) (+ (* 100 h) (+ (* 10 t) (List.len xs))))
                      ((None) -1)))
                    ((None) -1)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 234 Int64))
  (call   main (: 1 Int64)) (output (: 434 Int64))
  (call   main (: -4 Int64)) (output (: -206 Int64)))

;; ── mp: DRAW-KEYED Map operations ────────────────────────────────────────────
;; Map keys and values decided by the thread. mp1's draws pick BOTH the insert
;; key and the lookup key (hit-old, hit-updated, and miss each reachable by
;; input); mp2's map VALUES are draws inserted in key order, weighted lookups
;; replaying the sequence; mp3's draw picks the Map.remove key — lookups of
;; all three keys show exactly one hole where the thread pointed.

(case "mp1 draws pick the Map INSERT key and the LOOKUP key — hit-old, hit-updated, and miss all reachable by input"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((k1 (+ (% (E.next) 3) 1)))
                  (let ((k2 (+ (% (E.next) 4) 1)))
                    (let ((m (Map.insert (Map.insert (Map.insert (Map.insert (map) 1 10) 2 20) 3 30) k1 77)))
                      (+ (* 100 (match (Map.lookup m k2)
                                  ((Some v) v)
                                  ((None) -5)))
                         (+ (* 10 k1) k2)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 2012 Int64))
  (call   main (: 3 Int64)) (output (: 7711 Int64))
  (call   main (: 2 Int64)) (output (: -466 Int64)))

(case "mp2 map VALUES are draws inserted in key order — weighted lookups replay the draw sequence through the map"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (get (: m (Map Int64 Int64)) (: k Int64))
              (match (Map.lookup m k) ((Some v) v) ((None) -999)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 5))))
                (let ((m (Map.insert (Map.insert (Map.insert (map) 1 (E.next)) 2 (E.next)) 3 (E.next))))
                  (+ (* 100 (get m 1)) (+ (* 10 (get m 2)) (get m 3))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 60 Int64))
  (call   main (: 1 Int64)) (output (: 171 Int64))
  (call   main (: -2 Int64)) (output (: -162 Int64)))

(case "mp3 a draw picks the Map.remove key — lookups of all three keys show exactly one hole where the thread pointed"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (get (: m (Map Int64 Int64)) (: k Int64))
              (match (Map.lookup m k) ((Some v) v) ((None) -1)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((k (+ (% (E.next) 3) 1)))
                  (let ((m (Map.remove (Map.insert (Map.insert (Map.insert (map) 1 10) 2 20) 3 30) k)))
                    (+ (* 100 (get m 1)) (+ (* 10 (get m 2)) (get m 3)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 130 Int64))
  (call   main (: 1 Int64)) (output (: 1020 Int64))
  (call   main (: 2 Int64)) (output (: 1199 Int64)))

;; ── ds: DISCARDED-statement dispatches ───────────────────────────────────────
;; Dead-value elimination must not eliminate the dispatch. ds1 discards four
;; draws in a do-chain (each still advances); ds2 discards a whole op-result
;; chain (both hops advance); ds3 discards a handle EXPRESSION whose interior
;; draws from the outer thread (frame opens, draws, closes, value dies); ds4
;; discards an if whose arms draw DIFFERENT counts (exactly the taken branch's
;; advances survive).

(case "ds1 FOUR discarded draws in a do-chain before the kept one — every discarded dispatch still advances the thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (do (E.next) (E.next) (E.next) (E.next) (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 9 Int64))
  (call   main (: 0 Int64)) (output (: 4 Int64))
  (call   main (: -9 Int64)) (output (: -5 Int64)))

(case "ds2 the DISCARDED statement is itself an op-result chain — both hops advance the thread even though the value dies"
  (input  (do
            (effect E (op a (-> Int64)) (op b (-> Int64 Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((a () s (resume s (+ s 2)))
                 (b (x) s (resume (+ x s) (+ s 3)))
                 (probe () s (resume s s)))
                (do (E.b (E.a)) (E.probe))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 10 Int64))
  (call   main (: 0 Int64)) (output (: 5 Int64))
  (call   main (: -9 Int64)) (output (: -4 Int64)))

(case "ds3 a DISCARDED handle expression whose interior draws from the outer thread — the frame opens, draws, closes, and the value dies"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect I (op pick (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (do (handle I 0
                      ((pick () t (resume t t)))
                      (do (E.next) (I.pick)))
                    (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -7 Int64)) (output (: -6 Int64)))

(case "ds4 a DISCARDED if whose arms draw DIFFERENT counts — the taken branch's advances survive the discard"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (do (if (> (E.next) 0) (do (E.next) (E.next)) (E.next))
                    (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 8 Int64))
  (call   main (: 0 Int64)) (output (: 2 Int64))
  (call   main (: -3 Int64)) (output (: -1 Int64)))

;; ── se: DRAW-DRIVEN Set operations ───────────────────────────────────────────
;; se1 collects five draws under a CYCLING state (duplicates collapse; len and
;; membership pin the distinct draws); se2 gates a branch on MEMBERSHIP of a
;; draw (the taken branch draws again); se3 builds a set from parity-locked
;; cycling draws (+2 mod 4) and probes it with a LATER state read — the
;; in-cycle probe hits, the off-cycle probe misses, and len separates
;; in-cycle starts from an out-of-cycle entry.

(case "se1 five draws collected into a Set under a cycling state — duplicates collapse, len and membership pin the distinct draws"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (% (+ s 1) 3))))
                (let ((st (Set.of (list (E.next) (E.next) (E.next) (E.next) (E.next)))))
                  (+ (* 100 (Set.len st))
                     (+ (if (Set.contains st n) 10 0)
                        (if (Set.contains st 5) 1 0))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 310 Int64))
  (call   main (: 5 Int64)) (output (: 411 Int64))
  (call   main (: 7 Int64)) (output (: 410 Int64)))

(case "se2 MEMBERSHIP of a draw decides a branch that draws again — the set gates the thread's continuation"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 10))))
                (let ((st (Set.of (list 10 20 30))))
                  (if (Set.contains st (E.next))
                      (+ 1000 (E.next))
                      (+ 2000 (E.next))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1020 Int64))
  (call   main (: 5 Int64)) (output (: 2015 Int64))
  (call   main (: 30 Int64)) (output (: 1040 Int64)))

(case "se3 a set BUILT from cycling draws probed by a LATER state read — the in-cycle probe hits, the off-cycle probe misses"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (% (+ s 2) 4)))
                 (probe () s (resume s s)))
                (let ((st (Set.of (list (E.next) (E.next) (E.next)))))
                  (let ((p (E.probe)))
                    (+ (* 1000 (if (Set.contains st p) 1 5))
                       (+ (* 100 (if (Set.contains st (+ p 1)) 1 5))
                          (+ (* 10 (Set.len st)) p)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1522 Int64))
  (call   main (: 1 Int64)) (output (: 1523 Int64))
  (call   main (: 6 Int64)) (output (: 1530 Int64)))

;; ── rw: Record.with driven by DRAWS ──────────────────────────────────────────
;; Functional record updates fed by the thread. rw1 chains two withs whose new
;; values are draws (the second write sees the advanced state); rw2's draw
;; PARITY picks WHICH field is updated (exactly one write lands); rw3's new
;; values PROJECT from the record being updated plus a draw — self-referential
;; updates chained through the thread.

(case "rw1 two sequential Record.with updates each take a DRAW — the second write sees the advanced state, projections read both back"
  (input  (do
            (effect E (op next (-> Int64)) (op span (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 3)))
                 (span () s (resume (- s 0) s)))
                (let ((r0 (record (= x 1) (= y 2))))
                  (let ((r1 (Record.with r0 #"x" (E.next))))
                    (let ((r2 (Record.with r1 #"y" (E.next))))
                      (+ (* 100 (. r2 x)) (+ (* 10 (. r2 y)) (- (E.span) n))))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 256 Int64))
  (call   main (: 0 Int64)) (output (: 36 Int64))
  (call   main (: -4 Int64)) (output (: -404 Int64)))

(case "rw2 draw PARITY picks WHICH field Record.with updates — projections of both fields show exactly one write landed"
  (input  (do
            (effect E (op next (-> Int64)) (op span (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 3)))
                 (span () s (resume s s)))
                (let ((d (E.next)))
                  (let ((r0 (record (= x 1) (= y 2))))
                    (let ((r (if (= (% d 2) 0)
                                 (Record.with r0 #"x" d)
                                 (Record.with r0 #"y" d))))
                      (+ (* 100 (. r x)) (+ (* 10 (. r y)) (- (E.span) n))))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 223 Int64))
  (call   main (: 5 Int64)) (output (: 153 Int64))
  (call   main (: -4 Int64)) (output (: -377 Int64)))

(case "rw3 each Record.with value PROJECTS from the record it updates plus a draw — self-referential functional updates chain through the thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 4))))
                (let ((r0 (record (= x 1) (= y 2))))
                  (let ((r1 (Record.with r0 #"x" (+ (* 10 (. r0 x)) (E.next)))))
                    (let ((r2 (Record.with r1 #"y" (+ (. r1 x) (+ (. r1 y) (E.next))))))
                      (+ (* 100 (. r2 x)) (. r2 y)))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 1220 Int64))
  (call   main (: 0 Int64)) (output (: 1016 Int64))
  (call   main (: -3 Int64)) (output (: 710 Int64)))

;; ── sd: STRING content via draws ─────────────────────────────────────────────
;; sd1's draw COUNT drives string repetition through a recursion (byte-len
;; pins how many concats the thread ordered); sd2's draw PARITY picks WHICH
;; string each op returns (concat ORDER visible in content equality at equal
;; length); sd3 bounds a String.slice window with draws — start and end both
;; come from the thread, byte-len pins the window width.

(case "sd1 a draw COUNT drives string repetition through a recursion — String.byte-len pins how many times the thread said to concat"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (rep (: k Int64) (: acc String))
              (if (<= k 0) acc (rep (- k 1) (String.concat acc "ab"))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((k (+ (% (E.next) 3) 1)))
                  (+ (* 100 (String.byte-len (rep k "")))
                     (- (E.probe) n)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 201 Int64))
  (call   main (: 1 Int64)) (output (: 401 Int64))
  (call   main (: 2 Int64)) (output (: 601 Int64)))

(case "sd2 draw parity picks WHICH string each op returns — the concat order of two draws is visible in content equality"
  (input  (do
            (effect E (op pick (-> String)))
            (def (main (: n Int64))
              (handle E n
                ((pick () s (resume (if (= (% s 2) 0) "xy" "pqr") (+ s 1))))
                (let ((a (E.pick)))
                  (let ((b (E.pick)))
                    (let ((st (String.concat a b)))
                      (+ (* 100 (String.byte-len st))
                         (if (= st "xypqr") 10 (if (= st "pqrxy") 20 30))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 510 Int64))
  (call   main (: 1 Int64)) (output (: 520 Int64)))

(case "sd3 a draw-bounded String.slice window — start and end both come from the thread, byte-len pins the window width"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 2))))
                (let ((st (% (E.next) 4)))
                  (let ((en (+ st (+ (% (E.next) 3) 1))))
                    (match (String.slice "abcdefgh" st en)
                      ((Some w) (+ (* 100 (String.byte-len w)) (+ (* 10 st) (- en st))))
                      ((None _u) -1))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 303 Int64))
  (call   main (: 1 Int64)) (output (: 111 Int64))
  (call   main (: 5 Int64)) (output (: 212 Int64)))

;; ── ta/by/syd: interior aborts in towers, and bytes/symbols via draws ────────
;; ta2 aborts INSIDE the inner frame of a same-effect tower (Bail innermost;
;; both E threads keep their positions; the inner frame lives in a def taking
;; the outer draw as a parameter — the let-crossing form hits the known
;; bind-once CDZ0101). The cross-frame abort (Bail BETWEEN the E frames) is a
;; known fold decline; witness banked. by1 builds bytes from three wrapped
;; draws and reads at a draw-picked index. syd1's op returns a SYMBOL picked
;; by state parity — symbol equality gates a branch that draws again.

(case "ta2 an abort INSIDE the inner frame of a same-effect tower — Bail innermost, both E threads keep their positions"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect Bail (op out (-> Int64 Int64)))
            (def (inner-run (: k Int64))
              (handle E (* 10 k)
                ((next () s (resume s (+ s 5))))
                (+ (handle Bail 0
                     ((out (v) t (+ 1000 v)))
                     (let ((d (E.next)))
                       (if (> d 52) (Bail.out d) d)))
                   (E.next))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (* 10 (inner-run (E.next))) (- (E.next) n))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 451 Int64))
  (call   main (: 6 Int64)) (output (: 11251 Int64))
  (call   main (: 0 Int64)) (output (: 51 Int64))
  (call   main (: -3 Int64)) (output (: -549 Int64)))

(case "by1 bytes built from THREE draws then read at a draw-picked index — Bytes.at follows the thread into the buffer"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 7))))
                (let ((b (Bytes.of (list (UInt8.wrap (+ (% (E.next) 200) 56)) (UInt8.wrap (+ (% (E.next) 200) 56)) (UInt8.wrap (+ (% (E.next) 200) 56))))))
                  (let ((i (% (E.next) 3)))
                    (match (Bytes.at b i)
                      ((Some v) (+ (* 100 v) (+ (* 10 (Bytes.len b)) i)))
                      (None -1))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 5630 Int64))
  (call   main (: 1 Int64)) (output (: 6431 Int64))
  (call   main (: 2 Int64)) (output (: 7232 Int64)))

(case "syd1 an op returns a SYMBOL picked by state parity — symbol equality gates a branch that draws again"
  (input  (do
            (effect E (op tag (-> Symbol)) (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((tag () s (resume (if (= (% s 2) 0) #"alpha" #"beta") (+ s 1)))
                 (next () s (resume s (+ s 1))))
                (if (= (E.tag) #"alpha")
                    (+ 100 (E.next))
                    (+ 200 (E.next)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 105 Int64))
  (call   main (: 7 Int64)) (output (: 208 Int64))
  (call   main (: -2 Int64)) (output (: 99 Int64)))

;; ── dc: DEF relay chains under one handler ───────────────────────────────────
;; Multi-hop call relays where every hop performs against the SAME enclosing
;; frame (distinct from ed's per-def handlers and cn's nested shadow seeds).
;; dc1 relays three hops deep, each def drawing then calling the next (weights
;; pin depth order); dc2's relay ARGUMENT is a draw (argument-before-body
;; order across the def boundary); dc3's MIDDLE hop is chosen by a draw —
;; a two-draw or one-draw callee, the tail draw pinning the total advance.

(case "dc1 a THREE-hop def relay under ONE handler — each def draws then calls the next, weights pin the depth order"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (h3) (E.next))
            (def (g2) (+ (* 10 (E.next)) (h3)))
            (def (f1) (+ (* 100 (E.next)) (g2)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (f1)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 345 Int64))
  (call   main (: 0 Int64)) (output (: 12 Int64))
  (call   main (: -2 Int64)) (output (: -210 Int64)))

(case "dc2 the relay call's ARGUMENT is a draw — the callee draws again and combines, argument-before-body order pinned"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (f (: x Int64)) (+ (* 10 x) (E.next)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (f (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -4 Int64)) (output (: -43 Int64)))

(case "dc3 the MIDDLE relay hop is chosen by a draw — the branch decides between a two-draw and a one-draw callee, a tail draw pins the total"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (g2) (+ (* 10 (E.next)) (E.next)))
            (def (h1) (E.next))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (* 100 (if (> (E.next) 0) (g2) (h1)))
                   (- (E.next) n))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 5603 Int64))
  (call   main (: 0 Int64)) (output (: 102 Int64))
  (call   main (: -3 Int64)) (output (: -198 Int64)))

;; ── tq: TWO-effect regions with same-effect shadows inside ───────────────────
;; tq1 rebinds Q locally while P draws thread THROUGH the shadow untouched;
;; tq2's shadow SEED mixes P and Q draws (both outer threads advance before
;; the shadow opens and resume after it closes); tq3 wraps ONLY the middle
;; argument of a pure call in a Q-shadow — its neighbors dispatch to the
;; outer P and Q frames.

(case "tq1 a Q-shadow inside a TWO-effect region — P draws thread THROUGH the shadow while Q is locally rebound"
  (input  (do
            (effect P (op next (-> Int64)))
            (effect Q (op next (-> Int64)))
            (def (main (: n Int64))
              (handle P n
                ((next () s (resume s (+ s 1))))
                (handle Q 100
                  ((next () t (resume t (+ t 10))))
                  (+ (P.next)
                     (+ (Q.next)
                        (+ (handle Q 7000
                             ((next () t (resume t (+ t 100))))
                             (+ (Q.next) (P.next)))
                           (Q.next)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7221 Int64))
  (call   main (: 0 Int64)) (output (: 7211 Int64))
  (call   main (: -8 Int64)) (output (: 7195 Int64)))

(case "tq2 the Q-shadow's SEED mixes P and Q draws — both outer threads advance before the shadow opens, and resume after"
  (input  (do
            (effect P (op next (-> Int64)))
            (effect Q (op next (-> Int64)))
            (def (main (: n Int64))
              (handle P n
                ((next () s (resume s (+ s 1))))
                (handle Q 100
                  ((next () t (resume t (+ t 10))))
                  (+ (handle Q (+ (* 100 (P.next)) (Q.next))
                       ((next () t (resume t t)))
                       (Q.next))
                     (+ (Q.next) (P.next))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 413 Int64))
  (call   main (: 0 Int64)) (output (: 211 Int64))
  (call   main (: -5 Int64)) (output (: -294 Int64)))

(case "tq3 a Q-shadow wraps only the MIDDLE argument of a pure call — its neighbors dispatch to the outer P and Q frames"
  (input  (do
            (effect P (op next (-> Int64)))
            (effect Q (op next (-> Int64)))
            (def (sum3 (: a Int64) (: b Int64) (: c Int64)) (+ a (+ b c)))
            (def (main (: n Int64))
              (handle P n
                ((next () s (resume s (+ s 1))))
                (handle Q 100
                  ((next () t (resume t (+ t 10))))
                  (sum3 (P.next)
                        (handle Q 9000
                          ((next () t (resume t (+ t 9))))
                          (Q.next))
                        (Q.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 9105 Int64))
  (call   main (: 0 Int64)) (output (: 9100 Int64))
  (call   main (: -3 Int64)) (output (: 9097 Int64)))

;; ── tq (cont.): double shadows and def-installed shadows ────────────────────
;; tq4 shadows BOTH effects in one inner region (two fresh threads run their
;; course; both outer threads untouched and resume exactly); tq5 installs the
;; Q-shadow inside a DEF that also draws P — the P dispatch crosses the def
;; boundary AND the shadow to the caller's frame.

(case "tq4 BOTH effects shadowed in one inner region — two fresh threads run their course, both outer threads untouched"
  (input  (do
            (effect P (op next (-> Int64)))
            (effect Q (op next (-> Int64)))
            (def (main (: n Int64))
              (handle P n
                ((next () s (resume s (+ s 1))))
                (handle Q 100
                  ((next () t (resume t (+ t 10))))
                  (+ (P.next)
                     (+ (Q.next)
                        (+ (handle P 40
                             ((next () s (resume s (+ s 4))))
                             (handle Q 7000
                               ((next () t (resume t (+ t 700))))
                               (+ (P.next) (+ (Q.next) (P.next)))))
                           (+ (P.next) (Q.next))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 7305 Int64))
  (call   main (: 0 Int64)) (output (: 7295 Int64))
  (call   main (: -9 Int64)) (output (: 7277 Int64)))

(case "tq5 a DEF installs the Q-shadow and draws P from inside it — the P dispatch crosses the def AND the shadow to the caller's frame"
  (input  (do
            (effect P (op next (-> Int64)))
            (effect Q (op next (-> Int64)))
            (def (qshadow)
              (handle Q 9000
                ((next () t (resume t (+ t 9))))
                (+ (Q.next) (P.next))))
            (def (main (: n Int64))
              (handle P n
                ((next () s (resume s (+ s 1))))
                (handle Q 100
                  ((next () t (resume t (+ t 10))))
                  (+ (P.next)
                     (+ (qshadow)
                        (+ (Q.next) (P.next)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 9118 Int64))
  (call   main (: 0 Int64)) (output (: 9103 Int64))
  (call   main (: -6 Int64)) (output (: 9085 Int64)))

;; ── mr: MUTUAL recursion drawing at every level ──────────────────────────────
;; The landed mutual pin performs only at the BASE; these draw per level. mr1
;; alternates ev/od with x10/x1 weights; mr2 splits the pair across TWO
;; effects (ev advances P, od advances Q); mr3 picks the NEXT callee by draw
;; parity (the descent path follows the thread, three-way group); mr4 threads
;; an ACCUMULATOR (ev doubles + draw, od triples + draw); mr5 stresses depth
;; at TWENTY alternating levels. A SECOND entry into the pair under one
;; handler remains the documented decline (fold serves one mutual chain);
;; witness banked.

(case "mr1 mutual recursion drawing at EVERY level — even levels weight their draw x10, odd levels x1"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (ev (: k Int64))
              (if (<= k 0) 0 (+ (* 10 (E.next)) (od (- k 1)))))
            (def (od (: k Int64))
              (if (<= k 0) 0 (+ (E.next) (ev (- k 1)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (ev 4)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 68 Int64))
  (call   main (: 0 Int64)) (output (: 24 Int64))
  (call   main (: -3 Int64)) (output (: -42 Int64)))

(case "mr2 the mutual pair draws from DIFFERENT effects — ev advances P, od advances Q, both threads interleave down the descent"
  (input  (do
            (effect P (op next (-> Int64)))
            (effect Q (op next (-> Int64)))
            (def (ev (: k Int64))
              (if (<= k 0) 0 (+ (* 10 (P.next)) (od (- k 1)))))
            (def (od (: k Int64))
              (if (<= k 0) 0 (+ (Q.next) (ev (- k 1)))))
            (def (main (: n Int64))
              (handle P n
                ((next () s (resume s (+ s 1))))
                (handle Q 100
                  ((next () t (resume t (+ t 10))))
                  (ev 4))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 260 Int64))
  (call   main (: 0 Int64)) (output (: 220 Int64))
  (call   main (: -5 Int64)) (output (: 120 Int64)))

(case "mr3 the NEXT callee in the mutual group is picked by draw parity — the descent path itself follows the thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (walk (: k Int64))
              (if (<= k 0)
                  0
                  (let ((d (E.next)))
                    (if (= (% d 2) 0)
                        (+ (* 10 d) (a (- k 1)))
                        (+ d (b (- k 1)))))))
            (def (a (: k Int64)) (+ 1000 (walk k)))
            (def (b (: k Int64)) (+ 2000 (walk k)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (walk 3)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 4063 Int64))
  (call   main (: 1 Int64)) (output (: 5024 Int64))
  (call   main (: -4 Int64)) (output (: 3937 Int64)))

(case "mr4 an ACCUMULATOR threads the mutual pair — ev doubles it plus a draw, od triples it plus a draw, alternating scales"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (ev (: k Int64) (: acc Int64))
              (if (<= k 0) acc (od (- k 1) (+ (* 2 acc) (E.next)))))
            (def (od (: k Int64) (: acc Int64))
              (if (<= k 0) acc (ev (- k 1) (+ (* 3 acc) (E.next)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (ev 4 0)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 71 Int64))
  (call   main (: 0 Int64)) (output (: 15 Int64))
  (call   main (: -3 Int64)) (output (: -69 Int64)))

(case "mr5 TWENTY alternating mutual levels with the scaling accumulator — depth stress on the cross-function fold"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (ev (: k Int64) (: acc Int64))
              (if (<= k 0) acc (od (- k 1) (+ (* 2 acc) (E.next)))))
            (def (od (: k Int64) (: acc Int64))
              (if (<= k 0) acc (ev (- k 1) (+ (* 3 acc) (E.next)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (ev 20 0)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 79815335 Int64))
  (call   main (: 0 Int64)) (output (: 31442395 Int64))
  (call   main (: -10 Int64)) (output (: -452287005 Int64)))

;; ── gd: PURE guards around the effect thread ─────────────────────────────────
;; Post-CDZ0407 (guards must be side-effect-free) these pin the ALLOWED side.
;; gd1 tiers three pure guards over a draw-bound scrutinee; gd2 guards
;; SUM-PAYLOAD binders (guarded/unguarded twins per variant); gd3's guard
;; COMPARES the scrutinee draw to an earlier let-bound draw (two thread values
;; in one predicate, non-monotone stepper makes both verdicts reachable);
;; gd4 puts pure guards INSIDE the handler arm to grade the live state (three
;; dispatches cross the tiers as the state climbs); gd5 guards TUPLE pattern
;; binders (all three orderings input-reachable).

(case "gd1 PURE guards over a draw-bound scrutinee — three guard tiers select on the drawn value, the trailing probe pins one advance"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (+ (* 10 (match (E.next)
                           ((guard x (> x 5)) (+ 100 x))
                           ((guard x (> x 0)) (+ 200 x))
                           (x (+ 300 x))))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 1071 Int64))
  (call   main (: 3 Int64)) (output (: 2031 Int64))
  (call   main (: -2 Int64)) (output (: 2981 Int64)))

(case "gd2 pure guards on SUM-PAYLOAD binders — guarded and unguarded twins of each variant arm, five paths input-reachable"
  (input  (do
            (type Mode (A) (B Int64) (C Int64 Int64))
            (effect E (op mode (-> Mode)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((mode () s (resume (match (% s 3)
                                      (0 (A))
                                      (1 (B s))
                                      (_ (C s s)))
                                    (+ s 1)))
                 (probe () s (resume s s)))
                (+ (* 10 (match (E.mode)
                           ((guard (B x) (> x 3)) (+ 100 x))
                           ((B x) (+ 200 x))
                           ((guard (C x y) (> (+ x y) 10)) (+ 300 (+ x y)))
                           ((C x y) (+ 400 (+ x y)))
                           ((A) 7)))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 1071 Int64))
  (call   main (: 1 Int64)) (output (: 2011 Int64))
  (call   main (: 8 Int64)) (output (: 3161 Int64))
  (call   main (: 2 Int64)) (output (: 4041 Int64))
  (call   main (: 0 Int64)) (output (: 71 Int64)))

(case "gd3 the guard COMPARES the scrutinee draw to an earlier let-bound draw — two thread values meet in one pure predicate"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (if (= (% s 2) 0) (+ s 1) (- s 2)))))
                (let ((a (E.next)))
                  (match (E.next)
                    ((guard b (> b a)) (+ (* 10 (+ 100 b)) (- b a)))
                    (b (+ (* 10 (+ 300 b)) (- a b)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1011 Int64))
  (call   main (: 1 Int64)) (output (: 2992 Int64))
  (call   main (: 3 Int64)) (output (: 3012 Int64)))

(case "gd4 pure guards INSIDE the handler arm grade the live state — three dispatches cross the tier boundaries as the state climbs"
  (input  (do
            (effect E (op grade (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((grade () s (resume (match s
                                       ((guard v (> v 10)) 3)
                                       ((guard v (> v 0)) 2)
                                       (_v 1))
                                     (+ s 4))))
                (+ (* 100 (E.grade)) (+ (* 10 (E.grade)) (E.grade)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 223 Int64))
  (call   main (: -2 Int64)) (output (: 122 Int64))
  (call   main (: 11 Int64)) (output (: 333 Int64)))

(case "gd5 guards over TUPLE pattern binders — the predicate compares the tuple's own components, all three orderings reachable"
  (input  (do
            (effect E (op pair (-> (Tuple Int64 Int64))) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((pair () s (resume (tuple s (* 2 s)) (+ s 3)))
                 (probe () s (resume s s)))
                (+ (* 10 (match (E.pair)
                           ((guard (tuple a b) (< a b)) (+ 100 b))
                           ((guard (tuple a b) (= a b)) 200)
                           ((tuple a b) (+ 300 b))))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 1083 Int64))
  (call   main (: 0 Int64)) (output (: 2003 Int64))
  (call   main (: -5 Int64)) (output (: 2903 Int64)))

;; ── ns: RECORDS inside SUM payloads through dispatch ─────────────────────────
;; (Record-type fields in canonical `(: name T)` ascription per RT4.) ns1
;; builds Wrap{x,y} from two draws in the arm, matched then projected; ns2
;; ROUND-TRIPS it — matched via an unbox helper, field-updated with a draw,
;; re-wrapped, echoed through a second op, re-matched (the nested-match form
;; is the known op-built-scrutinee fold decline); ns3 selects between TWO
;; record-payload variants by state parity; ns4 nests a sum-wrapped record as
;; a FIELD of another record payload (helper unboxers traverse both layers);
;; ns5 pins that the nested two-layer match FOLDS when the outer scrutinee is
;; a pure literal — only op-built outer scrutinees decline.

(case "ns1 a RECORD rides inside a sum payload through dispatch — the arm builds it from two draws, the body matches then projects"
  (input  (do
            (type Box (Wrap (Record (: x Int64) (: y Int64))))
            (effect E (op make (-> Box)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((make () s (resume (Box.Wrap (record (= x s) (= y (+ s 2)))) (+ s 4)))
                 (probe () s (resume s s)))
                (match (E.make)
                  ((Box.Wrap r) (+ (* 100 (. r x)) (+ (* 10 (. r y)) (- (E.probe) n)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 354 Int64))
  (call   main (: 0 Int64)) (output (: 24 Int64))
  (call   main (: -4 Int64)) (output (: -416 Int64)))

(case "ns2 the sum-wrapped record round-trips FLATTENED — matched, field-updated with a draw, re-wrapped, echoed, re-matched at top level"
  (input  (do
            (type Box (Wrap (Record (: x Int64) (: y Int64))))
            (effect E (op make (-> Box)) (op keep (-> Box Box)) (op next (-> Int64)) (op probe (-> Int64)))
            (def (unbox (: b Box))
              (match b ((Box.Wrap r) r)))
            (def (main (: n Int64))
              (handle E n
                ((make () s (resume (Box.Wrap (record (= x s) (= y (+ s 2)))) (+ s 4)))
                 (keep (b) s (resume b s))
                 (next () s (resume s (+ s 4)))
                 (probe () s (resume s s)))
                (let ((r (unbox (E.make))))
                  (let ((r2 (unbox (E.keep (Box.Wrap (Record.with r #"y" (+ (. r y) (E.next))))))))
                    (+ (* 100 (. r2 x)) (+ (* 10 (. r2 y)) (- (E.probe) n)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 428 Int64))
  (call   main (: 0 Int64)) (output (: 68 Int64))
  (call   main (: -6 Int64)) (output (: -652 Int64)))

(case "ns3 TWO record-payload variants selected by state parity — each arm projects its own record shape"
  (input  (do
            (type Shape
              (Pt (Record (: x Int64) (: y Int64)))
              (Ln (Record (: a Int64) (: b Int64) (: len Int64))))
            (effect E (op make (-> Shape)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((make () s (resume (if (= (% s 2) 0)
                                        (Shape.Pt (record (= x s) (= y (+ s 1))))
                                        (Shape.Ln (record (= a s) (= b (* 2 s)) (= len (* 3 s)))))
                                    (+ s 5)))
                 (probe () s (resume s s)))
                (+ (* 10 (match (E.make)
                           ((Shape.Pt r) (+ (* 100 (. r x)) (* 10 (. r y))))
                           ((Shape.Ln r) (+ (. r a) (+ (. r b) (. r len))))))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 4505 Int64))
  (call   main (: 3 Int64)) (output (: 185 Int64))
  (call   main (: 0 Int64)) (output (: 105 Int64))
  (call   main (: -5 Int64)) (output (: -295 Int64)))

(case "ns4 a record payload whose FIELD is itself a sum-wrapped record — helper unboxers traverse two wrap layers built from one dispatch"
  (input  (do
            (type Box (Wrap (Record (: x Int64) (: y Int64))))
            (type Big (Node (Record (: tag Int64) (: inner Box))))
            (effect E (op make (-> Big)) (op probe (-> Int64)))
            (def (unnode (: g Big)) (match g ((Big.Node o) o)))
            (def (unwrap (: b Box)) (match b ((Box.Wrap r) r)))
            (def (main (: n Int64))
              (handle E n
                ((make () s (resume (Big.Node (record (= tag (+ s 4))
                                               (= inner (Box.Wrap (record (= x s) (= y (+ s 2)))))))
                                    (+ s 6)))
                 (probe () s (resume s s)))
                (let ((outer (unnode (E.make))))
                  (let ((r (unwrap (. outer inner))))
                    (+ (* 100 (. outer tag))
                       (+ (* 10 (. r x))
                          (+ (. r y) (* 1000 (- (E.probe) n)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 6735 Int64))
  (call   main (: 0 Int64)) (output (: 6402 Int64))
  (call   main (: -4 Int64)) (output (: 5958 Int64)))

(case "ns5 the nested two-layer match FOLDS when the scrutinee is a pure literal — only op-built scrutinees push it off the fold"
  (input  (do
            (type Box (Wrap (Record (: x Int64) (: y Int64))))
            (type Big (Node (Record (: tag Int64) (: inner Box))))
            (effect E (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((probe () s (resume s s)))
                (match (Big.Node (record (= tag n) (= inner (Box.Wrap (record (= x 1) (= y 2))))))
                  ((Big.Node outer)
                    (match (. outer inner)
                      ((Box.Wrap r) (+ (* 100 (. outer tag)) (+ (* 10 (. r x)) (. r y)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 312 Int64))
  (call   main (: 0 Int64)) (output (: 12 Int64))
  (call   main (: -4 Int64)) (output (: -388 Int64)))

;; ── rc/bt/ng/sw: rich handler-STATE shapes ───────────────────────────────────
;; rc1 a RECORD state (fields advance differently per rebuild); rc2 the arm
;; updates ONE field chosen by the fields' own parity; rc3 a record whose
;; FIELD is a tuple (counter ticks while the pair walks a Fibonacci step);
;; bt1 a BOOL state toggles per dispatch (not in state position); ng1 a
;; MIXED-type (Tuple Int64 Bool) state — the flip signs alternate draws;
;; sw1 a three-slot state SWAPS its pair while a counter ticks.

(case "rc1 a RECORD handler state — the arm projects both fields and rebuilds with different advances, sums pin two dispatches"
  (input  (do
            (effect E (op snap (-> (Record (: a Int64) (: b Int64)))))
            (def (main (: n Int64))
              (handle E (record (= a n) (= b 100))
                ((snap () s (resume s (record (= a (+ (. s a) 1)) (= b (* (. s b) 2))))))
                (let ((r1 (E.snap)))
                  (let ((r2 (E.snap)))
                    (+ (+ (. r1 a) (. r1 b))
                       (* 10 (+ (. r2 a) (. r2 b))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2165 Int64))
  (call   main (: 0 Int64)) (output (: 2110 Int64))
  (call   main (: -3 Int64)) (output (: 2077 Int64)))

(case "rc2 the arm updates ONE record-state field chosen by the fields' own parity — three snapshots pin the alternating writes"
  (input  (do
            (effect E (op snap (-> (Record (: a Int64) (: b Int64)))))
            (def (main (: n Int64))
              (handle E (record (= a n) (= b 100))
                ((snap () s (resume s (if (= (% (+ (. s a) (. s b)) 2) 0)
                                          (record (= a (+ (. s a) 7)) (= b (. s b)))
                                          (record (= a (. s a)) (= b (+ (. s b) 7)))))))
                (let ((r1 (E.snap)))
                  (let ((r2 (E.snap)))
                    (let ((r3 (E.snap)))
                      (+ (+ (. r1 a) (. r1 b))
                         (+ (* 10 (+ (. r2 a) (. r2 b)))
                            (* 100 (+ (. r3 a) (. r3 b))))))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 13014 Int64))
  (call   main (: 1 Int64)) (output (: 12681 Int64))
  (call   main (: -2 Int64)) (output (: 12348 Int64)))

(case "rc3 a record state whose FIELD is a tuple — the counter ticks while the pair walks a Fibonacci step per dispatch"
  (input  (do
            (effect E (op snap (-> (Record (: ctr Int64) (: pair (Tuple Int64 Int64))))))
            (def (main (: n Int64))
              (handle E (record (= ctr n) (= pair (tuple 1 2)))
                ((snap () s (resume s (match (. s pair)
                                        ((tuple lo hi) (record (= ctr (+ (. s ctr) 1))
                                                               (= pair (tuple hi (+ lo hi)))))))))
                (let ((r1 (E.snap)))
                  (let ((r2 (E.snap)))
                    (let ((r3 (E.snap)))
                      (match (. r1 pair)
                        ((tuple a1 b1) (match (. r2 pair)
                          ((tuple a2 b2) (match (. r3 pair)
                            ((tuple a3 b3)
                              (+ (* 100 (+ (. r1 ctr) (+ a1 b1)))
                                 (+ (* 10 (+ (. r2 ctr) (+ a2 b2)))
                                    (+ (. r3 ctr) (+ a3 b3)))))))))))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 814 Int64))
  (call   main (: 0 Int64)) (output (: 370 Int64))
  (call   main (: -3 Int64)) (output (: 37 Int64)))

(case "bt1 a BOOL handler state TOGGLES per dispatch — three draws read the alternating flag, seeded by input parity"
  (input  (do
            (effect E (op flag (-> Int64)))
            (def (main (: n Int64))
              (handle E (= (% n 2) 0)
                ((flag () b (resume (if b 1 0) (not b))))
                (+ (* 100 (E.flag)) (+ (* 10 (E.flag)) (E.flag)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 101 Int64))
  (call   main (: 3 Int64)) (output (: 10 Int64)))

(case "ng1 the arm NEGATES alternate draws — a Bool flip in a tuple state signs the rising thread (+,-,+)"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n false)
                ((next () s (match s
                              ((tuple v flip)
                                (resume (if flip (- 0 v) v)
                                        (tuple (+ v 2) (not flip)))))))
                (+ (* 100 (E.next)) (+ (* 10 (E.next)) (E.next)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 257 Int64))
  (call   main (: 0 Int64)) (output (: -16 Int64))
  (call   main (: -2 Int64)) (output (: -198 Int64)))

(case "sw1 a three-slot state SWAPS its pair while a counter ticks — the encoding exposes position, order, and dispatch count at once"
  (input  (do
            (effect E (op swap (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n (+ n 1) 0)
                ((swap () s (match s
                              ((tuple a b k)
                                (resume (+ (* 100 a) (+ (* 10 b) k))
                                        (tuple b a (+ k 1)))))))
                (+ (E.swap) (+ (E.swap) (E.swap)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 783 Int64))
  (call   main (: 0 Int64)) (output (: 123 Int64))
  (call   main (: -3 Int64)) (output (: -867 Int64)))

;; ── bw/u: BIT operations and UInt64 threads ──────────────────────────────────
;; bw1 mask/set-bit/draw-driven-shift in the body; bw2 an XOR accumulator
;; through a recursion; bw3 a shift-up-then-back round trip over masked draws
;; (value AND count from the thread); bw4 the ARM bit-mixes its argument with
;; the live state. u1 a UInt64 state thread ABOVE Int64.max (high half
;; survives dispatch); u3 MIN-adjacent/max UInt64 literals echo through as op
;; ARGUMENTS exactly; u4 a high-half UInt64 rides as a SUM payload (variant by
;; parity, Hi arm range-checks). Top-wrap wrapping-add on the high half is a
;; staged decline; witness banked.

(case "bw1 BIT operations over draws — mask, set-bit, and a draw-driven shift count all read the live thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 5))))
                (let ((d1 (E.next)))
                  (let ((d2 (E.next)))
                    (+ (* 100 (& d1 7))
                       (+ (* 10 (<< 1 (& d1 3)))
                          (| d2 8)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 388 Int64))
  (call   main (: 0 Int64)) (output (: 23 Int64))
  (call   main (: 6 Int64)) (output (: 651 Int64)))

(case "bw2 an XOR accumulator folds four draws through a recursion — bit-mixing order-sensitive under the stride"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (mix (: k Int64) (: acc Int64))
              (if (<= k 0) acc (mix (- k 1) (^ acc (E.next)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 3))))
                (+ (* 10 (mix 4 0)) 12)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 92 Int64))
  (call   main (: 0 Int64)) (output (: 132 Int64))
  (call   main (: 9 Int64)) (output (: 252 Int64)))

(case "bw3 shift-up then shift-back round trip over MASKED draws — both the value and the shift count come from the thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 6))))
                (let ((d (& (E.next) 15)))
                  (let ((k (& (E.next) 3)))
                    (let ((up (<< d k)))
                      (+ (* 1000 (if (= (>> up k) d) 1 5))
                         (+ (* 10 up) k)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1061 Int64))
  (call   main (: 0 Int64)) (output (: 1002 Int64))
  (call   main (: 10 Int64)) (output (: 1100 Int64)))

(case "bw4 the ARM bit-mixes its argument with the live state — low nibble from the arg, bits 4-5 stamped from the state"
  (input  (do
            (effect E (op tag (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((tag (x) s (resume (+ (& x 15) (<< (& s 3) 4)) (+ s 1))))
                (+ (* 100 (E.tag 9)) (E.tag 20))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 2536 Int64))
  (call   main (: 0 Int64)) (output (: 920 Int64))
  (call   main (: 6 Int64)) (output (: 4152 Int64)))

(case "u1 a UInt64 state thread ABOVE Int64.max — high-half values survive dispatch, mod-10 digits pin the sequence"
  (input  (do
            (effect E (op next (-> UInt64)))
            (def (main (: n UInt64))
              (handle E (+ (: 9223372036854775808 UInt64) n)
                ((next () s (resume s (+ s (: 1 UInt64)))))
                (let ((d1 (E.next)))
                  (let ((d2 (E.next)))
                    (+ (* (: 10 UInt64) (% d1 (: 10 UInt64))) (% d2 (: 10 UInt64)))))))
            (export main)))
  (call   main (: 5 UInt64)) (output (: 34 UInt64))
  (call   main (: 0 UInt64)) (output (: 89 UInt64))
  (call   main (: 7 UInt64)) (output (: 56 UInt64)))

(case "u3 UInt64 arguments ABOVE Int64.max echo through dispatch exactly — 2^63+41 and u64-max both survive, count pins trips"
  (input  (do
            (effect E (op keep (-> UInt64 UInt64)) (op count (-> UInt64)))
            (def (main (: u UInt64))
              (handle E (: 0 UInt64)
                ((keep (x) s (resume x (+ s (: 1 UInt64))))
                 (count () s (resume s s)))
                (+ (* (: 100 UInt64) (if (= (E.keep (: 9223372036854775849 UInt64)) (: 9223372036854775849 UInt64)) (: 1 UInt64) (: 9 UInt64)))
                   (+ (* (: 10 UInt64) (if (= (E.keep (: 18446744073709551615 UInt64)) (: 18446744073709551615 UInt64)) (: 1 UInt64) (: 9 UInt64)))
                      (E.count)))))
            (export main)))
  (call   main (: 0 UInt64)) (output (: 112 UInt64)))

(case "u4 a UInt64 ABOVE Int64.max rides as a SUM payload — parity picks Lo/Hi, the Hi arm range-checks the high half"
  (input  (do
            (type UBox (Lo UInt64) (Hi UInt64))
            (effect E (op make (-> UBox)) (op probe (-> UInt64)))
            (def (main (: n UInt64))
              (handle E n
                ((make () s (resume (if (= (% s (: 2 UInt64)) (: 0 UInt64))
                                        (UBox.Lo s)
                                        (UBox.Hi (+ (: 9223372036854775808 UInt64) s)))
                                    (+ s (: 1 UInt64))))
                 (probe () s (resume s s)))
                (+ (* (: 10 UInt64) (match (E.make)
                                      ((UBox.Lo v) (+ (: 100 UInt64) v))
                                      ((UBox.Hi v) (if (>= v (: 9223372036854775808 UInt64)) (: 1 UInt64) (: 9 UInt64)))))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 4 UInt64)) (output (: 1041 UInt64))
  (call   main (: 3 UInt64)) (output (: 11 UInt64)))

;; ── fe/sb/pf/mm/cl/cx: arm-side computation shapes ───────────────────────────
;; fe5 Float64 ARGUMENTS halved in the arm (exact fractions both directions);
;; fe6 a Float64 TUPLE state (one slot additive, one doubling); sb1 the
;; text-to-bytes bridge follows the thread twice (draw-picked string, draw
;; index); pf1 an AGGREGATOR arm returns the running sum of every value fed;
;; mm1 a MIN/MAX arm tightens a (lo,hi) window over three feeds; cl1 a
;; NARROWING clamp (window shrinks per dispatch); cx1 a COUNTDOWN arm
;; exhausts mid-sequence (floor returns the arm's own constant).

(case "fe5 Float64 arguments HALVED in the arm — exact binary fractions cross dispatch both directions, count folds in"
  (input  (do
            (effect E (op halve (-> Float64 Float64)) (op count (-> Float64)))
            (def (main (: u Float64))
              (handle E 0.0
                ((halve (x) s (resume (* x 0.5) (+ s 1.0)))
                 (count () s (resume s s)))
                (+ (E.halve 3.0) (+ (E.halve 0.25) (E.count)))))
            (export main)))
  (call   main (: 0.0 Float64)) (output (: 3.625 Float64)))

(case "fe6 a Float64 TUPLE state — one slot advances additively, the other doubles, both exact across two dispatches"
  (input  (do
            (effect E (op pair (-> (Tuple Float64 Float64))))
            (def (main (: u Float64))
              (handle E (tuple 0.5 1.0)
                ((pair () s (match s
                              ((tuple a b) (resume s (tuple (+ a 1.5) (* b 2.0)))))))
                (match (E.pair)
                  ((tuple a1 b1)
                    (match (E.pair)
                      ((tuple a2 b2) (+ (* 100.0 a1) (+ (* 10.0 b1) (+ a2 b2)))))))))
            (export main)))
  (call   main (: 0.0 Float64)) (output (: 64.0 Float64)))

(case "sb1 String.to-bytes of a draw-picked string read at a draw index — the text-to-bytes bridge follows the thread twice"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((b (String.to-bytes (if (= (% (E.next) 2) 0) "abc" "wxyz"))))
                  (let ((i (% (E.next) (Bytes.len b))))
                    (match (Bytes.at b i)
                      ((Some v) (+ (* 10 v) (- (E.probe) n)))
                      (None -1))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 992 Int64))
  (call   main (: 3 Int64)) (output (: 1192 Int64))
  (call   main (: 0 Int64)) (output (: 982 Int64)))

(case "pf1 an AGGREGATOR arm — the op returns the running sum of every value fed to it, three feeds pin the accumulation"
  (input  (do
            (effect E (op feed (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E 0
                ((feed (x) s (resume (+ s x) (+ s x))))
                (let ((r1 (E.feed n)))
                  (let ((r2 (E.feed 7)))
                    (let ((r3 (E.feed n)))
                      (+ (* 100 r1) (+ (* 10 r2) r3)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 413 Int64))
  (call   main (: 0 Int64)) (output (: 77 Int64))
  (call   main (: -2 Int64)) (output (: -147 Int64)))

(case "mm1 a MIN/MAX tracking arm — three feeds tighten the (lo,hi) tuple state, readers project the final spread"
  (input  (do
            (effect E (op feed (-> Int64 Int64)) (op lo (-> Int64)) (op hi (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple 1000 -1000)
                ((feed (x) s (match s
                               ((tuple l h) (resume x (tuple (if (< x l) x l)
                                                             (if (> x h) x h))))))
                 (lo () s (match s ((tuple l h) (resume l s))))
                 (hi () s (match s ((tuple l h) (resume h s)))))
                (do (E.feed n)
                    (E.feed 7)
                    (E.feed (- 0 n))
                    (+ (* 100 (E.lo)) (E.hi)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: -293 Int64))
  (call   main (: 0 Int64)) (output (: 7 Int64))
  (call   main (: -9 Int64)) (output (: -891 Int64)))

(case "cl1 a NARROWING clamp arm — the [lo,hi] window shrinks by one each side per dispatch, three args meet three windows"
  (input  (do
            (effect E (op clamp (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E (tuple 0 10)
                ((clamp (x) s (match s
                                ((tuple l h)
                                  (resume (if (< x l) l (if (> x h) h x))
                                          (tuple (+ l 1) (- h 1)))))))
                (let ((a (E.clamp n)))
                  (let ((b (E.clamp 5)))
                    (let ((c (E.clamp (+ n 3))))
                      (+ (* 100 a) (+ (* 10 b) c)))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 758 Int64))
  (call   main (: -4 Int64)) (output (: 52 Int64))
  (call   main (: 12 Int64)) (output (: 1058 Int64)))

(case "cx1 a COUNTDOWN arm exhausts mid-sequence — positive states pass through, the floor returns a sentinel-free constant"
  (input  (do
            (effect E (op take (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((take () s (resume (if (> s 0) s 999) (- s 2))))
                (+ (E.take) (+ (E.take) (+ (E.take) (E.take))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1008 Int64))
  (call   main (: 3 Int64)) (output (: 2002 Int64))
  (call   main (: 1 Int64)) (output (: 2998 Int64)))

;; ── md/gc/pw/dg/cs + ne/rs/abf: arithmetic-machine arms and sum op-inputs ────
;; md1 a MODULAR ring state (stride-3 walk, seed reduced mod 7 at install);
;; gc1 a EUCLID-step arm (one gcd step per dispatch, b=0 fixpoint); pw1 a
;; TRIPLING state crossing fixed thresholds; dg1 a DIGIT-extractor arm (peel
;; low digit, floor by ten); cs1 a COLLATZ-step arm (even halves, odd
;; triples-plus-one). ne1 an OPTION built from a draw crosses as an op
;; ARGUMENT (arm matches it against live state); rs1 the RESULT mirror
;; (Ok scales, Err negates); abf1 ONE handler mixes a resumptive and an
;; abortive op (two marks advance, the abort carries their mix out).

(case "md1 a MODULAR ring state — the thread walks a size-7 ring with stride 3, entry point reduced mod 7 at the seed"
  (input  (do
            (effect E (op step (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 7)
                ((step () s (resume s (% (+ s 3) 7))))
                (+ (* 100 (E.step)) (+ (* 10 (E.step)) (E.step)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 251 Int64))
  (call   main (: 6 Int64)) (output (: 625 Int64))
  (call   main (: 13 Int64)) (output (: 625 Int64)))

(case "gc1 a EUCLID-step arm — each dispatch advances (a,b) one gcd step, low digits of the descent spell the trace"
  (input  (do
            (effect E (op step (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n 18)
                ((step () s (match s
                              ((tuple a b)
                                (resume a (if (= b 0) (tuple a b) (tuple b (% a b))))))))
                (let ((d1 (E.step)))
                  (let ((d2 (E.step)))
                    (let ((d3 (E.step)))
                      (let ((d4 (E.step)))
                        (+ (* 1000 (% d1 10))
                           (+ (* 100 (% d2 10))
                              (+ (* 10 (% d3 10)) (% d4 10))))))))))
            (export main)))
  (call   main (: 48 Int64)) (output (: 8826 Int64))
  (call   main (: 21 Int64)) (output (: 1833 Int64)))

(case "pw1 a TRIPLING state crosses fixed thresholds — three compares catch the crossing at input-dependent depth"
  (input  (do
            (effect E (op over (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E (if (> n 0) n 1)
                ((over (th) s (resume (if (> s th) 1 0) (* s 3))))
                (+ (* 100 (E.over 4)) (+ (* 10 (E.over 40)) (E.over 400)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 100 Int64))
  (call   main (: 20 Int64)) (output (: 110 Int64))
  (call   main (: 150 Int64)) (output (: 111 Int64)))

(case "dg1 a DIGIT-extractor arm — each dispatch peels the low digit and floors the state by ten, three peels reverse the tail"
  (input  (do
            (effect E (op peel (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((peel () s (resume (% s 10) (/ s 10))))
                (+ (* 100 (E.peel)) (+ (* 10 (E.peel)) (E.peel)))))
            (export main)))
  (call   main (: 4728 Int64)) (output (: 827 Int64))
  (call   main (: 56 Int64)) (output (: 650 Int64))
  (call   main (: 900 Int64)) (output (: 9 Int64)))

(case "cs1 a COLLATZ-step arm — even states halve, odd states triple-plus-one, low digits of four reads trace the orbit"
  (input  (do
            (effect E (op step (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((step () s (resume s (if (= (% s 2) 0) (/ s 2) (+ (* 3 s) 1)))))
                (let ((d1 (E.step)))
                  (let ((d2 (E.step)))
                    (let ((d3 (E.step)))
                      (let ((d4 (E.step)))
                        (+ (* 1000 (% d1 10))
                           (+ (* 100 (% d2 10))
                              (+ (* 10 (% d3 10)) (% d4 10))))))))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 6305 Int64))
  (call   main (: 7 Int64)) (output (: 7214 Int64))
  (call   main (: 5 Int64)) (output (: 5684 Int64)))

(case "ne1 an OPTION built from a draw crosses dispatch as an op ARGUMENT — the arm matches it against the live state"
  (input  (do
            (effect E (op next (-> Int64)) (op score (-> (Option Int64) Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (score (o) s (resume (match o
                                        ((Some v) (+ (* 10 v) s))
                                        ((None) s))
                                      (+ s 1)))
                 (probe () s (resume s s)))
                (let ((d (E.next)))
                  (+ (* 10 (E.score (if (> d 0) (Some d) (None))))
                     (- (E.probe) n)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 342 Int64))
  (call   main (: 0 Int64)) (output (: 12 Int64))
  (call   main (: -4 Int64)) (output (: -28 Int64)))

(case "rs1 a RESULT built from draw parity crosses dispatch as an op ARGUMENT — Ok scales with state, Err folds in negated"
  (input  (do
            (type Res (Ok Int64) (Err Int64))
            (effect E (op next (-> Int64)) (op judge (-> Res Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (judge (r) s (resume (match r
                                        ((Res.Ok v) (+ (* 100 v) s))
                                        ((Res.Err v) (- (- 0 v) s)))
                                      (+ s 1)))
                 (probe () s (resume s s)))
                (let ((d (E.next)))
                  (+ (* 10 (E.judge (if (= (% d 2) 0) (Res.Ok d) (Res.Err d))))
                     (- (E.probe) n)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 4052 Int64))
  (call   main (: 3 Int64)) (output (: -68 Int64))
  (call   main (: -2 Int64)) (output (: -2008 Int64)))

(case "abf1 one handler mixes a RESUMPTIVE op and an ABORTIVE op — two marks advance the state, the abort carries their mix out"
  (input  (do
            (effect Bail (op mark (-> Int64)) (op out (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Bail n
                ((mark () t (resume t (+ t 5)))
                 (out (v) t (+ 1000 v)))
                (let ((m1 (Bail.mark)))
                  (let ((m2 (Bail.mark)))
                    (+ (Bail.out (+ (* 10 m1) m2)) 777)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1038 Int64))
  (call   main (: 0 Int64)) (output (: 1005 Int64))
  (call   main (: -4 Int64)) (output (: 961 Int64)))

;; ── ta3a/ss2/lp1/ix1/tw1/ch4/hof1/lam1/ag2: composition faces ────────────────
;; ta3a a THREE-argument op (arm folds all positions with the state); ss2 a
;; STRING state grows by a parity-picked suffix; lp1 a LIST state (push
;; returns pre-push length, a recursive walk sums the final list from a
;; reader op); ix1 the op argument indexes a list held in state (in-range
;; projects, out-of-range hits the arm fallback); tw1 arms of TWO effects
;; share one pure helper; ch4 the SAME op chained into itself; hof1 a
;; higher-order apply-twice over a draw; lam1 inline and let-bound lambdas
;; applied to one draw (pure closures, drawn args); ag2 TWO aggregator
;; effects with same-NAMED ops (qualified dispatch disambiguates).

(case "ta3a a THREE-argument op — the arm folds all three positions with the live state, two calls see it advance"
  (input  (do
            (effect E (op mix3 (-> Int64 Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((mix3 (a b c) s (resume (+ (* 100 a) (+ (* 10 b) (+ c s))) (+ s 1))))
                (+ (E.mix3 1 2 3) (E.mix3 4 5 6))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 580 Int64))
  (call   main (: 5 Int64)) (output (: 590 Int64))
  (call   main (: -3 Int64)) (output (: 574 Int64)))

(case "ss2 a STRING state GROWS by a parity-picked suffix per dispatch — byte-len pins the concatenation history"
  (input  (do
            (effect E (op grow (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple "" n)
                ((grow () s (match s
                              ((tuple acc k)
                                (resume (String.byte-len acc)
                                        (tuple (String.concat acc (if (= (% k 2) 0) "ab" "xyz"))
                                               (+ k 1)))))))
                (do (E.grow) (E.grow) (E.grow)
                    (+ (* 10 (E.grow)) 3))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 73 Int64))
  (call   main (: 3 Int64)) (output (: 83 Int64))
  (call   main (: 1 Int64)) (output (: 83 Int64)))

(case "lp1 a LIST handler state — each feed pushes and returns the pre-push length, a summing walk reads the final list"
  (input  (do
            (effect E (op push (-> Int64 Int64)) (op total (-> Int64)))
            (def (sum-list (: xs (List Int64)) (: i Int64) (: acc Int64))
              (match (List.at xs i)
                ((Some v) (sum-list xs (+ i 1) (+ acc v)))
                ((None) acc)))
            (def (main (: n Int64))
              (handle E (list)
                ((push (x) s (resume (List.len s) (List.push s x)))
                 (total () s (resume (sum-list s 0 0) s)))
                (let ((a (E.push n)))
                  (let ((b (E.push (* 2 n))))
                    (let ((c (E.push 7)))
                      (+ (* 1000 (E.total))
                         (+ (* 100 a) (+ (* 10 b) c))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 16012 Int64))
  (call   main (: 0 Int64)) (output (: 7012 Int64))
  (call   main (: -2 Int64)) (output (: 1012 Int64)))

(case "ix1 the op argument INDEXES a list held in a two-slot state — in-range reads project, out-of-range yields the arm's fallback"
  (input  (do
            (effect E (op at (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E (tuple (list 10 20 30 40) n)
                ((at (u) s (match s
                             ((tuple xs k)
                               (resume (match (List.at xs (% k 6))
                                         ((Some v) v)
                                         ((None) -1))
                                       (tuple xs (+ k 2)))))))
                (+ (* 100 (E.at 0)) (E.at 0))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 2040 Int64))
  (call   main (: 4 Int64)) (output (: -90 Int64))
  (call   main (: 3 Int64)) (output (: 3999 Int64)))

(case "tw1 arms of TWO different effects call one shared PURE helper — square-plus-one applied to each live state"
  (input  (do
            (effect P (op sq (-> Int64)))
            (effect Q (op sq (-> Int64)))
            (def (sq1 (: x Int64)) (+ (* x x) 1))
            (def (main (: n Int64))
              (handle P n
                ((sq () s (resume (sq1 s) (+ s 1))))
                (handle Q 100
                  ((sq () t (resume (sq1 t) (+ t 10))))
                  (+ (P.sq) (+ (Q.sq) (P.sq))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 10016 Int64))
  (call   main (: 0 Int64)) (output (: 10004 Int64))
  (call   main (: -3 Int64)) (output (: 10016 Int64)))

(case "ch4 the SAME op chained into itself — E.b of E.b of E.a, the middle hop's argument is already a hop"
  (input  (do
            (effect E (op a (-> Int64)) (op b (-> Int64 Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((a () s (resume s (+ s 2)))
                 (b (x) s (resume (+ x s) (+ s 3)))
                 (probe () s (resume s s)))
                (+ (* 10 (E.b (E.b (E.a)))) (- (E.probe) n))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 138 Int64))
  (call   main (: 0 Int64)) (output (: 78 Int64))
  (call   main (: -5 Int64)) (output (: -72 Int64)))

(case "hof1 a higher-order APPLY-TWICE over a draw — the fn value crosses the call while the thread advances underneath"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (dbl1 (: x Int64)) (+ (* 2 x) 1))
            (def (twice (: f (-> Int64 Int64)) (: x Int64)) (f (f x)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (+ (* 10 (twice dbl1 (E.next))) (- (E.probe) n))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 151 Int64))
  (call   main (: 0 Int64)) (output (: 31 Int64))
  (call   main (: -4 Int64)) (output (: -129 Int64)))

(case "lam1 INLINE and LET-BOUND lambdas applied to one draw — both closure forms read the same drawn value"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((d (E.next)))
                  (let ((f (fn ((: y Int64)) (* 2 (+ y 10)))))
                    (+ (* 100 ((fn ((: x Int64)) (- (* 3 x) 1)) d))
                       (+ (* 10 (f d)) (- (E.probe) n)))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 741 Int64))
  (call   main (: 0 Int64)) (output (: 101 Int64))
  (call   main (: -3 Int64)) (output (: -859 Int64)))

(case "ag2 TWO aggregator effects interleaved — each keeps its own running sum, weighted reads pin the four-feed order"
  (input  (do
            (effect P (op feed (-> Int64 Int64)))
            (effect Q (op feed (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle P 0
                ((feed (x) s (resume (+ s x) (+ s x))))
                (handle Q 0
                  ((feed (x) t (resume (+ t x) (+ t x))))
                  (let ((r1 (P.feed n)))
                    (let ((r2 (Q.feed 10)))
                      (let ((r3 (P.feed n)))
                        (let ((r4 (Q.feed 10)))
                          (+ r1 (+ (* 2 r2) (+ (* 3 r3) (* 4 r4)))))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 121 Int64))
  (call   main (: 0 Int64)) (output (: 100 Int64))
  (call   main (: -5 Int64)) (output (: 65 Int64)))

;; ── dl/np/ho2/pm/an/do/eq/ct: binder, closure, and syntax faces ──────────────
;; dl1 TEN chained lets each drawing once (weights 1..10 pin every binder);
;; np1 a NESTED record value from one draw (double projections); ho2 a PURE
;; factory closure applied to a draw (captured constant pure — the safe
;; mirror of the performing-init zone); pm1 LITERAL match arms grade the
;; state inside the arm; an1 explicitly ASCRIBED draws are observationally
;; transparent; do1 ONE draw consumed by both if-branches (read-only binder
;; reuse); eq1 structural EQUALITY of draw-built records (both verdicts
;; reachable); ct1 the LOOP BOUND is itself a draw.

(case "dl1 TEN chained lets each drawing once — position weights 1..10 pin every binder to its draw"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((v1 (E.next)))
                  (let ((v2 (E.next)))
                    (let ((v3 (E.next)))
                      (let ((v4 (E.next)))
                        (let ((v5 (E.next)))
                          (let ((v6 (E.next)))
                            (let ((v7 (E.next)))
                              (let ((v8 (E.next)))
                                (let ((v9 (E.next)))
                                  (let ((v10 (E.next)))
                                    (+ (* 10 (+ (* 1 v1) (+ (* 2 v2) (+ (* 3 v3) (+ (* 4 v4) (+ (* 5 v5) (+ (* 6 v6) (+ (* 7 v7) (+ (* 8 v8) (+ (* 9 v9) (* 10 v10)))))))))))
                                       (- (E.probe) n))))))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6060 Int64))
  (call   main (: 0 Int64)) (output (: 3310 Int64))
  (call   main (: -7 Int64)) (output (: -540 Int64)))

(case "np1 a NESTED record value built from one draw — projections through two levels read the same drawn base"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((d (E.next)))
                  (let ((r (record (= a d) (= b (record (= x (* 2 d)) (= y (+ d 5)))))))
                    (+ (* 100 (. r a))
                       (+ (* 10 (. (. r b) x))
                          (+ (. (. r b) y)
                             (* 1000 (- (E.probe) n)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1368 Int64))
  (call   main (: 0 Int64)) (output (: 1005 Int64))
  (call   main (: -4 Int64)) (output (: 521 Int64)))

(case "ho2 a PURE factory returns a closure applied to a draw — the captured constant is pure, only the argument is drawn"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (mk (: k Int64)) (fn ((: x Int64)) (+ (* x k) 1)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((f (mk 7)))
                  (+ (* 10 (f (E.next))) (- (E.probe) n)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 221 Int64))
  (call   main (: 0 Int64)) (output (: 11 Int64))
  (call   main (: -2 Int64)) (output (: -129 Int64)))

(case "pm1 LITERAL match arms grade the state INSIDE the handler arm — a mod-4 walker crosses all four literal rows"
  (input  (do
            (effect E (op tag (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 4)
                ((tag () s (resume (match s
                                     (0 70)
                                     (1 81)
                                     (2 92)
                                     (_ 63))
                                   (% (+ s 1) 4))))
                (+ (E.tag) (+ (* 10 (E.tag)) (* 100 (E.tag))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 10080 Int64))
  (call   main (: 2 Int64)) (output (: 7722 Int64))
  (call   main (: 3 Int64)) (output (: 8863 Int64)))

(case "an1 explicitly ASCRIBED draws — (: (E.next) Int64) in let and argument positions changes nothing observable"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((a (: (E.next) Int64)))
                  (+ (* 10 (+ a (: (E.next) Int64))) (- (E.probe) n)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 92 Int64))
  (call   main (: 0 Int64)) (output (: 12 Int64))
  (call   main (: -3 Int64)) (output (: -48 Int64)))

(case "do1 ONE draw consumed by BOTH branches of an if with different weights — the binder is read exactly once per run"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((d (E.next)))
                  (+ (if (> d 2) (+ (* 100 d) 7) (- (* 10 d) 7))
                     (* 1000 (- (E.probe) n))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1507 Int64))
  (call   main (: 1 Int64)) (output (: 1003 Int64))
  (call   main (: -6 Int64)) (output (: 933 Int64)))

(case "eq1 structural EQUALITY of records built from two draws — a parity-dependent stride decides whether the fields line up"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (if (= (% s 2) 0) (+ s 2) (+ s 1))))
                 (probe () s (resume s s)))
                (let ((a (E.next)))
                  (let ((b (E.next)))
                    (+ (if (= (record (= x a) (= y (+ a 1))) (record (= x a) (= y b))) 100 200)
                       (- (E.probe) n))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 204 Int64))
  (call   main (: 3 Int64)) (output (: 103 Int64))
  (call   main (: 7 Int64)) (output (: 103 Int64)))

(case "ct1 the LOOP BOUND is itself a draw — a first dispatch sizes the walk, the walk then draws that many times"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (walk (: k Int64) (: acc Int64))
              (if (<= k 0) acc (walk (- k 1) (+ acc (E.next)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (+ (* 100 (walk (+ (% (E.next) 4) 1) 0))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 1204 Int64))
  (call   main (: 0 Int64)) (output (: 102 Int64))
  (call   main (: 7 Int64)) (output (: 3805 Int64)))

;; ── abmin/oa: conditional foreign-abort HOMING (findings #11 and #11-B) ──────
;; Pins the FIXED behavior of the abort-homing family: a foreign abort under a
;; conditional must propagate past inner abort-only handles to ITS OWN
;; handler. abmin4 the minimal if+inner-handle face; abmin2 the effectful-let
;; face (the draw is the conditional's scrutinee, a resumptive E frame in
;; play); ab8 a draw ROUTES between two nested abort handlers (else-if chain);
;; abmin10 TWO foreign frames — the abort crosses both; abmin12 both branches
;; abort, one foreign one local; abmin9-abr the RESUMPTIVE contrast (with a
;; resuming arm the value-return semantics are correct — 900307 here is the
;; right answer). oa1 an Option-returning op's None arm aborts LOCALLY (the
;; safe zone beside the #11-B def-boundary decline).

(case "abmin4 if WITHOUT let"
  (input  (do
            (effect A (op out (-> Int64 Int64)))
            (effect B (op bout (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A 0
                ((out (v) t (+ 9000 v)))
                (+ (* 100 (handle B 0
                            ((bout (v) t (+ 500 v)))
                            (if (= (% n 3) 0) (A.out n) n)))
                   7)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 9003 Int64)))

(case "abmin2 outer-abort under a LET+IF inside the unrelated inner handle"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect A (op out (-> Int64 Int64)))
            (effect B (op bout (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (handle A 0
                  ((out (v) t (+ 9000 v)))
                  (+ (* 100 (handle B 0
                              ((bout (v) t (+ 500 v)))
                              (let ((d (E.next)))
                                (if (= (% d 3) 0) (A.out d) d))))
                     (- (E.next) n)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 9003 Int64)))

(case "ab8 a draw picks WHICH of two nested abort handlers fires — outer-abort skips the inner arm's scale and the tail draw"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect Ob (op out (-> Int64 Int64)))
            (effect Ib (op out (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (handle Ob 0
                  ((out (v) t (+ 9000 v)))
                  (+ (* 100 (handle Ib 0
                              ((out (v) t (+ 500 v)))
                              (let ((d (E.next)))
                                (if (= (% d 3) 0)
                                    (Ob.out d)
                                    (if (= (% d 3) 1) (Ib.out d) d)))))
                     (- (E.next) n)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 9003 Int64))
  (call   main (: 1 Int64)) (output (: 50101 Int64))
  (call   main (: 2 Int64)) (output (: 201 Int64))
  (call   main (: -4 Int64)) (output (: -399 Int64)))

(case "abmin10 TWO nested foreign abort-only handles under the conditional abort"
  (input  (do
            (effect A (op out (-> Int64 Int64)))
            (effect B (op bout (-> Int64 Int64)))
            (effect C (op cout (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A 0
                ((out (v) t (+ 9000 v)))
                (+ (* 100 (handle B 0
                            ((bout (v) t (+ 500 v)))
                            (+ (* 10 (handle C 0
                                       ((cout (v) t (+ 70 v)))
                                       (if (> n 0) (A.out n) n)))
                               3)))
                   7)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 9003 Int64)))

(case "abmin12 BOTH branches abort (to different handlers) — no value path remains"
  (input  (do
            (effect A (op out (-> Int64 Int64)))
            (effect B (op bout (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A 0
                ((out (v) t (+ 9000 v)))
                (+ (* 100 (handle B 0
                            ((bout (v) t (+ 500 v)))
                            (if (> n 0) (A.out n) (B.bout n))))
                   7)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 9003 Int64))
  (call   main (: -2 Int64)) (output (: 49807 Int64)))

(case "abmin9 the RESUMPTIVE flip of abmin4 — A's arm resumes, so 900307 IS the correct answer here"
  (input  (do
            (effect A (op out (-> Int64 Int64)))
            (effect B (op bout (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle A 0
                ((out (v) t (resume (+ 9000 v) t)))
                (+ (* 100 (handle B 0
                            ((bout (v) t (+ 500 v)))
                            (if (> n 0) (A.out n) n)))
                   7)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 900307 Int64))
  (call   main (: -2 Int64)) (output (: -193 Int64)))

(case "oa1 an Option-returning op's None arm aborts LOCALLY — the local bail-out in a match arm homes correctly"
  (input  (do
            (effect E (op fetch (-> (Option Int64))) (op probe (-> Int64)))
            (effect Bail (op out (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((fetch () s (resume (if (= (% s 2) 0) (Some s) (None)) (+ s 3)))
                 (probe () s (resume s s)))
                (+ (* 100 (handle Bail 0
                            ((out (v) t (+ 500 v)))
                            (match (E.fetch)
                              ((Some v) (* 10 v))
                              ((None) (Bail.out 77)))))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 4003 Int64))
  (call   main (: 1 Int64)) (output (: 57703 Int64))
  (call   main (: -2 Int64)) (output (: -1997 Int64)))


; ── Post-recursion state threading (finding #12) + the handle-COMPOSITION position matrix ──
; Two families. First, the finding-#12 faces: a performing SELF-RECURSIVE callee (a walk that
; draws until a predicate hits) whose out-state must reach the continuation — before the fix
; (thread a let-wrapped recursive dispatch's out-state to a trailing observer) the state advances
; inside the callee were silently DISCARDED for the code after the call (the walk-then-observe,
; accumulator, and filtered-fold shapes below returned wrong values; the mutual-recursion floor
; still declines, pinned separately as the row-mr witness). Second, the position matrix: a WHOLE
; nested (handle …) expression is just an expression, so it may sit in ANY evaluation position of
; an enclosing handled region — as a SEED (si1/si2, incl. an aborting seed whose abort value
; becomes the outer init), as a seed built FROM a performing recursion (sr1), as an op ARGUMENT
; (op1, the argument region also performing the OUTER effect), inside a handler ARM's resume
; value (ar1), at a performing recursion's EXIT LEAF (rw1), as BOTH arguments of one 2-ary op
; (pa1, argument order pins the outer thread), as a THREE-link seed chain (sc1), and as an IF's
; CONDITION with the chosen branch reading condition-advanced state (if1). Each case's rows are
; hand-computed; all pass on wasm, rust, and rust-async.

(case "im2min5 a performing self-recursive walk then a TRAILING draw — the callee-advanced state reaches the continuation (finding #12 face)"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (walk (: k Int64))
              (let ((d (E.next)))
                (if (= (% d 7) 0) (* 100 d) (walk (+ k 1)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((w (walk 0)))
                  (+ w (E.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 708 Int64))
  (call   main (: 12 Int64)) (output (: 1415 Int64))
  (call   main (: 7 Int64)) (output (: 708 Int64)))

(case "rowacc5 the walk carries an ACCUMULATOR — the post-recursion read still sees the callee-advanced state (finding #12 accumulator face)"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (walk (: k Int64))
              (let ((d (E.next)))
                (if (= (% d 7) 0) (+ (* 100 d) (* 10 k)) (walk (+ k 1)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (walk 0) (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 728 Int64)))

(case "sk1 a FILTERED fold — only even draws accumulate but every draw advances the thread, kept-count and span both pinned"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (fold (: k Int64) (: acc Int64) (: kept Int64))
              (if (<= k 0)
                  (+ (* 100 acc) (* 10 kept))
                  (let ((d (E.next)))
                    (if (= (% d 2) 0)
                        (fold (- k 1) (+ acc d) (+ kept 1))
                        (fold (- k 1) acc kept)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (+ (fold 4 0 0) (- (E.probe) n))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 624 Int64))
  (call   main (: 1 Int64)) (output (: 624 Int64))
  (call   main (: -4 Int64)) (output (: -576 Int64)))

(case "si1 a WHOLE inner handle expression as an outer handler's SEED"
  (input  (do
            (effect A (op tick (-> Unit Int64)))
            (effect B (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle B
                (handle A n ((tick (u) s (resume s (+ s 3))))
                  (+ (A.tick) (* 10 (A.tick))))
                ((get (u) t (resume t (+ t 1))))
                (+ (B.get) (* 100 (B.get)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 8685 Int64))
  (call   main (: 0 Int64)) (output (: 3130 Int64))
  (call   main (: -4 Int64)) (output (: -1314 Int64)))

(case "si2 the SEED handle ABORTS — the outer state is the abort value, the seed body's tail never runs"
  (input  (do
            (effect A (op tick (-> Unit Int64)))
            (effect B (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle B
                (handle A n ((tick (u) s (+ s 100))) (do (A.tick) 999))
                ((get (u) t (resume t (+ t 1))))
                (+ (B.get) (* 100 (B.get)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 10705 Int64))
  (call   main (: 0 Int64)) (output (: 10200 Int64))
  (call   main (: -4 Int64)) (output (: 9796 Int64)))

(case "sr1 a performing self-recursive walk's result SEEDS an inner handle, then a trailing outer draw"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect B (op get (-> Unit Int64)))
            (def (walk (: k Int64))
              (let ((d (E.next)))
                (if (= (% d 7) 0) (* 100 d) (walk (+ k 1)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (handle B (walk 0)
                     ((get (u) t (resume t (+ t 1))))
                     (+ (B.get) (* 100 (B.get))))
                   (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 70808 Int64))
  (call   main (: 12 Int64)) (output (: 141515 Int64))
  (call   main (: 0 Int64)) (output (: 101 Int64))
  (call   main (: -13 Int64)) (output (: -70606 Int64)))

(case "op1 a WHOLE nested handle expression as an op's ARGUMENT beside an outer draw"
  (input  (do
            (effect E (op next (-> Int64)) (op put (-> Int64 Int64)))
            (effect B (op g (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (put (v) s (resume (+ v s) s)))
                (E.put
                  (handle B 100
                    ((g (u) t (resume t (+ t 5))))
                    (+ (B.g) (+ (B.g) (E.next)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 216 Int64))
  (call   main (: 0 Int64)) (output (: 206 Int64))
  (call   main (: -10 Int64)) (output (: 186 Int64)))

(case "ar1 a WHOLE nested handle expression inside a handler ARM's resume value"
  (input  (do
            (effect E (op boost (-> Int64 Int64)))
            (effect B (op g (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E n
                ((boost (v) s
                  (resume
                    (handle B (+ s v)
                      ((g (u) t (resume t (+ t 3))))
                      (+ (B.g) (B.g)))
                    (+ s 1))))
                (+ (E.boost 10) (E.boost 20))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 88 Int64))
  (call   main (: 0 Int64)) (output (: 68 Int64))
  (call   main (: -17 Int64)) (output (: 0 Int64)))

(case "rw1 a nested handle at a performing RECURSION's exit leaf, then a trailing outer draw"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect B (op g (-> Unit Int64)))
            (def (walk (: k Int64))
              (let ((d (E.next)))
                (if (= (% d 7) 0)
                    (handle B d
                      ((g (u) t (resume t (+ t 1))))
                      (+ (B.g) (* 10 (B.g))))
                    (walk (+ k 1)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (walk 0) (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 95 Int64))
  (call   main (: 0 Int64)) (output (: 11 Int64))
  (call   main (: -13 Int64)) (output (: -73 Int64)))

(case "pa1 two SIBLING nested handles as the two arguments of one 2-ary op — each region draws the outer thread in arg order"
  (input  (do
            (effect E (op next (-> Int64)) (op pair (-> Int64 Int64 Int64)))
            (effect B (op g (-> Unit Int64)))
            (effect C (op h (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (pair (a b) s (resume (+ a (* 100 b)) s)))
                (E.pair
                  (handle B 10 ((g (u) t (resume t t))) (+ (B.g) (E.next)))
                  (handle C 20 ((h (u) t (resume t t))) (+ (C.h) (E.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2615 Int64))
  (call   main (: 0 Int64)) (output (: 2110 Int64))
  (call   main (: -30 Int64)) (output (: -920 Int64)))

(case "sc1 a THREE-link seed CHAIN of DISTINCT effects — each handle's whole region value seeds the next"
  (input  (do
            (effect A (op tick (-> Unit Int64)))
            (effect B (op get (-> Unit Int64)))
            (effect C (op h (-> Unit Int64)))
            (def (main (: n Int64))
              (handle C
                (handle B
                  (handle A n ((tick (u) s (resume s (+ s 2))))
                    (+ (A.tick) (* 10 (A.tick))))
                  ((get (u) t (resume t (+ t 1))))
                  (+ (B.get) (* 10 (B.get))))
                ((h (u) w (resume w (+ w 5))))
                (+ (C.h) (C.h))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1675 Int64))
  (call   main (: 0 Int64)) (output (: 465 Int64))
  (call   main (: -6 Int64)) (output (: -987 Int64)))

(case "if1 a WHOLE nested handle expression as an IF's CONDITION beside outer draws"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect B (op g (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (if (> (handle B 0 ((g (u) t (resume t t))) (+ (B.g) (E.next))) 0)
                    (+ 100 (E.next))
                    (- (E.next) 100))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 106 Int64))
  (call   main (: 0 Int64)) (output (: -99 Int64))
  (call   main (: -7 Int64)) (output (: -106 Int64)))


; ── Stateful-protocol arms, draw-driven control flow, and binder/scrutinee faces (breaker riders) ──
; A pooled batch of independent faces. Protocol/aggregate arms: an explicit feedback loop where each
; result re-enters as the next op argument (tw2); a bitmask cycle detector that stops on the first
; repeated residue (cy1); a begin/end protocol flag rejecting double-begin and end-without-begin
; (pk1); a running (sum,count) average with a truncating divide incl. a negative total (rn1); a
; put/get store whose put returns the displaced value (pg1). Draw-driven control flow: sign
; trichotomy routing three rows three ways (tri1); an if condition computed from LET-bound draws
; (ov1). Cross-thread and binder faces: an inner arm's resume value drawing from TWO different
; outer effects (wc1); an inner let SHADOWING a draw binder without disturbing the original (shl1);
; and a WHOLE nested handle as a MATCH's scrutinee, the region building an Option whose chosen arm
; draws again — incl. the negative-operand truncated-mod parity face (ms1). All rows hand-computed;
; all pass on wasm, rust, and rust-async.

(case "tw2 an explicit FEEDBACK loop — each call feeds the previous result back as the op's first argument beside a fresh draw"
  (input  (do
            (effect E (op mix (-> Int64 Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((mix (r) s (resume (* 2 (+ r s)) (+ s 2)))
                 (probe () s (resume s s)))
                (+ (* 10 (E.mix (E.mix (E.mix 1))))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 666 Int64))
  (call   main (: 0 Int64)) (output (: 246 Int64))
  (call   main (: -5 Int64)) (output (: -454 Int64)))

(case "cy1 a CYCLE detector — a bitmask accumulator of seen residues stops the walk on the first repeat"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (walk (: seen Int64) (: steps Int64))
              (if (>= steps 6)
                  (+ (* 100 steps) seen)
                  (let ((d (% (E.next) 4)))
                    (if (= (& seen (<< 1 d)) 0)
                        (walk (| seen (<< 1 d)) (+ steps 1))
                        (+ (* 100 (+ steps 1)) seen)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 2))))
                (walk 0 0)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 305 Int64))
  (call   main (: 1 Int64)) (output (: 310 Int64))
  (call   main (: 2 Int64)) (output (: 305 Int64)))

(case "pk1 a BEGIN/END protocol arm — a Bool flag rejects double-begin and end-without-begin, the sequence encodes the violations"
  (input  (do
            (effect E (op begin (-> Int64)) (op end (-> Int64)))
            (def (main (: n Int64))
              (handle E false
                ((begin () open (if open (resume -1 open) (resume 1 true)))
                 (end () open (if open (resume 7 false) (resume -1 open))))
                (let ((r1 (E.begin)))
                  (let ((r2 (E.begin)))
                    (let ((r3 (E.end)))
                      (let ((r4 (E.end)))
                        (+ (* 1000 r1) (+ (* 100 r2) (+ (* 10 r3) r4)))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 969 Int64)))

(case "rn1 a RUNNING-AVERAGE state (sum,count) — three feeds then a truncating divide read, negative total exercises toward-zero"
  (input  (do
            (effect E (op feed (-> Int64 Int64)) (op avg (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple 0 0)
                ((feed (x) s (match s
                               ((tuple tot cnt) (resume x (tuple (+ tot x) (+ cnt 1))))))
                 (avg () s (match s
                             ((tuple tot cnt) (resume (+ (* 10 (/ tot cnt)) cnt) s)))))
                (do (E.feed n) (E.feed (+ n 6)) (E.feed 3)
                    (E.avg))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 53 Int64))
  (call   main (: 0 Int64)) (output (: 33 Int64))
  (call   main (: -9 Int64)) (output (: -27 Int64)))

(case "tri1 sign TRICHOTOMY of a draw — negative, zero-literal, and positive rows each route distinctly"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (+ (* 10 (match (E.next)
                           ((guard d (< d 0)) (- 100 d))
                           (0 555)
                           (d (+ 200 d))))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 2041 Int64))
  (call   main (: 0 Int64)) (output (: 5551 Int64))
  (call   main (: -6 Int64)) (output (: 1061 Int64)))

(case "ov1 an if CONDITION from LET-bound draws — two draws feed a comparison that routes the branch"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((a (E.next)))
                  (let ((b (E.next)))
                    (+ (if (> (+ a b) 5) 100 200)
                       (* 10 (- (E.probe) n)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 120 Int64))
  (call   main (: 0 Int64)) (output (: 220 Int64))
  (call   main (: 2 Int64)) (output (: 220 Int64)))

(case "pg1 a PUT/GET store — put returns the value it displaces, a counter tracks writes, get reads the survivor"
  (input  (do
            (effect E (op put (-> Int64 Int64)) (op get (-> Int64)) (op writes (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple 0 0)
                ((put (x) s (match s
                              ((tuple last ctr) (resume last (tuple x (+ ctr 1))))))
                 (get () s (match s ((tuple last ctr) (resume last s))))
                 (writes () s (match s ((tuple last ctr) (resume ctr s)))))
                (let ((r1 (E.put (* 10 n))))
                  (let ((r2 (E.put 7)))
                    (+ r1 (+ r2 (+ (* 100 (E.get)) (* 1000 (E.writes)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 2730 Int64))
  (call   main (: 0 Int64)) (output (: 2700 Int64))
  (call   main (: -2 Int64)) (output (: 2680 Int64)))

(case "wc1 the inner arm's resume value draws from TWO different outer effects — both threads advance per inner dispatch"
  (input  (do
            (effect P (op next (-> Int64)))
            (effect Q (op next (-> Int64)))
            (effect I (op ask (-> Int64)))
            (def (main (: n Int64))
              (handle P n
                ((next () s (resume s (+ s 1))))
                (handle Q 100
                  ((next () t (resume t (+ t 10))))
                  (handle I 0
                    ((ask () u (resume (+ (P.next) (Q.next)) u)))
                    (+ (I.ask) (* 1000 (I.ask)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 114103 Int64))
  (call   main (: 0 Int64)) (output (: 111100 Int64))
  (call   main (: -7 Int64)) (output (: 104093 Int64)))

(case "shl1 an inner let SHADOWS a draw binder — the shadow scales it locally, the original stays visible after the shadow closes"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((d (E.next)))
                  (+ (let ((d (* d 100))) d)
                     (+ d (* 10 (- (E.probe) n)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 313 Int64))
  (call   main (: 0 Int64)) (output (: 10 Int64))
  (call   main (: -2 Int64)) (output (: -192 Int64)))

(case "ms1 a WHOLE nested handle expression as a MATCH's SCRUTINEE — the region builds an Option, the chosen arm draws again"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect B (op g (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (match (handle B 0
                         ((g (u) t (resume t t)))
                         (let ((v (+ (B.g) (E.next))))
                           (if (= (% v 2) 0) (Some v) (None))))
                  ((Some x) (+ (* 10 x) (E.next)))
                  ((None) (- (E.next) 7)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 45 Int64))
  (call   main (: 3 Int64)) (output (: -3 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -5 Int64)) (output (: -11 Int64)))


; ── Handle-composition follow-ons, abort-value chains, and compound data through the thread ──
; Follow-ons to the batch-196 position matrix: two SEQUENTIAL handles of the SAME inner effect,
; each seeded by a fresh outer draw (sq1 — sibling re-instantiation, independent inner threads);
; a CONDITIONALLY-aborting handle inside a SEED whose condition draw comes from the outer thread
; (sab1 — the finding-#11 shape class exercised at the seed position, abort value or fall-through
; both becoming the outer init); a do-def binding a whole cross-effect handle region (dd5); and a
; draw seeding a mid-body handler install whose install expression itself performs (ex1). Abort
; data flow: the abort VALUE built by a three-op chain, every hop advancing the enclosing thread
; before the unwind (a2p1). Compound data through the thread: a Bool op argument computed from a
; draw's residue, the arm branching on the delivered flag — incl. the negative-residue face (hb1);
; a tuple-returning op whose destructured halves feed TWO different later ops (ts1); a def
; returning a tuple of two draws destructured by the caller (tb1); a NESTED tuple state rebuilt
; two levels deep per dispatch (nt1); and a Map with TUPLE values, draw-keyed inserts and
; destructuring lookups (ml1). All rows hand-computed; all pass on wasm, rust, and rust-async.

(case "sq1 two SEQUENTIAL handles of the SAME inner effect, each seeded by a fresh outer draw — independent inner threads, one advancing outer thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect B (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (handle B (E.next)
                     ((get (u) t (resume t (+ t 2))))
                     (+ (B.get) (B.get)))
                   (handle B (* 10 (E.next))
                     ((get (u) t (resume t (+ t 3))))
                     (+ (B.get) (B.get))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 135 Int64))
  (call   main (: 0 Int64)) (output (: 25 Int64))
  (call   main (: -3 Int64)) (output (: -41 Int64)))

(case "sab1 a CONDITIONALLY-aborting handle inside the SEED — the abort value or the fall-through both become the outer init"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect B (op get (-> Unit Int64)))
            (effect Bail (op out (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (handle B
                     (handle Bail 0
                       ((out (v) t (+ 100 v)))
                       (let ((d (E.next)))
                         (if (> d 0) (do (Bail.out d) 999) (* 5 d))))
                     ((get (u) t (resume t (+ t 1))))
                     (+ (B.get) (* 10 (B.get))))
                   (E.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1171 Int64))
  (call   main (: 0 Int64)) (output (: 11 Int64))
  (call   main (: -2 Int64)) (output (: -101 Int64)))

(case "hb1 a BOOL op argument computed by comparing a draw's residue — the arm branches on the delivered flag against live state"
  (input  (do
            (effect E (op next (-> Int64)) (op judge (-> Bool Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (judge (f) s (resume (if f (+ 100 s) (- 0 s)) (+ s 5))))
                (do (E.next)
                    (+ (E.judge (= (% (E.next) 3) 0)) (E.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 119 Int64))
  (call   main (: 2 Int64)) (output (: 113 Int64))
  (call   main (: 0 Int64)) (output (: 5 Int64))
  (call   main (: -4 Int64)) (output (: 101 Int64)))

(case "ts1 a tuple-returning op SPLIT-refed — each destructured half feeds a DIFFERENT later op against advancing state"
  (input  (do
            (effect E (op split (-> (Tuple Int64 Int64)))
                      (op mixa (-> Int64 Int64))
                      (op mixb (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((split () s (resume (tuple s (* 2 s)) (+ s 1)))
                 (mixa (a) s (resume (+ a s) (+ s 2)))
                 (mixb (b) s (resume (* b s) s)))
                (match (E.split)
                  ((tuple a b) (+ (E.mixa a) (E.mixb b))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 91 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -3 Int64)) (output (: -5 Int64)))

(case "a2p1 the abort VALUE is a three-op chain — every hop advances the enclosing thread before the region tears down"
  (input  (do
            (effect E (op a (-> Int64)) (op b (-> Int64 Int64)) (op c (-> Int64 Int64)) (op probe (-> Int64)))
            (effect Bail (op out (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((a () s (resume s (+ s 2)))
                 (b (x) s (resume (+ x s) (+ s 3)))
                 (c (x) s (resume (* 2 x) (+ s 5)))
                 (probe () s (resume s s)))
                (+ (* 10 (handle Bail 0
                           ((out (v) t (+ 1000 v)))
                           (+ (Bail.out (E.c (E.b (E.a)))) 777)))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 10130 Int64))
  (call   main (: 0 Int64)) (output (: 10050 Int64))
  (call   main (: -4 Int64)) (output (: 9890 Int64)))

(case "ml1 a Map whose VALUES are tuples — a draw-keyed insert of a drawn pair, lookups destructure both hit and neighbor"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (get2 (: m (Map Int64 (Tuple Int64 Int64))) (: k Int64))
              (match (Map.lookup m k)
                ((Some p) p)
                ((None) (tuple -1 -1))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 2))))
                (let ((k (+ (% (E.next) 3) 1)))
                  (let ((v1 (E.next)))
                    (let ((m (Map.insert (Map.insert (Map.insert (Map.insert (map) 1 (tuple 10 20)) 2 (tuple 30 40)) 3 (tuple 50 60)) k (tuple v1 (* 2 v1)))))
                      (match (get2 m k)
                        ((tuple a b)
                          (match (get2 m (if (= k 1) 2 1))
                            ((tuple c d) (+ a (+ b (+ c d))))))))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 42 Int64))
  (call   main (: 0 Int64)) (output (: 76 Int64))
  (call   main (: 4 Int64)) (output (: 48 Int64)))

(case "dd5 a do-def BINDS a whole cross-effect handle region — the region's seed draws from the outer thread"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect Q (op get (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (do (def r (handle Q (E.next)
                             ((get () t (resume t t)))
                             (Q.get)))
                    (+ (* 100 r) (E.next)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 304 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -4 Int64)) (output (: -403 Int64)))

(case "tb1 a def RETURNS a tuple of two draws — the caller destructures the multi-value result of a performing helper"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (pair2)
              (tuple (E.next) (E.next)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 3)))
                 (probe () s (resume s s)))
                (match (pair2)
                  ((tuple a b) (+ (* 100 a) (+ (* 10 b) (- (E.probe) n)))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 256 Int64))
  (call   main (: 0 Int64)) (output (: 36 Int64))
  (call   main (: -4 Int64)) (output (: -404 Int64)))

(case "nt1 a NESTED tuple state (a (b c)) — the arm destructures two levels and rebuilds with three different strides"
  (input  (do
            (effect E (op sum (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n (tuple 100 7000))
                ((sum () s (match s
                             ((tuple a inner)
                               (match inner
                                 ((tuple b c)
                                   (resume (+ a (+ b c))
                                           (tuple (+ a 1) (tuple (+ b 10) (+ c 700))))))))))
                (+ (E.sum) (* 10 (E.sum)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 85243 Int64))
  (call   main (: 0 Int64)) (output (: 85210 Int64))
  (call   main (: -6 Int64)) (output (: 85144 Int64)))

(case "ex1 a draw SEEDS a mid-body handler install for a different effect — the install expression itself performs"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (effect Q (op get (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (+ (handle Q (* 100 (E.next))
                     ((get () t (resume t t)))
                     (Q.get))
                   (* 10 (- (E.probe) n)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 310 Int64))
  (call   main (: 0 Int64)) (output (: 10 Int64))
  (call   main (: -2 Int64)) (output (: -190 Int64)))


; ── Two-site-arm radius, float/quantity threads, and record cross-arm state (breaker riders) ──
; Pooled pins. Two-site-arm boundary faces: a guarded match on a perform result inside a two-site
; arm's served branch (gr2); a two-site arm beside a state-REPLACING second op over scalar state
; (rp1); a perform under NOT in a condition — the negated dispatch gate (nb1). Recursion/handled-
; region shapes: mutual recursion carrying a (List Int64) accumulator, the scalar-element control
; for the mutual floor (mx1); an outer handle of an effect the inner region fully discharges —
; idempotent double-handling, the outer never fires (id1). Non-integer threads: a Float64 handler
; state through a two-site arm with a magnitude threshold (fl1); a TRIPLING Float64 thread crossing
; a fixed threshold at input-dependent depth (fr1); two perform-drawn quantities of one dimension
; adding — same-unit combine over two dispatches (qs1). Shared-state protocols: readers and a SPIN
; writer over one tuple state (tp4). And the record twin of the collection cross-arm shape: a
; RECORD state updated by one arm and field-read by siblings — full field types at the seed, so
; the empty-collection element-var gap does not apply (rr1, green control for that filed issue).
; All rows hand-computed; all pass on wasm, rust, and rust-async.

(case "gr2 guarded match on a perform result INSIDE a two-site arm's served branch"
  (input  (do
            (effect St (op sift (-> Int64 Int64)) (op roll (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
                 (roll (u) s (resume s (+ s 3))))
                (match (St.roll)
                  ((guard v (> v 6)) (* v 100))
                  (v (+ (St.sift 20) (+ (St.sift 3) v))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 25 Int64)))

(case "rp1 two-site arm + a state-REPLACING second op over a SCALAR state"
  (input  (do
            (effect St (op sift (-> Int64 Int64)) (op reset (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((sift (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s)))
                 (reset (u) s (resume s 100)))
                (+ (St.sift 20) (+ (St.reset) (St.sift 30)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 51 Int64)))

(case "nb1 a perform under NOT in a condition (the negated dispatch gate)"
  (input  (do
            (effect St (op check (-> Unit Int64)) (op count (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((check (u) s (resume s (+ s 1)))
                 (count (u) s (resume s s)))
                (if (not (> (St.check) 3))
                  (* 100 (St.count))
                  (St.count))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 200 Int64)))

(case "mx1 mutual recursion with a (List Int64) accumulator (scalar element control)"
  (input  (do
            (def (evens (: n Int64) (: acc (List Int64)))
              (if (= n 0) acc (odds (- n 1) (List.push acc n))))
            (def (odds (: n Int64) (: acc (List Int64)))
              (if (= n 0) acc (evens (- n 1) acc)))
            (def (main (: k Int64))
              (List.len (evens k (list))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 3 Int64)))

(case "id1 handling an ALREADY-DISCHARGED effect: outer handle of an effect the inner fully consumed"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 999
                ((a (u) s (resume (- 0 s) s)))
                (handle St n
                  ((a (u) s (resume s (+ s 1))))
                  (+ (St.a) (St.a)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 11 Int64)))

(case "fl1 a Float64 handler state through a two-site arm (threshold on the magnitude)"
  (input  (do
            (effect St (op feed (-> Int64 Int64)))
            (def (main (: a Int64))
              (handle St 0.5
                ((feed (v) s (if (> v 10) (resume v (+ s 0.25)) (resume 0 s))))
                (+ (* 100 (St.feed 20)) (+ (* 10 (St.feed a)) (St.feed 30)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2030 Int64)))

(case "fr1 a TRIPLING Float64 thread crosses a fixed threshold — three compares catch the crossing depth, exact integer-valued floats"
  (input  (do
            (effect E (op over (-> Float64)))
            (def (main (: seed Float64))
              (handle E seed
                ((over () s (resume (if (> s 4.0) 1.0 0.0) (* s 3.0))))
                (+ (* 100.0 (E.over)) (+ (* 10.0 (E.over)) (E.over)))))
            (export main)))
  (call   main (: 1.0 Float64)) (output (: 1.0 Float64))
  (call   main (: 2.0 Float64)) (output (: 11.0 Float64))
  (call   main (: 8.0 Float64)) (output (: 111.0 Float64)))

(case "qs1 two perform-drawn quantities of one dimension ADD — same-unit combine over two crossings"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (let ((q1 (Qty.of (St.next) (Unit.base #"meter"))))
                  (let ((q2 (Qty.of (St.next) (Unit.base #"meter"))))
                    (Qty.value (+ q1 q2))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 11 Int64)))

(case "tp4 readers and a SPIN writer share one tuple state — sum flows into slot a while old a shifts to b"
  (input  (do
            (effect E (op geta (-> Int64)) (op getb (-> Int64)) (op spin (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n 100)
                ((geta () s (match s ((tuple a b) (resume a s))))
                 (getb () s (match s ((tuple a b) (resume b s))))
                 (spin () s (match s ((tuple a b) (resume (+ a b) (tuple (+ a b) a))))))
                (let ((r1 (E.geta)))
                  (let ((r2 (E.getb)))
                    (do (E.spin)
                        (+ r1 (+ (* 10 r2) (+ (* 100 (E.geta)) (* 1000 (E.getb))))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 14303 Int64))
  (call   main (: 0 Int64)) (output (: 11000 Int64))
  (call   main (: -5 Int64)) (output (: 5495 Int64)))

(case "rr1 RECORD handler state updated by one arm, field-read by a SIBLING arm plus a third counter arm — the record twin of the collection cross-arm shape"
  (input  (do
            (effect E (op tick (-> Int64)) (op rd (-> Int64)) (op cur (-> Int64)))
            (def (main (: n Int64))
              (handle E (record (= cnt n) (= tot 0))
                ((tick () st (resume (. st cnt)
                                     (record (= cnt (+ (. st cnt) 1))
                                             (= tot (+ (. st tot) (* 10 (. st cnt)))))))
                 (rd () st (resume (. st tot) st))
                 (cur () st (resume (. st cnt) st)))
                (+ (E.tick) (+ (E.tick) (+ (E.rd) (E.cur))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 128 Int64))
  (call   main (: 0 Int64)) (output (: 13 Int64))
  (call   main (: -3 Int64)) (output (: -56 Int64)))


; ── Seed-binder reuse, host Bytes boundary, and the ascribed-seed workaround (breaker riders) ──
; sh2x extends the freshen-the-seed fix (the sh2d/sh2n flip): the let binder feeds the nested
; handle's SEED and is read again AFTER the region — the freshened seed reference and the tail
; reference must resolve to the same binder. ba1 pins an EMPTY Bytes argument crossing the wasm
; host boundary. tk-ann1 pins the ASCRIBED empty-collection seed: with an explicit
; (: Map.empty (Map (Tuple Int64 Int64) Int64)) ascription the tuple-keyed map state works across
; three arms on every backend — the workaround face of the open-element-Var inference gap (the
; unascribed twin is a filed issue; its witnesses will join this family when the cross-arm
; unification lands). All rows hand-computed; all pass on wasm, rust, and rust-async.

(case "sh2x a let binder feeds a nested handle's SEED and is read again AFTER the region — the freshened seed reference and the tail reference are the same binder"
  (input  (do
            (effect E (op next (-> Int64)))
            (effect B (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((k (+ n 3)))
                  (+ (handle B (* k 2)
                       ((get (u) t (resume t (+ t 1))))
                       (+ (B.get) (B.get)))
                     k))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 41 Int64))
  (call   main (: 0 Int64)) (output (: 16 Int64))
  (call   main (: -4 Int64)) (output (: -4 Int64)))

(case "ba1 an EMPTY Bytes arg crosses the wasm host boundary"
  (input  (do
            (effect io (op sink (-> Bytes Int64)))
            (def (main (: k Int64))
              (host (io)
                (io.sink (Bytes.of (list)))))
            (export main)))
  (host-responses (respond io.sink (: 42 Int64)))
  (host-calls (call io.sink))
  (call   main (: 0 Int64)) (output (: 42 Int64)))

(case "tk-ann1 an ASCRIBED empty-Map seed (: Map.empty (Map (Tuple Int64 Int64) Int64)) used across three arms — the annotation grounds the element type every arm sees"
  (input  (do
            (effect E (op rec (-> Int64)) (op qry (-> Int64 Int64 Int64)) (op cnt (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n (: Map.empty (Map (Tuple Int64 Int64) Int64)))
                ((rec () st (match st
                              ((tuple s m)
                               (resume s (tuple (+ s 2)
                                                (Map.insert m (tuple s (+ s 1)) (* 10 s)))))))
                 (qry (a b) st (match st
                                 ((tuple s m)
                                  (resume (match (Map.lookup m (tuple a b))
                                            ((Some v) v)
                                            ((None) -1))
                                          st))))
                 (cnt () st (match st ((tuple s m) (resume s st)))))
                (do (E.rec) (+ (E.qry n (+ n 1)) (E.cnt)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 57 Int64)))


; ── A binary FRAME as handler state (breaker bf) ──────────────────────────────────────────────
; The wire-protocol accumulator idiom: the handler state IS a growing Bytes frame, seeded by the
; empty (bin) literal. bf1 appends u8 records per dispatch and bin-DECODES the accumulated state
; in a sibling arm — head byte plus remainder length through `(bin (u8 hd) (bytes tl))` (the
; remainder segment head is `bytes`, and a bare variable arm also serves; there is no `rest`
; head). bf2 grows the frame by u16 BIG-ENDIAN records and decodes the SECOND record through a
; fixed-width first segment — record framing survives width changes and mid-frame reads. bf3
; closes the loop: the arm ENCODES (tag,val) u8 pairs, a replay op returns the WHOLE accumulated
; frame, and the BODY decodes all four segments back — encode-in-arm, decode-in-body, with the
; frame crossing the handler boundary as a value. All rows hand-computed; all pass on wasm, rust,
; and rust-async.

(case "bf1 a growing BYTES FRAME as handler state — each op appends a u8 record, the final op bin-decodes head + rest length"
  (input  (do
            (effect W (op log (-> Int64 Int64)) (op dump (-> Int64)))
            (def (main (: n Int64))
              (handle W (bin)
                ((log (v) fr (resume v (Bytes.concat fr (bin (u8 (UInt8.wrap v))))))
                 (dump () fr (match fr
                               ((bin (u8 hd) (bytes tl))
                                (resume (+ (* 100 (Int64.of hd)) (Bytes.len tl)) fr))
                               (_other (resume -1 fr)))))
                (do (W.log (+ 10 n)) (W.log (+ 20 n)) (W.dump))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1101 Int64))
  (call   main (: 4 Int64)) (output (: 1401 Int64))
  (call   main (: 0 Int64)) (output (: 1001 Int64)))

(case "bf2 the frame grows by u16 BE records — dump decodes the SECOND record through a fixed-width first segment"
  (input  (do
            (effect W (op log (-> Int64 Int64)) (op second (-> Int64)))
            (def (main (: n Int64))
              (handle W (bin)
                ((log (v) fr (resume v (Bytes.concat fr (bin (u16 (UInt16.wrap v))))))
                 (second () fr (match fr
                                 ((bin (u16 first) (u16 mid) (bytes tl))
                                  (resume (Int64.of mid) fr))
                                 (_other (resume -1 fr)))))
                (do (W.log (+ 100 n)) (W.log (+ 200 n)) (W.log (+ 300 n)) (W.second))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 201 Int64))
  (call   main (: 4 Int64)) (output (: 204 Int64))
  (call   main (: 0 Int64)) (output (: 200 Int64)))

(case "bf3 tagged-record round-trip — arms ENCODE (tag,val) u8 pairs into the frame state, the body decodes the REPLAYED frame"
  (input  (do
            (effect W (op rec (-> Int64 Int64 Int64)) (op replay (-> Bytes)))
            (def (main (: n Int64))
              (handle W (bin)
                ((rec (t v) fr (resume v (Bytes.concat fr (bin (u8 (UInt8.wrap t)) (u8 (UInt8.wrap v))))))
                 (replay () fr (resume fr fr)))
                (do (W.rec 1 (+ 10 n)) (W.rec 2 (+ 20 n))
                    (match (W.replay)
                      ((bin (u8 t1) (u8 v1) (u8 t2) (u8 v2))
                       (+ (* 1000 (+ (* 100 (Int64.of t1)) (Int64.of v1)))
                          (+ (* 100 (Int64.of t2)) (Int64.of v2))))
                      (_other -1)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 111221 Int64))
  (call   main (: 4 Int64)) (output (: 114224 Int64))
  (call   main (: 0 Int64)) (output (: 110220 Int64)))

(case "a resuming arm whose resume VALUE is a transform of state and next-state is a distinct advance, observed twice"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume (* s 2) (+ s 1))))
                (+ (St.next) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 22 Int64))
  (call   main (: 0 Int64)) (output (: 2 Int64)))

(case "a payload op arg feeds BOTH the resume value and the next-state through DIFFERENT operators, dispatched twice"
  (input  (do
            (effect St (op step (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St n
                ((step (d) s (resume (* s d) (+ s d))))
                (+ (St.step 2) (St.step 3))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 31 Int64))
  (call   main (: 0 Int64)) (output (: 6 Int64)))

(case "a mutually-recursive performer pair whose out-state a trailing caller draw observes declines cleanly (completeness gap)"
  (doc    "The completeness boundary where the #12 post-recursion-state fold (which threads a SELF-recursive
           callee's out-state to a trailing observer) meets a MUTUAL-recursive SCC (breaker row-mr). `ev`/`od`
           mutually recurse, each performing `E.next`; the caller reads the SCC's final out-state via a trailing
           `(E.next)` after `(ev 2)`. The self-recursive face folds (the #12 fix, f60b44c42), but threading a
           MUTUAL SCC's out-state across the whole group to a caller observer needs group-wide multi-value
           specialization tying the partners together — a later increment. Single-return would drop the
           partners' advances (a silent wrong value), so the fold DECLINES cleanly (an honest not-yet-reducible
           todo) rather than miscompile. When the group multi-value + caller-observed-outstate arc lands, this
           FOLDS to 3405: main(3) draws 3 (ev), 4 (od) → ev 2 = 10·3 + 4 = 34; the trailing (E.next) reads the
           post-SCC state 5 → 100·34 + 5 = 3405 (verified via the linear-equivalent draw sequence). The output
           is pinned (3405); when the arc lands, flip this case's baseline entry todo→pass.")
  (input  (do
            (effect E (op next (-> Int64)))
            (def (ev (: k Int64)) (if (<= k 0) 0 (+ (* 10 (E.next)) (od (- k 1)))))
            (def (od (: k Int64)) (if (<= k 0) 0 (+ (E.next) (ev (- k 1)))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (+ (* 100 (ev 2)) (E.next))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3405 Int64)))

(case "a single handler with three ops each mutating the state by a DIFFERENT function, composed in sequence"
  (input  (do
            (effect R (op inc (-> Int64)) (op dbl (-> Int64)) (op cur (-> Int64)))
            (def (main (: n Int64))
              (handle R n
                ((inc () s (resume s (+ s 1)))
                 (dbl () s (resume s (* s 2)))
                 (cur () s (resume s s)))
                (do (R.inc) (R.dbl) (R.cur))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 12 Int64))
  (call   main (: 0 Int64)) (output (: 2 Int64)))

(case "a mutual-group demand-perform-demand arm in a let-wrapped dispatch folds down a runtime-opaque spine"
  (doc    "The mutual-SCC group-fold's let-wrapped-dispatch pre-check arm (the group analogue of the #12
           single-performer arm). `demand`/`cache`/`compute` are a mutual SCC over a per-node state effect;
           `compute`'s arm is a `(let ((a (demand child))) (match (St.put …) (_ (demand child))))` — a demand-
           perform-demand whose let-INIT is a mutual-partner call and whose body dispatch performs then calls
           a partner AGAIN. The group pre-check `group_multivalue_leaves_threadable` had only if/match/leaf
           arms, so this `(let inits dispatch)` fell to the LEAF case and the partner call under the match arm
           declined the whole SCC up front (not-yet-reducible), even though `thread_returning_tuple` already
           descends a let-wrapped dispatch. Fixed by mirroring the twin's `(let inits dispatch)` arm into the
           group pre-check. `get` always misses (forces compute), `kids` returns a smaller id until 0 (a finite
           runtime-opaque spine the partial evaluator cannot pre-reduce), `put` is a no-op resume. `demand 3`
           folds down the spine to the leaf id 0 (compute's None arm returns the id when kids runs out).")
  (input  (do
            (effect St (op get (-> Int64 (Option Int64))) (op put (-> (Tuple Int64 Int64) Unit))
                       (op kids (-> Int64 (Option Int64))))
            (def (demand (: id Int64))
              (match (St.get id)
                (((. Option Some) v) v)
                (((. Option None) u) (cache id (compute id)))))
            (def (cache (: id Int64) (: v Int64)) (match (St.put (tuple id v)) (_ v)))
            (def (compute (: id Int64))
              (match (St.kids id)
                (((. Option Some) childId)
                  (let ((a (demand childId)))
                    (match (St.put (tuple id a)) (_ (demand childId)))))
                (((. Option None) u) id)))
            (def (run (: root Int64))
              (handle St root
                ((get (id) s (resume (. Option None) s))
                 (put (pair) s (match pair ((tuple x y) (resume unit s))))
                 (kids (id) s (resume (if (<= id 0) (. Option None) (Some (- id 1))) s)))
                (demand root)))
            (export run)))
  (call   run (: 3 Int64)) (output (: 0 Int64)))


; ── Float64 threads: regime switches, cross-draw compares, and the trichotomy (breaker fx) ────
; The float state thread under BRANCHED evolution — all rows exact dyadics so equality is exact.
; fx1 puts a REGIME SWITCH in the next-state expression (doubling below a threshold, halving
; above); three draws sum the trajectory across the crossing. fx2 COMPARES two consecutive draws
; — a negative seed flips the order (doubling a negative moves DOWN), a tail draw pins the
; thread. fx3 threads a float slot with the regime switch BESIDE an int counter in one tuple —
; mixed-width slots advance independently. fx4 is the float sign TRICHOTOMY: positive scales,
; negative negates, and an exact 0.0 draw routes to the constant arm — the equality face of
; float branching. All hand-computed; all pass on wasm, rust, and rust-async.

(case "fx1 a Float64 state with a REGIME SWITCH in the arm — doubling below the threshold, halving above, three draws sum the trajectory"
  (input  (do
            (effect E (op draw (-> Float64)))
            (def (main (: n Int64))
              (handle E (+ 1.0 (Float64.of-int n))
                ((draw () s (resume s (if (< s 10.0) (* s 2.0) (* s 0.5)))))
                (+ (E.draw) (+ (E.draw) (E.draw)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 21.0 Float64))
  (call   main (: 0 Int64)) (output (: 7.0 Float64))
  (call   main (: 5 Int64)) (output (: 24.0 Float64)))

(case "fx2 float COMPARISON of two consecutive draws routes the branch — a negative seed FLIPS the order, a tail draw pins the doubling thread"
  (input  (do
            (effect E (op draw (-> Float64)))
            (def (main (: n Int64))
              (handle E (+ 1.0 (Float64.of-int n))
                ((draw () s (resume s (* s 2.0))))
                (let ((d1 (E.draw)))
                  (let ((d2 (E.draw)))
                    (+ (if (> d2 d1) (- d2 d1) (- d1 d2))
                       (E.draw))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 15.0 Float64))
  (call   main (: -5 Int64)) (output (: -12.0 Float64))
  (call   main (: 0 Int64)) (output (: 5.0 Float64)))

(case "fx3 a FLOAT slot with a regime-switch beside an INT counter in one tuple state — mixed-width slots thread independently"
  (input  (do
            (effect E (op draw (-> Float64)) (op count (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple (+ 1.0 (Float64.of-int n)) 0)
                ((draw () st (match st
                               ((tuple s c)
                                (resume s (tuple (if (< s 10.0) (* s 2.0) (* s 0.5)) (+ c 1))))))
                 (count () st (match st ((tuple s c) (resume c st)))))
                (+ (E.draw) (+ (E.draw) (+ (E.draw) (Float64.of-int (E.count)))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 24.0 Float64))
  (call   main (: 0 Int64)) (output (: 10.0 Float64))
  (call   main (: 5 Int64)) (output (: 27.0 Float64)))

(case "fx4 float sign TRICHOTOMY of a draw — positive scales, negative negates, exact 0.0 routes to the constant arm"
  (input  (do
            (effect E (op draw (-> Float64)))
            (def (main (: n Int64))
              (handle E (Float64.of-int n)
                ((draw () s (resume s (+ s 1.0))))
                (let ((d (E.draw)))
                  (+ (if (> d 0.0) (* d 10.0)
                         (if (< d 0.0) (- 0.0 d) 99.0))
                     (E.draw)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34.0 Float64))
  (call   main (: 0 Int64)) (output (: 100.0 Float64))
  (call   main (: -2 Int64)) (output (: 1.0 Float64)))

(case "an outer op-arg match with an inner state match reading the op-arg payload threads per dispatch"
  (doc    "The stale-sum-payload-across-dispatches fix (breaker #13). The arm destructures its op ARG with an
           OUTER match `(match c ((Cmd.Go k) …))` and the handler STATE with an INNER match `(match s …)`
           whose branches read the outer payload binder `k` DIRECTLY. `peel_resume_from_arm_body` used to
           rebuild the next-state by re-wrapping the WHOLE matched expression — sound for a state-scrutinee
           match, but for the OUTER OP-ARG match it threaded the op-arg payload `k` through STATE, so a later
           dispatch's own `k` was conflated with the state-threaded one (a silent wrong value, all backends:
           `(+ (M.step (Cmd.Go 15)) (M.step (Cmd.Go 7)))` gave 45 not 37). Fixed by folding the op-arg
           constructor match at dispatch time (`fold_ctor_match`) so only the inner state match threads.
           main(5): dispatch1 Go 15 state Idle -> 15 (state Run 15); dispatch2 Go 7 state Run(15) -> 15+7=22;
           sum 37.")
  (input  (do
            (type Mode (Idle) (Run Int64))
            (type Cmd (Go Int64))
            (effect M (op step (-> Cmd Int64)))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (c) s
                  (match c
                    ((Cmd.Go k) (match s
                                  ((Mode.Idle) (resume k (Mode.Run k)))
                                  ((Mode.Run j) (resume (+ j k) (Mode.Run (+ j k)))))))))
                (+ (M.step (Cmd.Go (+ 10 n)))
                   (M.step (Cmd.Go 7)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 37 Int64)))

(case "two DIFFERENT ops sharing the outer-arg-match inner-state-match arm shape each thread their own payload"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (type Cmd (Go Int64))
            (effect M (op step (-> Cmd Int64)) (op step2 (-> Cmd Int64)))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (c) s
                  (match c
                    ((Cmd.Go k) (match s
                                  ((Mode.Idle) (resume k (Mode.Run k)))
                                  ((Mode.Run j) (resume (+ j k) (Mode.Run (+ j k))))))))
                 (step2 (c) s
                  (match c
                    ((Cmd.Go k) (match s
                                  ((Mode.Idle) (resume k (Mode.Run k)))
                                  ((Mode.Run j) (resume (+ j k) (Mode.Run (+ j k)))))))))
                (+ (M.step (Cmd.Go (+ 10 n)))
                   (M.step2 (Cmd.Go 7)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 37 Int64))
  (call   main (: 0 Int64)) (output (: 27 Int64)))

(case "three dispatches of an outer-arg-match inner-state-match arm each read their own payload not dispatch one's"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (type Cmd (Go Int64))
            (effect M (op step (-> Cmd Int64)))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (c) s
                  (match c
                    ((Cmd.Go k) (match s
                                  ((Mode.Idle) (resume k (Mode.Run k)))
                                  ((Mode.Run j) (resume (+ j k) (Mode.Run (+ j k)))))))))
                (+ (M.step (Cmd.Go (+ 10 n)))
                   (+ (M.step (Cmd.Go 7))
                      (M.step (Cmd.Go 1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 60 Int64)))

(case "a multi-variant op arg matched against a sum state with an abortive-and-resumptive arm mix threads per dispatch"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (type Cmd (Go Int64) (Halt))
            (effect M (op step (-> Cmd Int64)))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (c) s
                  (match c
                    ((Cmd.Go k) (match s
                                  ((Mode.Idle) (resume k (Mode.Run k)))
                                  ((Mode.Run j) (resume (+ j k) (Mode.Run (+ j k))))))
                    ((Cmd.Halt) (resume -5 (Mode.Idle))))))
                (+ (M.step (Cmd.Go (+ 10 n)))
                   (+ (M.step (Cmd.Go 7))
                      (+ (M.step (Cmd.Halt))
                         (M.step (Cmd.Go 1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 33 Int64))
  (call   main (: 0 Int64)) (output (: 23 Int64))
  (call   main (: -3 Int64)) (output (: 17 Int64)))

; ── Cross-arm element propagation for empty-collection seeds + two-thread races (breaker) ─────
; First family: an EMPTY collection literal in a handler-state seed leaves its element type open;
; the concrete type is fixed in ONE arm (an insert/push) and read in a SIBLING arm (a lookup/
; contains/index). Inference now propagates the solved element type across all arms onto the
; seed (the fix this family pinned): tk1 tuple-KEYED Map with draw-built keys (hit + miss rows),
; tk3 the minimal three-arm trigger, tv1 the VALUE-position twin, sk4 the Set-element face, lv1
; the List-element face. The ascribed-seed workaround (tk-ann1) and non-empty-seed forms were
; already pinned green. Second family: a RACE between two effect threads — a recursive walk
; draws BOTH effects per round until the fast thread catches the slow one's input-dependent head
; start (ra1; the n=5 row catches on round one, zero recursions), and the race's exit value
; SEEDING a third effect's handle (ra2 — a multi-effect recursion entirely inside a seed
; expression). All rows hand-computed; all pass on wasm, rust, and rust-async.

(case "tk1 a Map state with TUPLE keys built from consecutive draws — record twice, query a hit and a miss"
  (input  (do
            (effect E (op rec (-> Int64))
                      (op qry (-> Int64 Int64 Int64))
                      (op cnt (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n Map.empty)
                ((rec () st (match st
                              ((tuple s m)
                               (resume s (tuple (+ s 2)
                                                (Map.insert m (tuple s (+ s 1)) (* 10 s)))))))
                 (qry (a b) st (match st
                                 ((tuple s m)
                                  (resume (match (Map.lookup m (tuple a b))
                                            ((Some v) v)
                                            ((None) -1))
                                          st))))
                 (cnt () st (match st ((tuple s m) (resume s st)))))
                (do (E.rec) (E.rec)
                    (+ (E.qry n (+ n 1))
                       (+ (* 1000 (E.qry (+ n 9) (+ n 10)))
                          (E.cnt))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -941 Int64))
  (call   main (: 0 Int64)) (output (: -996 Int64))
  (call   main (: -3 Int64)) (output (: -1029 Int64)))

(case "tk3 an UNASCRIBED empty-Map seed with tuple keys solved in one arm, read in siblings — cross-arm element propagation (3 arms)"
  (input  (do
            (effect E (op rec (-> Int64)) (op qry (-> Int64 Int64 Int64)) (op cnt (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n Map.empty)
                ((rec () st (match st
                              ((tuple s m)
                               (resume s (tuple (+ s 2)
                                                (Map.insert m (tuple s (+ s 1)) (* 10 s)))))))
                 (qry (a b) st (match st
                                 ((tuple s m)
                                  (resume (match (Map.lookup m (tuple a b))
                                            ((Some v) v)
                                            ((None) -1))
                                          st))))
                 (cnt () st (match st ((tuple s m) (resume s st)))))
                (do (E.rec) (+ (E.qry n (+ n 1)) (E.cnt)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 57 Int64)))

(case "tv1 tuple VALUES with scalar keys across three arms — element propagation is value-position too"
  (input  (do
            (effect E (op rec (-> Int64)) (op qry (-> Int64 Int64)) (op cnt (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n Map.empty)
                ((rec () st (match st
                              ((tuple s m)
                               (resume s (tuple (+ s 2)
                                                (Map.insert m s (tuple s (* 10 s))))))))
                 (qry (k) st (match st
                               ((tuple s m)
                                (resume (match (Map.lookup m k)
                                          ((Some p) (match p ((tuple a b) (+ a b))))
                                          ((None) -1))
                                        st))))
                 (cnt () st (match st ((tuple s m) (resume s st)))))
                (do (E.rec) (+ (E.qry n) (E.cnt)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 62 Int64))
  (call   main (: 0 Int64)) (output (: 2 Int64)))

(case "sk4 a Set of TUPLES as handler state with three arms — insert in one arm, contains in a sibling"
  (input  (do
            (effect E (op add (-> Int64))
                      (op has (-> Int64 Int64 Int64))
                      (op cnt (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n (Set.of (list)))
                ((add () st (match st
                              ((tuple s ss)
                               (resume s (tuple (+ s 2)
                                                (Set.insert ss (tuple s (+ s 1))))))))
                 (has (a b) st (match st
                                 ((tuple s ss)
                                  (resume (if (Set.contains ss (tuple a b)) 1 0) st))))
                 (cnt () st (match st ((tuple s ss) (resume s st)))))
                (do (E.add) (E.add)
                    (+ (E.has n (+ n 1))
                       (+ (* 1000 (E.has (+ n 9) (+ n 10)))
                          (E.cnt))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 10 Int64))
  (call   main (: 0 Int64)) (output (: 5 Int64))
  (call   main (: -3 Int64)) (output (: 2 Int64)))

(case "lv1 a List of TUPLES from the empty (list) literal seed — pushed in one arm, indexed in a sibling"
  (input  (do
            (effect E (op push (-> Int64))
                      (op rd (-> Int64))
                      (op cnt (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n (list))
                ((push () st (match st
                               ((tuple s xs)
                                (resume s (tuple (+ s 2)
                                                 (List.push xs (tuple s (* 2 s))))))))
                 (rd () st (match st
                             ((tuple s xs)
                              (resume (match (List.at xs 0)
                                        ((Some p) (match p ((tuple a b) (+ a b))))
                                        ((None) -1))
                                      st))))
                 (cnt () st (match st ((tuple s xs) (resume s st)))))
                (do (E.push) (E.push) (+ (E.rd) (E.cnt)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 24 Int64))
  (call   main (: 0 Int64)) (output (: 4 Int64))
  (call   main (: -3 Int64)) (output (: -8 Int64)))

(case "ra1 a RACE between two effect threads — a recursive walk draws BOTH per round until the fast thread catches the slow one's head start"
  (input  (do
            (effect A (op next (-> Int64)))
            (effect B (op next (-> Int64)))
            (def (race (: steps Int64))
              (let ((a (A.next)))
                (let ((b (B.next)))
                  (if (< a b) (race (+ steps 1)) (+ (* 100 steps) (- a b))))))
            (def (main (: n Int64))
              (handle A n
                ((next () s (resume (+ s 5) (+ s 5))))
                (handle B (+ n (+ (* 2 (if (< (% n 5) 0) (- 0 (% n 5)) (% n 5))) 3))
                  ((next () t (resume (+ t 2) (+ t 2))))
                  (race 0))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 0 Int64))
  (call   main (: 1 Int64)) (output (: 101 Int64))
  (call   main (: -4 Int64)) (output (: 301 Int64)))

(case "ra2 the race walk's exit value SEEDS a third effect's handle — two-thread termination feeding the position matrix"
  (input  (do
            (effect A (op next (-> Int64)))
            (effect B (op next (-> Int64)))
            (effect C (op g (-> Unit Int64)))
            (def (race (: steps Int64))
              (let ((a (A.next)))
                (let ((b (B.next)))
                  (if (< a b) (race (+ steps 1)) (+ (* 100 steps) (- a b))))))
            (def (main (: n Int64))
              (handle A n
                ((next () s (resume (+ s 5) (+ s 5))))
                (handle B (+ n (+ (* 2 (if (< (% n 5) 0) (- 0 (% n 5)) (% n 5))) 3))
                  ((next () t (resume (+ t 2) (+ t 2))))
                  (handle C (race 0)
                    ((g (u) w (resume w (+ w 1))))
                    (+ (C.g) (C.g))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64))
  (call   main (: 1 Int64)) (output (: 203 Int64))
  (call   main (: -4 Int64)) (output (: 603 Int64)))


; ── Peel-boundary green faces around the #13 op-arg-match fix (breaker cm complements) ────────
; Complements to the four landed stale-sum-payload pins (the outer-arg-match × inner-state-match
; faces): these pin the shapes around the peel's boundary that always threaded correctly and must
; KEEP doing so. The state-OUTER nesting (the peeled arm-body match IS the state match — threading
; it is the correct behavior the fix must preserve); the inner state-match routed through a helper
; def (the payload arrives fresh at the call boundary); a let-derived value interposed between the
; two matches (interposed bindings re-evaluate per dispatch); and an inner match on a locally-built
; Option rather than the state (a non-state inner scrutinee). All hand-computed; all pass on wasm,
; rust, and rust-async.

(case "the STATE-outer nesting of the dual-sum arm threads fresh payloads across three dispatches — the peel's state-match threading face"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (type Cmd (Go Int64))
            (effect M (op step (-> Cmd Int64)))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (c) s
                  (match s
                    ((Mode.Idle) (match c ((Cmd.Go k) (resume k (Mode.Run k)))))
                    ((Mode.Run j) (match c ((Cmd.Go k) (resume (+ j k) (Mode.Run (+ j k)))))))))
                (+ (M.step (Cmd.Go (+ 10 n)))
                   (+ (M.step (Cmd.Go 7))
                      (M.step (Cmd.Go 1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 60 Int64))
  (call   main (: 0 Int64)) (output (: 45 Int64))
  (call   main (: -3 Int64)) (output (: 36 Int64)))

(case "the inner sum-state match routed through a HELPER DEF taking the payload as a parameter — a def boundary in the peeled arm body"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (type Cmd (Go Int64))
            (effect M (op step (-> Cmd Int64)))
            (def (decide (: s Mode) (: k Int64))
              (match s
                ((Mode.Idle) (tuple k (Mode.Run k)))
                ((Mode.Run j) (tuple (+ j k) (Mode.Run (+ j k))))))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (c) s
                  (match c
                    ((Cmd.Go k) (match (decide s k)
                                  ((tuple v s2) (resume v s2)))))))
                (+ (M.step (Cmd.Go (+ 10 n)))
                   (M.step (Cmd.Go 7)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 37 Int64))
  (call   main (: 0 Int64)) (output (: 27 Int64)))

(case "a LET-derived value between the op-arg match and the state match — interposed bindings re-evaluate per dispatch"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (type Cmd (Go Int64))
            (effect M (op step (-> Cmd Int64)))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (c) s
                  (match c
                    ((Cmd.Go k)
                     (let ((m (* 2 k)))
                       (match s
                         ((Mode.Idle) (resume m (Mode.Run m)))
                         ((Mode.Run j) (resume (+ j m) (Mode.Run (+ j m))))))))))
                (+ (M.step (Cmd.Go (+ 10 n)))
                   (M.step (Cmd.Go 7)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 74 Int64))
  (call   main (: 0 Int64)) (output (: 54 Int64)))

(case "an inner match on a LOCALLY-BUILT Option from the payload (scalar state) — a non-state inner scrutinee beside the dispatch"
  (input  (do
            (type Cmd (Go Int64))
            (effect M (op step (-> Cmd Int64)))
            (def (main (: n Int64))
              (handle M 0
                ((step (c) s
                  (match c
                    ((Cmd.Go k)
                     (match (if (> k 8) (Some k) (None))
                       ((Some x) (resume (+ x s) (+ s 1)))
                       ((None) (resume 0 s)))))))
                (+ (M.step (Cmd.Go (+ 10 n)))
                   (M.step (Cmd.Go 7)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 15 Int64))
  (call   main (: 0 Int64)) (output (: 10 Int64)))

(case "a recursive callee performing TWO effects per round threads both slots' out-state to a trailing observer"
  (doc    "The multi-draw recursion state-fork (breaker #14). `race` draws A then B each round and self-recurses;
           the caller reads A's post-recursion out-state via a trailing `(A.next)` after `(race 0)`. The
           multi-value state-fork used to thread only a `(let inits DISPATCH)` whose body was directly if/match;
           a NESTED let chain `(let a=(A.next) in (let b=(B.next) in (if …)))` fell to the leaf case and forced
           single-return, dropping the advance — the trailing draw read PRE-recursion state (10 not 15). Fixed by
           descending nested-let chains. main(5): round0 A=10 B=13 (10<13 recurse), round1 A=15 B=18 (15<18…)
           — the finite draw sequence resolves; the trailing (A.next) reads the ADVANCED state.")
  (input  (do
            (effect A (op next (-> Int64)))
            (effect B (op next (-> Int64)))
            (def (race (: k Int64))
              (let ((a (A.next)))
                (let ((b (B.next)))
                  (if (< a b) (race (+ k 1)) k))))
            (def (main (: n Int64))
              (handle A n
                ((next () s (resume (+ s 5) (+ s 5))))
                (handle B (+ n 3)
                  ((next () t (resume (+ t 2) (+ t 2))))
                  (let ((steps (race 0)))
                    (+ (* 100 steps) (A.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 15 Int64)))

(case "a recursive callee drawing the SAME op twice per round threads the out-state to a trailing observer"
  (doc    "The same-effect face of the multi-draw fork (breaker #14 ra5): `race` draws `E.next` TWICE per round
           via a nested let, self-recurses on `(< (+ a b) 30)`. The two draws sit in a nested `(let a in let b
           in if)` chain; without the nested-let descent the recursion single-returns and the trailing `(E.next)`
           reads pre-recursion state (110 not 130). main(5): round0 a=10 b=15 (state 5->10->15, 25<30 recurse);
           round1 a=20 b=25 (state->20->25, 45>=30 return k=1); trailing (E.next) reads 25->30 = 100*1+30 = 130.")
  (input  (do
            (effect E (op next (-> Int64)))
            (def (race (: k Int64))
              (let ((a (E.next)))
                (let ((b (E.next)))
                  (if (< (+ a b) 30) (race (+ k 1)) k))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume (+ s 5) (+ s 5))))
                (let ((steps (race 0)))
                  (+ (* 100 steps) (E.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 130 Int64)))

(case "a recursive round with a let draw and a bare do-discarded draw threads the out-state to a trailing observer"
  (doc    "The do-body face of the multi-draw fork (breaker #14 ra6): `race` draws `E.next` into `a`, then a
           `(do (E.next) (if …))` performs a SECOND draw for-effect (value discarded) before the dispatch. A
           `let` whose body is a `do` with a performing head fell to the leaf case; fixed by giving the fold a
           `do` arm (thread the for-effect stmts, recurse on the tail). main(5): round0 a=10 (state 5->10), do
           draws 15 (->15), a=10<20 recurse; round1 a=20 (->20), do draws 25 (->25), a=20<20 false return k=1;
           trailing (E.next) 25->30 = 100*1+30 = 130. A dropped out-state would read pre-recursion (110).")
  (input  (do
            (effect E (op next (-> Int64)))
            (def (race (: k Int64))
              (let ((a (E.next)))
                (do (E.next)
                    (if (< a 20) (race (+ k 1)) k))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume (+ s 5) (+ s 5))))
                (let ((steps (race 0)))
                  (+ (* 100 steps) (E.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 130 Int64)))

; ── Record op payloads by projection + the guard-vs-state face (breaker rp2/rp3/gp1) ──────────
; rp2/rp3: a record PAYLOAD consumed by FIELD PROJECTION (no outer match — the arm-peel's match
; path is never involved) beside an inner sum-state match, fresh per dispatch across Idle→Run.
; rp2 is the flat record; rp3 nests a record inside the payload and projects two levels. Both pin
; the RT4 canonical (Record (: k Int64)) ascription in effect-op signature position and the
; Phase-B canonical (= name value) triple in the record literals. gp1: a match GUARD inside the
; handler arm comparing the op payload against the LIVE STATE binder — the admit path grows the
; state by the payload, the reject path leaves it; the three rows cover reject-both, admit-both,
; and the negative-seed thresholds. (A guard that PERFORMS is CDZ0407-rejected by design — the
; purity policy reaches arm position; pinned by the diagnostics family.) All rows hand-computed;
; all pass on wasm, rust, and rust-async.

(case "rp2 a RECORD op argument read by field beside an inner sum-state match — the record-payload face of the per-dispatch arm"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (effect M (op step (-> (Record (: k Int64) (: w Int64)) Int64)))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (c) s
                  (match s
                    ((Mode.Idle) (resume (* (. c k) (. c w)) (Mode.Run (* (. c k) (. c w)))))
                    ((Mode.Run j) (resume (+ j (* (. c k) (. c w))) (Mode.Run j))))))
                (+ (M.step (record (= k (+ 10 n)) (= w 2)))
                   (M.step (record (= k 3) (= w 4))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 72 Int64))
  (call   main (: 0 Int64)) (output (: 52 Int64)))

(case "rp3 the record payload carries a NESTED record — two-level field projection inside the arm beside the state match"
  (input  (do
            (type Mode (Idle) (Run Int64))
            (effect M (op step (-> (Record (: m (Record (: k Int64))) (: w Int64)) Int64)))
            (def (main (: n Int64))
              (handle M (Mode.Idle)
                ((step (c) s
                  (match s
                    ((Mode.Idle) (resume (* (. (. c m) k) (. c w)) (Mode.Run (* (. (. c m) k) (. c w)))))
                    ((Mode.Run j) (resume (+ j (* (. (. c m) k) (. c w))) (Mode.Run j))))))
                (+ (M.step (record (= m (record (= k (+ 10 n)))) (= w 2)))
                   (M.step (record (= m (record (= k 3))) (= w 4))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 72 Int64))
  (call   main (: 0 Int64)) (output (: 52 Int64)))

(case "gp1 a match GUARD inside the handler ARM compares the op PAYLOAD against the live STATE binder — the guard routes admit/reject per dispatch"
  (input  (do
            (effect E (op judge (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((judge (v) s
                  (match v
                    ((guard x (> x s)) (resume 1 (+ s x)))
                    (_x (resume 0 s)))))
                (+ (* 10 (E.judge 3)) (E.judge 4))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 0 Int64))
  (call   main (: 0 Int64)) (output (: 11 Int64))
  (call   main (: -3 Int64)) (output (: 11 Int64)))


; ── Guard conditions over the LIVE STATE in handler-arm position (breaker gp follow-ons) ──────
; Extends the landed gp1 state-read guard with three faces of the arm-guard purity boundary.
; gp3: the guard condition calls a PURE HELPER over the state binder — purity analysis admits a
; pure def call in guard position (a PERFORMING guard is CDZ0407-rejected by design). gp4: a
; guard LADDER — two guarded arms with different state-derived thresholds (2s, then s) classify
; each payload three ways, pinning first-match-wins ordering over state-reading guards in ARM
; position (the gl family pins let-lifted BODY ladders). gp5: the guard DESTRUCTURES the tuple
; payload AND reads the state in one condition — admit by the pair's sum against the live
; threshold, with an admit-admit row via a negative seed. All rows hand-computed; all pass on
; wasm, rust, and rust-async.

(case "gp3 the guard condition calls a PURE HELPER over the state binder — purity analysis admits the def call in guard position"
  (input  (do
            (effect E (op judge (-> Int64 Int64)))
            (def (sq (: t Int64)) (* t t))
            (def (main (: n Int64))
              (handle E n
                ((judge (v) s
                  (match v
                    ((guard x (> x (sq s))) (resume 1 (+ s x)))
                    (_x (resume 0 s)))))
                (+ (* 10 (E.judge 5)) (E.judge 50))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 11 Int64))
  (call   main (: 0 Int64)) (output (: 11 Int64))
  (call   main (: -4 Int64)) (output (: 1 Int64)))

(case "gp4 a guard LADDER in the arm — two guarded arms with different state-derived thresholds classify each payload three ways"
  (input  (do
            (effect E (op classify (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((classify (v) s
                  (match v
                    ((guard x (> x (* 2 s))) (resume 2 (+ s 1)))
                    ((guard x (> x s)) (resume 1 (+ s 1)))
                    (_x (resume 0 s)))))
                (+ (* 10 (E.classify 8)) (E.classify 3))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 22 Int64))
  (call   main (: 4 Int64)) (output (: 10 Int64))
  (call   main (: 9 Int64)) (output (: 0 Int64)))

(case "gp5 the guard DESTRUCTURES the tuple payload AND reads the state — (guard (tuple a b) (> (+ a b) s)) admits by the pair's sum against the live threshold"
  (input  (do
            (effect E (op rate (-> (Tuple Int64 Int64) Int64)))
            (def (main (: n Int64))
              (handle E n
                ((rate (p) s
                  (match p
                    ((guard (tuple a b) (> (+ a b) s)) (resume (+ (* 10 a) b) (+ s (+ a b))))
                    ((tuple _a _b) (resume 0 s)))))
                (+ (* 100 (E.rate (tuple 3 4))) (E.rate (tuple 1 2)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 3400 Int64))
  (call   main (: 8 Int64)) (output (: 0 Int64))
  (call   main (: -5 Int64)) (output (: 3412 Int64)))

(case "gp6 a CTOR-pattern guard with a SAME-CTOR non-guarded sibling admits per-dispatch across TWO dispatches (breaker #15)"
  (doc    "breaker FINDING #15 (HIGH silent-miscompile, all 3 backends). A ctor-pattern guard `(guard (Wrap
           v) (> v s))` followed by a SAME-CTOR non-guarded sibling `(Wrap _v)` — over ≥2 dispatches of the
           op — collapsed: BOTH dispatches (incl the one whose guard HOLDS) ran the sibling fallback, so a
           value that should ADMIT via the guard silently took the 0-arm. Single-dispatch, a WILDCARD sibling
           (`_other`), and scalar/tuple-payload guards (gp1/gp4/gp5) all fold correctly — the break was
           specific to a ctor-pattern guard shadowed by a same-ctor sibling. ROOT: `fold_ctor_match` (the
           case-of-known-ctor fold the multi-dispatch arm re-instantiation runs) skipped the guarded arm as
           refutable but then folded to the LATER same-discriminant sibling, discarding the guard's admit
           path. Fix: `fold_ctor_match` DECLINES (leaves a runtime match, which evaluates the guard) when a
           skipped guarded arm's ctor discriminant matches the scrutinee — an undecided same-ctor guard
           shadows any later arm. `main 5`: dispatch-1 `(Wrap 5)` guard `5>0` HOLDS → `resume 5 5` → 10*5 =
           50, and threads the next-state s→5; dispatch-2 `(Wrap 3)` guard `3>5` now FAILS → fallback 0 →
           50 + 0 = 50 (the guard's own next-state advance is what makes dispatch-2 miss). `main 0`:
           dispatch-1 `(Wrap 0)` guard `0>0` FAILS → 0 (s stays 0); dispatch-2 `(Wrap 3)` guard `3>0` HOLDS →
           resume 3 → 0*10 + 3 = 3.")
  (input  (do
            (type Box (Wrap Int64))
            (effect E (op rate (-> Box Int64)))
            (def (main (: k Int64))
              (handle E 0
                ((rate (c) s
                  (match c
                    ((guard (Wrap v) (> v s)) (resume v v))
                    ((Wrap _v) (resume 0 s)))))
                (+ (* 10 (E.rate (Wrap k))) (E.rate (Wrap 3)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 50 Int64))
  (call   main (: 0 Int64)) (output (: 3 Int64)))

(case "gp7 a LITERAL-payload arm and a same-ctor general arm both select per-dispatch across TWO dispatches (breaker #15 guardless face)"
  (doc    "breaker FINDING #15 WIDER face — the guard is NOT required; the trigger is [two same-ctor arms in a
           handler match] × [≥2 dispatches]. A LITERAL-payload arm `(Wrap 0)` before a general `(Wrap v)` arm:
           over two dispatches the LITERAL arm was lost — dispatch-2's `(Wrap 0)` took the general arm's value
           (0) instead of the literal arm's 100. Same root as the guarded gp6: `fold_ctor_match` (the
           case-of-known-ctor fold the multi-dispatch arm re-instantiation runs) skipped the undecidable
           literal-payload arm and folded to the LATER same-discriminant general arm, discarding the literal's
           selection path. Fix: fold_ctor_match DECLINES (leaves a runtime match) when a same-disc arm has a
           LITERAL payload sub-pattern it can't statically decide. `main 7`: dispatch-1 `(Wrap 7)` misses the
           literal → general v=7 → 1000*7 = 7000; dispatch-2 `(Wrap 0)` HITS the literal → 100 → 7000 + 100 =
           7100. `main 0`: dispatch-1 `(Wrap 0)` HITS literal → 100 → 1000*100 = 100000; dispatch-2 `(Wrap 0)`
           HITS literal → 100 → 100100.")
  (input  (do
            (type Box (Wrap Int64))
            (effect E (op rate (-> Box Int64)))
            (def (main (: k Int64))
              (handle E 0
                ((rate (c) s
                  (match c
                    ((Wrap 0) (resume 100 s))
                    ((Wrap v) (resume v s)))))
                (+ (* 1000 (E.rate (Wrap k))) (E.rate (Wrap 0)))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 7100 Int64))
  (call   main (: 0 Int64)) (output (: 100100 Int64)))

(case "gp8 a THREE-arm same-ctor ladder (two literals + general) selects each arm per-dispatch across THREE dispatches (breaker #15 ladder face)"
  (doc    "breaker FINDING #15 ladder face — the fix generalizes to N same-ctor arms, not just two. THREE
           same-ctor arms `((Wrap 0) 100) ((Wrap 1) 200) ((Wrap v) v)` over three dispatches: the bug dropped
           BOTH literal arms (every dispatch took the general arm). fold_ctor_match now DECLINES the whole fold
           the moment it would skip ANY refined same-disc arm (here the first literal `(Wrap 0)`), so the
           runtime match evaluates all three arms — however many. `main 5`: dispatch-1 `(Wrap 5)` → general 5;
           dispatch-2 `(Wrap 0)` → literal 100 (×10); dispatch-3 `(Wrap 1)` → literal 200 (×100); 5 + 1000 +
           20000 = 21005. `main -3`: dispatch-1 `(Wrap -3)` → general -3; +1000 +20000 = 20997.")
  (input  (do
            (type Box (Wrap Int64))
            (effect E (op rate (-> Box Int64)))
            (def (main (: k Int64))
              (handle E 0
                ((rate (c) s
                  (match c
                    ((Wrap 0) (resume 100 s))
                    ((Wrap 1) (resume 200 s))
                    ((Wrap v) (resume v s)))))
                (+ (E.rate (Wrap k)) (+ (* 10 (E.rate (Wrap 0))) (* 100 (E.rate (Wrap 1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 21005 Int64))
  (call   main (: -3 Int64)) (output (: 20997 Int64)))


; ── Data-driven slot routing and whole-state permutation (breaker ss5-7 / rg1-2) ──────────────
; The multi-slot tuple state under DATA-DRIVEN shape changes. Slot routing: an op ARG selects
; which slot to bump through an if-chain (ss5, with a slot-0 revisit reading its own bump); the
; selector DERIVES from the state itself via |a mod 3| (ss6 — same-slot revisit, two-slot walk,
; and the negative-seed abs face); a 2-ary op carries selector AND magnitude (ss7, a trailing
; read pinning the accumulated slot). Whole-state permutation: the arm ROTATES (a b c)→(b c a)
; per dispatch returning the evicted head — four weighted reads wrap the ring (rg1); the rotation
; DIRECTION flips on the evicted head's parity, a branch choosing between two permutations in
; next-state position, incl. the negative-odd truncated-mod face (rg2). All rows hand-computed;
; all pass on wasm, rust, and rust-async.

(case "ss5 an op ARG selects WHICH tuple-state slot to bump — index-routed slot mutation, four dispatches revisit slot 0"
  (input  (do
            (effect E (op sel (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E (tuple n 10 100)
                ((sel (i) st (match st
                               ((tuple a b c)
                                (if (= i 0) (resume a (tuple (+ a 1) b c))
                                    (if (= i 1) (resume b (tuple a (+ b 1) c))
                                        (resume c (tuple a b (+ c 1)))))))))
                (+ (E.sel 0) (+ (E.sel 2) (+ (E.sel 1) (E.sel 0))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 121 Int64))
  (call   main (: 0 Int64)) (output (: 111 Int64)))

(case "ss6 the slot SELECTOR is derived from the state itself — |a mod 3| routes each bump, a same-slot revisit and a two-slot walk both pinned"
  (input  (do
            (effect E (op step (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n 10 100)
                ((step () st (match st
                               ((tuple a b c)
                                (let ((m (% a 3)))
                                  (let ((i (if (< m 0) (- 0 m) m)))
                                    (if (= i 0) (resume a (tuple (+ a 1) b c))
                                        (if (= i 1) (resume b (tuple a (+ b 1) c))
                                            (resume c (tuple a b (+ c 1)))))))))))
                (+ (E.step) (E.step))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 201 Int64))
  (call   main (: 0 Int64)) (output (: 10 Int64))
  (call   main (: -4 Int64)) (output (: 21 Int64)))

(case "ss7 a 2-ary op carries SELECTOR and MAGNITUDE — which slot and by how much are both payload-driven, a trailing read pins slot 0"
  (input  (do
            (effect E (op sel (-> Int64 Int64 Int64)) (op rd0 (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n 10 100)
                ((sel (i d) st (match st
                                 ((tuple a b c)
                                  (if (= i 0) (resume a (tuple (+ a d) b c))
                                      (if (= i 1) (resume b (tuple a (+ b d) c))
                                          (resume c (tuple a b (+ c d))))))))
                 (rd0 () st (match st ((tuple a b c) (resume a st)))))
                (+ (E.sel 0 3) (+ (E.sel 2 7) (E.rd0)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 113 Int64))
  (call   main (: 0 Int64)) (output (: 103 Int64)))

(case "rg1 the arm ROTATES the tuple state (a b c)->(b c a) per dispatch and returns the evicted head — four weighted reads wrap the ring"
  (input  (do
            (effect E (op pop (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n 10 100)
                ((pop () st (match st
                              ((tuple a b c) (resume a (tuple b c a))))))
                (+ (E.pop)
                   (+ (* 2 (E.pop))
                      (+ (* 3 (E.pop))
                         (* 4 (E.pop)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 345 Int64))
  (call   main (: 0 Int64)) (output (: 320 Int64))
  (call   main (: -3 Int64)) (output (: 305 Int64)))

(case "rg2 the rotation DIRECTION flips on the evicted head's parity — even rotates left, odd rotates right, negative-odd exercises truncated mod"
  (input  (do
            (effect E (op pop (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n 10 100)
                ((pop () st (match st
                              ((tuple a b c)
                               (resume a (if (= (% a 2) 0)
                                             (tuple b c a)
                                             (tuple c a b)))))))
                (+ (E.pop) (+ (* 2 (E.pop)) (* 3 (E.pop))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 220 Int64))
  (call   main (: 0 Int64)) (output (: 320 Int64))
  (call   main (: -3 Int64)) (output (: 188 Int64)))


; ── String-keyed locks: payload-vs-state equality, rolling codes, and lockout (breaker lk) ────
; The access-control idiom family over STRING comparison in the arm. lk1: the arm compares the
; string PAYLOAD against the stored KEY by equality — parity picks the key, the lock state gates
; three verdicts (miss / unlock / already-open). lk2: a ROLLING code — each successful match
; advances the key index through a (List String) state via List.at, |n mod 3| picking the start.
; lk3: LOCKOUT — a fail counter flips a dead flag that rejects even the correct key thereafter,
; with the dispatch sequence branching in the body. String-vs-string equality between payload
; and state was previously unpinned (the sd pins compare built content against literals). All
; rows hand-computed; all pass on wasm, rust, and rust-async.

(case "lk1 a STRING-keyed lock — the arm compares the string payload against the stored key by equality; parity picks the key, a fail counter rides along"
  (input  (do
            (effect L (op try (-> String Int64)))
            (def (main (: n Int64))
              (handle L (tuple (if (= (% n 2) 0) "ab" "cd") 1 0)
                ((try (a) st (match st
                               ((tuple key locked fails)
                                (if (= locked 1)
                                    (if (= a key)
                                        (resume 1 (tuple key 0 fails))
                                        (resume 0 (tuple key 1 (+ fails 1))))
                                    (resume 2 st))))))
                (let ((r1 (L.try "cd")))
                  (let ((r2 (L.try "ab")))
                    (let ((r3 (L.try "cd")))
                      (match (L.try "zz")
                        (_last (+ (* 100 r1) (+ (* 10 r2) r3)))))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 12 Int64))
  (call   main (: 1 Int64)) (output (: 122 Int64))
  (call   main (: 0 Int64)) (output (: 12 Int64)))

(case "lk2 a ROLLING-code lock — each successful match advances the key index through a list of string keys, |n mod 3| picks the start"
  (input  (do
            (effect L (op try (-> String Int64)))
            (def (main (: n Int64))
              (handle L (tuple (list "aa" "bb" "cc")
                               (let ((m (% n 3))) (if (< m 0) (- 0 m) m)))
                ((try (a) st (match st
                               ((tuple keys ki)
                                (match (List.at keys ki)
                                  ((Some key)
                                   (if (= a key)
                                       (resume 1 (tuple keys (% (+ ki 1) 3)))
                                       (resume 0 st)))
                                  ((None) (resume -1 st)))))))
                (+ (* 1000 (L.try "aa"))
                   (+ (* 100 (L.try "bb"))
                      (+ (* 10 (L.try "bb"))
                         (L.try "cc"))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1101 Int64))
  (call   main (: 1 Int64)) (output (: 101 Int64))
  (call   main (: 2 Int64)) (output (: 1 Int64)))

(case "lk3 LOCKOUT after two failures — the fail counter flips a dead flag that rejects even the correct key thereafter"
  (input  (do
            (effect L (op try (-> String Int64)))
            (def (main (: n Int64))
              (handle L (tuple 0 0)
                ((try (a) st (match st
                               ((tuple fails dead)
                                (if (= dead 1)
                                    (resume 9 st)
                                    (if (= a "ab")
                                        (resume 1 st)
                                        (resume 0 (tuple (+ fails 1)
                                                         (if (>= (+ fails 1) 2) 1 0)))))))))
                (if (> n 0)
                    (+ (* 100 (L.try "xx")) (+ (* 10 (L.try "xx")) (L.try "ab")))
                    (+ (* 100 (L.try "xx")) (+ (* 10 (L.try "ab")) (L.try "ab"))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 9 Int64))
  (call   main (: 0 Int64)) (output (: 11 Int64))
  (call   main (: -2 Int64)) (output (: 11 Int64)))

(case "slmin11 tuple(scalar,string) state, put rebuilds the tuple with a branch-picked String.concat suffix, 2 puts + 2 reads (invalid-wasm slot-clobber: the rope-retain dup and the counter step must not share a scratch slot at two widths)"
  (input  (do
            (effect E (op put (-> Int64)) (op size (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n "ab")
                ((put () st (match st
                              ((tuple s r)
                               (resume s (tuple (+ s 1)
                                                (String.concat r (if (= (% s 3) 0) "x" "yz")))))))
                 (size () st (match st ((tuple s r) (resume (String.byte-len r) st)))))
                (do (E.put) (E.put) (+ (E.size) (E.size)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 10 Int64))
  (call   main (: 1 Int64)) (output (: 12 Int64)))

(case "by5 the slmin11 slot-clobber shape on a BYTES slot, phi-merged Bytes.concat growth in a tuple state, 2 puts + 2 reads"
  (input  (do
            (effect E (op put (-> Int64)) (op size (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n (Bytes.of (list (UInt8.wrap 9))))
                ((put () st (match st
                              ((tuple s b)
                               (resume s (tuple (+ s 1)
                                                (Bytes.concat b (if (= (% s 3) 0)
                                                                    (Bytes.of (list (UInt8.wrap 1)))
                                                                    (Bytes.of (list (UInt8.wrap 2) (UInt8.wrap 3))))))))))
                 (size () st (match st ((tuple s b) (resume (Bytes.len b) st)))))
                (do (E.put) (E.put) (+ (E.size) (E.size)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 8 Int64))
  (call   main (: 1 Int64)) (output (: 10 Int64))
  (call   main (: -2 Int64)) (output (: 10 Int64)))


; ── Sliding windows over the payload stream + the slot-clobber extended witnesses (breaker) ───
; Sliding windows: sw2 keeps the last TWO payloads in a bounded list state and resumes the window
; sum via a recursive helper over its own list; sw3 makes the window CAPACITY itself state — a
; grow op resizes mid-stream with the windowed MAX tracked across the resize; sw4 is the windowed
; AVERAGE with the truncating divide over the live length (a negative window exercises
; toward-zero); sw5 counts DISTINCT values via a Set built per dispatch from the last-3 window.
; Slot-clobber extended witnesses (the Bytes.concat rhs base-floor reuse fixed by the
; emit-above-high-water change; the minimal slmin11/by5 pins landed with the fix): sl1 composes
; the branch-picked rope growth with a byte-len-seeded nested handle; tr2 pins that an untouched
; SIBLING rope field neither masks nor causes the clobber. All rows hand-computed; all pass on
; wasm, rust, and rust-async.

(case "sw2 a SLIDING WINDOW of the last two payloads — the arm keeps a bounded list state and resumes the window sum"
  (input  (do
            (effect E (op feed (-> Int64 Int64)))
            (def (sum-at (: xs (List Int64)) (: i Int64) (: acc Int64))
              (match (List.at xs i)
                ((Some v) (sum-at xs (+ i 1) (+ acc v)))
                ((None) acc)))
            (def (sum2 (: xs (List Int64))) (sum-at xs 0 0))
            (def (main (: n Int64))
              (handle E (list)
                ((feed (v) win
                  (let ((grown (List.push win v)))
                    (let ((kept (if (> (List.len grown) 2)
                                    (match (List.at grown 1)
                                      ((Some a) (match (List.at grown 2)
                                                  ((Some b) (list a b))
                                                  ((None) grown)))
                                      ((None) grown))
                                    grown)))
                      (resume (sum2 kept) kept)))))
                (+ (* 100 (E.feed n)) (+ (* 10 (E.feed 7)) (E.feed (+ n 1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 633 Int64))
  (call   main (: 0 Int64)) (output (: 78 Int64))
  (call   main (: -3 Int64)) (output (: -255 Int64)))

(case "sw3 the window CAPACITY is itself state — a grow op resizes the window mid-stream, windowed MAX tracked across the resize"
  (input  (do
            (effect E (op feed (-> Int64 Int64)) (op grow (-> Int64)))
            (def (max-at (: xs (List Int64)) (: i Int64) (: best Int64))
              (match (List.at xs i)
                ((Some v) (max-at xs (+ i 1) (if (> v best) v best)))
                ((None) best)))
            (def (drop-to (: xs (List Int64)) (: cap Int64))
              (if (> (List.len xs) cap)
                  (match (List.at xs (- (List.len xs) cap))
                    ((Some h)
                     (match (List.at xs (- (List.len xs) 1))
                       ((Some t) (if (= cap 2) (list h t)
                                     (match (List.at xs (- (List.len xs) 2))
                                       ((Some m) (list h m t))
                                       ((None) xs))))
                       ((None) xs)))
                    ((None) xs))
                  xs))
            (def (main (: n Int64))
              (handle E (tuple (list) 2)
                ((feed (v) st (match st
                                ((tuple win cap)
                                 (let ((kept (drop-to (List.push win v) cap)))
                                   (resume (max-at kept 0 -1000000) (tuple kept cap))))))
                 (grow () st (match st
                               ((tuple win cap) (resume cap (tuple win 3))))))
                (let ((r1 (E.feed n)))
                  (let ((r2 (E.feed 7)))
                    (do (E.grow)
                        (let ((r3 (E.feed (+ n 9))))
                          (let ((r4 (E.feed 1)))
                            (+ r1 (+ (* 2 r2) (+ (* 3 r3) (* 4 r4)))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 117 Int64))
  (call   main (: 0 Int64)) (output (: 77 Int64))
  (call   main (: -8 Int64)) (output (: 55 Int64)))

(case "sw4 windowed AVERAGE with the truncating divide — window sum over live length, a negative window exercises toward-zero"
  (input  (do
            (effect E (op feed (-> Int64 Int64)))
            (def (sum-at (: xs (List Int64)) (: i Int64) (: acc Int64))
              (match (List.at xs i)
                ((Some v) (sum-at xs (+ i 1) (+ acc v)))
                ((None) acc)))
            (def (tail3 (: xs (List Int64)))
              (if (> (List.len xs) 3)
                  (match (List.at xs (- (List.len xs) 3))
                    ((Some a) (match (List.at xs (- (List.len xs) 2))
                                ((Some b) (match (List.at xs (- (List.len xs) 1))
                                            ((Some c) (list a b c))
                                            ((None) xs)))
                                ((None) xs)))
                    ((None) xs))
                  xs))
            (def (main (: n Int64))
              (handle E (list)
                ((feed (v) win
                  (let ((kept (tail3 (List.push win v))))
                    (resume (/ (sum-at kept 0 0) (List.len kept)) kept))))
                (let ((r1 (E.feed n)))
                  (let ((r2 (E.feed 7)))
                    (let ((r3 (E.feed (+ n 2))))
                      (let ((r4 (E.feed -5)))
                        (+ r1 (+ (* 2 r2) (+ (* 3 r3) (* 4 r4))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 47 Int64))
  (call   main (: 0 Int64)) (output (: 19 Int64))
  (call   main (: -6 Int64)) (output (: -9 Int64)))

(case "sw5 DISTINCT-count over the window — a Set built per dispatch from the last-3 list measures dedupe, n=7 collides with the constant feed"
  (input  (do
            (effect E (op feed (-> Int64 Int64)))
            (def (tail3 (: xs (List Int64)))
              (if (> (List.len xs) 3)
                  (match (List.at xs (- (List.len xs) 3))
                    ((Some a) (match (List.at xs (- (List.len xs) 2))
                                ((Some b) (match (List.at xs (- (List.len xs) 1))
                                            ((Some c) (list a b c))
                                            ((None) xs)))
                                ((None) xs)))
                    ((None) xs))
                  xs))
            (def (distinct-at (: xs (List Int64)) (: i Int64) (: seen (Set Int64)))
              (match (List.at xs i)
                ((Some v) (distinct-at xs (+ i 1) (Set.insert seen v)))
                ((None) (Set.len seen))))
            (def (main (: n Int64))
              (handle E (list)
                ((feed (v) win
                  (let ((kept (tail3 (List.push win v))))
                    (resume (distinct-at kept 0 (Set.of (list))) kept))))
                (let ((r1 (E.feed n)))
                  (let ((r2 (E.feed n)))
                    (let ((r3 (E.feed 7)))
                      (let ((r4 (E.feed n)))
                        (+ (* 1000 r1) (+ (* 100 r2) (+ (* 10 r3) r4)))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1122 Int64))
  (call   main (: 7 Int64)) (output (: 1111 Int64))
  (call   main (: 0 Int64)) (output (: 1122 Int64)))

(case "sl1 a STRING slot grows by a mod-picked suffix and its BYTE-LEN seeds a nested handle — the composed face of the slot-clobber fix"
  (input  (do
            (effect E (op put (-> Int64)) (op size (-> Int64)))
            (effect B (op g (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E (tuple n "ab")
                ((put () st (match st
                              ((tuple s r)
                               (resume s (tuple (+ s 1)
                                                (String.concat r (if (= (% s 3) 0) "x" "yz")))))))
                 (size () st (match st ((tuple s r) (resume (String.byte-len r) st)))))
                (do (E.put) (E.put)
                    (+ (handle B (E.size)
                         ((g (u) t (resume t (+ t 10))))
                         (+ (B.g) (B.g)))
                       (E.size)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 25 Int64))
  (call   main (: 1 Int64)) (output (: 28 Int64))
  (call   main (: -2 Int64)) (output (: 28 Int64)))

(case "tr2 TWO string fields in the tuple with only one phi-grown — the sibling rope field is undisturbed (slot-clobber extended witness)"
  (input  (do
            (effect E (op put (-> Int64)) (op size (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple n "ab" "cd")
                ((put () st (match st
                              ((tuple s a b)
                               (resume s (tuple (+ s 1) a
                                                (String.concat b (if (= (% s 3) 0) "x" "yz")))))))
                 (size () st (match st
                               ((tuple s a b)
                                (resume (+ (* 100 (String.byte-len a)) (String.byte-len b)) st)))))
                (do (E.put) (E.put) (+ (E.size) (E.size)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 410 Int64))
  (call   main (: 1 Int64)) (output (: 412 Int64))
  (call   main (: -2 Int64)) (output (: 412 Int64)))


; ── Pipeline chains through the dispatch thread (breaker ac) ──────────────────────────────────
; Each op's RESULT feeds the next call's ARGUMENT while the handler state advances underneath.
; ac1 alternates a 1-ary and a 2-ary op in one nested call chain. ac2 let-binds the chain's
; MIDDLE result and routes the branch that picks WHICH op finishes the pipeline. ac3 BOUNCES the
; chain between two effects — the inner handler's result feeds the outer effect's op, whose
; result feeds the inner again, each thread advancing independently. All rows hand-computed;
; all pass on wasm, rust, and rust-async.

(case "ac1 a pipeline chain alternating a 1-ary and a 2-ary op — each result feeds the next call's argument while the thread advances"
  (input  (do
            (effect E (op inc (-> Int64 Int64)) (op mix (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((inc (x) s (resume (+ x s) (+ s 1)))
                 (mix (x y) s (resume (+ (* x y) s) (+ s 2))))
                (E.inc (E.mix (E.inc 3) 10))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 94 Int64))
  (call   main (: 0 Int64)) (output (: 34 Int64))
  (call   main (: -3 Int64)) (output (: -2 Int64)))

(case "ac2 the chain's MIDDLE result routes the branch that picks WHICH op finishes the pipeline"
  (input  (do
            (effect E (op inc (-> Int64 Int64)) (op dbl (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((inc (x) s (resume (+ x s) (+ s 1)))
                 (dbl (x) s (resume (+ (* 2 x) s) (+ s 2))))
                (let ((mid (E.dbl (E.inc 3))))
                  (if (> mid 10) (E.inc mid) (E.dbl mid)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 30 Int64))
  (call   main (: 0 Int64)) (output (: 17 Int64))
  (call   main (: -9 Int64)) (output (: -46 Int64)))

(case "ac3 the pipeline BOUNCES between two effects — inner B's result feeds outer E's op, whose result feeds B again"
  (input  (do
            (effect E (op inc (-> Int64 Int64)))
            (effect B (op g (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((inc (x) s (resume (+ x s) (+ s 1))))
                (handle B 100
                  ((g (x) t (resume (+ x t) (+ t 5))))
                  (B.g (E.inc (B.g 3))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 213 Int64))
  (call   main (: 0 Int64)) (output (: 208 Int64))
  (call   main (: -4 Int64)) (output (: 204 Int64)))


; ── Map DRAIN dynamics in the arm (breaker md2/md3) ───────────────────────────────────────────
; The keyed-store lifecycle under removal — the Map twin of the landed Set drain (se3). md2:
; Map.remove keyed by the op arg, a re-take of the drained key routes to the miss path, a
; trailing size pins the shrink (one row drains a negative value). md3 COMPOSES insert and
; remove in one next-state: the drained value re-files under key+3, the THIRD take hits the
; re-filed entry, and the final take finds it moved again. All rows hand-computed; all pass on
; wasm, rust, and rust-async.

(case "md2 the arm DRAINS a Map entry per dispatch — Map.remove keyed by the op arg, a re-take of the same key routes to the miss path"
  (input  (do
            (effect E (op take (-> Int64 Int64)) (op size (-> Int64)))
            (def (main (: n Int64))
              (handle E (Map.insert (Map.insert (Map.insert Map.empty 1 (+ 10 n)) 2 20) 3 30)
                ((take (k) m (match (Map.lookup m k)
                               ((Some v) (resume v (Map.remove m k)))
                               ((None) (resume -1 m))))
                 (size () m (resume (Map.len m) m)))
                (+ (* 1000 (E.take 1))
                   (+ (* 100 (E.take 1))
                      (+ (* 10 (E.take 3))
                         (E.size))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 15201 Int64))
  (call   main (: 0 Int64)) (output (: 10201 Int64))
  (call   main (: -13 Int64)) (output (: -2799 Int64)))

(case "md3 the drained value RE-FILES under a shifted key — the third take HITS the re-filed entry, the final take finds it moved on"
  (input  (do
            (effect E (op take (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E (Map.insert (Map.insert Map.empty 1 (+ 10 n)) 2 20)
                ((take (k) m (match (Map.lookup m k)
                               ((Some v) (resume v (Map.insert (Map.remove m k) (+ k 3) v)))
                               ((None) (resume -1 m)))))
                (+ (* 1000 (E.take 1))
                   (+ (* 100 (E.take 2))
                      (+ (* 10 (E.take 4))
                         (E.take 4))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 17149 Int64))
  (call   main (: 0 Int64)) (output (: 12099 Int64))
  (call   main (: -12 Int64)) (output (: -21 Int64)))


; ── Cross-effect INTERLEAVING at the body level (breaker il) ──────────────────────────────────
; Two independent effect threads advancing in alternation. il1 is the strict O-I-O-I lockstep —
; positional weights pin the exact dispatch order across the nested-handle boundary. il2 makes
; the interleave WIDTH data-driven: parity picks one-or-two O draws per I tick, with a
; performing `burst` helper called from the nested body summing each O burst. (The inner arm's
; NEXT-STATE drawing the outer effect DECLINES — the performing-next-state boundary, pinned by
; the rv3 family; resume-VALUE position folds per the landed wc1.) All rows hand-computed; all
; pass on wasm, rust, and rust-async.

(case "il1 strict O-I-O-I body interleave — two independent threads advance in lockstep, positional weights pin the dispatch order"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect I (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle I 100
                  ((tick () t (resume t (+ t 2))))
                  (+ (O.next)
                     (+ (* 10 (I.tick))
                        (+ (* 100 (O.next))
                           (* 1000 (I.tick))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 103403 Int64))
  (call   main (: 0 Int64)) (output (: 103100 Int64)))

(case "il2 the interleave WIDTH is data-driven — parity picks one-or-two O draws per I tick, a helper sums each O burst"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect I (op tick (-> Int64)))
            (def (burst (: k Int64))
              (if (= k 2) (+ (O.next) (O.next)) (O.next)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (handle I 100
                  ((tick () t (resume t (+ t 2))))
                  (let ((k (if (= (% n 2) 0) 2 1)))
                    (+ (burst k)
                       (+ (* 10 (I.tick))
                          (+ (* 100 (burst k))
                             (* 1000 (I.tick)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 103403 Int64))
  (call   main (: 2 Int64)) (output (: 103905 Int64))
  (call   main (: -1 Int64)) (output (: 102999 Int64)))


; ── Catch-and-retry protocols over conditional aborts (breaker cr) ────────────────────────────
; The exception-recovery idiom family built on the conditional-abort machinery. cr1
; catch-and-reseed: a conditionally-aborting region's value (abort payload OR normal result)
; seeds a SECOND effect's handle — the sequential region→seed chain (the sab1 pin nests the
; aborter INSIDE a seed). cr2 RETRY-until-success: a recursive driver re-runs a
; conditionally-aborting helper (a fresh abort handle per call) until the drawn attempt passes,
; counting tries while the counter thread persists ACROSS the retried handles. cr3 bounds the
; budget: two attempts then give up, the abort payload carrying the negated failed attempt for
; the fallback path. All rows hand-computed; all pass on wasm, rust, and rust-async.

(case "cr1 catch-and-reseed — a conditionally-aborting region's value (abort payload OR normal result) seeds a SECOND handle"
  (input  (do
            (effect R (op raise (-> Int64 Int64)))
            (effect T (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle T
                (handle R 0
                  ((raise (v) u v))
                  (if (= (% n 2) 0)
                      (+ n 1)
                      (do (R.raise (* n 10)) 999)))
                ((tick () t (resume t (+ t 3))))
                (+ (T.tick) (T.tick))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 63 Int64))
  (call   main (: 4 Int64)) (output (: 13 Int64))
  (call   main (: -5 Int64)) (output (: -97 Int64)))

(case "cr2 RETRY-until-success — a recursive driver re-runs a conditionally-aborting region until the drawn attempt passes, counting tries"
  (input  (do
            (effect C (op draw (-> Int64)))
            (effect R (op fail (-> Int64 Int64)))
            (def (attempt-once)
              (handle R 0
                ((fail (v) u v))
                (let ((a (C.draw)))
                  (if (= (% a 3) 0) (* 1000 a) (do (R.fail -1) 999)))))
            (def (retry (: tries Int64))
              (let ((r (attempt-once)))
                (if (< r 0) (retry (+ tries 1)) (+ (* 100 (+ tries 1)) (/ r 1000)))))
            (def (main (: n Int64))
              (handle C n
                ((draw () s (resume s (+ s 1))))
                (retry 0)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 103 Int64))
  (call   main (: 1 Int64)) (output (: 303 Int64))
  (call   main (: -2 Int64)) (output (: 300 Int64)))

(case "cr3 a BOUNDED retry budget — two attempts then give up, the fallback negating the last failed attempt"
  (input  (do
            (effect C (op draw (-> Int64)))
            (effect R (op fail (-> Int64 Int64)))
            (def (attempt-once)
              (handle R 0
                ((fail (v) u v))
                (let ((a (C.draw)))
                  (if (= (% a 5) 0) (* 1000 a) (do (R.fail (- 0 a)) 999)))))
            (def (retry (: tries Int64))
              (if (>= tries 2)
                  -999999
                  (let ((r (attempt-once)))
                    (if (> r -999999)
                        (if (>= r 0) (+ (* 100 (+ tries 1)) (/ r 1000)) 
                            (if (>= (+ tries 1) 2) r (retry (+ tries 1))))
                        r))))
            (def (main (: n Int64))
              (handle C n
                ((draw () s (resume s (+ s 1))))
                (retry 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64))
  (call   main (: 4 Int64)) (output (: 205 Int64))
  (call   main (: 1 Int64)) (output (: -2 Int64)))


; ── Arm-built band SUMS scored by body closures (breaker bd) ──────────────────────────────────
; The arm CLASSIFIES its own state into a band sum and the body consumes it through a let-bound
; closure. bd1: a tri-band Lo/Mid/Hi sum built from range tests in the arm, scored by a closure
; applied three times as the thread climbs — each row crosses a DIFFERENT band boundary. bd2:
; the scoring closure CAPTURES A DRAW — the weight itself comes from the thread before the
; classified reads (the sound pure-capture face composed with the band consumer). All rows
; hand-computed; all pass on wasm, rust, and rust-async.

(case "bd1 the arm CLASSIFIES its state into a tri-band SUM — the body matches Lo/Mid/Hi per dispatch as the thread climbs through the bands"
  (input  (do
            (type Band (Lo Int64) (Mid Int64) (Hi Int64))
            (effect E (op probe (-> Band)))
            (def (main (: n Int64))
              (handle E n
                ((probe () s
                  (resume (if (< s 0) (Band.Lo s)
                              (if (< s 10) (Band.Mid s) (Band.Hi s)))
                          (+ s 6))))
                (let ((score (fn ((: b Band))
                               (match b
                                 ((Band.Lo x) (- 0 x))
                                 ((Band.Mid x) (* 10 x))
                                 ((Band.Hi x) (+ x 1000))))))
                  (+ (score (E.probe)) (+ (score (E.probe)) (score (E.probe)))))))
            (export main)))
  (call   main (: -4 Int64)) (output (: 104 Int64))
  (call   main (: 2 Int64)) (output (: 1114 Int64))
  (call   main (: 8 Int64)) (output (: 2114 Int64)))

(case "bd2 the scoring CLOSURE captures a DRAW — the weight itself comes from the thread before the classified reads"
  (input  (do
            (type Band (Mid Int64) (Hi Int64))
            (effect E (op next (-> Int64)) (op probe (-> Band)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 6)))
                 (probe () s (resume (if (< s 10) (Band.Mid s) (Band.Hi s)) (+ s 6))))
                (let ((w (E.next)))
                  (let ((score (fn ((: b Band))
                                 (match b
                                   ((Band.Mid x) (* w x))
                                   ((Band.Hi x) (+ w x))))))
                    (+ (score (E.probe)) (score (E.probe)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 12 Int64))
  (call   main (: 5 Int64)) (output (: 38 Int64))
  (call   main (: -7 Int64)) (output (: -28 Int64)))


; ── String REASONS inside crossing Result sums (breaker sr2/sr3) ──────────────────────────────
; A Result-typed op whose Err carries a STRING reason, discriminated by string EQUALITY in the
; body's consumer closure. sr2 uses a literal reason; sr3 BUILDS the reason in the arm —
; String.concat of a prefix and a sign-picked tag — and matches it against literals in the body
; (the built-vs-literal equality face inside a sum crossing the dispatch boundary). All rows
; hand-computed; all pass on wasm, rust, and rust-async.

(case "sr2 a Result-typed op whose Err carries a STRING reason — the body matches Ok payload vs the reason by string equality"
  (input  (do
            (type Res (Ok Int64) (Err String))
            (effect E (op step (-> Res)))
            (def (main (: n Int64))
              (handle E n
                ((step () s
                  (resume (if (= (% s 2) 0) (Res.Ok s) (Res.Err "odd")) (+ s 3))))
                (let ((score (fn ((: r Res))
                               (match r
                                 ((Res.Ok v) v)
                                 ((Res.Err why) (if (= why "odd") 7 1))))))
                  (+ (* 10 (score (E.step))) (score (E.step))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 27 Int64))
  (call   main (: 1 Int64)) (output (: 74 Int64))
  (call   main (: -4 Int64)) (output (: -33 Int64)))

(case "sr3 the Err reason is BUILT in the arm — String.concat of a prefix and a sign-picked tag, matched against literals in the body"
  (input  (do
            (type Res (Ok Int64) (Err String))
            (effect E (op step (-> Res)))
            (def (main (: n Int64))
              (handle E n
                ((step () s
                  (resume (if (= (% s 2) 0)
                              (Res.Ok s)
                              (Res.Err (String.concat "e-" (if (< s 0) "lo" "hi"))))
                          (+ s 3))))
                (let ((score (fn ((: r Res))
                               (match r
                                 ((Res.Ok v) v)
                                 ((Res.Err why) (if (= why "e-lo") 3 9))))))
                  (+ (* 10 (score (E.step))) (score (E.step))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 94 Int64))
  (call   main (: -3 Int64)) (output (: 30 Int64))
  (call   main (: -4 Int64)) (output (: -37 Int64)))


; ── Option lifecycles and HEAP-NUMERIC handler states (breaker oc4-6 / bg3-4 / rq1-3) ─────────
; Option lifecycles: oc4 runs the full None→Some→None→Some RESET cycle (put promotes or
; accumulates, reset reports and clears); oc5 holds TWO Option slots in one tuple with a SWAP
; exchanging them; oc6 nests a (total,count) TUPLE inside the Some, updated element-wise through
; a two-level match. Heap-numeric states: bg3 threads a BIGINT — tripling crosses the i64
; boundary mid-thread and the exact multi-limb value survives (render note: gate rows are bare
; digits, no N suffix); bg4 routes tri-band verdicts by BIGINT comparison in the arm, one row
; comparing a genuine multi-limb value. rq1 threads a RATIONAL — three exact fractional adds
; (1/2+1/3+1/6) land on a whole value (canonical n/1 render); rq2 reads floor/ceil/numerator/
; denominator (which return BIGINT — wrapped via Int64.of), pinning canonicalization (8/4→2/1)
; and negative rounding; rq3 drains by 1/3 per tick under a sign guard until the rational
; crosses zero. All rows hand-computed; all pass on wasm, rust, and rust-async.

(case "oc4 an Option state with a RESET cycle — put promotes None to Some or accumulates inside Some; reset reports and returns to None; a fresh put re-promotes"
  (input  (do
            (effect E (op put (-> Int64 Int64)) (op reset (-> Int64)))
            (def (main (: n Int64))
              (handle E (None)
                ((put (v) st (match st
                               ((Some cur) (resume (+ cur v) (Some (+ cur v))))
                               ((None) (resume 0 (Some v)))))
                 (reset () st (match st
                                ((Some cur) (resume cur (None)))
                                ((None) (resume -1 st)))))
                (+ (E.put n)
                   (+ (* 10 (E.put 7))
                      (+ (* 100 (E.reset))
                         (* 1000 (E.put 3)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1320 Int64))
  (call   main (: 0 Int64)) (output (: 770 Int64))
  (call   main (: -2 Int64)) (output (: 550 Int64)))

(case "oc5 TWO Option slots in one tuple state — independent put lifecycles, a SWAP op exchanges them, reads confirm the exchange"
  (input  (do
            (effect E (op puta (-> Int64 Int64)) (op putb (-> Int64 Int64))
                      (op swap (-> Int64)) (op reada (-> Int64)) (op readb (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple (None) (None))
                ((puta (v) st (match st ((tuple a b) (resume 0 (tuple (Some v) b)))))
                 (putb (v) st (match st ((tuple a b) (resume 0 (tuple a (Some v))))))
                 (swap () st (match st ((tuple a b) (resume 1 (tuple b a)))))
                 (reada () st (match st ((tuple a b) (resume (match a ((Some x) x) ((None) -1)) st))))
                 (readb () st (match st ((tuple a b) (resume (match b ((Some x) x) ((None) -1)) st)))))
                (do (E.puta n) (E.putb 7) (E.swap)
                    (+ (* 10 (E.reada)) (E.readb)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 75 Int64))
  (call   main (: 0 Int64)) (output (: 70 Int64))
  (call   main (: -3 Int64)) (output (: 67 Int64)))

(case "oc6 an Option-of-TUPLE state — the Some payload is a (total,count) pair updated element-wise per dispatch, a final read reports both"
  (input  (do
            (effect E (op mark (-> Int64 Int64)) (op report (-> Int64)))
            (def (main (: n Int64))
              (handle E (None)
                ((mark (v) st (match st
                                ((Some p) (match p
                                            ((tuple a c) (resume (+ a v) (Some (tuple (+ a v) (+ c 1)))))))
                                ((None) (resume 0 (Some (tuple v 1))))))
                 (report () st (match st
                                 ((Some p) (match p ((tuple a c) (resume (+ (* 1000 c) a) st))))
                                 ((None) (resume -1 st)))))
                (+ (E.mark n)
                   (+ (* 10 (E.mark 4))
                      (+ (* 100 (E.mark -2))
                         (E.report))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3797 Int64))
  (call   main (: 0 Int64)) (output (: 3242 Int64))
  (call   main (: -7 Int64)) (output (: 2465 Int64)))

(case "bg3 a BIGINT handler state — tripling per dispatch crosses the i64 boundary mid-thread, the exact multi-limb value survives"
  (input  (do
            (effect E (op triple (-> Int64)) (op report (-> BigInt)))
            (def (main (: n Int64))
              (handle E (+ (BigInt.of 1000000000000000000) (BigInt.of n))
                ((triple () s (resume 1 (* s (BigInt.of 3))))
                 (report () s (resume s s)))
                (do (E.triple) (E.triple) (E.triple) (E.report))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 27000000000000000027 BigInt))
  (call   main (: 0 Int64)) (output (: 27000000000000000000 BigInt)))

(case "bg4 BIGINT comparison in the arm routes tri-band verdicts — doubling walks past both thresholds, one row compares a genuine multi-limb value"
  (input  (do
            (effect E (op judge (-> Int64)))
            (def (main (: n Int64))
              (handle E (* (BigInt.of n) (BigInt.of 1000000000000000000))
                ((judge () s
                  (resume (if (> s (BigInt.of 5000000000000000000)) 2
                              (if (< s (- (BigInt.of 0) (BigInt.of 5000000000000000000))) 0 1))
                          (* s (BigInt.of 2)))))
                (+ (* 100 (E.judge)) (+ (* 10 (E.judge)) (E.judge)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 112 Int64))
  (call   main (: -2 Int64)) (output (: 110 Int64))
  (call   main (: 4 Int64)) (output (: 122 Int64)))

(case "rq1 a RATIONAL handler state — three exact fractional adds (1/2 + 1/3 + 1/6) land on a WHOLE value — the canonical n/1 render pinned"
  (input  (do
            (effect E (op add (-> Int64 Int64)) (op report (-> Rational)))
            (def (main (: n Int64))
              (handle E (Rational.of n 1)
                ((add (d) s (resume 1 (+ s (Rational.of 1 d))))
                 (report () s (resume s s)))
                (do (E.add 2) (E.add 3) (E.add 6) (E.report))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1/1 Rational))
  (call   main (: 1 Int64)) (output (: 2/1 Rational))
  (call   main (: -1 Int64)) (output (: 0/1 Rational)))

(case "rq2 floor/ceil/numerator/denominator READ a rational state — canonicalization (8/4 to 2/1) and negative rounding both pinned"
  (input  (do
            (effect E (op fl (-> Int64)) (op ce (-> Int64)) (op nu (-> Int64)) (op de (-> Int64)))
            (def (main (: n Int64))
              (handle E (Rational.of n 4)
                ((fl () s (resume (Int64.of (Rational.floor s)) s))
                 (ce () s (resume (Int64.of (Rational.ceil s)) s))
                 (nu () s (resume (Int64.of (Rational.numerator s)) s))
                 (de () s (resume (Int64.of (Rational.denominator s)) s)))
                (+ (* 1000 (E.fl))
                   (+ (* 100 (E.ce))
                      (+ (* 10 (E.nu)) (E.de))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1254 Int64))
  (call   main (: 8 Int64)) (output (: 2221 Int64))
  (call   main (: -3 Int64)) (output (: -1026 Int64)))

(case "rq3 a FRACTIONAL drain with a sign guard — subtracting 1/3 per tick until the rational state crosses zero, the recursion counts full ticks"
  (input  (do
            (effect E (op drain (-> Int64)))
            (def (spin (: ticks Int64))
              (let ((sig (E.drain)))
                (if (< sig 0) (+ (* 100 ticks) sig) (spin (+ ticks 1)))))
            (def (main (: n Int64))
              (handle E (Rational.of n 3)
                ((drain () s
                  (let ((s2 (- s (Rational.of 1 3))))
                    (resume (if (< s2 (Rational.of 0 1)) (Int64.of (Rational.numerator s2)) 0) s2))))
                (spin 0)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 199 Int64))
  (call   main (: 1 Int64)) (output (: 99 Int64))
  (call   main (: 0 Int64)) (output (: -1 Int64)))


; ── User GENERIC sums resolving BY NAME (breaker gs — the resolve-path fix promotion) ─────────
; A user-declared generic sum referenced by NAME in type positions once declined CDZ0101
; "unbound name"; the resolve-path fix makes the applied form check and run (the bare no-arg
; form now gives the correct CDZ0203 needs-type-argument). The position matrix over the fix:
; gs1 the applied annotation (Container Int64) on a def param; gs2 TWO type parameters
; ((Pair Int64 Int64), both slots extracted); gs3 the applied generic as an effect-op RESULT
; (the arm wraps the advancing state); gs4 a HEAP payload ((Container (List Int64)), summed
; after unwrap); gs5 SELF-application ((Container (Container Int64)), double unwrap); gs6 a
; TWO-VARIANT generic ((Either a b), parity-routed construction, both arms matched); gs7 the
; op ARGUMENT position (the arm unwraps payloads into the state); gs8 the generic AS the
; handler state (seeded wrapped, unwrapped and re-wrapped per dispatch). All rows hand-computed;
; all pass on wasm, rust, and rust-async.

(case "gs1 a user-declared GENERIC sum resolves by NAME in a param annotation — the applied (Container Int64) checks and the payload unwraps"
  (input  (do
            (type (Container a) (Full a))
            (def (unwrap (: b (Container Int64))) (match b ((Full v) v)))
            (def (main (: k Int64)) (unwrap (Full k)))
            (export main)))
  (call   main (: 7 Int64)) (output (: 7 Int64))
  (call   main (: -12 Int64)) (output (: -12 Int64)))

(case "gs2 a TWO-parameter generic sum by name — (Pair Int64 Int64) in the annotation, both payload slots extracted"
  (input  (do
            (type (Pair a b) (Both a b))
            (def (mix (: p (Pair Int64 Int64))) (match p ((Both x y) (+ (* 10 x) y))))
            (def (main (: k Int64)) (mix (Both k 3)))
            (export main)))
  (call   main (: 7 Int64)) (output (: 73 Int64))
  (call   main (: -2 Int64)) (output (: -17 Int64)))

(case "gs3 the applied generic sum CROSSES a dispatch — (Container Int64) as an op result, unwrapped in the body after the arm wraps the state"
  (input  (do
            (type (Container a) (Full a))
            (effect E (op box (-> (Container Int64))))
            (def (main (: k Int64))
              (handle E k
                ((box () s (resume (Full s) (+ s 1))))
                (+ (match (E.box) ((Full v) v))
                   (* 10 (match (E.box) ((Full v) v))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 87 Int64))
  (call   main (: -4 Int64)) (output (: -34 Int64)))

(case "gs4 the applied generic wraps a HEAP payload — (Container (List Int64)) in the annotation, the list summed after unwrap"
  (input  (do
            (type (Container a) (Full a))
            (def (sum-at (: xs (List Int64)) (: i Int64) (: acc Int64))
              (match (List.at xs i)
                ((Some v) (sum-at xs (+ i 1) (+ acc v)))
                ((None) acc)))
            (def (unwrap-sum (: b (Container (List Int64))))
              (match b ((Full xs) (sum-at xs 0 0))))
            (def (main (: k Int64)) (unwrap-sum (Full (list k 7 1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 13 Int64))
  (call   main (: -9 Int64)) (output (: -1 Int64)))

(case "gs5 the generic applied to ITSELF — (Container (Container Int64)) double-wraps and double-unwraps"
  (input  (do
            (type (Container a) (Full a))
            (def (unwrap2 (: b (Container (Container Int64))))
              (match b ((Full inner) (match inner ((Full v) (* 2 v))))))
            (def (main (: k Int64)) (unwrap2 (Full (Full k))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 12 Int64))
  (call   main (: -5 Int64)) (output (: -10 Int64)))

(case "gs6 a TWO-VARIANT generic (Either a b) by name — parity routes construction between Left and Right, the annotated consumer matches both"
  (input  (do
            (type (Either a b) (Left a) (Right b))
            (def (score (: e (Either Int64 Int64)))
              (match e
                ((Left x) (* 10 x))
                ((Right y) y)))
            (def (main (: k Int64))
              (score (if (= (% k 2) 0) (Left k) (Right (+ k 1)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 40 Int64))
  (call   main (: 3 Int64)) (output (: 4 Int64))
  (call   main (: -2 Int64)) (output (: -20 Int64)))

(case "gs7 the applied generic in the op ARGUMENT position — the arm unwraps (Container Int64) payloads into the accumulating state"
  (input  (do
            (type (Container a) (Full a))
            (effect E (op feed (-> (Container Int64) Int64)))
            (def (main (: k Int64))
              (handle E 0
                ((feed (c) s (match c
                               ((Full v) (resume (+ s v) (+ s v))))))
                (+ (* 10 (E.feed (Full k))) (E.feed (Full 5)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 38 Int64))
  (call   main (: -4 Int64)) (output (: -39 Int64)))

(case "gs8 the applied generic AS the handler state — (Container Int64) seeds the handle, the arm unwraps and re-wraps per dispatch"
  (input  (do
            (type (Container a) (Full a))
            (effect E (op tick (-> Int64)))
            (def (main (: k Int64))
              (handle E (Full k)
                ((tick () st (match st
                               ((Full v) (resume v (Full (+ v 3)))))))
                (+ (* 10 (E.tick)) (E.tick))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 47 Int64))
  (call   main (: -1 Int64)) (output (: -8 Int64)))

; ── Symbol modes, width-boundary states, snapshots, and float specials (breaker batch 218) ────
; Four small arcs. sy7: a SYMBOL mode discriminant beside an accumulator — symbol equality routes
; idle→run promotion vs run-mode accumulation, stop resets. Width boundaries: u64s1 threads a
; UInt64 state ABOVE 2^63 (unsigned comparison stays correct as the thread crosses — a signedness
; miscompile would flip every row); i8s1 walks an Int8 state to EXACTLY 127 under a guard (the
; boundary-exact no-trap face). Snapshots: sn1 resumes the WHOLE tuple state as a value; sn2 pins
; snapshot IMMUTABILITY across interleaved bumps; sn3 crosses the snapshot to a DEF-parameter
; consumer; ls1 is the HEAP twin — a list snapshot stays immutable while the state grows past it.
; Float specials through the state thread: fx5 saturates to infinity mid-thread (never-traps in
; arm position); fx6 births NaN in the arm (s2−s2) with canonical equality distinguishing it;
; fx7 threads NEGATIVE ZERO (canonical -0.0 ≠ +0.0; IEEE addition washes the sign; seed via
; (* -1.0 a) — (- 0.0 a) gives +0.0 at zero). All rows hand-computed; all pass on wasm, rust,
; and rust-async.

(case "sy7 a SYMBOL mode BESIDE an accumulator in the tuple state — symbol equality routes the go arm between idle and run, stop resets the mode"
  (input  (do
            (effect E (op go (-> Int64)) (op stop (-> Int64)))
            (def (main (: n Int64))
              (handle E (tuple (Symbol.of "idle") n)
                ((go () st (match st
                             ((tuple m acc)
                              (if (= m (Symbol.of "idle"))
                                  (resume 1 (tuple (Symbol.of "run") acc))
                                  (resume (+ acc 10) (tuple m (+ acc 10)))))))
                 (stop () st (match st
                               ((tuple m acc) (resume acc (tuple (Symbol.of "idle") acc))))))
                (+ (E.go)
                   (+ (* 10 (E.go))
                      (+ (* 100 (E.stop))
                         (* 1000 (E.go)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2651 Int64))
  (call   main (: 0 Int64)) (output (: 2101 Int64))
  (call   main (: -3 Int64)) (output (: 1771 Int64)))

(case "u64s1 a UInt64 handler state ABOVE the i64 boundary — unsigned comparison in the arm stays correct as the thread advances past 2^63"
  (input  (do
            (effect E (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E (if (> n 0) (: 9223372036854775813 UInt64)
                            (if (= n 0) (: 9223372036854775808 UInt64)
                                (: 9223372036854775802 UInt64)))
                ((probe () s
                  (resume (if (> s (: 9223372036854775808 UInt64)) 1 0)
                          (+ s (: 5 UInt64)))))
                (+ (* 10 (E.probe)) (E.probe))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 11 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -6 Int64)) (output (: 0 Int64)))

(case "i8s1 an Int8 handler state walked to the TOP of its range under a guard — the +40 stride stops exactly where one more step would overflow"
  (input  (do
            (effect E (op step (-> Int64)))
            (def (walk (: steps Int64))
              (let ((v (E.step)))
                (if (> v 87) (+ (* 1000 steps) v) (walk (+ steps 1)))))
            (def (main (: n Int8))
              (handle E n
                ((step () s
                  (if (> s (: 87 Int8))
                      (resume (Int64.of s) s)
                      (resume (Int64.of (+ s (: 40 Int8))) (+ s (: 40 Int8))))))
                (walk 0)))
            (export main)))
  (call   main (: 0 Int8)) (output (: 2120 Int64))
  (call   main (: -100 Int8)) (output (: 4100 Int64))
  (call   main (: 87 Int8)) (output (: 127 Int64)))

(case "sn1 a snapshot op resumes the WHOLE tuple state as a value — the body destructures the live pair after mixed bumps"
  (input  (do
            (effect E (op bumpa (-> Int64)) (op bumpb (-> Int64)) (op snap (-> (Tuple Int64 Int64))))
            (def (main (: n Int64))
              (handle E (tuple n 10)
                ((bumpa () st (match st ((tuple a b) (resume a (tuple (+ a 1) b)))))
                 (bumpb () st (match st ((tuple a b) (resume b (tuple a (+ b 5))))))
                 (snap () st (resume st st)))
                (do (E.bumpa) (E.bumpa) (E.bumpb)
                    (match (E.snap)
                      ((tuple a b) (+ (* 100 a) b))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 715 Int64))
  (call   main (: 0 Int64)) (output (: 215 Int64))
  (call   main (: -3 Int64)) (output (: -85 Int64)))

(case "sn2 TWO whole-state snapshots straddle interleaved bumps — the first snapshot is IMMUTABLE, the pair differ by exactly the bumps between them"
  (input  (do
            (effect E (op bumpa (-> Int64)) (op bumpb (-> Int64)) (op snap (-> (Tuple Int64 Int64))))
            (def (main (: n Int64))
              (handle E (tuple n 10)
                ((bumpa () st (match st ((tuple a b) (resume a (tuple (+ a 1) b)))))
                 (bumpb () st (match st ((tuple a b) (resume b (tuple a (+ b 5))))))
                 (snap () st (resume st st)))
                (do (E.bumpa)
                    (let ((s1 (E.snap)))
                      (do (E.bumpb) (E.bumpa)
                          (let ((s2 (E.snap)))
                            (match s1
                              ((tuple a1 b1)
                               (match s2
                                 ((tuple a2 b2)
                                  (+ (+ a1 b1) (* 10 (+ a2 b2)))))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 236 Int64))
  (call   main (: 0 Int64)) (output (: 181 Int64))
  (call   main (: -8 Int64)) (output (: 93 Int64)))

(case "sn3 the snapshot crosses to a DEF helper — the crossed state tuple is consumed by a function parameter, not a body match"
  (input  (do
            (effect E (op bumpa (-> Int64)) (op bumpb (-> Int64)) (op snap (-> (Tuple Int64 Int64))))
            (def (mix (: p (Tuple Int64 Int64)))
              (match p ((tuple a b) (+ (* 2 a) b))))
            (def (main (: n Int64))
              (handle E (tuple n 10)
                ((bumpa () st (match st ((tuple a b) (resume a (tuple (+ a 1) b)))))
                 (bumpb () st (match st ((tuple a b) (resume b (tuple a (+ b 5))))))
                 (snap () st (resume st st)))
                (do (E.bumpa) (E.bumpb) (mix (E.snap)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 27 Int64))
  (call   main (: 0 Int64)) (output (: 17 Int64))
  (call   main (: -4 Int64)) (output (: 9 Int64)))

(case "ls1 a LIST-state snapshot crosses mid-growth — the held list is immutable while the state keeps growing, sum and final length both pinned"
  (input  (do
            (effect E (op push (-> Int64 Int64)) (op snap (-> (List Int64))) (op len (-> Int64)))
            (def (sum-at (: xs (List Int64)) (: i Int64) (: acc Int64))
              (match (List.at xs i)
                ((Some v) (sum-at xs (+ i 1) (+ acc v)))
                ((None) acc)))
            (def (main (: n Int64))
              (handle E (list n)
                ((push (v) xs (resume v (List.push xs v)))
                 (snap () xs (resume xs xs))
                 (len () xs (resume (List.len xs) xs)))
                (do (E.push 7)
                    (let ((s1 (E.snap)))
                      (do (E.push 9)
                          (+ (sum-at s1 0 0) (* 100 (E.len))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 312 Int64))
  (call   main (: 0 Int64)) (output (: 307 Int64))
  (call   main (: -3 Int64)) (output (: 304 Int64)))

(case "fx5 the float state SATURATES to infinity mid-thread — a squaring ladder crosses Float64.max, the arm's finite/inf verdict flips per dispatch"
  (input  (do
            (effect E (op sq (-> Int64)))
            (def (main (: a Float64))
              (handle E a
                ((sq () s
                  (let ((s2 (* s s)))
                    (resume (if (> s2 1.7e308) 1 0) s2))))
                (+ (* 10 (E.sq)) (E.sq))))
            (export main)))
  (call   main (: 1.0e100 Float64)) (output (: 1 Int64))
  (call   main (: 1.0e50 Float64)) (output (: 0 Int64))
  (call   main (: 1.0e200 Float64)) (output (: 11 Int64)))

(case "fx6 NaN born in the ARM — s2−s2 is 0.0 while finite and NaN once the thread saturates; canonical equality distinguishes them per dispatch"
  (input  (do
            (effect E (op step (-> Int64)))
            (def (main (: a Float64))
              (handle E a
                ((step () s
                  (let ((s2 (* s s)))
                    (let ((d (- s2 s2)))
                      (resume (if (= d 0.0) 1 (if (= d Float64.nan) 2 0)) s2)))))
                (+ (* 10 (E.step)) (E.step))))
            (export main)))
  (call   main (: 1.0e100 Float64)) (output (: 12 Int64))
  (call   main (: 1.0e50 Float64)) (output (: 11 Int64)))

(case "fx7 NEGATIVE ZERO through the state thread — canonical equality separates -0.0 from +0.0, and IEEE addition washes the sign out mid-thread"
  (input  (do
            (effect E (op probe (-> Int64)))
            (def (main (: a Float64))
              (handle E (* -1.0 a)
                ((probe () s
                  (resume (if (= s 0.0) 1 0) (+ s 0.0))))
                (+ (* 10 (E.probe)) (E.probe))))
            (export main)))
  (call   main (: 0.0 Float64)) (output (: 1 Int64))
  (call   main (: 5.0 Float64)) (output (: 0 Int64)))


; ── Bytes PAYLOADS built and consumed across dispatch (breaker bp) ────────────────────────────
; Byte frames as first-class dispatch data. bp1: a BYTES payload inside an arm-built Result —
; even states Ok-wrap a one-byte frame built in the arm (Bytes.of + UInt8.wrap), odd states Err;
; the body reads length and first byte. bp2: a Bytes TRANSFORMER op — the body frames a byte,
; the arm decodes/adds-five/re-encodes, the body decodes the transform. bp3: the arm REVERSES a
; three-byte frame by per-index rebuild, positional weights pinning the swap. bp4: two one-byte
; op RESULTS concatenated in the body — the joined frame's length and both bytes decode. All
; rows hand-computed; all pass on wasm, rust, and rust-async.

(case "bp1 a BYTES payload inside an arm-built Result — even states Ok-wrap a one-byte frame, the body reads len and first byte"
  (input  (do
            (type BRes (Ok Bytes) (Err))
            (effect E (op step (-> BRes)))
            (def (main (: n Int64))
              (handle E (if (< n 0) (- 0 n) n)
                ((step () s
                  (resume (if (= (% s 2) 0)
                              (BRes.Ok (Bytes.of (list (UInt8.wrap s))))
                              (BRes.Err))
                          (+ s 1))))
                (match (E.step)
                  ((BRes.Ok b) (+ (* 100 (Bytes.len b))
                                  (match (Bytes.at b 0)
                                    ((Some v) (Int64.of v))
                                    ((None) -9))))
                  ((BRes.Err) -1))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 104 Int64))
  (call   main (: 7 Int64)) (output (: -1 Int64)))

(case "bp2 a Bytes TRANSFORMER op — the body frames a byte, the arm decodes it, adds five, re-encodes; the body decodes the transform"
  (input  (do
            (effect E (op xf (-> Bytes Bytes)))
            (def (main (: n Int64))
              (handle E 0
                ((xf (b) s
                  (match (Bytes.at b 0)
                    ((Some v) (resume (Bytes.of (list (UInt8.wrap (+ (Int64.of v) 5)))) s))
                    ((None) (resume b s)))))
                (match (Bytes.at (E.xf (Bytes.of (list (UInt8.wrap (if (< n 0) (- 0 n) n))))) 0)
                  ((Some v) (Int64.of v))
                  ((None) -9))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 15 Int64))
  (call   main (: -3 Int64)) (output (: 8 Int64)))

(case "bp3 the arm REVERSES a three-byte frame — per-index rebuild through Bytes.at, positional weights pin the swap"
  (input  (do
            (effect E (op rev (-> Bytes Bytes)))
            (def (byte-at (: b Bytes) (: i Int64))
              (match (Bytes.at b i) ((Some v) (Int64.of v)) ((None) 0)))
            (def (main (: n Int64))
              (handle E 0
                ((rev (b) s
                  (resume (Bytes.of (list (UInt8.wrap (byte-at b 2))
                                          (UInt8.wrap (byte-at b 1))
                                          (UInt8.wrap (byte-at b 0))))
                          s)))
                (let ((r (E.rev (Bytes.of (list (UInt8.wrap (if (< n 0) (- 0 n) n))
                                                (UInt8.wrap 20)
                                                (UInt8.wrap 30))))))
                  (+ (* 10000 (byte-at r 0))
                     (+ (* 100 (byte-at r 1))
                        (byte-at r 2))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 302005 Int64))
  (call   main (: -7 Int64)) (output (: 302007 Int64)))

(case "bp4 two one-byte op results CONCATENATED in the body — the joined frame's length and both bytes decode"
  (input  (do
            (effect E (op mk (-> Int64 Bytes)))
            (def (byte-at (: b Bytes) (: i Int64))
              (match (Bytes.at b i) ((Some v) (Int64.of v)) ((None) 0)))
            (def (main (: n Int64))
              (handle E 0
                ((mk (v) s (resume (Bytes.of (list (UInt8.wrap v))) s)))
                (let ((j (Bytes.concat (E.mk (if (< n 0) (- 0 n) n)) (E.mk 42))))
                  (+ (* 1000 (Bytes.len j))
                     (+ (* 10 (byte-at j 0))
                        (- (byte-at j 1) 40))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2052 Int64))
  (call   main (: -9 Int64)) (output (: 2092 Int64)))

; ── Wide op-argument rows (breaker batch 220) ─────────────────────────────────
; A five-parameter op pins the widest arm parameter row in the corpus: per-slot
; weighted folds prove each position lands in its own binder (a swap or clobber
; at any width-5 slot perturbs the weighted sum), and an all-draws argument row
; pins strict left-to-right argument evaluation at width five.

(case "q5 a FIVE-argument op — the arm folds all five positions with distinct weights, two calls permute the arguments"
  (input  (do
            (effect E (op quint (-> Int64 Int64 Int64 Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((quint (a b c d e) s
                  (resume (+ a (+ (* 2 b) (+ (* 3 c) (+ (* 4 d) (+ (* 5 e) s)))))
                          (+ s 1))))
                (+ (* 100 (E.quint 1 2 3 4 5))
                   (E.quint 5 4 3 2 1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 5536 Int64))
  (call   main (: 5 Int64)) (output (: 6041 Int64))
  (call   main (: -3 Int64)) (output (: 5233 Int64)))

(case "q6 all FIVE arguments are draws — left-to-right evaluation order pinned at width five by distinct weights"
  (input  (do
            (effect E (op next (-> Int64))
                      (op quint (-> Int64 Int64 Int64 Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (quint (a b c d e) s
                  (resume (+ a (+ (* 2 b) (+ (* 3 c) (+ (* 4 d) (+ (* 5 e) s)))))
                          (+ s 1))))
                (E.quint (E.next) (E.next) (E.next) (E.next) (E.next))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 45 Int64))
  (call   main (: 3 Int64)) (output (: 93 Int64))
  (call   main (: -2 Int64)) (output (: 13 Int64)))

; ── Site-6 through-block fold: capture + trap-order faces (breaker batch 221) ──
; The Site-6 commuting conversion floats pure let-wrapper bindings out of a
; branch-performing let-init. These pin its two riskiest faces: a wrapper binding
; that SHADOWS an outer binding the body reads (the float must not capture), and
; a wrapper init that can TRAP (division by the argument) — the float must keep
; the trap ordered before the conditional, and the n=0 call still traps.

(case "s6a wrapper binding shadows an outer binding the BODY reads"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (let ((b 10)
                      (v (let ((b 1)) (if (= b 1) (St.get) 99))))
                  (+ (* 100 v) (+ (* 10 b) (St.get))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 404 Int64)))

(case "s6d the floated wrapper init can trap — division by the argument stays ORDERED before the conditional"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (let ((v (let ((t (/ 100 n))) (if (= t 25) (St.get) t))))
                  (+ (* 10 v) (St.get)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 45 Int64))
  (call   main (: 5 Int64)) (output (: 205 Int64))
  (call   main (: 0 Int64)) (trap "divide by zero"))

; ── Open-row projection × effect dispatch (breaker batch 223) ─────────────────
; Row polymorphism's per-call-site slot resolution meets the dispatch boundary:
; a projector applied at two record widths whose rows hold fresh draws, an
; arm-BUILT record projected open-row by the body, and a RECORD handler state
; projected open-row inside the arm itself (the arm's instantiation independent
; of the body's).

(case "or1 an open-row projector applied to TWO record widths where one field is a fresh DRAW — per-call-site slots under effect state"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (get-x r) (. r x))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (+ (get-x (record (= x (St.get))))
                   (* 100 (get-x (record (= a 9) (= x (St.get)) (= z 8)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 403 Int64))
  (call   main (: 0 Int64)) (output (: 100 Int64)))

(case "or2 the handler ARM builds a record and the body projects it open-row at two widths — arm-built rows cross the dispatch boundary"
  (input  (do
            (effect Mk (op pack (-> Int64 (Record (: x Int64) (: t Int64)))))
            (def (get-x r) (. r x))
            (def (main (: n Int64))
              (handle Mk n
                ((pack (a) s (resume (record (= x (* 10 a)) (= t s)) (+ s 1))))
                (let ((r1 (Mk.pack 2))
                      (r2 (Mk.pack 3)))
                  (+ (get-x r1)
                     (+ (* 100 (get-x r2))
                        (+ (* 10000 (. r1 t)) (* 1000000 (. r2 t))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6053020 Int64))
  (call   main (: 0 Int64)) (output (: 1003020 Int64)))

(case "or3 a RECORD handler state projected open-row inside the arm — the arm's row instantiation is independent of the body's"
  (input  (do
            (effect St (op bump (-> Int64 Int64)))
            (def (get-x r) (. r x))
            (def (main (: n Int64))
              (handle St (record (= x n) (= hits 0))
                ((bump (a) s (resume (+ (get-x s) a)
                                     (record (= x (+ (get-x s) a)) (= hits (+ (. s hits) 1))))))
                (let ((b1 (St.bump 10)))
                  (let ((b2 (St.bump 100)))
                    (+ b1 (* 1000 b2))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 113013 Int64))
  (call   main (: 0 Int64)) (output (: 110010 Int64)))

; ── Draw-captures across later state (breaker batch 224) ──────────────────────
; A closure capturing an effect draw freezes the VALUE: cd1 invokes the capture
; only after a later draw advanced state (the capture must not re-read), and cd4
; sends the capture through a higher-order def that applies it twice — capture
; fixed while the call arguments vary, with a post-call draw pinning the thread.

(case "cd1 a closure CAPTURES a draw then is called after LATER draws — the captured value must not re-read state"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (let ((d1 (St.get)))
                  (let ((f (fn (k) (+ (* 100 d1) k))))
                    (let ((d2 (St.get)))
                      (+ (f d2) (* 10000 (St.get))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 50304 Int64))
  (call   main (: 0 Int64)) (output (: 20001 Int64)))

(case "cd4 a captured draw crosses a HIGHER-ORDER def boundary — helper applies the closure twice with different args"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (twice f) (+ (f 1) (* 10 (f 2))))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (let ((d (St.get)))
                  (+ (twice (fn (k) (+ (* 100 d) k))) (* 100000 (St.get))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 403321 Int64))
  (call   main (: 0 Int64)) (output (: 100021 Int64)))

(case "an observed tail-recursive performer keeps constant stack on all three backends"
  (doc    "A source-tail-recursive performer whose OUT-STATE is observed AFTER the recursion. `grow` self-calls
           in the discard-match tail and performs `Acc.push` each iteration; the body observes `(Acc.size)` after
           `(grow n)`, so the multi-value fold upgrades `grow` to return (value, out-state). That upgraded loop
           must still dispatch in CONSTANT stack — the established mandate for tail-recursive performers. Pins
           breaker finding 16: the wasm backend previously did NOT recognize the multi-value-upgraded tail
           self-call (rewritten to a let-bound-call + identity-repackage tuple) as a tail edge, so it pushed a
           frame per iteration and trapped 'call stack exhausted' between depth 5000 and 8000, while rust ran it
           via LLVM TCO; fixed by teaching the wasm loop machinery to recognize the upgraded shape. At depth
           10000 all three backends now run constant-stack. Each push resumes with next-state s+1 (advancing the
           counter) but grow DISCARDS the push result via the `_` arm and returns grow(n-1), so grow(n) = 0; the
           body is (+ 0 (Acc.size)) and after n pushes the counter is n, so main(10000) = 10000. The result value
           is incidental — the case exists to witness that the observed multi-value upgrade dispatches in
           constant stack at scale, not to compute a sum.")
  (input  (do
            (effect Acc (op push (-> Int64 Int64)) (op size (-> Int64)))
            (def (grow (: n Int64))
              (if (< n 1) 0 (match (Acc.push n) (_ (grow (- n 1))))))
            (def (main (: n Int64))
              (handle Acc 0
                ((push (v) s (resume s (+ s 1)))
                 (size () s (resume s s)))
                (let ((g (grow n))) (+ g (Acc.size)))))
            (export main)))
  (call   main (: 10000 Int64)) (output (: 10000 Int64)))

; ── Bytes state under recursion + slice windows (breaker batch 225) ───────────
; A Bytes handler state accumulated by RECURSIVE dispatches (one byte per hop,
; rope growth in the state thread), and an arm answering a slice WINDOW over the
; growing state — the by1/by2 straight-line pins' recursive and windowed twins.

(case "br1 a BYTES handler state grows one byte per recursive dispatch — the final frame's length and bytes pin the accumulation"
  (input  (do
            (effect Acc
              (op push (-> Int64 Int64))
              (op dump (-> Bytes)))
            (def (walk (: k Int64))
              (if (= k 0)
                  0
                  (match (Acc.push k) (_ (walk (- k 1))))))
            (def (at-or (: b Bytes) (: i Int64))
              (match (Bytes.at b i) ((Some v) v) ((None u) -1)))
            (def (main (: n Int64))
              (handle Acc (Bytes.of (list))
                ((push (v) s (resume (Bytes.len s) (Bytes.concat s (Bytes.of (list (UInt8.wrap (+ 60 v)))))))
                 (dump () s (resume s s)))
                (match (walk n)
                  (_ (let ((b (Acc.dump)))
                       (+ (* 1000 (Bytes.len b))
                          (+ (* 10 (at-or b 0))
                             (at-or b (- (Bytes.len b) 1)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3691 Int64)))

(case "br2 the arm returns a SLICE of the growing Bytes state — a window over rope-accumulated bytes crosses dispatch"
  (input  (do
            (effect Acc
              (op push (-> Int64 Bytes)))
            (def (at-or (: b Bytes) (: i Int64))
              (match (Bytes.at b i) ((Some v) v) ((None u) -1)))
            (def (main (: n Int64))
              (handle Acc (Bytes.of (list (UInt8.wrap 5) (UInt8.wrap 6)))
                ((push (v) s
                  (let ((grown (Bytes.concat s (Bytes.of (list (UInt8.wrap v))))))
                    (match (Bytes.slice grown 1 (- (Bytes.len grown) 1))
                      ((Some w) (resume w grown))
                      ((None u) (resume grown grown))))))
                (let ((w1 (Acc.push 40)))
                  (let ((w2 (Acc.push 50)))
                    (+ (* 100000 (Bytes.len w1))
                       (+ (* 10000 (at-or w1 0))
                          (+ (* 100 (Bytes.len w2))
                             (+ (* 10 (at-or w2 0)) (at-or w2 (- (Bytes.len w2) 1))))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 260410 Int64)))

(case "a computed-index String.at over an effect-grown rope emits valid wasm"
  (doc    "Pins breaker finding 18 (v-wasm-opt fix 27aba9cc5). A String handler state grown by a RECURSIVE
           effect-dispatch walk, then a String.at whose op ARGUMENT is a COMPUTED index. The wasm emit
           previously floored both the string operand and the index operand at the same fixed scratch slot;
           when the string traces to the multi-value-upgraded effect-state thread its transient scratch
           recorded that slot as an i32 rope handle, and the computed i64 index then reused the same slot,
           producing an invalid component (expected i32 found i64). Fixed by floating the index-operand
           floor to floor.max(high) in Core::StrAt, mirroring String.slice. rust and rust-async ran it green
           throughout; this pins the wasm emit valid. walk grows the rope to n copies of 'z'; pick reads the
           last with a computed index (n-1) and returns its byte-len 1.")
  (input  (do
            (effect S (op add (-> Int64 Int64)) (op pick (-> Int64 Int64)))
            (def (walk (: k Int64))
              (if (< k 1) 0 (let ((_d (S.add k))) (walk (- k 1)))))
            (def (main (: n Int64))
              (handle S ""
                ((add (v) s (resume 0 (String.concat s "z")))
                 (pick (i) s
                  (resume (match (String.at s i)
                            ((Some c) (String.byte-len c))
                            ((None _u) -1))
                          s)))
                (let ((_w (walk n)))
                  (S.pick (- n 1)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 1 Int64))
  (call   main (: 1 Int64)) (output (: 1 Int64)))

(case "a computed-index Bytes.at over a to-bytes view of an effect-grown rope emits valid wasm"
  (doc    "Second face of breaker finding 18 (same fix 27aba9cc5). The to-bytes-view computed Bytes.at read
           lowers through a DISTINCT emit site Core::BytesAt, which had the IDENTICAL fixed-floor bug: the
           String.to-bytes rope view over the effect-state thread is an i32 handle at the shared floor and the
           computed i64 index reused its slot, yielding invalid wasm. Fixed by the same float-the-index-floor
           treatment in Core::BytesAt alongside Core::StrAt. This pins the second emit site so a future edit
           that re-fixes only one of the two reopens the class here. The rope grows to n copies of 'z' (ASCII
           122); Bytes.at at the computed index (n-1) returns 122.")
  (input  (do
            (effect S (op add (-> Int64 Int64)) (op pick (-> Int64 Int64)))
            (def (walk (: k Int64))
              (if (< k 1) 0 (let ((_d (S.add k))) (walk (- k 1)))))
            (def (main (: n Int64))
              (handle S ""
                ((add (v) s (resume 0 (String.concat s "z")))
                 (pick (i) s
                  (resume (match (Bytes.at (String.to-bytes s) i)
                            ((Some b) b)
                            ((None _u) -1))
                          s)))
                (let ((_w (walk n)))
                  (S.pick (- n 1)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 122 Int64))
  (call   main (: 1 Int64)) (output (: 122 Int64)))

(case "an aborting arm's unread op-argument still threads its foreign perform's state advance"
  (doc    "Pins the ABORTING-arm face of the strict-fold #17 foreign-perform-in-unread-arg fix (v-effects:
           resumptive f4be3f419, aborting-arm residual 5a0ceaf12). An op argument that performs a SECOND
           in-scope effect is observable at the perform site regardless of whether the handling arm reads it.
           Here A's `bail` arm ABORTS (returns 55, never resumes), and its unread argument is `(B.tick)` under
           an outer B counter. The B.tick MUST still advance B's state at the perform site before the abort
           discards A's continuation. Pre-fix the abort path's beta-substitution dropped the argument's
           dispatch, so the later `(B.tick)` read the stale seed; fixed by threading the aborting arm's unread
           op-arg foreign perform via the same #cv lift the resumptive path uses. Seeded n: A.bail aborts to 55
           (times 1000 = 55000), the argument B.tick advances n→n+1 returning n, then the outer B.tick returns
           n+1, so main(n) = 55000 + (n+1). main(3) = 55004, main(0) = 55001 — the +1 over the pre-fix 55003/
           55000 witnesses the foreign advance fired.")
  (input  (do
            (effect A (op bail (-> Int64 Int64)))
            (effect B (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle B n
                ((tick () t (resume t (+ t 1))))
                (+ (* 1000 (handle A 0
                             ((bail (v) s 55))
                             (+ 7777 (A.bail (B.tick)))))
                   (B.tick))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 55004 Int64))
  (call   main (: 0 Int64)) (output (: 55001 Int64)))
; ── Width-3 mutual SCC + Bool recursion guards (breaker batch 226) ────────────
; The landed group multi-value fold pin is a WIDTH-2 mutual SCC; tm2 pins width
; 3 (two tail legs route by VALUE, the third combines the cycle result with a
; post-put draw). bg1 pins a Bool DRAW as the recursion CONDITION itself — the
; guard-position consumer, distinct from the landed value-ladder pins.

(case "tm2 a THREE-function mutual SCC — two tail legs route by value, the third combines the cycle result with a post-put draw"
  (input  (do
            (effect St (op get (-> Unit Int64)) (op put (-> Int64 Unit)))
            (def (fa (: n Int64)) (if (= n 0) (St.get) (fb n)))
            (def (fb (: n Int64)) (if (= n 1) (St.get) (fc n)))
            (def (fc (: n Int64))
              (let ((child (fa (- n 1))))
                (match (St.put n) (_ (+ child (St.get))))))
            (def (main (: k Int64))
              (handle St 0
                ((get (u) s (resume s s))
                 (put (v) s (resume unit (+ s v))))
                (fa k)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 7 Int64))
  (call   main (: 5 Int64)) (output (: 30 Int64)))

(case "bg1 a BOOL op result is the recursion CONDITION itself — the walk continues while draws stay true"
  (input  (do
            (effect T (op more (-> Bool)) (op tick (-> Int64)))
            (def (walk (: acc Int64))
              (if (T.more)
                  (walk (+ acc (T.tick)))
                  acc))
            (def (main (: n Int64))
              (handle T n
                ((more () s (resume (< s 4) s))
                 (tick () s (resume s (+ s 1))))
                (+ (* 10 (walk 0)) (T.tick))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 64 Int64))
  (call   main (: 4 Int64)) (output (: 4 Int64)))

; ── Short-circuit guards + nested-CHAMP state (breaker batch 227) ─────────────
; bg2 pins a short-circuit AND recursion guard whose RIGHT operand is a state-
; advancing Bool draw (the left bound runs out or the draw stops the walk);
; m2m1 pins a Map-of-Maps handler state — the counters-by-category shape with
; per-dispatch two-level rebuild and old-cell answers.

(case "bg2 the recursion guard is an AND of a bool draw and a pure bound check — short-circuit must not skip the draw's state advance observation"
  (input  (do
            (effect T (op odd (-> Bool)) (op tick (-> Int64)))
            (def (walk (: k Int64) (: acc Int64))
              (if (and (< k 6) (T.odd))
                  (walk (+ k 1) (+ (* 10 acc) (T.tick)))
                  acc))
            (def (main (: n Int64))
              (handle T n
                ((odd () s (resume (= (% s 2) 1) (+ s 1)))
                 (tick () s (resume s (+ s 1))))
                (+ (* 100 (walk 0 0)) (T.tick))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 24691213 Int64))
  (call   main (: 2 Int64)) (output (: 3 Int64)))

(case "m2m1 a MAP-OF-MAPS handler state — each dispatch bumps a (category key) cell, the drain reads across both levels"
  (input  (do
            (effect Tally
              (op bump (-> Int64 Int64 Int64))
              (op read (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle Tally (Map.insert Map.empty 1 (Map.insert Map.empty 10 n))
                ((bump (k j) s
                  (let ((inner (match (Map.lookup s k) ((Some im) im) ((None u) Map.empty))))
                    (let ((old (match (Map.lookup inner j) ((Some v) v) ((None u) 0))))
                      (resume old (Map.insert s k (Map.insert inner j (+ old 1)))))))
                 (read (k j) s
                  (resume (match (Map.lookup s k)
                            ((Some im) (match (Map.lookup im j) ((Some v) v) ((None u) -1)))
                            ((None u) -2))
                          s)))
                (let ((a (Tally.bump 1 10)))
                  (let ((b (Tally.bump 2 20)))
                    (let ((c (Tally.bump 2 20)))
                      (+ (* 1000000 a)
                         (+ (* 10000 b)
                            (+ (* 100 c)
                               (+ (* 10 (Tally.read 1 10)) (Tally.read 2 20))))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5000162 Int64))
  (call   main (: 0 Int64)) (output (: 112 Int64)))

; ── Abort-after-advance + arm-built value-eq (breaker batch 228) ──────────────
; ag3 pins the abort-arm reading the ADVANCED state after one resumptive
; dispatch (the fold's 1-dispatch-before-abort boundary, green side); dh1 pins
; value-eq on two ARM-BUILT lists per dispatch — the owned-handle reclaim runs
; once per dispatch while the state advances (drop-hoist regression guard).

(case "ag3 one resumptive dispatch then an ABORT on the same handler — the abort arm reads the advanced state"
  (input  (do
            (effect St (op put (-> Int64 Int64)) (op halt (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((put (v) s (resume s (+ s v)))
                 (halt (u) s (* 100 s)))
                (match (St.put n) (_ (+ 7777 (St.halt))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 300 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64))
  (call   main (: -4 Int64)) (output (: -400 Int64)))

(case "dh1 value-eq on two ARM-BUILT lists inside a dispatch — the borrowed-handle reclaim after = must not double-drop"
  (input  (do
            (effect Q (op probe (-> Int64 Bool)))
            (def (main (: n Int64))
              (handle Q n
                ((probe (v) s
                  (resume (= (list v (+ v 1)) (list s (+ s 1))) (+ s 1))))
                (+ (if (Q.probe n) 1 0)
                   (+ (* 10 (if (Q.probe n) 1 0))
                      (* 100 (if (Q.probe (+ n 2)) 1 0))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 101 Int64))
  (call   main (: 0 Int64)) (output (: 101 Int64)))

; ── List-of-lists state + set algebra across dispatch (breaker batch 229) ─────
; ll1 pins a (List (List Int64)) handler state — rows appended per dispatch,
; two-level element reads via nested Option matches in the arm; sga1 pins
; Set.union/intersection/difference computed in the arm against the threaded
; state (an argument-built set per dispatch; a grow between two probes flips
; the same probe's answer).

(case "ll1 a LIST-OF-LISTS handler state — each dispatch appends a fresh row, the drain reads a middle row's middle element"
  (input  (do
            (effect Rows (op add (-> Int64 Int64)) (op pick (-> Int64 Int64 Int64)))
            (def (row-at (: xss (List (List Int64))) (: i Int64) (: j Int64))
              (match (List.at xss i)
                ((Some xs) (match (List.at xs j) ((Some v) v) ((None _u) -1)))
                ((None _u) -2)))
            (def (main (: n Int64))
              (handle Rows (list)
                ((add (v) s (resume (List.len s) (List.push s (list v (* v 10) (* v 100)))))
                 (pick (i j) s (resume (row-at s i j) s)))
                (let ((a (Rows.add n)))
                  (let ((b (Rows.add (+ n 1))))
                    (let ((c (Rows.add (+ n 2))))
                      (+ (* 10000 (Rows.pick 1 1))
                         (+ (* 100 (Rows.pick 2 0)) (Rows.pick 0 2))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 400800 Int64))
  (call   main (: 0 Int64)) (output (: 100200 Int64)))

(case "sga1 the arm answers with SET ALGEBRA over its state and an argument-built set — union, intersection, and difference sizes cross dispatch"
  (input  (do
            (effect S (op probe (-> Int64 Int64)) (op grow (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (Set.of (list n (+ n 2)))
                ((probe (v) s
                  (let ((arg (Set.of (list v (+ v 1)))))
                    (resume (+ (* 100 (Set.len (Set.union s arg)))
                               (+ (* 10 (Set.len (Set.intersection s arg)))
                                  (Set.len (Set.difference s arg))))
                            s)))
                 (grow (v) s (resume (Set.len s) (Set.insert s v))))
                (let ((a (S.probe n)))
                  (let ((b (S.grow (+ n 1))))
                    (let ((c (S.probe n)))
                      (+ (* 10000 a) (+ (* 100 b) c)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3110521 Int64))
  (call   main (: 0 Int64)) (output (: 3110521 Int64)))

(case "nested recursive performers through a non-recursive intermediate — out-state threads across the hop"
  (doc    "The INDIRECTION face of finding #19 (breaker/corpus-bugfix): the recursion-boundary out-state
           demand must reach a recursive performer even when an intervening NON-recursive helper (`via`) sits
           between the enclosing recursive `outer` and the recursive `inner`. Pre-fix (before `5a788c845`) the
           transitive closure stopped at the direct callee — `via` being non-recursive broke the chain, so
           `outer` stayed single-value and dropped `inner`'s S.tick advances every outer iteration (a silent
           wrong value: 9 not 7). `5a788c845` makes the reach transitive THROUGH non-recursive helpers to the
           recursive leaf, so the composed shape no longer SILENTLY MISCOMPILES. Threading it to the final
           value is the same cross-def recursion-boundary fold follow-on as the direct face; until then it
           DECLINES cleanly (the honest not-yet-reducible todo) rather than the pre-fix silent wrong value.
           Correct value pinned: main(1)=7. Depth-general (a two-hop via1→via2→inner drops identically pre-fix);
           a straight-line non-recursive helper folds fine (it reaches no recursive leaf). Uniform on all 3
           backends.")
  (input  (do
            (effect S (op depth (-> Int64)) (op tick (-> Int64)))
            (def (inner (: k Int64) (: acc Int64))
              (if (< k 1) acc (inner (- k 1) (+ acc (S.tick)))))
            (def (via (: k Int64)) (inner k 0))
            (def (outer (: k Int64) (: acc Int64))
              (if (< k 1) acc
                (let ((d (S.depth)))
                  (outer (- k 1) (+ acc (via d))))))
            (def (main (: n Int64))
              (handle S n
                ((depth () s (resume (% s 3) (+ s 1)))
                 (tick () s (resume s (+ s 1))))
                (outer 3 0)))
            (export main)))
  (call   main (: 1 Int64)) (output (: 7 Int64)))
; ── Perform-per-iteration scale + observed-loop values (breaker batch 230) ────
; ps1 pins a 100k-iteration tail loop PERFORMING every iteration (unobserved —
; constant stack; the observed twin is the breaker-16 sentinel); px1 pins the
; observed loop's VALUES at shallow depths — the finding-16 divergence was
; stack-only, never value corruption.

(case "ps1 a 100k-iteration tail loop that PERFORMS every iteration — dispatch itself must run in constant stack"
  (input  (do
            (effect Ctr (op next (-> Int64)))
            (def (loop (: n Int64) (: acc Int64))
              (if (< n 1) acc (loop (- n 1) (+ acc (Ctr.next)))))
            (def (main (: n Int64))
              (handle Ctr 0
                ((next () s (resume s (+ s 1))))
                (loop n 0)))
            (export main)))
  (call   main (: 100000 Int64)) (output (: 4999950000 Int64))
  (call   main (: 3 Int64)) (output (: 3 Int64)))

(case "px1 observed tail performer at depth 100 — values must agree while the stack face is under repair"
  (input  (do
            (effect Acc (op push (-> Int64 Int64)) (op size (-> Int64)))
            (def (grow (: n Int64))
              (if (< n 1) 0 (match (Acc.push n) (_ (grow (- n 1))))))
            (def (main (: n Int64))
              (handle Acc 0
                ((push (v) s (resume s (+ s 1)))
                 (size () s (resume s s)))
                (let ((g (grow n))) (+ (* 10 g) (Acc.size)))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 100 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64))
  (call   main (: 7 Int64)) (output (: 7 Int64)))

; ── Mixed-endian dispatch + next-state trap (breaker batch 232) ───────────────
; me1/me2 pin mixed-endian frames ACROSS the dispatch boundary (per-segment
; byte order surviving the crossing in both directions, including an arm that
; re-encodes with SWAPPED endianness); nst1 pins a trap in the arm's NEXT-STATE
; expression firing on the dispatch whose poisoned state a later dispatch
; consumes (the consumed face; the unconsumed faces are the strict-fold family).

(case "me1 a MIXED-endian frame crosses the dispatch boundary as an op ARGUMENT — the arm decodes big and little fields independently"
  (input  (do
            (effect Codec (op parse (-> Bytes Int64)))
            (def (main (: n Int64))
              (handle Codec 0
                ((parse (frame) s
                  (match frame
                    ((bin (u16 x) (u16 y le)) (resume (+ (* 100000 x) y) s))
                    (_other (resume -1 s)))))
                (Codec.parse (bin (u16 (UInt16.wrap n)) (u16 (UInt16.wrap (+ n 514)) le)))))
            (export main)))
  (call   main (: 258 Int64)) (output (: 25800772 Int64))
  (call   main (: 0 Int64)) (output (: 514 Int64)))

(case "me2 the arm RE-ENCODES its two decoded fields with SWAPPED endianness and the body decodes the swap"
  (input  (do
            (effect Codec (op flip (-> Bytes Bytes)))
            (def (main (: n Int64))
              (handle Codec 0
                ((flip (frame) s
                  (match frame
                    ((bin (u16 x) (u16 y le))
                      (resume (bin (u16 (UInt16.wrap x) le) (u16 (UInt16.wrap y))) s))
                    (_other (resume frame s)))))
                (match (Codec.flip (bin (u16 (UInt16.wrap n)) (u16 (UInt16.wrap (+ n 3)) le)))
                  ((bin (u16 a le) (u16 b)) (+ (* 100000 a) b))
                  (_other -1))))
            (export main)))
  (call   main (: 258 Int64)) (output (: 25800261 Int64))
  (call   main (: 500 Int64)) (output (: 50000503 Int64)))

(case "nst1 the arm's NEXT-STATE expression divides by a state-derived quantity — both signs thread, the zero seed traps"
  (input  (do
            (effect St (op step (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((step () s (resume s (/ 100 (- s 4)))))
                (+ (* 10 (St.step)) (St.step))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 110 Int64))
  (call   main (: 4 Int64)) (trap "divide by zero")
  (call   main (: 2 Int64)) (output (: -30 Int64)))

; ── Compound transformers + verdict flips (breaker batch 233) ─────────────────
; t2t1 pins a tuple-to-tuple transformer op (compound in AND out) chained twice;
; nl2 pins the nested-record match binder in LIST-ELEMENT position with a rest
; binder (canonical (= x a) pattern-field spelling); rsw1 pins the arm's Ok/Err
; verdict FLIPPING between two identical performs as the state passes the
; payload, including the exact flip-boundary seed.

(case "t2t1 a TUPLE-to-TUPLE transformer op chained twice — the arm swaps components and salts with the state, both crossings exact"
  (input  (do
            (effect S (op swap2 (-> (Tuple Int64 Int64) (Tuple Int64 Int64))))
            (def (main (: n Int64))
              (handle S 0
                ((swap2 (p) s
                  (match p ((tuple a b) (resume (tuple (+ b s) a) (+ s 1))))))
                (match (S.swap2 (tuple n 20))
                  ((tuple x y)
                    (match (S.swap2 (tuple x y))
                      ((tuple u v) (+ (* 1000 u) v)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 4020 Int64))
  (call   main (: 0 Int64)) (output (: 1020 Int64)))

(case "nl2 the nested-record binder in LIST-ELEMENT pattern position — the head record's field binds, the rest carries full records"
  (input  (do
            (def (main (: n Int64))
              (let ((xs (list (record (= x n) (= y 2)) (record (= x 7) (= y 8)))))
                (match xs
                  ((list (record (= x a)) .. rest)
                    (+ (* 1000 a)
                       (+ (* 10 (List.len rest))
                          (match (List.at rest 0)
                            ((Some r2) (. r2 y))
                            ((None _u) -1)))))
                  (_other -9))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3018 Int64))
  (call   main (: 0 Int64)) (output (: 18 Int64)))

(case "rsw1 the arm's Ok/Err verdict FLIPS between two identical performs as the state passes the payload — both variants cross and unwrap"
  (input  (do
            (effect S (op step (-> Int64 (Result Int64 Int64))))
            (def (unwrap-or (: r (Result Int64 Int64)) (: d Int64))
              (match r ((Ok v) v) ((Err e) (+ d e))))
            (def (main (: n Int64))
              (handle S n
                ((step (v) s
                  (resume (if (< v s) (Ok (* v 10)) (Err (- v s))) (+ s 1))))
                (+ (* 1000 (unwrap-or (S.step 3) -100))
                   (unwrap-or (S.step 3) -100))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 30030 Int64))
  (call   main (: 2 Int64)) (output (: -99100 Int64))
  (call   main (: 3 Int64)) (output (: -99970 Int64)))

; ── Shadow-forward faces + compact of the grown rope (breaker batch 234) ──────
; sf1/sf2 pin an inner SAME-effect shadow whose arm RE-PERFORMS the effect it
; discharges (routing outward past itself) — in the resume-value and next-state
; positions; bcp1 pins Bytes.compact of the effect-grown rope inside the arm
; with a computed-index read of the flat rep and a re-thread.

(case "sf1 an inner SAME-effect handler's arm re-performs the effect it discharges — the re-perform routes to the OUTER handler, both states advance independently"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (handle St 100
                  ((get () t (resume (+ (St.get) t) (+ t 10))))
                  (+ (St.get) (* 1000 (St.get))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 114103 Int64))
  (call   main (: 0 Int64)) (output (: 111100 Int64)))

(case "sf2 the inner arm's NEXT-STATE re-performs to the outer — the forward sits in the state-thread position, not the resume value"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (handle St 100
                  ((get () t (resume t (+ t (St.get)))))
                  (+ (St.get) (* 1000 (St.get))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 103100 Int64))
  (call   main (: 0 Int64)) (output (: 100100 Int64)))

(case "bcp1 Bytes.compact of the EFFECT-GROWN rope inside the arm — the compacted flat rep reads exactly at a computed index and re-threads"
  (input  (do
            (effect S (op add (-> Int64 Int64)) (op flat (-> Int64 Int64)))
            (def (walk (: k Int64))
              (if (< k 1) 0 (let ((_d (S.add k))) (walk (- k 1)))))
            (def (main (: n Int64))
              (handle S (Bytes.of (list))
                ((add (v) s (resume 0 (Bytes.concat s (Bytes.of (list (UInt8.wrap (+ 60 v)))))))
                 (flat (i) s
                  (let ((c (Bytes.compact s)))
                    (resume (+ (* 100 (Bytes.len c))
                               (match (Bytes.at c i) ((Some v) v) ((None _u) -1)))
                            c))))
                (let ((_w (walk n)))
                  (S.flat (- n 1)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 461 Int64))
  (call   main (: 1 Int64)) (output (: 161 Int64)))

; ── Per-branch strides + square-and-multiply (breaker batch 235) ──────────────
; pbr1/pbr2 pin the arm resuming under a CONDITIONAL where each branch carries
; a DIFFERENT value AND stride (if-branches with a 50-jump that flips the next
; route; a three-way match by residue class); sqm1 pins the square-and-multiply
; modular-exponent kernel — (base,acc) squares per dispatch, 1-bits multiply.

(case "pbr1 PER-BRANCH resume with different value AND stride — the else branch jumps the state 50, flipping the next dispatch's route"
  (input  (do
            (effect S (op route (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S n
                ((route (v) s
                  (if (< v s)
                      (resume (* v 10) (+ s 1))
                      (resume (+ v 100) (+ s 50)))))
                (+ (S.route 3) (* 1000 (S.route 3)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 30030 Int64))
  (call   main (: 1 Int64)) (output (: 30103 Int64))
  (call   main (: 3 Int64)) (output (: 30103 Int64)))

(case "pbr2 THREE-way match-branch resumes — each residue class answers and strides differently, three dispatches walk the classes"
  (input  (do
            (effect S (op step (-> Int64)))
            (def (main (: n Int64))
              (handle S n
                ((step () s
                  (match (% s 3)
                    (0 (resume (* s 100) (+ s 1)))
                    (1 (resume (- 0 s) (+ s 2)))
                    (_ (resume s (+ s 3))))))
                (+ (S.step) (+ (* 100 (S.step)) (* 100000 (S.step))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 29999900 Int64))
  (call   main (: 1 Int64)) (output (: -370001 Int64))
  (call   main (: 2 Int64)) (output (: 800502 Int64)))

(case "sqm1 SQUARE-AND-MULTIPLY over the state — (base,acc) squares every dispatch, multiplies on 1-bits mod 1000, the n-bit's effect observed by a final read"
  (input  (do
            (effect S (op bit (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (tuple 3 1)
                ((bit (b) st
                  (match st
                    ((tuple base acc)
                      (resume acc
                              (tuple (% (* base base) 1000)
                                     (if (= b 1) (% (* acc base) 1000) acc)))))))
                (let ((_a (S.bit 1)))
                  (let ((_b (S.bit 0)))
                    (let ((_c (S.bit 1)))
                      (let ((_d (S.bit n)))
                        (S.bit 0)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 243 Int64))
  (call   main (: 1 Int64)) (output (: 323 Int64)))

; ── Algorithm-trace states (breaker batch 236) ────────────────────────────────
; Three classic algorithms as handler compositions: cz1 a COLLATZ walk observing
; every step (data-dependent dispatch counts per seed, budget-guarded); gcd1
; EUCLID with a logged divisor chain; fib1 the FIBONACCI recurrence as a tuple
; transition — (a,b) -> (b,a+b), both fields reordered per hop.

(case "cz1 COLLATZ-driven dispatch counting — the walk observes each step, the counter state tallies data-dependent iteration counts"
  (input  (do
            (effect S (op obs (-> Int64 Int64)) (op count (-> Int64)))
            (def (collatz (: x Int64) (: k Int64))
              (if (< k 1) x
                (if (= x 1) x
                  (let ((_o (S.obs x)))
                    (collatz (if (= (% x 2) 0) (/ x 2) (+ (* 3 x) 1)) (- k 1))))))
            (def (main (: n Int64))
              (handle S 0
                ((obs (v) c (resume v (+ c 1)))
                 (count () c (resume c c)))
                (let ((r (collatz n 30)))
                  (+ (* 1000 r) (S.count)))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 1008 Int64))
  (call   main (: 7 Int64)) (output (: 1016 Int64))
  (call   main (: 1 Int64)) (output (: 1000 Int64)))

(case "gcd1 EUCLID with a logged trace — each remainder step performs, the accumulator sums the divisor chain, data-dependent step counts"
  (input  (do
            (effect S (op log (-> Int64 Int64)) (op sum (-> Int64)))
            (def (gcd (: a Int64) (: b Int64) (: k Int64))
              (if (< k 1) a
                (if (= b 0) a
                  (let ((_l (S.log b)))
                    (gcd b (% a b) (- k 1))))))
            (def (main (: n Int64))
              (handle S 0
                ((log (v) acc (resume v (+ acc v)))
                 (sum () acc (resume acc acc)))
                (let ((g (gcd n 12 20)))
                  (+ (* 1000 g) (S.sum)))))
            (export main)))
  (call   main (: 18 Int64)) (output (: 6018 Int64))
  (call   main (: 35 Int64)) (output (: 1024 Int64))
  (call   main (: 12 Int64)) (output (: 12012 Int64)))

(case "fib1 the FIBONACCI recurrence as a state transition — (a,b) becomes (b,a+b) per dispatch, five draws walk the sequence"
  (input  (do
            (effect S (op next (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple 0 1)
                ((next () st
                  (match st ((tuple a b) (resume a (tuple b (+ a b)))))))
                (let ((f1 (S.next)))
                  (let ((_f2 (S.next)))
                    (let ((_f3 (S.next)))
                      (let ((_f4 (S.next)))
                        (let ((f5 (S.next)))
                          (+ (* 1000 f5) (+ (* 10 f1) n)))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 3000 Int64))
  (call   main (: 7 Int64)) (output (: 3007 Int64)))

; --- breaker batch 237: run-length 3-tuple state, negative digit-peel (truncated div/rem), parity-alternating two-effect draws ---
(case "rle1 RUN-LENGTH tracking — the 3-tuple state carries (last,run,best); the n=5 seed extends one run to five, n=7 breaks it at two"
  (input  (do
            (effect S (op feed (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (tuple 0 0 0)
                ((feed (v) st
                  (match st
                    ((tuple last run best)
                      (let ((nrun (if (= v last) (+ run 1) 1)))
                        (resume nrun (tuple v nrun (if (> nrun best) nrun best))))))))
                (let ((_a (S.feed 5)))
                  (let ((_b (S.feed 5)))
                    (let ((_c (S.feed n)))
                      (let ((_d (S.feed n)))
                        (S.feed n)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64))
  (call   main (: 7 Int64)) (output (: 3 Int64)))

(case "dgn1 digit-peel of a NEGATIVE state — truncated division and dividend-sign remainder agree through the thread, three negative digits"
  (input  (do
            (effect S (op digit (-> Int64)))
            (def (main (: n Int64))
              (handle S n
                ((digit () s (resume (% s 10) (/ s 10))))
                (let ((d1 (S.digit)))
                  (let ((d2 (S.digit)))
                    (let ((d3 (S.digit)))
                      (+ (* 100 d1) (+ (* 10 d2) d3)))))))
            (export main)))
  (call   main (: -251 Int64)) (output (: -152 Int64))
  (call   main (: -8 Int64)) (output (: -800 Int64)))

(case "pal1 PARITY-ALTERNATING two-effect draws — one recursive driver picks which effect to draw per hop, both threads advance interleaved"
  (input  (do
            (effect A (op get (-> Int64)))
            (effect B (op get (-> Int64)))
            (def (walk (: k Int64) (: acc Int64))
              (if (< k 1) acc
                (walk (- k 1) (+ (* 10 acc) (if (= (% k 2) 0) (A.get) (B.get))))))
            (def (main (: n Int64))
              (handle A n
                ((get () s (resume s (+ s 1))))
                (handle B 50
                  ((get () t (resume t (+ t 2))))
                  (walk 4 0))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 6072 Int64))
  (call   main (: 8 Int64)) (output (: 13142 Int64)))

; --- breaker batch 238: finding-21 regression fence — computed perform args x two-lookup-match arms
; (width-partition fix aaee597d5): multimap, scalar-map, checked-shift arg, three-param op,
; record-wrapped state, abort path, chained keys, cross-handler ---
(case "mml1 a MULTIMAP handler state — (Map Int64 (List Int64)) buckets grow per-key across dispatches; put answers the new bucket length and total sums a bucket later"
  (input  (do
            (effect S
              (op put (-> Int64 Int64 Int64))
              (op total (-> Int64 Int64)))
            (def (sum-list (: xs (List Int64)) (: i Int64) (: acc Int64))
              (match (List.at xs i)
                ((Some v) (sum-list xs (+ i 1) (+ acc v)))
                ((None u) acc)))
            (def (append-at (: m (Map Int64 (List Int64))) (: k Int64) (: v Int64))
              (match (Map.lookup m k)
                ((Some xs) (Map.insert m k (List.push xs v)))
                ((None u) (Map.insert m k (list v)))))
            (def (bucket-len (: m (Map Int64 (List Int64))) (: k Int64))
              (match (Map.lookup m k) ((Some xs) (List.len xs)) ((None u) 0)))
            (def (bucket-sum (: m (Map Int64 (List Int64))) (: k Int64))
              (match (Map.lookup m k) ((Some xs) (sum-list xs 0 0)) ((None u) 0)))
            (def (main (: n Int64))
              (handle S Map.empty
                ((put (k v) m (let ((m2 (append-at m k v))) (resume (bucket-len m2 k) m2)))
                 (total (k) m (resume (bucket-sum m k) m)))
                (let ((a (S.put n n)))
                  (let ((b (S.put (+ n 1) (* 2 n))))
                    (let ((c (S.put n (+ n 10))))
                      (let ((d (S.total n)))
                        (let ((e (S.total (+ n 1))))
                          (let ((f (S.total 99)))
                            (+ (* 10 (+ (* 100 (+ (* 100 (+ (* 10 (+ (* 10 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 11216060 Int64))
  (call   main (: 7 Int64)) (output (: 11224140 Int64)))

(case "mk21 a scalar-map handler whose second put uses a COMPUTED key — the checked-add scratch and the Option-handle slot stay width-partitioned"
  (input  (do
            (effect S (op put (-> Int64 Int64 Int64)))
            (def (append-at (: m (Map Int64 Int64)) (: k Int64) (: v Int64))
              (match (Map.lookup m k)
                ((Some x) (Map.insert m k v))
                ((None u) (Map.insert m k v))))
            (def (main (: n Int64))
              (handle S Map.empty
                ((put (k v) m (let ((m2 (append-at m k v)))
                  (resume (match (Map.lookup m2 k) ((Some x) x) ((None u) 0)) m2))))
                (let ((a (S.put n n)))
                  (let ((b (S.put (+ n 1) (* 2 n))))
                    (+ (* 10 a) b)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 36 Int64)))

(case "wp2 a CHECKED-SHIFT computed perform arg feeding the two-lookup-match arm — shl scratch width does not alias the Option handle"
  (input  (do
            (effect S (op put (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle S Map.empty
                ((put (k v) m
                  (let ((m2 (match (Map.lookup m k)
                              ((Some x) (Map.insert m k v))
                              ((None u) (Map.insert m k v)))))
                    (resume (match (Map.lookup m2 k) ((Some x) x) ((None u) 0)) m2))))
                (S.put (<< n 2) n)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3 Int64)))

(case "wp3 a THREE-param op with two computed args through the two-lookup-match arm — every arg slot and the handle slot stay disjoint"
  (input  (do
            (effect S (op put (-> Int64 Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle S Map.empty
                ((put (k v w) m
                  (let ((m2 (match (Map.lookup m k)
                              ((Some x) (Map.insert m k (+ v w)))
                              ((None u) (Map.insert m k (+ v w))))))
                    (resume (match (Map.lookup m2 k) ((Some x) x) ((None u) 0)) m2))))
                (S.put (+ n 1) (* n 2) (- n 1))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 8 Int64)))

(case "rmp1 a RECORD state wrapping a Map plus a counter — computed perform keys; the arm answers 10*lookup + the ADVANCED counter so both fields are observed"
  (input  (do
            (effect S (op put (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle S (record (= m Map.empty) (= cnt 0))
                ((put (k v) st
                  (let ((m2 (Map.insert (. st m) k v)))
                    (let ((c2 (+ (. st cnt) 1)))
                      (resume (+ (* 10 (match (Map.lookup m2 k) ((Some x) x) ((None u) 0))) c2)
                              (record (= m m2) (= cnt c2)))))))
                (let ((a (S.put (+ n 1) n)))
                  (let ((b (S.put (* 2 n) (+ n 5))))
                    (+ (* 100 a) b)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3182 Int64))
  (call   main (: 9 Int64)) (output (: 9242 Int64)))

(case "ab21 the arm ABORTS (no resume) with a value built from two lookup-matches over a computed-key insert — the width-partition holds on the abort path"
  (input  (do
            (effect S (op put (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle S (Map.insert Map.empty n (* n 3))
                ((put (k v) m
                  (let ((m2 (match (Map.lookup m k)
                              ((Some x) (Map.insert m k 5))
                              ((None u) (Map.insert m k 5)))))
                    (+ (* 100 (match (Map.lookup m2 k) ((Some x) x) ((None u) 0)))
                       (match (Map.lookup m2 n) ((Some y) y) ((None u) 0))))))
                (S.put (+ n 1) n)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 509 Int64))
  (call   main (: 10 Int64)) (output (: 530 Int64)))

(case "ch21 CHAINED computed keys — the second perform's key is computed FROM the first's answer, both dispatches walk the two-lookup-match arm"
  (input  (do
            (effect S (op put (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle S Map.empty
                ((put (k v) m
                  (let ((m2 (match (Map.lookup m k)
                              ((Some x) (Map.insert m k v))
                              ((None u) (Map.insert m k v)))))
                    (resume (match (Map.lookup m2 k) ((Some x) x) ((None u) 0)) m2))))
                (let ((a (S.put (+ n 1) n)))
                  (let ((b (S.put (* a 2) (+ a 1))))
                    (+ (* 10 a) b)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34 Int64))
  (call   main (: 8 Int64)) (output (: 89 Int64)))

(case "xh1 the INNER arm performs the OUTER op with a COMPUTED key and the outer arm is the two-lookup-match Map shape — the width-partition crosses a handler boundary"
  (input  (do
            (effect T (op put (-> Int64 Int64 Int64)))
            (effect S (op bump (-> Int64)))
            (def (main (: n Int64))
              (handle T Map.empty
                ((put (k v) m
                  (let ((m2 (match (Map.lookup m k)
                              ((Some x) (Map.insert m k v))
                              ((None u) (Map.insert m k v)))))
                    (resume (match (Map.lookup m2 k) ((Some x) x) ((None u) 0)) m2))))
                (handle S n
                  ((bump () s
                    (let ((t (T.put (+ s 1) s)))
                      (resume (+ (* 10 t) s) (+ s t)))))
                  (let ((a (S.bump)))
                    (let ((b (S.bump)))
                      (+ (* 100 a) b))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3366 Int64))
  (call   main (: 7 Int64)) (output (: 7854 Int64)))

; --- breaker batch 239: saturating clamp both-bounds, match-over-op-result in another op's argument, atomic tuple slot swap ---
(case "clp1 a SATURATING counter — the arm clamps every transition to 0..10 via a pure helper, both bounds hit in one run"
  (input  (do
            (effect S (op nudge (-> Int64 Int64)))
            (def (clamp (: x Int64))
              (if (< x 0) 0 (if (> x 10) 10 x)))
            (def (main (: n Int64))
              (handle S n
                ((nudge (d) s
                  (let ((nx (clamp (+ s d))))
                    (resume nx nx))))
                (let ((a (S.nudge 7)))
                  (let ((b (S.nudge 7)))
                    (let ((c (S.nudge -25)))
                      (+ (* 10000 a) (+ (* 100 b) c)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 71000 Int64))
  (call   main (: 5 Int64)) (output (: 101000 Int64)))

(case "mia1 a MATCH over one op's Option result sits in ANOTHER op's argument position — unwrap-then-perform composed twice, verdicts flip"
  (input  (do
            (effect S (op cls (-> Int64 (Option Int64))) (op use (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S n
                ((cls (v) s (resume (if (< v s) (Some (* v 2)) (: (None unit) (Option Int64))) (+ s 1)))
                 (use (v) s (resume (+ v s) (+ s 10))))
                (+ (S.use (match (S.cls 3) ((Some x) x) ((None _u) -50)))
                   (* 1000 (S.use (match (S.cls 3) ((Some x) x) ((None _u) -50)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 23012 Int64))
  (call   main (: 2 Int64)) (output (: 19953 Int64)))

(case "swp1 an ATOMIC slot swap — the swap dispatch exchanges both tuple fields in one transition, reads before and after pin the exchange"
  (input  (do
            (effect S (op geta (-> Int64)) (op getb (-> Int64)) (op swap (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple n 100)
                ((geta () st (match st ((tuple a b) (resume a st))))
                 (getb () st (match st ((tuple a b) (resume b st))))
                 (swap () st (match st ((tuple a b) (resume (+ a b) (tuple b a))))))
                (let ((a1 (S.geta)))
                  (let ((_s (S.swap)))
                    (let ((a2 (S.geta)))
                      (let ((b2 (S.getb)))
                        (+ (* 100000 a1) (+ (* 100 a2) b2))))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 710007 Int64))
  (call   main (: 0 Int64)) (output (: 10000 Int64)))

; --- breaker batch 240: dynamic shift amounts with checked-overflow trap, parallel AND/OR/XOR accumulators, min-stack ---
(case "shd1 DYNAMIC shift amounts from the state — two drawn widths, the value-63 draw traps the checked shift overflow"
  (input  (do
            (effect S (op amt (-> Int64)))
            (def (main (: n Int64))
              (handle S n
                ((amt () s (resume s (+ s 31))))
                (let ((a (<< 1 (S.amt))))
                  (let ((b (<< 1 (S.amt))))
                    (+ a b)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 4294967298 Int64))
  (call   main (: 0 Int64)) (output (: 2147483649 Int64))
  (call   main (: 32 Int64)) (trap "integer overflow"))

(case "bwa1 THREE parallel bit-accumulators in one state — running AND, OR, and XOR folds over the drawn payloads, read as a sum"
  (input  (do
            (effect S (op mix (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (tuple 0 0 0)
                ((mix (v) st
                  (match st
                    ((tuple ao oo xo)
                      (resume (+ ao (+ oo xo))
                              (tuple (& (if (= ao 0) v ao) v) (| oo v) (^ xo v)))))))
                (let ((_a (S.mix 12)))
                  (let ((_b (S.mix 10)))
                    (let ((_c (S.mix n)))
                      (S.mix 0))))))
            (export main)))
  (call   main (: 6 Int64)) (output (: 14 Int64))
  (call   main (: 15 Int64)) (output (: 32 Int64)))

(case "mns1 a MIN-TRACKING stack state — (stack, min) pushes thread the heap and the scalar together, mid-run and final min reads"
  (input  (do
            (effect S (op push (-> Int64 Int64)) (op mn (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple (list) 9999)
                ((push (v) st
                  (match st
                    ((tuple stk mn)
                      (resume (List.len stk)
                              (tuple (List.push stk v) (if (< v mn) v mn))))))
                 (mn () st (match st ((tuple _stk m) (resume m st)))))
                (let ((_a (S.push 5)))
                  (let ((_b (S.push n)))
                    (let ((m1 (S.mn)))
                      (let ((_c (S.push 1)))
                        (+ (* 100 m1) (S.mn))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 301 Int64))
  (call   main (: 8 Int64)) (output (: 501 Int64)))

; --- breaker batch 241: Boyer-Moore vote state, integer fixed-point EMA, Map.remove-reinsert churn ---
(case "bmv1 BOYER-MOORE majority vote — the (leader,votes) state deposes on exhausted votes, the challenger seed flips the winner"
  (input  (do
            (effect S (op vote (-> Int64 Int64)) (op lead (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple 0 0)
                ((vote (c) st
                  (match st
                    ((tuple leader votes)
                      (if (= c leader)
                          (resume votes (tuple leader (+ votes 1)))
                          (if (< votes 1)
                              (resume 0 (tuple c 1))
                              (resume votes (tuple leader (- votes 1))))))))
                 (lead () st (match st ((tuple l _v) (resume l st)))))
                (let ((_a (S.vote 7)))
                  (let ((_b (S.vote 7)))
                    (let ((_c (S.vote n)))
                      (let ((_d (S.vote n)))
                        (let ((_e (S.vote n)))
                          (S.lead))))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 7 Int64))
  (call   main (: 9 Int64)) (output (: 9 Int64)))

(case "ema1 an INTEGER EMA state — each dispatch blends (3*ema + 100*v)/4 at 100x scale, convergence toward the fed value from both sides"
  (input  (do
            (effect S (op feed (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (* n 100)
                ((feed (v) ema
                  (let ((nema (/ (+ (* ema 3) (* v 100)) 4)))
                    (resume (/ nema 100) nema))))
                (let ((a (S.feed 8)))
                  (let ((b (S.feed 8)))
                    (let ((c (S.feed 8)))
                      (+ (* 10000 a) (+ (* 100 b) c)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 20304 Int64))
  (call   main (: 16 Int64)) (output (: 141211 Int64)))

(case "mrv1 REMOVE-then-REINSERT churn on a Map state — del answers the removed value (0 when absent); for n=98 the second del hits the n+1 key it planted"
  (input  (do
            (effect S
              (op put (-> Int64 Int64 Int64))
              (op del (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S Map.empty
                ((put (k v) m
                  (let ((m2 (Map.insert m k v)))
                    (resume (Map.len m2) m2)))
                 (del (k) m
                  (resume (match (Map.lookup m k) ((Some x) x) ((None u) 0))
                          (Map.remove m k))))
                (let ((a (S.put n 5)))
                  (let ((b (S.put (+ n 1) 7)))
                    (let ((c (S.del n)))
                      (let ((d (S.put n 9)))
                        (let ((e (S.del 99)))
                          (+ (* 10 (+ (* 100 (+ (* 10 (+ (* 10 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 125020 Int64))
  (call   main (: 98 Int64)) (output (: 125027 Int64)))

; --- breaker batch 242: growing-string slice windows, nested Option (Option _) op result, arm-built string-keyed buckets ---
(case "srw1 a GROWING STRING state with a computed slice window per dispatch — each grow appends two chars, the arm answers the byte-len of an interior window built from the drawn offset"
  (input  (do
            (effect S (op grow (-> String Int64 Int64)))
            (def (main (: n Int64))
              (handle S "ab"
                ((grow (add lo) s
                  (let ((s2 (String.concat s add)))
                    (resume (match (String.slice s2 lo (- (String.byte-len s2) 1))
                              ((Some w) (String.byte-len w))
                              ((None u) -1))
                            s2))))
                (let ((a (S.grow "cd" (+ n 1))))
                  (let ((b (S.grow "ef" (+ n 2))))
                    (+ (* 100 a) (* 10 b))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 230 Int64))
  (call   main (: 1 Int64)) (output (: 120 Int64)))

(case "noo1 an op whose result is NESTED Option (Option Int64) — the arm classifies the state into None / Some None / Some (Some s), the body's nested match distinguishes all three in one run"
  (input  (do
            (effect S (op draw (-> (Option (Option Int64)))))
            (def (main (: n Int64))
              (handle S n
                ((draw () s
                  (resume (if (< s 0)
                              (: (None unit) (Option (Option Int64)))
                              (if (= s 0)
                                  (Some (: (None unit) (Option Int64)))
                                  (Some (Some s))))
                          (- s 1))))
                (let ((f (fn ((: o (Option (Option Int64))))
                           (match o
                             ((Some inner) (match inner ((Some x) (* x 10)) ((None _u) 0)))
                             ((None _u) -5)))))
                  (let ((a (f (S.draw))))
                    (let ((b (f (S.draw))))
                      (let ((c (f (S.draw))))
                        (+ (* 10000 (+ a 9)) (+ (* 100 (+ b 9)) (+ c 9)))))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 190904 Int64))
  (call   main (: 2 Int64)) (output (: 291909 Int64)))

(case "smk1 a STRING-KEYED Map state whose keys are BUILT IN THE ARM — parity routes each value to a concat-computed bucket, accumulate-or-insert answers the bucket total"
  (input  (do
            (effect S (op tag (-> String Int64 Int64)))
            (def (main (: n Int64))
              (handle S Map.empty
                ((tag (pre v) m
                  (let ((key (String.concat pre (if (= (% v 2) 0) "-e" "-o"))))
                    (let ((total (match (Map.lookup m key)
                                   ((Some x) (+ x v))
                                   ((None u) v))))
                      (resume total (Map.insert m key total))))))
                (let ((a (S.tag "a" n)))
                  (let ((b (S.tag "a" (+ n 2))))
                    (let ((c (S.tag "b" (+ n 1))))
                      (+ (* 10000 a) (+ (* 100 b) c)))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 41005 Int64))
  (call   main (: 7 Int64)) (output (: 71608 Int64)))

; --- breaker batch 243: Result with heap-list payload, Option-of-List state lifecycle, tuple pairing two collections ---
(case "rsl1 a Result whose Ok payload is a HEAP LIST snapshot of the state — the first snap Errs on the empty list, the second Oks the grown snapshot, both cross resume"
  (input  (do
            (effect S
              (op snap (-> (Result (List Int64) Int64)))
              (op push (-> Int64 Int64)))
            (def (score (: r (Result (List Int64) Int64)))
              (match r
                ((Ok xs) (+ (* 10 (List.len xs)) (match (List.at xs 0) ((Some h) h) ((None u) 0))))
                ((Err e) (* e -1))))
            (def (main (: n Int64))
              (handle S (list)
                ((snap () xs
                  (resume (if (= (List.len xs) 0)
                              (: (Err 7) (Result (List Int64) Int64))
                              (Ok xs))
                          xs))
                 (push (v) xs (resume (List.len xs) (List.push xs v))))
                (let ((a (score (S.snap))))
                  (let ((_p (S.push n)))
                    (let ((_q (S.push (+ n 1))))
                      (let ((b (score (S.snap))))
                        (+ (* 100 a) b)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: -677 Int64))
  (call   main (: 50 Int64)) (output (: -630 Int64)))

(case "olc1 an Option-of-LIST handler state lifecycle — None is uninitialized, push initializes-or-appends, take scores and RESETS to None, a later push re-initializes"
  (input  (do
            (effect S
              (op push (-> Int64 Int64))
              (op take (-> Int64)))
            (def (main (: n Int64))
              (handle S (: (None unit) (Option (List Int64)))
                ((push (v) st
                  (let ((xs2 (match st
                               ((Some xs) (List.push xs v))
                               ((None u) (list v)))))
                    (resume (List.len xs2) (Some xs2))))
                 (take () st
                  (resume (match st
                            ((Some xs) (+ (* 10 (List.len xs))
                                          (match (List.at xs 0) ((Some h) h) ((None u) 0))))
                            ((None u) 0))
                          (: (None unit) (Option (List Int64))))))
                (let ((a (S.push n)))
                  (let ((b (S.push (+ n 1))))
                    (let ((c (S.take)))
                      (let ((d (S.push (+ n 2))))
                        (let ((e (S.take)))
                          (+ (* 100 (+ (* 10 (+ (* 100 (+ (* 10 a) b)) c)) d)) e))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1223115 Int64))
  (call   main (: 8 Int64)) (output (: 1228120 Int64)))

(case "tug1 a TUPLE state pairing TWO collections — a List and a Map advance through different ops, and a cross op reads BOTH halves in one answer"
  (input  (do
            (effect S
              (op pushl (-> Int64 Int64))
              (op putm (-> Int64 Int64 Int64))
              (op cross (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple (: (list) (List Int64)) Map.empty)
                ((pushl (v) st
                  (match st
                    ((tuple xs m) (let ((xs2 (List.push xs v)))
                      (resume (List.len xs2) (tuple xs2 m))))))
                 (putm (k v) st
                  (match st
                    ((tuple xs m) (let ((m2 (Map.insert m k v)))
                      (resume (Map.len m2) (tuple xs m2))))))
                 (cross () st
                  (match st
                    ((tuple xs m)
                      (resume (+ (* 10 (match (List.at xs 0)
                                         ((Some h) (match (Map.lookup m h) ((Some x) x) ((None u) 0)))
                                         ((None u) -1)))
                                 (List.len xs))
                              st)))))
                (let ((a (S.pushl n)))
                  (let ((b (S.putm n (* 2 n))))
                    (let ((c (S.pushl (+ n 1))))
                      (let ((d (S.cross)))
                        (+ (* 1000 (+ (* 10 (+ (* 10 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 112062 Int64))
  (call   main (: 0 Int64)) (output (: 112002 Int64)))
