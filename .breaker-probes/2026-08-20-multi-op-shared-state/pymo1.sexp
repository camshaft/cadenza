(case "pymo1 probe: a TWO-OP effect sharing one threaded state — inc answers s and threads (+ s 5), get answers (* s 10) and threads s unchanged; the body interleaves inc/get/inc/get so the two ops read and advance the SAME handler state in alternation, testing that the tail-resumptive fold threads state across DISTINCT ops of one effect (not just repeated dispatch of a single op)"
  (input (do
  (effect E (op inc (-> Int64)) (op get (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((inc () s (resume s (+ s 5)))
       (get () s (resume (* s 10) s)))
      (+ (* 1000 (E.inc)) (+ (* 100 (E.get)) (+ (* 10 (E.inc)) (E.get))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 7170 Int64))
  (call   main (: 0 Int64)) (output (: 5150 Int64)))
