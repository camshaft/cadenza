(case "bg1 a bin-pattern arm whose GUARD condition performs — decode binds feed a dispatching guard"
  (input  (do
            (effect St (op quota (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((quota (u) s (resume s (+ s 1))))
                (match (bin (u8 (UInt8.wrap 7)) (u8 (UInt8.wrap 42)))
                  ((guard (bin (u8 tag) (u8 val)) (> val (St.quota)))
                    (+ (* 100 tag) val))
                  (_other -1))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 742 Int64))
  (call   main (: 50 Int64)) (output (: -1 Int64)))
