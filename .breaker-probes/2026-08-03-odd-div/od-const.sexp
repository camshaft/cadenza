(case "odx7 CONST Int24 min / -1 (fold path)"
  (input  (Int64.of (/ ((. (Int 24) wrap) -8388608) ((. (Int 24) wrap) -1))))
  (error  CDZ0304))
