(case "cnm1 runtime UInt8 checked-mul at the exact 255 top"
  (input (do
    (def (main (: a Int64) (: b Int64))
      (match (UInt8.checked-mul (UInt8.wrap a) (UInt8.wrap b))
        ((Option.Some v) (if (= v (UInt8.wrap 255)) 2 1))
        ((Option.None) 0)))
    (export main)))
  (call main (: 15 Int64) (: 17 Int64)) (output (: 2 Int64))
  (call main (: 16 Int64) (: 16 Int64)) (output (: 0 Int64)))

(case "cnm2 runtime Int8 checked-mul hitting -128 EXACTLY fits (the signed exact-fit twin)"
  (input (do
    (def (main (: a Int64) (: b Int64))
      (match (Int8.checked-mul (Int8.wrap a) (Int8.wrap b))
        ((Option.Some v) (if (= v (Int8.wrap -128)) 2 1))
        ((Option.None) 0)))
    (export main)))
  (call main (: -8 Int64) (: 16 Int64)) (output (: 2 Int64))
  (call main (: 8 Int64) (: 16 Int64)) (output (: 0 Int64)))

(case "cnm3 runtime Int8 checked-mul of -128 by -1 yields None (the narrow sign-flip)"
  (input (do
    (def (main (: a Int64) (: b Int64))
      (match (Int8.checked-mul (Int8.wrap a) (Int8.wrap b))
        ((Option.Some v) 1)
        ((Option.None) 0)))
    (export main)))
  (call main (: -128 Int64) (: -1 Int64)) (output (: 0 Int64))
  (call main (: -128 Int64) (: 1 Int64)) (output (: 1 Int64)))

(case "cnm4 runtime UInt16 checked-mul at the 65535 top"
  (input (do
    (def (main (: a Int64) (: b Int64))
      (match (UInt16.checked-mul (UInt16.wrap a) (UInt16.wrap b))
        ((Option.Some v) (if (= v (UInt16.wrap 65535)) 2 1))
        ((Option.None) 0)))
    (export main)))
  (call main (: 257 Int64) (: 255 Int64)) (output (: 2 Int64))
  (call main (: 256 Int64) (: 256 Int64)) (output (: 0 Int64)))
