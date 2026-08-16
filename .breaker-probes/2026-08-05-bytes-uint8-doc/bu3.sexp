(case "bu3 control: runtime element WITH UInt8.wrap folds"
  (input  (do
            (def (main (: n Int64))
              (Bytes.len (Bytes.of (list (UInt8.wrap n) (UInt8.wrap 2)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 2 Int64)))
