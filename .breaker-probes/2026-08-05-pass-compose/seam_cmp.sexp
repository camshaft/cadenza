(do (def (main (: x Int8)) (if (< (Int8.wrapping-add x (Int8.wrap 1)) (: 0 Int8)) 1 0)) (export main))
