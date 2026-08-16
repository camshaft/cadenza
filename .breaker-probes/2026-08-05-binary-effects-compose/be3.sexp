(case "be3 constant bin construction + performs elsewhere in the body folds"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (+ (Bytes.len (bin (u16 258) (u8 7))) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 8 Int64)))
