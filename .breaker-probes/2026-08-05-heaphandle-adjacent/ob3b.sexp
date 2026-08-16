(case "ob3b control: single-branch arm resuming (Option Bytes) Some only"
  (input  (do
            (effect Src (op read (-> Int64 (Option Bytes))))
            (def (main (: n Int64))
              (handle Src 0
                ((read (v) s (resume (Option.Some (Bytes.of (list (UInt8.wrap v)))) s)))
                (match (Src.read n) ((Option.Some b) (Bytes.len b)) ((Option.None) -1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1 Int64)))
