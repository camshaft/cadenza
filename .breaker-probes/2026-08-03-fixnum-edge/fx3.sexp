(case "fx3 shrink back INTO the fixnum window re-canonicalizes (boxed-1 = fixnum max literal)"
  (input  (do
            (def (main (: k Int64))
              (let ((back (- (+ 536870911 k) k)))
                (+ (* 10 (if (= back 536870911) 1 0))
                   (Set.len (Set.of (list back 536870911))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 11 Int64)))
