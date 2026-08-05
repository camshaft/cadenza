; BREAKER FINDING adv-69 (2026-08-04, trunk 3983feb5a) — SILENT MISCOMPILE, wrong value on ALL
; THREE backends (wasm / rust / rust-async), O0..O3 identical (shared lowering, not emit):
;
; A perform in a CONDITIONAL BRANCH, where the conditional is wrapped in a NESTED BLOCK
; (inner let / do / match-scrutinee) that itself sits in a value-consumed LET-INIT position,
; loses its state ADVANCE at the block boundary — the init's continuation reads the
; BLOCK-ENTRY state, not the branch perform's out-state.
;
;   (handle St 3 ((get (u) s (resume s (+ s 1))))
;     (let ((v (let ((b true)) (if b (St.get) 99))))    ; get resumes 3, state 3->4
;       (+ (* 10 v) (St.get))))                          ; this get MUST resume 4
;   EXPECT 34.  OBSERVED 33 (continuation resumed 3 = the state at block entry).
;
; DIRECTION WITNESSES (each hand-recomputed):
;   y1 pre-perform first: (do (def a (St.get)) (def v <block>) ...) -> a=3(s->4); the block's
;      get correctly resumes 4 (IN-state crosses INTO the block fine!) but the final get resumes
;      4 not 5 -> 344 vs expected 345. ONLY the block's own advance is invisible downstream.
;   w6 two performs INSIDE the block: v=34 (threads correctly WITHIN the block; 3 then 4), but
;      the final get resumes 3 -> 3403 vs expected 3405 — the exit state reverts to block entry.
;   w3 false-cond/else-branch face: identical drop (33 vs 34) — not the taken-branch side.
;   x2 do-def spelling of the init: same drop (33 vs 34).
;   h2 match-selector instead of if inside the block: same drop (33 vs 34).
;   g3 block in MATCH-SCRUTINEE position instead of let-init: same drop (33 vs 34).
;
; CONTROLS all correct (the boundary is PRECISELY the nested-block wrapper):
;   c4/c6  the SAME if directly in the let-init (no block wrapper)          -> 73 PASS
;   d1/z2  nested let wrapping a BARE perform (no conditional)              -> PASS
;   z1     bare match-with-performing-arm directly in the init              -> 34 PASS
;   g2     cond bound in an OUTER let, if directly in init                  -> 34 PASS
;   x1     the if inside a HELPER FN called in the init                     -> 34 PASS
;   e1/e2/d2 the block/if in a DO-STATEMENT position (value discarded)      -> 73 PASS
;   y2     perform in the inner CONDITION (not branch)                      -> 54 PASS
;   h3     host-delegation face (response cursor, not handler state)        -> 56 PASS
;   g4     value-only (no downstream perform to observe)                    -> PASS
;
; LIKELY LOCUS (same class as the FIXED connective-scrutinee finding, MR 42ed25544 Site-5 hoist):
; effects.rs If thread arm returns the POST-CONDITION state as the if's out-state, dropping the
; branch advance; the Site-4/Site-5 hoist repairs the DIRECT init position (c4/c6 pass) but a
; nested block wrapper (let/do/match) around the conditional defeats the hoist's pattern match,
; so the block's out-state falls back to its entry state. The helper-fn control (x1) threads via
; the call path, which is why it stays correct.
;
; SEVERITY: HIGH — silent wrong value in an idiomatic shape: binding a conditionally-computed
; effectful value ((let ((v (let (...) (if c (E.op) default)))) ...)) is ordinary code. Every
; backend, every opt level; no diagnostic.

(case "REPRO adv-69 a block-wrapped branch perform in a let-init threads its advance to the continuation"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s (+ s 1))))
                (let ((v (let ((b true)) (if b (St.get) 99))))
                  (+ (* 10 v) (St.get)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34 Int64)))

(case "WITNESS adv-69 within-block threading works but the block exit state reverts to entry"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s (+ s 1))))
                (let ((v (let ((b true)) (if b (+ (* 10 (St.get)) (St.get)) 99))))
                  (+ (* 100 v) (St.get)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3405 Int64)))

(case "CONTROL adv-69 the same if directly in the init (no block wrapper) threads correctly"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s (+ s 1))))
                (let ((v (if true (St.get) 99)))
                  (+ (* 10 v) (St.get)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34 Int64)))

; ── BREAKER ESCALATION SWEEP (2026-08-04 tick 3, trunk 9d3ce0b06) — three MORE faces, all verified failing ──
; SEVERITY RAISED: the drop also loses HEAP-state mutations (not just a scalar counter reading stale):
;
; (a1) HEAP accumulator: (handle Log (list) ((add (v) s (resume v (List.push s v))) (count ...))
;        (let ((v (let ((b true)) (if b (Log.add 5) 99)))) (+ (* 10 v) (Log.count))))
;      The add's PUSH is lost — count reads the ENTRY list (len 0): observed 50, expected 51.
;      This is the diagnostic-accumulation idiom (the corpus's own "growing list" handler model) —
;      a conditionally-emitted Log.add inside any helper block silently vanishes from the log.
; (a2) DEPTH-2 wrapper ((let ((a 1)) (let ((b true)) (if b ...)))) — same drop (33 vs 34); the
;      boundary is ANY block nesting >= 1, not exactly one level.
; (a3) The block inside ANOTHER handler's ARM (arm resumes with the block's value, block performs
;      the OUTER effect): (handle St 3 (...) (handle Up 0 ((ask (u) t (resume <block> t))) ...))
;      — St's advance from inside Up's arm is dropped the same way (33 vs 34). So the boundary is
;      positional (block-wrapped conditional in a value-consumed spot), not specific to let-init in
;      a plain handle body.
; All three: wasm (rust/rust-async spot-checks on the original repro were identical; shared lowering).
