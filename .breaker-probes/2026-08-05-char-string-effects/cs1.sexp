(case "cs1 String.at element extraction across performs (scalar walk of an effect-built rope)"
  (input  (do
            (effect St (op mk (-> Unit String)))
            (def (main (: n Int64))
              (handle St n
                ((mk (u) s (resume (String.concat "ab" "cd") s)))
                (do
                  (def str (St.mk))
                  (+ (match (String.at str 0) ((Some c) (if (= c "a") 10 0)) ((None _u) -1))
                     (match (String.at str 3) ((Some c) (if (= c "d") 1 0)) ((None _u) -1))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 11 Int64)))
