(case "sa2 a slice VIEW of the rope state ESCAPES as the resume value and is read in the body"
  (input  (do
            (effect St (op cut (-> Unit String)))
            (def (main (: n Int64))
              (handle St (String.concat "xy" "zw")
                ((cut (u) s (resume (Option.expect (String.slice s 1 3) "in bounds") s)))
                (String.scalar-len (String.concat (St.cut) (St.cut)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 4 Int64)))
