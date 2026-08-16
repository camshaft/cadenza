(case "ob2 an (Option Bytes) as a Map VALUE: Some/None entries coexist and read back by branch"
  (input  (do
            (def (main (: n Int64))
              (do
                (def m (Map.insert (Map.insert Map.empty 1 (Option.Some (Bytes.of (list (UInt8.wrap n)))))
                                   2 (Option.None)))
                (+ (match (Map.lookup m 1)
                     ((Some (Option.Some b)) (Bytes.len b))
                     (_ -1))
                   (* 10 (match (Map.lookup m 2)
                     ((Some (Option.None)) 3)
                     (_ -1))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 31 Int64)))
