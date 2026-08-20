(case "pymm1 probe: the resume answer packs MAX*10 + MIN of the op-arg and the captured state — clamp(v) puts the larger in the tens place and the smaller in the ones, so the arm's if picks the ordering and two dispatches with different args straddle the threaded state"
  (input (do
  (effect E (op clamp (-> Int64 Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((clamp (v) s (resume (if (> v s) (+ (* v 10) s) (+ (* s 10) v)) (+ s 1))))
      (+ (* 1000 (E.clamp 3)) (E.clamp 8))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 31082 Int64))
  (call   main (: 0 Int64)) (output (: 30081 Int64)))
