(case "su1 Set.of sibling-unification range check (-41 into a UInt64-element set)"
  (input  (Set.len (Set.of (list (: 1 UInt64) -41))))
  (error  CDZ0302))
