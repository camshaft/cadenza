(case "g3 bare-binder guard over a string param, guard reads a SCALAR compare"
  (input  (do
            (def (band (: s String))
              (match s ((guard t (= (String.byte-len t) 5)) 1) (_ 3)))
            (def (main (: k Int64)) (band "apple"))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
