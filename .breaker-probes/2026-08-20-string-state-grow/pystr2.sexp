(case "pystr2 probe: a STRING handler state that GROWS per dispatch — tick answers (String.scalar-len s) and threads (String.concat s \"x\"), so each dispatch reads the current length then appends; seed is a 1/2/3-char string by n%3, exercising a heap String value threaded and rebuilt across the resume seam"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (if (= (% n 3) (: 0 Int64)) "a" (if (= (% n 3) (: 1 Int64)) "ab" "abc"))
      ((tick () s (resume (String.scalar-len s) (String.concat s "x"))))
      (+ (* 100 (E.tick)) (E.tick))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 203 Int64))
  (call   main (: 0 Int64)) (output (: 102 Int64)))
