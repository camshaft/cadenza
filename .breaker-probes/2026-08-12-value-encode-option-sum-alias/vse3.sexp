(case "vse3 CONTROL: two Option-Bytes fields alone encode intact"
  (input  (do
            (def (main (: n Int64))
              (record (= correlation (: (Some (String.to-bytes "ok")) (Option Bytes)))
                      (= other (: (None unit) (Option Bytes)))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: (record (= correlation (Some b"ok")) (= other (None unit))) (record (correlation (Option Bytes)) (other (Option Bytes))))))
