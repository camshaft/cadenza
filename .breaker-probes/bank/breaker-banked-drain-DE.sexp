(case "a heap binding shadowed by a derived heap binding keeps BOTH values live in scope"
  (doc    "The SELF-DERIVED shadow: `(let ((xs (List.push xs 99))) …)` rebinds xs to a value built
           FROM the binding it shadows — the initializer reads the OLD handle while introducing the
           NEW name. 100·len(new) + 10·sum0 + new[3] = 400 + 70 + 99 = 569 at n=7 (new list [7,2,3,99]
           len 4; sum0 read the outer xs[0] BEFORE the shadow). Two hazards pinned: a naive one-slot
           rebind evaluates the initializer against an already-cleared slot (the old handle is dead
           exactly when the push needs it), and an over-eager drop of the outer xs at shadow-entry
           invalidates the push's source. The self-derived-initializer face of the shadow family
           (the :57 pin interleaves binder KINDS; here one binder kind feeds itself).")
  (input  (do
            (def (main (: n Int64))
              (let ((xs (list n 2 3)))
                (let ((sum0 (Option.expect (List.at xs 0) "a")))
                  (let ((xs (List.push xs 99)))
                    (+ (* 100 (List.len xs))
                       (+ (* 10 sum0)
                          (Option.expect (List.at xs 3) "b")))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 569 Int64)))
