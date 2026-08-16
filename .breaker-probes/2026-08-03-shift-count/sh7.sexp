(case "sh7 MACHINE-width narrow shift overflow (UInt8: 100<<1 = 200... wait 200 fits; use 200<<1)"
  (input  (do
            (def (main (: k Int64)) (Int64.of (<< ((. (UInt 8) wrap) 200) ((. (UInt 8) wrap) k))))
            (export main)))
  (call   main (: 1 Int64)) (trap "overflow"))
