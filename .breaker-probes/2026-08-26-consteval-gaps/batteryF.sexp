; breaker const-eval sweep 5 — cval_eq compound-equality edges INSIDE const recursion (#3427 was
; structural == over sums/records/tuples; these probe its unswept compositions). CDZ0304 detector.

(case "cf01 equality of records with LIST fields inside const recursion folds"
  (input  (do
            (def (f (const (: n Int64)))
              (if (= (record (= xs (list 1 n))) (record (= xs (list 1 2))))
                  (trap "cf01 records equal")
                  (f (+ n 1))))
            (def (main) (f 1))
            (export main)))
  (error  CDZ0304 (message "cf01 records equal")))

(case "cf02 equality of sum values with BYTES payloads inside const recursion folds"
  (input  (do
            (type Tag (Mk Bytes))
            (def (f (const (: n Int64)))
              (if (= (Tag.Mk (Bytes.concat b"\x01" b"\x02")) (Tag.Mk b"\x01\x02"))
                  (trap "cf02 bytes-payload sums equal")
                  (f (+ n 1))))
            (def (main) (f 1))
            (export main)))
  (error  CDZ0304 (message "cf02 bytes-payload sums equal")))

(case "cf03 equality of deep Option<Record<Tuple>> nests inside const recursion folds"
  (input  (do
            (def (f (const (: n Int64)))
              (if (= (Option.Some (record (= t (tuple n 2)))) (Option.Some (record (= t (tuple 3 2)))))
                  (trap "cf03 deep nests equal")
                  (f (+ n 1))))
            (def (main) (f 1))
            (export main)))
  (error  CDZ0304 (message "cf03 deep nests equal")))

(case "cf04 equality of LISTS of records inside const recursion folds"
  (input  (do
            (def (f (const (: n Int64)))
              (if (= (list (record (= a n)) (record (= a 9))) (list (record (= a 4)) (record (= a 9))))
                  (trap "cf04 record lists equal")
                  (f (+ n 1))))
            (def (main) (f 1))
            (export main)))
  (error  CDZ0304 (message "cf04 record lists equal")))

(case "cf05 INEQUALITY short-circuit: mismatched-variant compare inside const recursion folds false"
  (input  (do
            (def (f (const (: n Int64)))
              (if (= (Option.Some n) (Option.None))
                  (trap "cf05 WRONG (never equal)")
                  (if (= n 3) (trap "cf05 correctly unequal thrice") (f (+ n 1)))))
            (def (main) (f 1))
            (export main)))
  (error  CDZ0304 (message "cf05 correctly unequal thrice")))
