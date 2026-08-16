(case "cs2 a scalar WALK over an effect-built multibyte rope (String.at across the seam + multibyte)"
  (input  (do
            (effect St (op mk (-> Unit String)))
            (def (count-x (: s String) (: i Int64) (: acc Int64))
              (if (>= i (String.scalar-len s)) acc
                (count-x s (+ i 1)
                  (match (String.at s i) ((Some c) (if (= c "∀") (+ acc 1) acc)) ((None _u) acc)))))
            (def (main (: n Int64))
              (handle St n
                ((mk (u) s (resume (String.concat "a∀b" "∀c∀") s)))
                (count-x (St.mk) 0 0)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 3 Int64)))
