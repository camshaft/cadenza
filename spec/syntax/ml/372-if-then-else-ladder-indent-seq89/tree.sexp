(@
  test
  (def
    (dd-miss-fills-and-returns)
    (comment
      "a demand on an ABSENT slot computes the fact, fills the column, returns the fact."
      (let
        (((tuple db1 fact) (demand-typed (sample-db) 0 (mk-int true 64))))
        (if
          (is-int-ty fact)
          (let
            (((tuple s w) (int-parts-of fact)))
            (if
              (and s (= w 64))
              (if (= (typed-filled db1) 1) unit (trap "filled 1"))
              (trap "Int64")))
          (trap "demand returns the fact"))))))
