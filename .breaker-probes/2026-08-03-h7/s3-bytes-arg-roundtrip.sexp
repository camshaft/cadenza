(case "s3 a runtime-SLICED Bytes arg crosses the host boundary (H7 view rep)"
  (input  (do
            (effect io (op sink (-> Bytes Int64)))
            (def (main (: k Int64))
              (host (io)
                (match (Bytes.slice (String.to-bytes (String.concat "abc" "defgh")) k 3)
                  ((Some cut) (io.sink cut))
                  ((None _u) -1))))
            (export main)))
  (host-responses (respond io.sink (: 99 Int64)))
  (host-calls (call io.sink))
  (call   main (: 2 Int64)) (output (: 99 Int64)))
