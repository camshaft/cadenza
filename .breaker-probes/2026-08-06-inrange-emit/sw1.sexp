(case "sw1 escalation: the annotated sibling LAST in a 3-element Set.of"
  (input  (Set.len (Set.of (list -41 7 (: 1 UInt64)))))
  (error  CDZ0302))
(case "sw2 escalation: Map KEY position sibling-width"
  (input  (Map.len (Map.insert (Map.insert Map.empty (: 5 UInt8) 1) 300 2)))
  (error  CDZ0302))
(case "sw3 in-range control: sibling-typed literals that FIT still compile"
  (input  (Set.len (Set.of (list (: 1 UInt64) 41))))
  (output (: 2 Int64)))
