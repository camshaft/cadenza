(example
  (id "iterator-pipeline")
  (name "Iterator pipeline (filter → map → fold)")
  (theme "data-and-collections")
  (surface "sexpr")
  (source (do
  (type Iter (Nil unit) (Cons (Tuple Int64 Iter)))

  (def
    (from-list xs)
    (match xs (#list() (Nil unit)) (#list(h .. t) (Cons #tuple(h (from-list t))))))

  (def
    (ifilter it p)
    (match
      it
      ((Nil _) (Nil unit))
      ((Cons c) (if (p (. c 0)) (Cons #tuple((. c 0) (ifilter (. c 1) p))) (ifilter (. c 1) p)))))

  (def
    (imap it f)
    (match it ((Nil _) (Nil unit)) ((Cons c) (Cons #tuple((f (. c 0)) (imap (. c 1) f))))))

  (def (ifold it acc f) (match it ((Nil _) acc) ((Cons c) (ifold (. c 1) (f acc (. c 0)) f))))

  (def
    (main)
    (ifold
      (imap (ifilter (from-list #list(1 2 3 4 5 6)) (fn (x) (= 0 (% x 2)))) (fn (x) (* x 3)))
      0
      (fn (a x) (+ a x))))

  (export main)))
  (expected 36))
