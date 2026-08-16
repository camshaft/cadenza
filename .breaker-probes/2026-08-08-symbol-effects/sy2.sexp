(case "sy2 a SYMBOL toggle state — equality routes the resume value while the arm flips fast/slow per dispatch"
  (input  (do
            (effect M (op mode (-> Int64)))
            (def (main (: n Int64))
              (handle M (if (> n 3) #"fast" #"slow")
                ((mode () s (resume (if (= s #"fast") 100 1) (if (= s #"fast") #"slow" #"fast"))))
                (+ (M.mode) (+ (M.mode) (M.mode)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 201 Int64))
  (call   main (: 0 Int64)) (output (: 102 Int64)))
