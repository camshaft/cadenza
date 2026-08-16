(case "w3 const Int8 wrapping-mul MIN by -1 wraps to MIN"
  (input  (Int64.of (Int8.wrapping-mul (Int8.wrap -128) (Int8.wrap -1))))
  (output (: -128 Int64)))
