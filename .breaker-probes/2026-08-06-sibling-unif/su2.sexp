(case "su2 Map value sibling-unification range check"
  (input  (Map.len (Map.insert (Map.insert Map.empty 1 (: 5 UInt8)) 2 300)))
  (error  CDZ0302))
