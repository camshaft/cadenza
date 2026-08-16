(case "bu4 boundary: a LITERAL out of u8 range (300) in Bytes.of"
  (input  (Bytes.len (Bytes.of (list 300))))
  (declines))
