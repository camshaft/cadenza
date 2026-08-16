(case "ss1 a two-site arm over a STRING state (append on pass, hold on fail)"
  (input  (do
            (effect St (op tag (-> Int64 Int64)) (op len (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St ""
                ((tag (v) s (if (> v 10) (resume v (String.concat s "x")) (resume 0 s)))
                 (len (u) s (resume (String.byte-len s) s)))
                (+ (St.tag 20) (+ (St.tag n) (+ (St.tag 30) (* 100 (St.len)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 250 Int64)))
