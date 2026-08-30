(example
  (id "symbol-keyed-map")
  (name "Symbols as Map keys")
  (theme "data-and-collections")
  (surface "sexpr")
  (source (do
  (def
    (main)
    (let
      ((palette
          ((. Map insert)
            ((. Map insert)
              ((. Map insert) ((. Map empty)) ((. Symbol of) "red") 16711680)
              ((. Symbol of) "green")
              65280)
            ((. Symbol of) "blue")
            255)))
      #tuple(((. Map lookup) palette ((. Symbol of) "green"))
        ((. Map lookup) palette ((. Symbol of) "teal"))
        (= ((. Symbol of) "red") ((. Symbol of) "red")))))

  (export main)))
  (expected (: #tuple((Some 65280) (None unit) true) (Tuple (Option Int64) (Option Int64) Bool))))
