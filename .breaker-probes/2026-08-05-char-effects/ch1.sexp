(case "ch1 a Char as effect op-arg and resume value (scalar-adjacent rep through the boundary)"
  (input  (do
            (effect Up (op shift (-> Char Char)))
            (def (main)
              (handle Up 0
                ((shift (c) s (resume c s)))
                (if (= (Up.shift #\a) #\a) 1 0)))
            (export main)))
  (output (: 1 Int64)))
