(do
  (effect Reg (op intern (-> String Symbol)) (op which (-> Symbol Int64)))
  (def (main (: n Int64))
    (handle Reg 0
      ((intern (t) s (resume (Symbol.of t) (+ s 1)))
       (which (sym) s (resume (if (= sym (Symbol.of "hot")) (* s 10) (- 0 s)) s)))
      (let ((a (Reg.intern "hot")))
        (+ (* 100 (Reg.which a))
           (Reg.which (Symbol.of "cold"))))))
  (export main))
