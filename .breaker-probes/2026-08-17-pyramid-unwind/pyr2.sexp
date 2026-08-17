(case "pyr2 MULTIPLICATIVE post-resume tolls — each dispatch's arm MULTIPLIES the resumed rest-of-body value by its own state-plus-two factor, two dispatches compose two factors around the body's positional fold, and because the factors differ per dispatch the product pins the unwind pairing (which factor saw which intermediate) beyond what addition can distinguish"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (+ (% n 3) 1)
                ((tick () s (* (resume s (+ s 1)) (+ s 2))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 640 Int64))
  (call   main (: 0 Int64)) (output (: 252 Int64)))
