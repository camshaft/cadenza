(case "pysh1 the SAME EFFECT SHADOWED with DIFFERENT TOLL SHAPES — the outer handler charges a thousandfold toll and serves the first draw while an inner handler over the SAME effect charges only a hundredfold toll and serves the two draws inside its region, the inner pyramid settles its cheap tolls before the outer frame's expensive one, and routing any draw to the wrong depth changes both which toll fires and which state answers"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s (+ (resume s (+ s 1)) (* 1000 (+ s 1)))))
                (+ (E.tick)
                   (handle E (: 50 Int64)
                     ((tick () s (+ (resume s (+ s 1)) (* 100 s))))
                     (+ (E.tick) (* 10 (E.tick)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 12661 Int64))
  (call   main (: 0 Int64)) (output (: 11660 Int64)))
