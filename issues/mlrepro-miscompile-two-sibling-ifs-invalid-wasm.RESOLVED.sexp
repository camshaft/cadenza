(do
  (type Ast (Int Int64) (Str String) (Name String) (List (List Ast)))
  (def (read-mag (: b Bytes) (: pos Int64) (: len Int64) (: acc Int64))
    (if (= len 0) (tuple acc pos)
      (read-mag b (+ pos 1) (- len 1) (+ (* acc 256) (Option.expect (Bytes.at b pos) "m")))))
  (def (read-leaf (: b Bytes) (: pos Int64))
    (let ((kind (Option.expect (Bytes.at b pos) "k")))
      (if (or (= kind 7) (= kind 10))
        (tuple (if (= kind 7) ((. Ast Str) "") ((. Ast Name) "")) (+ pos 2))
        (let ((mag (read-mag b (+ pos 1) 1 0)))
          (let ((v (. mag 0)))
            (tuple ((. Ast Int) (if (>= kind 3) (- 0 v) v)) (. mag 1)))))))
  (def (node-count (: x Ast))
    (match x (((. Ast Int) _) 1) (((. Ast Str) _) 1) (((. Ast Name) _) 1) (((. Ast List) es) 99)))
  (def (collect (: b Bytes) (: pos Int64) (: n Int64) (: acc (List Ast)))
    (if (= n 0) acc
      (let ((lp (read-leaf b pos)))
        (collect b (. lp 1) (- n 1) (List.push acc (. lp 0))))))
  (def (main)
    (let ((xs (collect b"\x00\x2a" 0 1 (list))))
      (node-count (Option.expect (List.at xs 0) "0"))))
  (export main))

;; RESOLVED 2026-07-15 (trunk@dd77ccc1b): VERIFIED FIXED — compiles to valid wasm + runs to the correct value (graded via (case) wrapper). The invalid-wasm/mis-typed-projection face is closed.
