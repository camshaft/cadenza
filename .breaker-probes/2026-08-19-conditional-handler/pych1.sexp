(case "pych1 probe: the HANDLE expression sits inside ONE branch of an if — the effect region is installed only when the guard holds; the other branch is a pure constant, so both arms type the same and the handler is set up conditionally at runtime"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (if (> n 5)
        (handle E (: 3 Int64)
          ((tick () s (resume (* s 10) (+ s 1))))
          (+ (E.tick) (E.tick)))
        (: 999 Int64)))
  (export main)))
  (call   main (: 10 Int64)) (output (: 70 Int64))
  (call   main (: 0 Int64)) (output (: 999 Int64)))
