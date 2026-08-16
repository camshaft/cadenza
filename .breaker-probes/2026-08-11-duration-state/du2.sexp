(case "du2 UInt64 overflow at the top of a DURATION state traps at RUNTIME — the second dispatch crosses MAX, n=0 threads unchanged"
  (input  (do
            (type Duration (Duration UInt64))
            (effect T (op probe (-> Int64 Int64)))
            (def (dur-ns (: d Duration)) (match d ((Duration.Duration ns) ns)))
            (def (main (: n Int64))
              (handle T (Duration.Duration (- UInt64.max (UInt64.wrap 1)))
                ((probe (v) s
                  (let ((nxt (Duration.Duration (+ (dur-ns s) (UInt64.wrap v)))))
                    (resume (if (= (dur-ns nxt) UInt64.max) 1 0) nxt))))
                (+ (T.probe n) (* 10 (T.probe n)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 0 Int64))
  (call   main (: 1 Int64)) (trap "integer overflow"))
