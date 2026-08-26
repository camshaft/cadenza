(case "tye1 a `?` unwraps a Result-returning op inside a handled body — Ok path sums, Err path short-circuits"
  (input (do
    (effect E (op fetch (-> (Result Int64 String))))
    (def (get2)
      (Ok (+ (try (E.fetch)) (try (E.fetch)))))
    (def (main (: n Int64))
      (handle E n
        ((fetch () s (resume (if (> s 0) (Ok (* s 10)) (Err "neg")) (+ s 1))))
        (match (get2)
          ((Ok v) v)
          ((Err _) -1))))
    (export main)))
  (call main (: 3 Int64)) (output (: 70 Int64))
  (call main (: 0 Int64)) (output (: -1 Int64)))

(case "tye2 a `?` over an Option-returning op — Some path sums, a None short-circuits past the second dispatch"
  (input (do
    (effect G (op find (-> (Option Int64))))
    (def (sum2)
      (Some (+ (try (G.find)) (try (G.find)))))
    (def (main (: n Int64))
      (handle G n
        ((find () s (resume (if (> s 2) (Option.Some s) Option.None) (+ s 1))))
        (match (sum2)
          ((Option.Some v) v)
          ((Option.None) -1))))
    (export main)))
  (call main (: 3 Int64)) (output (: 7 Int64))
  (call main (: 1 Int64)) (output (: -1 Int64)))
