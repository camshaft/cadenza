(case "h2 a closure capturing a RUNTIME String.slice-to-bytes is read correctly on TWO host calls"
  (input  (do
            (def (mk (: k Int64))
              (let ((s (String.concat "abc" "defgh")))
                (match (String.slice s k 6)
                  ((Some t)
                    (let ((b (String.to-bytes t)))
                      (fn ((: i Int64))
                        (match (Bytes.at b i) ((Some v) (Int64.of v)) ((None _u) -1)))))
                  ((None _u) (fn ((: i Int64)) -2)))))
            (export mk)))
  (call   mk (: 2 Int64) (: 0 Int64))
  (output (: 99 Int64)))
