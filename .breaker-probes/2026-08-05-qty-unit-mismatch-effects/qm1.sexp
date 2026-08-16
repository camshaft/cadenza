(case "qm1 unit MISMATCH across the resume rejects CDZ0201 — the unit discipline holds through the effect boundary"
  (input  (do
            (effect St (op read (-> Unit (Qty Int64 (Unit.base #"meter")))))
            (def (main (: n Int64))
              (handle St n
                ((read (u) s (resume (Qty.of n (Unit.base #"second")) s)))
                (Qty.value (St.read))))
            (export main)))
  (error  CDZ0201))
