(case "aa4 a THREE-WAY comparison arm — the resume value encodes gt/eq/lt as 1/10/100 against the advancing state"
  (input  (do
            (effect E (op probe (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((probe (v) s (resume (+ (if (> v s) 1 0) (+ (if (= v s) 10 0) (if (< v s) 100 0))) (+ s 1))))
                (+ (E.probe 5) (+ (E.probe 6) (E.probe 0)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 120 Int64))
  (call   main (: 6 Int64)) (output (: 300 Int64))
  (call   main (: 0 Int64)) (output (: 102 Int64)))
