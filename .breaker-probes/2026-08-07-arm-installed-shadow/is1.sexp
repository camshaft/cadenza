(case "is1 a draw-SELECTED match arm installs a nested same-effect shadow — outer state resumes after the branch-local region"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 1))))
                (let ((k (St.next)))
                  (+ (match k
                       (5 (handle St 70
                            ((next () s (resume s (+ s 7))))
                            (+ (St.next) (St.next))))
                       (_o (* 2 _o)))
                     (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 153 Int64))
  (call   main (: 3 Int64)) (output (: 10 Int64)))
