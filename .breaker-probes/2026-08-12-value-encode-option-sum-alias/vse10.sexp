(case "vse10 finding-22 face: two RESULT fields at different type args — same generic decl, different instantiations"
  (input  (do
            (def (main (: n Int64))
              (record (= first (: (Ok 7) (Result Int64 String)))
                      (= second (: (Err (String.to-bytes "no")) (Result Int64 Bytes)))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: (record (= first (Ok 7)) (= second (Err b"no"))) (record (first (Result Int64 String)) (second (Result Int64 Bytes))))))
