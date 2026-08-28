(case "cmul1 runtime checked-mul in range yields Some"
  (input (do (def (main (: a Int64) (: b Int64)) (Int64.checked-mul a b)) (export main)))
  (call main (: 6 Int64) (: 7 Int64))
  (output (: (Some 42) (Option Int64))))

(case "cmul2 runtime checked-mul just past Int64.max yields None (2^31 x 2^32)"
  (input (do (def (main (: a Int64) (: b Int64)) (Int64.checked-mul a b)) (export main)))
  (call main (: 2147483648 Int64) (: 4294967296 Int64))
  (output (: (None unit) (Option Int64))))

(case "cmul3 runtime checked-mul hitting Int64.min EXACTLY fits (-2^31 x 2^32) — the naive-magnitude-check killer"
  (input (do (def (main (: a Int64) (: b Int64)) (Int64.checked-mul a b)) (export main)))
  (call main (: -2147483648 Int64) (: 4294967296 Int64))
  (output (: (Some -9223372036854775808) (Option Int64))))

(case "cmul4 runtime checked-mul of Int64.min by -1 yields None (the sign-flip overflow)"
  (input (do (def (main (: a Int64) (: b Int64)) (Int64.checked-mul a b)) (export main)))
  (call main (: -9223372036854775808 Int64) (: -1 Int64))
  (output (: (None unit) (Option Int64))))

(case "cmul5 runtime checked-mul of Int64.min by 1 yields Some Int64.min (identity edge)"
  (input (do (def (main (: a Int64) (: b Int64)) (Int64.checked-mul a b)) (export main)))
  (call main (: -9223372036854775808 Int64) (: 1 Int64))
  (output (: (Some -9223372036854775808) (Option Int64))))
