(case "w9 runtime Int64 wrapping-add at MAX wraps to MIN"
  (input  (do
            (def (main (: x Int64))
              (Int64.wrapping-add x 1))
            (export main)))
  (call   main (: 9223372036854775807 Int64)) (output (: -9223372036854775808 Int64)))
