(example
  (id "memoized-fibonacci")
  (name "Memoized Fibonacci (Map cache)")
  (theme "data-and-collections")
  (surface "sexpr")
  (source (do
  (def
    (fib n mp)
    (match
      ((. Map lookup) mp n)
      ((Some v) #tuple(v mp))
      ((None)
        (if
          (< n 2)
          #tuple(n ((. Map insert) mp n n))
          (let
            ((a (fib (- n 1) mp)))
            (let
              ((b (fib (- n 2) (. a 1))))
              (let ((r (+ (. a 0) (. b 0)))) #tuple(r ((. Map insert) (. b 1) n r)))))))))

  (def (main) (. (fib 30 ((. Map empty))) 0))

  (export main)))
  (expected 832040))
