(case "m10 closure from an IF (perform condition) + perform-fed application — no collection at all"
  (input  (do
            (effect St (op feed (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((feed (u) s (resume s (+ s 1))))
                (let ((f (if (= (% (St.feed) 2) 1) (fn ((: x Int64)) (+ x 1000)) (fn ((: x Int64)) (* x 2)))))
                  (f (St.feed)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1006 Int64)))
