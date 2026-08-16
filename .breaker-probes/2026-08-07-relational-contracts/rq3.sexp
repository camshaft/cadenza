(case "rq3 contracts on BOTH ends of a call chain: outer @requires feeds inner @ensures territory"
  (input  (do
            (@ (ensures (> ret 0))
              (def (inner (: x Int64)) (+ x 1)))
            (@ (requires (>= n 0))
              (def (outer (: n Int64)) (inner n)))
            (def (main (: n Int64)) (outer n))
            (export main)))
  (call   main (: 5 Int64)) (output (: 6 Int64))
  (call   main (: -3 Int64)) (trap "unreachable"))
