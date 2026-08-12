(case "vse11 finding-22 face: two LIST fields of different element types"
  (input  (do
            (def (main (: n Int64))
              (record (= nums (list 1 2))
                      (= tags (list "a" "b"))))
            (export main)))
  (call   main (: 1 Int64))
  (output (: (record (= nums (list 1 2)) (= tags (list "a" "b"))) (record (nums (List Int64)) (tags (List String))))))
