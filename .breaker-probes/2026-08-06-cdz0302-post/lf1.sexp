(case "lf1 the fuzzer's list face post-fix"
  (input  (List.len (list (: 1 UInt64) -41)))
  (error  CDZ0302))
(case "lf2 over-max face"
  (input  (List.len (list (: 1 UInt8) 300)))
  (error  CDZ0302))
