(case "an unknown unit name in Unit.of rejects with the unknown-unit diagnostic"
  (doc    "`(Qty.of 3 (Unit.of #\"nosuchunit\"))` — the unit table has no `nosuchunit`, so the
           unknown-unit check rejects CDZ0201 (with its did-you-mean suggestion) rather than letting
           the fault leak to a later stage. Pins the check the 5eb76ba62 bounded-scan perf change
           leans on (its lock-in test guards the SCAN BOUND in-lib; this pins the user-visible reject
           at the corpus tier — a scan bound that accidentally skipped USER nodes would flip this
           while the lib test still passed).")
  (input  (do
            (def (main) (Qty.of 3 (Unit.of #"nosuchunit")))
            (export main)))
  (call   main)
  (error  CDZ0201))
