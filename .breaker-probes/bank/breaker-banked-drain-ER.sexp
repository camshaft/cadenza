(case "a record INSIDE a sum payload is row-op-updated and rewrapped without disturbing the original"
  (doc    "The STATE-MACHINE transition idiom: `damage` matches an `(Alive r)` payload and either
           rewraps the SAME variant around a with-updated record (hp 15 → 15000s) or TRANSITIONS to
           the other variant carrying a projected field (`(Dead (. r id))` → -7 at overkill — the
           payload read must happen BEFORE the branch discards r), while the ORIGINAL sum value
           persists untouched through both calls (1s digit: e0's hp still reads 20) → 14932. The #45
           pin (:535) covers without-extend rewrap in a sum payload; this adds the variant-TRANSITION
           face (update-or-transition on one match) and double-application persistence — the
           game-entity/session-lifecycle shape.")
  (input  (do
            (type Ent (Alive (Record (: hp Int64) (: id Int64))) (Dead Int64))
            (def (damage (: e Ent) (: d Int64))
              (match e
                ((Alive r) (let ((hp2 (- (. r hp) d)))
                             (if (> hp2 0) (Alive (Record.with r #"hp" hp2)) (Dead (. r id)))))
                ((Dead i) (Dead i))))
            (def (main (: n Int64))
              (let ((e0 (Alive (record (hp n) (id 7)))))
                (+ (* 1000 (match (damage e0 5) ((Alive r2) (. r2 hp)) ((Dead i) (- 0 i))))
                   (+ (* 10 (match (damage e0 100) ((Alive r3) (. r3 hp)) ((Dead i2) (- 0 i2))))
                      (match e0 ((Alive r4) (- (. r4 hp) (- n 2))) ((Dead i3) -99))))))
            (export main)))
  (call   main (: 20 Int64)) (output (: 14932 Int64)))
