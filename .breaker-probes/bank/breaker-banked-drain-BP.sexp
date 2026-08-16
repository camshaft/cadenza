(case "a tuple with a Char leaf declines compound ordering — Char has no blessed order"
  (doc    "`(< (mk #\\a) (mk #\\b))` where `mk` builds a runtime `(tuple 1 c)` with a Char component.
           Compound ordering is offered exactly when EVERY component offers a total order, and Char
           remains outside the blessed leaf vocabulary — scalar `(compare #\\a #\\b)` IS blessed and
           computes (13-strings:3092), but Char-in-a-compound follows the tuple walk, which declines
           rather than inventing an order. (Bytes USED to share this carve-out until PR#1120 blessed
           its lexicographic order — re-verified this pin still declines AFTER that blessing, so the
           Char and Float carve-outs are now the remaining family.) Uniform across backends; flips to
           a witness only if the Char leaf is blessed into the walk.")
  (input  (do
            (def (mk (: c Char)) (tuple 1 c))
            (def (main) (if (< (mk #\a) (mk #\b)) 1 0))
            (export main)))
  (declines))
