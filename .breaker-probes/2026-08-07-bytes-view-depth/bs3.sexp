(case "bs3 a compacted deep-slice view keys a Map like the directly-built bytes"
  (input  (do
            (def (main (: k Int64))
              (do
                (def b (Bytes.concat (Bytes.of (list 9 8)) (Bytes.of (list (UInt8.wrap k) 6 5))))
                (def view (Option.expect (Bytes.slice b 1 3) "v"))
                (match (Map.lookup (Map.insert Map.empty (Bytes.of (list 8 (UInt8.wrap k) 6)) 42) (Bytes.compact view))
                  ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 42 Int64)))
