; FINDING (breaker, 2026-07-28): a DO-DEF-BOUND perform inside a RECURSIVE fn called under a
; handle fails with the MANGLED-NAME diagnostic `check-all#eff2 has no body` (CDZ0201) — the
; effect-specialization of the recursive fn is created but empty. Both backends (shared).
;
;   ok    perform in EXPRESSION position in the recursion: (check-all (- i 1) (+ bad (Env.scale i)))
;         → computes 110 (the landed per-byte-performs pin family)
;   FAIL  the SAME loop with the perform DO-DEF-BOUND: (do (def scaled (Env.scale i))
;         (check-all (- i 1) (+ bad scaled))) → CDZ0201 `check-all#eff2 has no body`
;   (straight-line do-def-bound performs under a handle are FIXED (the F2/bin fix landed);
;    the RECURSIVE-fn variant is this new face)
;
; Same mangled-name leak as the abortive+recursive face (loop#eff2, filed); the underlying
; specialization gap likely shares a root: the effect-specializer clones the recursive fn's
; SIGNATURE but not its BODY when the body contains a do-def-bound perform. The property-runner
; idiom (gen → def-bound check → recurse) hits this immediately.
;
; GRADED REPRO (expected = the expression-spelling's semantics; FAILS CDZ0201 today):
(case "a do-def-bound perform inside a recursive fn computes under its handler"
  (input  (do
        (effect Env (op scale (-> Int64 Int64)))
        (def (check-all (: i Int64) (: bad Int64))
          (if (= i 0)
              bad
              (do
                (def scaled (Env.scale i))
                (check-all (- i 1) (+ bad scaled)))))
        (def (main (: k Int64))
          (handle Env k
            ((scale (v) s (resume (* v s) s)))
            (check-all 10 0)))
        (export main)))
  (call   main (: 2 Int64)) (output (: 110 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))

;; ------------------------------------------------------------------------------------------
;; TRIAGED-CONFIRMED (corpus-bugfix, trunk 51c0a2983, both backends): FAIL variant -> CDZ0201
;; `check-all#eff2 has no body`; OK inline-expression variant compiles (231 bytes). CONFIRMED a
;; DISTINCT specialization gap (not the F2 straight-line do-def/bin lineage — that's landed). The
;; effect-specializer clones the recursive fn's SIGNATURE but not its BODY when the body do-def-binds
;; a perform. Same mangled-name-leak family as the abortive loop#eff2 face (also v-effects). ROUTED
;; v-effects (specializer). HELD PIN expected 110/0 (from the inline-spelling's semantics). Diagnostic
;; should ALSO name `check-all`, not the mangled `check-all#eff2`. ON FIX: gate x3 -> 110/0; pin into
;; 14-effects beside the perform pins; baseline x3. (If v-effects lands a SAFE-FLOOR clean decline
;; first like F1/abortive, flip expected to (declines) then value-pin on the real fix — verify which.)

;; DISPOSITION -> EXPECT-(declines) safe floor (v-effects root-cause, 2026-07-28, note 17908). ROOT:
;; specialize_recursive RESERVES the spec def body:None + memoizes the mangled name BEFORE threading the
;; body (so the recursive self-call resolves via the memo). For a do-def-bound-perform recursive body,
;; thread() returns None and the ? early-returns, but the reserved body:None def + memo REMAIN -> a
;; self-call resolves to the bodyless def -> CDZ0201 'check-all#eff2 has no body'. FIX (safe floor): on
;; threading failure, poison/roll-back the reserved def so it (1) DECLINES cleanly 'not yet reducible' not
;; CDZ0201, and (2) never surfaces the mangled #eff name. Body-clone-to-compute-110 is a RICHER LATER inc.
;; => PIN expected = (declines) for the floor; FLIP to value 110/0 when the body-clone increment lands.
;; SEQUENCING: v-effects builds this AFTER their abortive-arm MR 94581e5f1 lands (same specializer/
;; reduce_handle seam). So HELD chains: 94581e5f1 (abortive) -> then the specializer floor fix -> then me.
;; ON LAND (v-effects floor fix): gate x3 -> (declines); pin into 14-effects; baseline x3.

;; FIX MR'd (v-effects, 2026-07-29): 0d2afb083 (queued). A bodyless #eff-marked spec def now declines
;; UNCODED naming the base fn ("this handler is not yet reducible ... the recursive function check-all
;; performs a discharged operation in a form the effect specializer does not yet handle"), NOT CDZ0201
;; 'check-all#eff2 has no body'. Both asks met: (1) clean decline not CDZ0201; (2) names check-all not #eff2.
;; Does NOT compute 110 yet (body-clone specialization later) -> keep pin (declines), flip to 110/0 then.
;; v-effects heads-up: their local wasm gate couldn't run (runtime c1344126 missing, lease-throttled) —
;; verified rust+rust-async green + roundtrip 5186/0; pr-sync gates wasm authoritatively. I `cargo xtask
;; build` first before gating (populate runtime). ON LAND (0d2afb083): gate x3 -> (declines); pin into 14-effects.
