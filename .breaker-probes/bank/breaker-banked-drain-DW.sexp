(case "a sum-record-sum-record alternating nest constructs and projects through four layers"
  (doc    "The ALTERNATING-rep nest: a sum whose payload is a record whose field is a sum whose
           payload is a record — construction lays out sum-tag/record/sum-tag/record in one value,
           and the consumer alternates match (tag dispatch + payload binder) with projection (field
           read) twice each: OW→(. r inner)→IW→(. q v/t) + (. r id) → 729 at n=7. The corpus nests
           same-rep (sum-in-sum, record-in-record) and one level of mixed (:29 record-carrying-sum);
           the four-layer alternation exercises the rep transition BOTH directions twice — a layout
           or descriptor confusion between the tagged and labeled reps at either depth corrupts a
           digit.")
  (input  (do
            (type In (IW (Record (: v Int64) (: t Int64))) (IE))
            (type Out (OW (Record (: inner In) (: id Int64))) (OE))
            (def (main (: n Int64))
              (match (OW (record (inner (IW (record (v n) (t 2)))) (id 9)))
                ((OW r)
                  (match (. r inner)
                    ((IW q) (+ (* 100 (. q v)) (+ (* 10 (. q t)) (. r id))))
                    ((IE) -1)))
                ((OE) -2)))
            (export main)))
  (call   main (: 7 Int64)) (output (: 729 Int64)))
