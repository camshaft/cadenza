(case "a FIVE-payload variant constructs, matches, and binds every position distinctly"
  (doc    "Payload-arity width: the corpus's widest matched variant is three scalar payloads; this one
           carries FIVE positions of MIXED reps (Int64, String, Int64, List, Int64) — construction
           lays out all five (two heap handles interleaved between scalars), and the match binds each
           position to the RIGHT slot: a=n, s=\"ab\", b=n+1, xs 3-long, c=10n, digit-packed to 72835
           at n=7. A payload layout that swapped the interleaved heap/scalar slots (the width-disjoint
           slot trap family), or a binder resolution off-by-one past position 3, corrupts exactly one
           digit — five distinguishable failure signatures in one witness.")
  (input  (do
            (type Wide (W Int64 String Int64 (List Int64) Int64))
            (def (main (: n Int64))
              (match (W n "ab" (+ n 1) (list 9 9 9) (* n 10))
                ((W a s b xs c)
                  (+ (* 10000 a)
                     (+ (* 1000 (String.byte-len s))
                        (+ (* 100 b) (+ (* 10 (List.len xs)) (- c 65))))))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 72835 Int64)))
