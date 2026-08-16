(case "sw4 list face of the in-range sibling emit"
  (input  (List.len (list (: 1 UInt64) 41)))
  (output (: 2 Int64)))
(case "sw5 Map value face of the in-range sibling emit"
  (input  (Map.len (Map.insert (Map.insert Map.empty 1 (: 5 UInt8)) 2 30)))
  (output (: 2 Int64)))
