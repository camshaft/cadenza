(do (def (f xs) (match xs ((list #map((= 5 v)) _r) v) ((list #map((= 1 w)) _r) (* w 10)) (_ -1)))
    (def (main (: n Int64)) (f (if (> n 0) (list #map((= 1 n)) #map((= 2 20))) (list #map((= 5 n))))))
    (export main))
