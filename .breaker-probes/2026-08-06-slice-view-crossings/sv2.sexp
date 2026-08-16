(case "sv2 a String.slice VIEW built in the ARM crosses back through resume — the body measures it"
  (input  (do
            (effect St (op mid (-> String String)))
            (def (main (: n Int64))
              (handle St 0
                ((mid (t) s (resume (match (String.slice t 1 4) ((Some w) w) ((None _u) "?")) s)))
                (String.byte-len (St.mid (String.concat "ab" "cdef")))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3 Int64)))
