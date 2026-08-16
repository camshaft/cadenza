(case "sa1 a SYMBOL as op ARGUMENT — the arm compares interned identity against its own intern"
  (input  (do
            (effect St (op classify (-> Symbol Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((classify (sym) s
                  (resume (+ (* 100 (if (= sym (Symbol.of "alpha")) 1 0))
                             (* 10 (if (= sym (Symbol.of "beta")) 1 0)))
                          s)))
                (+ (St.classify (Symbol.of (String.concat "al" "pha")))
                   (St.classify (Symbol.of "beta")))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 110 Int64)))
