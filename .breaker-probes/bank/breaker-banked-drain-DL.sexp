(case "a nested list pattern in rest position INSIDE a tuple element is the same shape error"
  (doc    "The generalization face of the nested-rest shape reject (PR#1206's Case-6mr co-anchor
           claims coverage for nested contexts): the offending `(list a .. (list b .. r))` sits
           INSIDE a tuple element pattern — the shape check must fire on the inner list pattern
           through the composed-pattern descent, rejecting the same CDZ0201 (not a leaked CDZ0101
           from the tuple walk missing it, and not a silent accept). Uniform ×3 targets.")
  (input  (do
            (def (main (: xs (List Int64)))
              (match (tuple xs 1)
                ((tuple (list a .. (list b .. r)) k) (+ a b))
                (_ -1)))
            (export main)))
  (call   main (list 1 2 3))
  (error  CDZ0201))

(case "a nested list pattern in rest position INSIDE a variant payload is the same shape error"
  (doc    "The variant-payload companion: the malformed rest sits inside a constructor pattern's
           payload position — `(Mk (list a .. (list b .. r)))` — reached through the SumPayload
           binder descent rather than the tuple walk. Same CDZ0201 shape reject, pinning that the
           check runs at every pattern-composition depth (the PR#1206 generalization the fix's doc
           names: nested tuple/variant rest + nested-rest-in-variant-payload).")
  (input  (do
            (type W (Mk (List Int64)))
            (def (main (: xs (List Int64)))
              (match (Mk xs)
                ((Mk (list a .. (list b .. r))) (+ a b))
                (_ -1)))
            (export main)))
  (call   main (list 1 2 3))
  (error  CDZ0201))
