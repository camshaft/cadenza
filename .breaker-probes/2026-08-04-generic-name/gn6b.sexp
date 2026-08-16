(case "gn6b same shape with built-in Option (control)"
  (input  (do
        (def (main (: k Int64))
          (let (((: b (Option String))) (Some k))
            (match b ((Some _v) 1) ((None _u) -1))))
        (export main)))
  (error  CDZ0301))
