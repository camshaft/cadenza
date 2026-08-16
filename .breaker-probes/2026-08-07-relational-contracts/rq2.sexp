(case "rq2 @ensures relating the RESULT to a PARAM (ret longer than the input) over heap values"
  (input  (do
            (@ (ensures (> (List.len ret) (List.len xs)))
              (def (grow (: xs (List Int64)) (: n Int64))
                (if (> n 0) (List.push xs n) xs)))
            (def (main (: n Int64))
              (List.len (grow (list 1 2) n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3 Int64))
  (call   main (: 0 Int64)) (trap "unreachable"))
