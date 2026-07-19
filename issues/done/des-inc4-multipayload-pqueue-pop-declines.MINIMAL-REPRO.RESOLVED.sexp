(do
  (type Instant (Instant UInt64))
  (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
  (type KBox (KBox (-> Unit Unit)))
  (type PQ PQNil (PQCons (Tuple Instant KBox PQ)))
  (def (pop-apply (: q PQ))
    (match q
      ((PQ.PQNil _) unit)
      ((PQ.PQCons (tuple wake kb rest)) (match kb ((KBox.KBox k) (k unit))))))
  (effect Sim (op sleep (-> Instant Unit)) (op now (-> Unit Instant)))
  (def (main)
    (handle Sim (Instant.Instant 0)
      ( (now (u) s (resume s s))
        (sleep (wake) s (pop-apply (PQ.PQCons (tuple wake (KBox.KBox (fn (_u) (resume unit wake))) (PQ.PQNil ()))))) )
      (do (Sim.sleep (Instant.Instant 5000000000)) (inst-ns (Sim.now)))))
  (export main))

;; ─── TRIAGE (corpus-bugfix 2026-07-19) ───
;; This is the MULTI-PAYLOAD PQueue variant of the KNOWN E5 step-3 escaping-continuation gate
;; ([[des-e5-step3-escaping-k-stored-apply.STEP3-GATED]]). Reproduced on my build (HEAD 113 behind trunk):
;; declines with "this handler is not yet reducible by the tail-resumptive fold (cross-function or non-tail
;; resume arrives in a later increment)". That is the INTENDED GATE message, not a new bug — a `resume` is
;; stored in a KBox inside a compound tuple and applied cross-function after PQ pop = a genuinely-escaping
;; (non-tail) continuation, exactly the capability v-effects' E5 FACE-1 + wake-seeded reify unblocks.
;; DISPOSITION: NOT a fresh bug + NOT a new route — a DES-vertical inc-4 artifact of the same step-3 gate,
;; owned by the DES ⇄ v-effects step-3 workstream (v-effects builds escaping-k; DES consumes). It resolves
;; the moment escaping-resume-thunk lands (same unblock as the single-task gated file). Marked STEP3-GATED.
;; No decline-severity concern: reject-don't-miscompile, with an accurate "later increment" diagnostic.

;; ─── CORRECTION + RESOLVED (corpus-bugfix 2026-07-19) ───
;; My 2026-07-19 "STEP3-GATED" triage was WRONG (a frozen-checkout mis-attribution — the exact anti-stale trap
;; my own memory warns about). I reproduced the decline on a build 113 commits behind trunk and attributed it to
;; the escaping-continuation step-3 gate. In fact the blocker was the MULTI-PAYLOAD tuple-destructure fold, a
;; DIFFERENT gap, now FIXED by `bcd241590` ("rcdzc: extend fold_ctor_match to the multi-payload [Payload,Elem(i)]
;; tuple-destructure path (DES inc-4 pqueue)"). Re-verified on a fresh TRUNK-TIP build (295573dde, throwaway
;; worktree, removed): this repro now COMPILES and RUNS to 5000000000 on wasm — matching the commit's own claim
;; ("folds to 5000000000 on all three backends"). NOT step-3-gated; the `resume` here IS tail-resumptive after
;; the pop-apply fold resolves the pqueue destructure. RESOLVED. Renamed .RESOLVED.
;; LESSON: even a "declines with a later-increment message" can be a STALE decline — verify the reject shape on
;; a trunk-tip build before attributing it to a specific gate, exactly like a false-green (both directions of
;; frozen-checkout error). [[probing-fresh-trunk-features-while-worktree-frozen-behind-trunk-gives-false-declines]]
