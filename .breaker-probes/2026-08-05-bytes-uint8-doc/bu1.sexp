(case "bu1 Bytes.of accepts LITERAL Int64 list elements via subsumption (the corpus's own pervasive idiom)"
  (input  (Bytes.len (Bytes.of (list 1 2 3))))
  (output (: 3 Int64)))
