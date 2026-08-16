(case "ch2 a Char as HANDLER STATE (the cx-family collection gap does not extend to effect state)"
  (input  (do
            (effect St (op get (-> Unit Char)) (op setb (-> Char Int64)))
            (def (main)
              (handle St #\a
                ((get (u) s (resume s s))
                 (setb (c) s (resume 1 c)))
                (do
                  (def _x (St.setb #\z))
                  (if (= (St.get) #\z) 1 0))))
            (export main)))
  (output (: 1 Int64)))
