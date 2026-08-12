(case "vse2 CONTROL: the Option-sum field alone (no Option-Bytes sibling) encodes intact"
  (input  (do
            (type P (A Bytes) (B (Record (: x String))))
            (def (main (: n Int64))
              (record (= payload (Some (P.B (record (= x "hi")))))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: (record (= payload (Some (B (record (= x "hi")))))) (record (payload (Option P))))))
