(case "la1 String.slice of a Map-looked-up String with perform-threaded start/end"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def table (Map.insert Map.empty 1 "abcdefgh"))
                (handle St n
                  ((next (u) s (resume s (+ s 1))))
                  (match (Map.lookup table 1)
                    ((Some str)
                      (match (String.slice str (St.next) (St.next))
                        ((Some sl) (String.byte-len sl))
                        ((None _u) -100)))
                    ((None _u) -200)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64)))
