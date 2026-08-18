(case "pym1 TWO OPS WITH DIFFERENT POST-RESUME TOLLS INTERLEAVED — hi answers plain state adding a thousandfold toll while lo answers doubled state adding only a hundredfold one, the body alternates hi lo hi so the unwind interleaves toll KINDS in reverse dispatch order, and a lowering that applies one op's toll shape to the other's frame misprices by an order of magnitude"
  (input  (do
            (effect E
              (op hi (-> Int64))
              (op lo (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((hi () s (+ (resume s (+ s 1)) (* 1000 (+ s 1))))
                 (lo () s (+ (resume (* s 2) (+ s 2)) (* 100 s))))
                (let ((a (E.hi)))
                  (let ((b (E.lo)))
                    (let ((c (E.hi)))
                      (+ a (+ (* 10 b) (* 100 c))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 7641 Int64))
  (call   main (: 0 Int64)) (output (: 5420 Int64)))
