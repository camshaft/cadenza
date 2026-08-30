(example
  (id "data-pipeline")
  (name "Data pipeline over records")
  (theme "data-and-collections")
  (surface "sexpr")
  (source (do
  (def (age r) (. r age))

  (def
    (ages xs i n acc)
    (if
      (= i n)
      acc
      (match
        ((. List at) xs i)
        ((Some r) (ages xs (+ i 1) n ((. List push) acc (age r))))
        ((None) acc))))

  (def
    (sum-age xs i n acc)
    (if
      (= i n)
      acc
      (match ((. List at) xs i) ((Some r) (sum-age xs (+ i 1) n (+ acc (age r)))) ((None) acc))))

  (def
    (main)
    (let
      ((people
          #list(#record((= name "Ada") (= age 36))
            #record((= name "Alan") (= age 41))
            #record((= name "Grace") (= age 40)))))
      #tuple((ages people 0 ((. List len) people) (: #list() (List Int64)))
        (/ (sum-age people 0 ((. List len) people) 0) ((. List len) people)))))

  (export main)))
  (expected (: #tuple(#list(36 41 40) 39) (Tuple (List Int64) Int64))))
