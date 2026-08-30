(example
  (id "map-swap-take-inventory")
  (name "Map swap & take (inventory)")
  (theme "data-and-collections")
  (surface "sexpr")
  (source (do
  (def
    (main)
    (let
      ((stock ((. Map insert) ((. Map insert) ((. Map empty)) 1 5) 2 2)))
      (match
        ((. Map swap) stock 1 9)
        (#tuple(old-apples restocked)
          (match
            ((. Map take) restocked 2)
            (#tuple(gone-pears final) #tuple(old-apples gone-pears ((. Map to-list) final))))))))

  (export main)))
  (expected (: #tuple((Some 5) (Some 2) #list(#tuple(1 9))) (Tuple (Option Int64) (Option Int64) (List (Tuple Int64 Int64))))))
