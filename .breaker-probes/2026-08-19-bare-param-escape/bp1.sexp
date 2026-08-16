(case "bp1 TODO-FLIP: bare-param escaping closure (v-effects fbe4fb204 decline-witness; flips when the lift slots bare params)"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def f (handle St n
                         ((get (u) s (resume s s)))
                         (let ((v (St.get)))
                           (fn (x) (+ x v)))))
                (f 12)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 17 Int64)))
