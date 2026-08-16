(case "so1 a SET op result crosses resume — membership-probed and measured per dispatch"
  (input  (do
            (effect St (op allowed (-> Int64 (Set Int64))))
            (def (main (: n Int64))
              (handle St 0
                ((allowed (k) s (resume (if (> k 0) (Set.of (list 2 5 9)) (Set.of (list))) s)))
                (+ (if (Set.contains (St.allowed n) 5) 10 0)
                   (Set.len (St.allowed 0)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 10 Int64)))
