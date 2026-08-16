(do (def (main (: x UInt8)) (Int64.of (+ (UInt8.wrapping-add x (UInt8.wrap 10)) (: 6 UInt8)))) (export main))
