(case "sh9 CONST odd-width shift overflow folds"
  (input  (Int64.of (<< ((. (Int 24) wrap) 4194304) ((. (Int 24) wrap) 1))))
  (error  CDZ0304))
