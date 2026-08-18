(case "pysh3 the SHADOW'S OWN INIT DRAWS FROM THE HANDLER IT IS ABOUT TO SHADOW — the inner handle over the same effect computes its starting value by performing on the still-unshadowed outer arm, the inner region then serves its one draw from that outer-drawn seed with the cheap toll, a final outer draw confirms the outer state advanced through the init, and both outer frames settle their expensive tolls around the whole inner region"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (+ (resume s (+ s 1)) (* 1000 (+ s 1)))))
                (+ (handle E (* 10 (E.tick))
                     ((tick () s (+ (resume s (+ s 1)) (* 100 s))))
                     (E.tick))
                   (* 10000 (E.tick)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 41010 Int64))
  (call   main (: 0 Int64)) (output (: 27000 Int64)))
