(case "wa2 wrapping-add of the op ARG and state in the resume value — wrap, exact-MAX, and identity rows"
  (input  (do
            (effect W (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle W n
                ((add (v) s (resume (Int64.wrapping-add v s) s)))
                (W.add 9223372036854775800)))
            (export main)))
  (call   main (: 10 Int64)) (output (: -9223372036854775806 Int64))
  (call   main (: 7 Int64)) (output (: 9223372036854775807 Int64))
  (call   main (: 0 Int64)) (output (: 9223372036854775800 Int64)))
