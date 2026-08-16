(case "lu3 a concat-assembled deep list equals its push-built twin element for element at depth"
  (input  (do
            (def (build (: i Int64) (: acc (List Int64)))
              (if (= i 0) acc (build (- i 1) (List.push acc i))))
            (def (dseg (: hi Int64) (: lo Int64) (: acc (List Int64)))
              (if (< hi lo) acc (dseg (- hi 1) lo (List.push acc hi))))
            (def (main (: n Int64))
              (do
                (def pushed (build n (list)))
                (def catted (List.concat (List.concat (dseg 100 51 (list)) (dseg 50 26 (list))) (dseg 25 1 (list))))
                (+ (* 10 (if (= catted pushed) 1 0))
                   (if (= (List.len catted) n) 1 0))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 11 Int64)))
