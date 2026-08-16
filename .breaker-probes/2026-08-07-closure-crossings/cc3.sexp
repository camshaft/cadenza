(case "cc3 a helper-BUILT closure whose capture is a draw passed as the helper's ARG — binds once (the sound path beside finding #10)"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (mk (: m Int64)) (fn ((: x Int64)) (* x m)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (let ((f (mk (St.next))))
                  (+ (f 10) (f (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 80 Int64))
  (call   main (: 2 Int64)) (output (: 26 Int64)))
