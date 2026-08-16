(case "w8 runtime Int16 wrapping-add at MAX wraps to MIN"
  (input  (do
            (def (main (: x Int16))
              (Int64.of (Int16.wrapping-add x (Int16.wrap 1))))
            (export main)))
  (call   main (: 32767 Int16)) (output (: -32768 Int64)))
