(case "ie1 escalation: in-range sibling-typed literal in a Map KEY position (the untested face)"
  (input  (Map.len (Map.insert (Map.insert Map.empty (: 5 UInt8) 1) 30 2)))
  (output (: 2 Int64)))
(case "ie2 escalation: MULTIPLE bare literals beside one annotated sibling in a large Set.of"
  (input  (Set.len (Set.of (list (: 1 UInt64) 41 99 255 12))))
  (output (: 5 Int64)))
(case "ie3 escalation: the sibling annotation LAST with bare in-range literals before it"
  (input  (Set.len (Set.of (list 41 99 (: 1 UInt64)))))
  (output (: 3 Int64)))
