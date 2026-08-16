(case "sy3 a symbol round-trip between TWO ops of one effect — interned by one, judged by the other"
  (input  (do
            (effect Reg (op intern (-> String Symbol)) (op which (-> Symbol Int64)))
            (def (main (: n Int64))
              (handle Reg 0
                ((intern (t) s (resume (Symbol.of t) (+ s 1)))
                 (which (sym) s (resume (if (= sym (Symbol.of "hot")) (* s 10) (- 0 s)) s)))
                (let ((a (Reg.intern "hot")))
                  (+ (* 100 (Reg.which a))
                     (Reg.which (Symbol.of "cold"))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 999 Int64)))
