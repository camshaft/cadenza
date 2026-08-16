(case "ca1 FLIP-WITNESS (finding #10): closure over a draw applied twice — expected 80/26, currently 122/62 (re-perform per application)"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (let ((f (let ((a (St.next))) (fn ((: x Int64)) (* a x)))))
                  (+ (f (St.next)) (f 10)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 80 Int64))
  (call   main (: 2 Int64)) (output (: 26 Int64)))
