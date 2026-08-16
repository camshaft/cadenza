(case "tm2 a THREE-function mutual SCC — two tail legs route by value, the third combines the cycle result with a post-put draw"
  (input  (do
            (effect St (op get (-> Unit Int64)) (op put (-> Int64 Unit)))
            (def (fa (: n Int64)) (if (= n 0) (St.get) (fb n)))
            (def (fb (: n Int64)) (if (= n 1) (St.get) (fc n)))
            (def (fc (: n Int64))
              (let ((child (fa (- n 1))))
                (match (St.put n) (_ (+ child (St.get))))))
            (def (main (: k Int64))
              (handle St 0
                ((get (u) s (resume s s))
                 (put (v) s (resume unit (+ s v))))
                (fa k)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 7 Int64))
  (call   main (: 5 Int64)) (output (: 30 Int64)))
