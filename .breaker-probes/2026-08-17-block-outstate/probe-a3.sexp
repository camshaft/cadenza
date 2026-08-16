(case "a3 adv-69 escalation: the block in a handler ARM body (arm-internal boundary)"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (effect Up (op ask (-> Unit Int64)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s (+ s 1))))
                (handle Up 0
                  ((ask (u) t (resume (let ((b true)) (if b (St.get) 99)) t)))
                  (+ (* 10 (Up.ask)) (St.get)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34 Int64)))
