(case "dv3 dv1's exact body with let instead of do/def (isolating the decline shape)"
  (input  (do
        (type Duration (Duration UInt64))
        (type Instant  (Instant  UInt64))
        (def (secs (: n UInt64)) (Duration.Duration (* n 1000000000)))
        (def (inst-ns (: t Instant)) (match t ((Instant.Instant n) n)))
        (def (dur-ns  (: d Duration)) (match d ((Duration.Duration n) n)))
        (def (at (: t Instant) (: d Duration)) (Instant.Instant (+ (inst-ns t) (dur-ns d))))
        (effect Sim (op sleep (-> Duration Unit)) (op now (-> Unit Instant)))
        (def (main (: k Int64))
          (handle Sim (Instant.Instant 0)
            ( (now   (u) s (resume s s))
              (sleep (d) s (resume unit (at s d))) )
            (do
              (Sim.sleep (secs (UInt64.wrap k)))
              (let ((t1 (/ (inst-ns (Sim.now)) (: 1000000000 UInt64))))
                (do
                  (Sim.sleep (secs (* t1 (: 2 UInt64))))
                  (Int64.wrap (/ (inst-ns (Sim.now)) (: 1000000000 UInt64))))))))
        (export main)))
  (call   main (: 3 Int64)) (output (: 9 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
