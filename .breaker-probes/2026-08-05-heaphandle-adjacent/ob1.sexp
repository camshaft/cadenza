(case "ob1 an (Option Bytes) round-trips construction/match with a NONE branch at runtime"
  (input  (do
            (def (pick (: n Int64))
              (if (> n 0) (Option.Some (Bytes.of (list (UInt8.wrap n) (UInt8.wrap 2)))) (Option.None)))
            (def (main (: n Int64))
              (+ (match (pick n) ((Option.Some b) (Bytes.len b)) ((Option.None) -1))
                 (* 10 (match (pick (- 0 n)) ((Option.Some _b) 1) ((Option.None) 2)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 22 Int64)))
