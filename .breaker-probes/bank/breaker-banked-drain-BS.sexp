; breaker probe X — List.update at RRB DEPTH 3: the fresh 1055 pin covers List.at/len over the
; 1100-element 3-level trie; UPDATE at that depth is the path-copy face (copy the root + one
; interior + one leaf, share ~34 interior nodes). An update that copied the wrong level, or
; aliased instead of copying, corrupts either the updated or the ORIGINAL list.
; Hand-derived: xs = [0,1,...,1099] (build i pushes i). ys = update xs 1050 -7.
;   read ys[1050] = -7 (updated); xs[1050] = 1050 (original INTACT — the sharing face);
;   ys[5] = 5 (untouched leaf shared); ys len = 1100.
;   main = ys[1050]*1000 + xs[1050] + ys[5] → -7000 + 1050 + 5 = -5945.

(case "a List.update at depth-3 RRB path-copies one spine and shares the rest"
  (input  (do
            (def (build (: i Int64) (: n Int64) acc)
              (if (= i n) acc (build (+ i 1) n (List.push acc i))))
            (def (main)
              (let ((xs (build 0 1100 (list))))
                (let ((ys (List.update xs 1050 -7)))
                  (+ (* 1000 (Option.expect (List.at ys 1050) "u"))
                     (+ (Option.expect (List.at xs 1050) "o")
                        (Option.expect (List.at ys 5) "s"))))))
            (export main)))
  (output (: -5945 Int64)))
