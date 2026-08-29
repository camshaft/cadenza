(do (type (W a) (Wrap (W a)) (End a))
    (def (unwrap (: w (W Int64))) (match w ((Wrap (Wrap (Wrap (End x)))) (* x 100)) ((Wrap (Wrap (End x))) (* x 10)) ((Wrap (End x)) x) ((End x) (- 0 x)) (_ -999)))
    (def (main (: n Int64)) (+ (unwrap (Wrap (Wrap (Wrap (End n))))) (unwrap (End n))))
    (export main))
