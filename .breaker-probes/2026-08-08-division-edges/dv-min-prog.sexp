(do
  (effect E (op minv (-> Int64)) (op negone (-> Int64)))
  (def (main (: u Int64))
    (handle E 0
      ((minv () s (resume -9223372036854775808 s))
       (negone () s (resume -1 s)))
      (/ (E.minv) (E.negone))))
  (export main))
