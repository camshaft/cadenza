(do
  (type W (Atom Int64) (Node (List Int64)))
  (def (one (: b Bytes) (: pos Int64))
    (if (= (Option.expect (Bytes.at b pos) "t") 0)
      (tuple ((. W Atom) (Option.expect (Bytes.at b (+ pos 1)) "v")) (+ pos 2))
      (tuple ((. W Atom) 99) (+ pos 2))))
  (def (wval (: s W)) (match s (((. W Atom) li) li) (((. W Node) ids) 0)))
  ;; NON-tail: the self-call result is consumed by wval (+ 0), so it's not in tail position
  (def (loop (: b Bytes) (: pos Int64) (: n Int64) (: last W))
    (if (= n 0) (wval last)
      (let ((r (one b pos))) (+ 0 (loop b (. r 1) (- n 1) (. r 0))))))
  (def (main (: pos Int64)) (loop b"\x00\x05\x00\x07" pos 1 ((. W Atom) 0)))
  (export main))
