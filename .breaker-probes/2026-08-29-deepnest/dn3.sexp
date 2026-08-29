(do (type (W a) (Wrap (W a)) (End a))
    (def (unwrap (: w (W Int64))) (match w ((End x) x) ((Wrap inner) (+ 10 (unwrap inner)))))
    (def (main (: n Int64)) (unwrap (Wrap (Wrap (Wrap (Wrap (Wrap (End n))))))))
    (export main))
