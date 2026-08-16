(case "ob3 an (Option Bytes) as the effect RESUME value: Some crosses, None crosses, both matched in body"
  (input  (do
            (effect Src (op read (-> Int64 (Option Bytes))))
            (def (main (: n Int64))
              (handle Src 0
                ((read (v) s
                  (if (> v 0)
                    (resume (Option.Some (Bytes.of (list (UInt8.wrap v)))) s)
                    (resume (Option.None) s))))
                (+ (match (Src.read n) ((Option.Some b) (Bytes.len b)) ((Option.None) -1))
                   (* 10 (match (Src.read (- 0 n)) ((Option.Some _b) 1) ((Option.None) 2))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 21 Int64)))
