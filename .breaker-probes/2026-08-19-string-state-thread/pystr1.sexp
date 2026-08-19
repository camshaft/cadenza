(case "pystr1 probe: a STRING-STATE handler threads String.concat across the seam — grow() appends a fixed suffix answering the new length, so two grows extend the seed string and a size read returns the accumulated length; heap String state survives the resume threading"
  (input (do
  (effect E (op grow (-> Int64)) (op size (-> Int64)))
  (def (main (: n Int64))
    (handle E "ab"
      ((grow () s (resume (String.scalar-len (String.concat s "xyz")) (String.concat s "xyz")))
       (size () s (resume (String.scalar-len s) s)))
      (+ (* 10000 (E.grow)) (+ (* 100 (E.grow)) (E.size)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 50808 Int64))
  (call   main (: 0 Int64)) (output (: 50808 Int64)))
