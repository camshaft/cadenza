(case "su3 tuple-position UNIFIED literal (control: tuples don't unify positions — should typecheck independently)"
  (input  (. (tuple (: 1 UInt64) -41) 1))
  (output (: -41 Int64)))
