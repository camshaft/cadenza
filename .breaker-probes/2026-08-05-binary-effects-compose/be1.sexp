(case "be1 bin SEGMENT VALUES come from performs (construction under a handler)"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (+ (* 1000 (Bytes.len (bin (u16 (UInt16.wrap (St.next))) (u8 (UInt8.wrap (St.next))))))
                   (match (Bytes.at (bin (u8 (UInt8.wrap (St.next)))) 0)
                     ((Some b) (Int64.of b))
                     ((None _u) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3007 Int64)))
