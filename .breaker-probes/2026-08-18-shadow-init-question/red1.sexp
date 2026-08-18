(case "red1 the SHADOW-INIT DRAW composed with one shadowed draw — the inner handle over the same effect seeds itself from the outer arm then serves a single draw, and the outer post-resume toll composes with the inner draw threading through the outer continuation (ruled correct by the distinct-effect differential: an inner handler over a DIFFERENT effect computes the identical value)"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (+ (resume s (+ s 1)) (* 1000 (+ s 1)))))
                (handle E (* 10 (E.tick))
                  ((tick () s (+ (resume s (+ s 1)) (* 100 s))))
                  (E.tick))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 7010 Int64))
  (call   main (: 0 Int64)) (output (: 4000 Int64)))
