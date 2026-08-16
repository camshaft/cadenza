(case "t1 a RUNTIME byte-slice torn mid-scalar is rejected by from-bytes; an aligned slice round-trips"
  (input  (do
            (def (main (: start Int64))
              (let ((b (String.to-bytes (String.concat "aé🎵" "z"))))
                (match (Bytes.slice b start 3)
                  ((Some cut)
                    (match (String.from-bytes cut)
                      ((Some t) (String.byte-len t))
                      ((None _u) -1)))
                  ((None _u) -2))))
            (export main)))
  (call   main (: 2 Int64)) (output (: -1 Int64))
  (call   main (: 3 Int64)) (output (: -1 Int64)))
