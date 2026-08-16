(case "runtime-branch-SELECTED whole collections compare content-equal across construction routes"
  (doc    "The branch-JOIN face of construction-path canonicalization (the AX..BE family pins
           edit-reached vs direct; here the whole map ARRIVES through an if-join, built in OPPOSITE
           insertion orders per branch): both branches must produce the byte-canonical {1↦10, 2↦20},
           so `=` against the direct build is 1 and len is 2 → 12 on BOTH branch outcomes. The if-join
           materializes the map into one slot — a join that specialized the slot to one branch's
           construction shape (or compared by insertion history) would flip a branch. Also the
           whole-collection upgrade of the :381 branch-built map (that one pins lookup; this pins
           canonical equality of the joined value itself).")
  (input  (do
            (def (main (: b Bool))
              (let ((m (if b (Map.insert (Map.insert Map.empty 1 10) 2 20)
                           (Map.insert (Map.insert Map.empty 2 20) 1 10))))
                (+ (* 10 (if (= m (Map.insert (Map.insert Map.empty 1 10) 2 20)) 1 0))
                   (Map.len m))))
            (export main)))
  (call   main (: true Bool)) (output (: 12 Int64))
  (call   main (: false Bool)) (output (: 12 Int64)))
