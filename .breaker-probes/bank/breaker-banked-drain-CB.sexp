(case "nominal Instant newtypes order by their UInt64 content including the top-bit boundary"
  (doc    "The DES `Instant` newtype shape (a nominal single-payload sum over UInt64 nanoseconds,
           27-des §3.2) compared through extraction at the SIGN boundary: the payload is UNSIGNED, so
           `Int64.max < 2^63` must be TRUE (an i64-signed compare of the extracted payload says false
           — 2^63 is negative as i64) and `UInt64.max < 1` must be FALSE (signed says true: -1 < 1).
           Small control 5<9 → 110. Pins that a match-extracted UInt64 sum payload keeps its unsigned
           compare through the newtype wrap/unwrap — the timestamp-ordering integrity every DES event
           queue rests on once instants pass 2^63 ns (~292 years, but a wrap-around clock or a
           relative-epoch scheme hits the top bit far earlier).")
  (input  (do
            (type Instant (Instant UInt64))
            (def (mk (: n UInt64)) (Instant.Instant n))
            (def (lt a b)
              (match a ((Instant.Instant x)
                (match b ((Instant.Instant y) (if (< x y) 1 0))))))
            (def (main)
              (+ (* 100 (lt (mk 5) (mk 9)))
                 (+ (* 10 (lt (mk 9223372036854775807) (mk 9223372036854775808)))
                    (lt (mk 18446744073709551615) (mk 1)))))
            (export main)))
  (output (: 110 Int64)))
