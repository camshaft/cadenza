(case "pyu8a1 probe: a bare narrow-int (UInt8) OP-RESULT / resume-answer type — tick's op returns UInt8 and the arm resumes (UInt8.wrapping-add 250 (UInt8.of s)) while Int64 state threads (+ s 1), read back via Int64.of; DECLINE-WITNESS for the tail-resumptive fold's bare-narrow-int-answer coverage gap (oracle at ruled-correct value, auto-flips when the fold admits narrow-int op results)"
  (input (do
  (effect E (op tick (-> UInt8)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s (resume (UInt8.wrapping-add (: 250 UInt8) (UInt8.of s)) (+ s 1))))
      (+ (* 1000 (Int64.of (E.tick))) (+ (* 100 (Int64.of (E.tick))) (Int64.of (E.tick))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 276453 Int64))
  (call   main (: 0 Int64)) (output (: 275352 Int64)))
