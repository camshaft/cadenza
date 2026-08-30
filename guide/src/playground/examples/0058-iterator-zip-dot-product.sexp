(example
  (id "iterator-zip-dot-product")
  (name "Iterator zip: dot product")
  (theme "data-and-collections")
  (surface "sexpr")
  (source (do
  (type Iter (Nil unit) (Cons (Tuple Int64 Iter)))

  (def
    (from-list xs)
    (match xs (#list() (Nil unit)) (#list(h .. t) (Cons #tuple(h (from-list t))))))

  (def
    (dot a b)
    (match
      a
      ((Nil _) 0)
      ((Cons pa)
        (match
          pa
          (#tuple(ha ra)
            (match b ((Nil _) 0) ((Cons pb) (match pb (#tuple(hb rb) (+ (* ha hb) (dot ra rb)))))))))))

  (def (main) (dot (from-list #list(1 2 3)) (from-list #list(4 5 6))))

  (export main)))
  (expected 32))
